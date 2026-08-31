//! Parse a pasted `curl …` command line into a [`RequestData`].
//!
//! Unknown options are rejected instead of guessed: silently skipping an
//! option that takes a value could otherwise make that value become the
//! imported request URL. A backslash followed by whitespace is treated as a
//! token break (a multi-line command pasted into the single-line URL input
//! arrives with `\<newline>` flattened to `\<space>`; POSIX "escaped space"
//! semantics would corrupt it).

use std::fmt;

use crate::types::{AuthConfig, AuthType, BodyType, FormDataRow, FormDataValue, HttpMethod, RawSubtype, RequestData};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurlImportError {
    UnsupportedOption(String),
    MissingOptionValue(String),
    InvalidOptionValue { option: String, expected: &'static str },
    EmptyUrl,
    MultipleUrls,
}

impl fmt::Display for CurlImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOption(option) => write!(
                f,
                "Unsupported cURL option `{option}`. The command was not imported because this option's arguments or request semantics are unknown."
            ),
            Self::MissingOptionValue(option) => {
                write!(f, "cURL option `{option}` requires a value. The command was not imported.")
            }
            Self::InvalidOptionValue { option, expected } => write!(
                f,
                "cURL option `{option}` has an invalid value; expected {expected}. The command was not imported."
            ),
            Self::EmptyUrl => write!(f, "The cURL command contains an empty URL and was not imported."),
            Self::MultipleUrls => write!(
                f,
                "The cURL command contains more than one URL. Import one request at a time."
            ),
        }
    }
}

impl std::error::Error for CurlImportError {}

/// Shell-style tokenizer. Single quotes take content verbatim; double quotes
/// honor `\"` and `\\`; outside quotes a backslash escapes the next char,
/// except before whitespace where it is a token break (see module docs).
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                has_token = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    current.push(q);
                }
            }
            '"' => {
                has_token = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(&next) = chars.peek()
                                && (next == '"' || next == '\\')
                            {
                                current.push(next);
                                chars.next();
                            } else {
                                current.push('\\');
                            }
                        }
                        _ => current.push(q),
                    }
                }
            }
            '\\' => {
                match chars.peek() {
                    // Line continuation / flattened continuation: token break.
                    Some(&next) if next.is_whitespace() && has_token => {
                        tokens.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                    // Continuation with nothing pending: skip it.
                    Some(&next) if next.is_whitespace() => {}
                    Some(&next) => {
                        current.push(next);
                        has_token = true;
                        chars.next();
                    }
                    None => {}
                }
            }
            c if c.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            _ => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        tokens.push(current);
    }
    tokens
}

/// Extract a flag's value: attached (`-XPOST`), separate (`-X POST`), or
/// `--request=POST`. Advances `i` past a separate value.
fn flag_value(
    tokens: &[String],
    i: &mut usize,
    short: &str,
    long: &str,
) -> Result<String, CurlImportError> {
    let tok = tokens[*i].clone();
    if let Some(rest) = tok.strip_prefix(long) {
        if rest.is_empty() {
            *i += 1;
            return tokens
                .get(*i)
                .cloned()
                .ok_or(CurlImportError::MissingOptionValue(tok));
        }
        if let Some(v) = rest.strip_prefix('=') {
            return Ok(v.to_string());
        }
        unreachable!("flag_value called only after matches_flag");
    }
    if !short.is_empty()
        && let Some(rest) = tok.strip_prefix(short)
    {
        if rest.is_empty() {
            *i += 1;
            return tokens
                .get(*i)
                .cloned()
                .ok_or(CurlImportError::MissingOptionValue(tok));
        }
        return Ok(rest.to_string()); // attached: -XPOST
    }
    unreachable!("flag_value called only after matches_flag");
}

/// Does this token invoke the given flag (exact, attached, or `=` form)?
fn matches_flag(tok: &str, short: &str, long: &str) -> bool {
    tok == short
        || tok == long
        || (!short.is_empty() && tok.starts_with(short) && tok.len() > short.len() && !tok.starts_with(long))
        || tok.starts_with(&format!("{}=", long))
}

/// Options whose effects are either presentation-only or already match the
/// HTTP client's behavior. Keep this list explicit: unknown option arity must
/// never be guessed.
fn is_safe_valueless_option(tok: &str) -> bool {
    matches!(
        tok,
        "-s" | "--silent" | "-S" | "--show-error" | "-L" | "--location" | "--compressed"
    )
}

fn set_url(url: &mut Option<String>, value: String) -> Result<(), CurlImportError> {
    if value.is_empty() {
        return Err(CurlImportError::EmptyUrl);
    }
    if url.is_some() {
        return Err(CurlImportError::MultipleUrls);
    }
    *url = Some(value);
    Ok(())
}

/// Parse one cURL command.
///
/// `Ok(None)` means the input is not yet an importable cURL command (it is not
/// `curl`, or it has no URL). Unsafe, unsupported, or ambiguous commands return
/// a visible error to the caller instead of a partially imported request.
pub fn parse_curl(input: &str) -> Result<Option<RequestData>, CurlImportError> {
    let tokens = tokenize(input);
    if tokens.first().map(String::as_str) != Some("curl") {
        return Ok(None);
    }

    let mut method: Option<HttpMethod> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data_parts: Vec<String> = Vec::new();
    let mut form_rows: Vec<FormDataRow> = Vec::new();
    let mut auth = AuthConfig::default();
    let mut end_of_options = false;

    let mut i = 1;
    while i < tokens.len() {
        let tok = tokens[i].clone();
        if end_of_options {
            set_url(&mut url, tok)?;
        } else if tok == "--" {
            end_of_options = true;
        } else if matches_flag(&tok, "-X", "--request") {
            let v = flag_value(&tokens, &mut i, "-X", "--request")?;
            method = Some(HttpMethod::from_str(&v).ok_or_else(|| {
                CurlImportError::InvalidOptionValue {
                    option: tok.clone(),
                    expected: "a supported HTTP method",
                }
            })?);
        } else if matches_flag(&tok, "-H", "--header") {
            let v = flag_value(&tokens, &mut i, "-H", "--header")?;
            let (k, val) = v.split_once(':').ok_or_else(|| {
                CurlImportError::InvalidOptionValue {
                    option: tok.clone(),
                    expected: "a `Name: value` header",
                }
            })?;
            if k.trim().is_empty() {
                return Err(CurlImportError::InvalidOptionValue {
                    option: tok,
                    expected: "a `Name: value` header",
                });
            }
            headers.push((k.trim().to_string(), val.trim().to_string()));
        } else if matches_flag(&tok, "", "--data-raw")
            || matches_flag(&tok, "", "--data-binary")
            || matches_flag(&tok, "", "--data-urlencode")
        {
            let long = if tok.starts_with("--data-raw") {
                "--data-raw"
            } else if tok.starts_with("--data-binary") {
                "--data-binary"
            } else {
                "--data-urlencode"
            };
            data_parts.push(flag_value(&tokens, &mut i, "", long)?);
        } else if matches_flag(&tok, "-d", "--data") {
            data_parts.push(flag_value(&tokens, &mut i, "-d", "--data")?);
        } else if matches_flag(&tok, "-F", "--form") {
            let v = flag_value(&tokens, &mut i, "-F", "--form")?;
            let (k, val) = v.split_once('=').ok_or_else(|| {
                CurlImportError::InvalidOptionValue {
                    option: tok.clone(),
                    expected: "a `name=value` form field",
                }
            })?;
            if k.is_empty() {
                return Err(CurlImportError::InvalidOptionValue {
                    option: tok,
                    expected: "a `name=value` form field",
                });
            }
            let value = match val.strip_prefix('@') {
                Some(path) => FormDataValue::File { path: path.to_string() },
                None => FormDataValue::Text(val.to_string()),
            };
            form_rows.push(FormDataRow { enabled: true, key: k.to_string(), value });
        } else if matches_flag(&tok, "-b", "--cookie") {
            let v = flag_value(&tokens, &mut i, "-b", "--cookie")?;
            // curl reads the -b argument as cookie data when it contains '=',
            // otherwise as a cookie-jar filename (which we cannot load).
            // Browser "Copy as cURL" always emits cookie data as a -b string.
            if v.contains('=') {
                headers.push(("Cookie".to_string(), v));
            }
        } else if matches_flag(&tok, "-u", "--user") {
            let v = flag_value(&tokens, &mut i, "-u", "--user")?;
            // Split on the first ':' into user/pass; a value with no ':' is a
            // username with an empty password (curl then prompts, we don't).
            let (user, pass) = match v.split_once(':') {
                Some((u, p)) => (u.to_string(), p.to_string()),
                None => (v, String::new()),
            };
            auth = AuthConfig {
                auth_type: AuthType::Basic,
                basic_username: user,
                basic_password: pass,
                ..AuthConfig::default()
            };
        } else if matches_flag(&tok, "", "--url") {
            let v = flag_value(&tokens, &mut i, "", "--url")?;
            set_url(&mut url, v)?;
        } else if is_safe_valueless_option(&tok) {
            // Explicitly supported no-op; do not generalize this branch.
        } else if tok.starts_with('-') {
            let option = tok.split_once('=').map_or(tok.as_str(), |(name, _)| name);
            return Err(CurlImportError::UnsupportedOption(option.to_string()));
        } else {
            set_url(&mut url, tok)?;
        }
        i += 1;
    }

    let Some(url) = url else {
        return Ok(None);
    };

    let body = if !form_rows.is_empty() {
        BodyType::FormData(form_rows)
    } else if !data_parts.is_empty() {
        let content_type = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.to_ascii_lowercase())
            .unwrap_or_default();
        let subtype = if content_type.contains("json") {
            RawSubtype::Json
        } else if content_type.contains("xml") {
            RawSubtype::Xml
        } else if content_type.contains("javascript") {
            RawSubtype::JavaScript
        } else {
            RawSubtype::Text
        };
        BodyType::Raw { content: data_parts.join("&"), subtype }
    } else {
        BodyType::None
    };

    let method = method.unwrap_or(if matches!(body, BodyType::None) {
        HttpMethod::GET
    } else {
        HttpMethod::POST
    });

    Ok(Some(RequestData { method, url, headers, body, auth }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, FormDataValue, HttpMethod, RawSubtype};

    fn parse(s: &str) -> RequestData {
        parse_curl(s)
            .expect("command should be safe to import")
            .expect("command should contain a request")
    }

    fn parse_error(s: &str) -> CurlImportError {
        parse_curl(s).expect_err("command should be rejected")
    }

    #[test]
    fn simple_get() {
        let r = parse("curl https://example.com/api");
        assert_eq!(r.method, HttpMethod::GET);
        assert_eq!(r.url, "https://example.com/api");
        assert!(r.headers.is_empty());
        assert!(matches!(r.body, BodyType::None));
    }

    #[test]
    fn non_curl_input_is_rejected() {
        assert!(parse_curl("wget https://example.com").unwrap().is_none());
        assert!(parse_curl("").unwrap().is_none());
        assert!(parse_curl("curl").unwrap().is_none()); // no URL
        assert!(parse_curl("https://example.com").unwrap().is_none());
    }

    #[test]
    fn single_quotes_preserve_content() {
        let r = parse("curl 'https://example.com/a b?x=1&y=2'");
        assert_eq!(r.url, "https://example.com/a b?x=1&y=2");
    }

    #[test]
    fn double_quotes_with_escapes() {
        let r = parse(r#"curl -H "X-Note: say \"hi\"" https://example.com"#);
        assert_eq!(r.headers, vec![("X-Note".to_string(), r#"say "hi""#.to_string())]);
    }

    #[test]
    fn explicit_method_flag() {
        assert_eq!(parse("curl -X PUT https://example.com").method, HttpMethod::PUT);
        assert_eq!(parse("curl --request DELETE https://example.com").method, HttpMethod::DELETE);
    }

    #[test]
    fn attached_and_equals_forms() {
        assert_eq!(parse("curl -XPOST https://example.com").method, HttpMethod::POST);
        assert_eq!(parse("curl --request=PATCH https://example.com").method, HttpMethod::PATCH);
    }

    #[test]
    fn headers_split_at_first_colon_and_trim() {
        let r = parse("curl -H 'X-Time: 12:30:00' https://example.com");
        assert_eq!(r.headers, vec![("X-Time".to_string(), "12:30:00".to_string())]);
    }

    #[test]
    fn multiple_headers_keep_order() {
        let r = parse("curl -H 'A: 1' -H 'B: 2' https://example.com");
        assert_eq!(
            r.headers,
            vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]
        );
    }

    #[test]
    fn data_implies_post_and_json_subtype_from_header() {
        let r = parse(
            "curl -H 'Content-Type: application/json' -d '{\"a\":1}' https://example.com",
        );
        assert_eq!(r.method, HttpMethod::POST);
        match r.body {
            BodyType::Raw { content, subtype } => {
                assert_eq!(content, "{\"a\":1}");
                assert_eq!(subtype, RawSubtype::Json);
            }
            other => panic!("expected raw body, got {:?}", other),
        }
    }

    #[test]
    fn explicit_method_wins_over_data_implied_post() {
        let r = parse("curl -X PUT -d 'x=1' https://example.com");
        assert_eq!(r.method, HttpMethod::PUT);
    }

    #[test]
    fn multiple_data_parts_join_with_ampersand() {
        let r = parse("curl -d a=1 -d b=2 https://example.com");
        match r.body {
            BodyType::Raw { content, subtype } => {
                assert_eq!(content, "a=1&b=2");
                assert_eq!(subtype, RawSubtype::Text);
            }
            other => panic!("expected raw body, got {:?}", other),
        }
    }

    #[test]
    fn form_fields_text_and_file() {
        let r = parse("curl -F name=alice -F avatar=@/tmp/a.png https://example.com");
        assert_eq!(r.method, HttpMethod::POST);
        match r.body {
            BodyType::FormData(rows) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].key, "name");
                assert!(matches!(&rows[0].value, FormDataValue::Text(t) if t == "alice"));
                assert_eq!(rows[1].key, "avatar");
                assert!(matches!(&rows[1].value, FormDataValue::File { path } if path == "/tmp/a.png"));
                assert!(rows.iter().all(|row| row.enabled));
            }
            other => panic!("expected form body, got {:?}", other),
        }
    }

    #[test]
    fn user_flag_becomes_basic_auth_config() {
        let r = parse("curl -u user:pass https://example.com");
        assert_eq!(r.auth.auth_type, crate::types::AuthType::Basic);
        assert_eq!(r.auth.basic_username, "user");
        assert_eq!(r.auth.basic_password, "pass");
        // No Authorization header is synthesized — the config computes it at send time.
        assert!(r.headers.iter().all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
    }

    #[test]
    fn user_flag_long_and_attached_forms() {
        assert_eq!(parse("curl --user u:p https://example.com").auth.basic_username, "u");
        assert_eq!(parse("curl --user=u:p https://example.com").auth.basic_password, "p");
        let r = parse("curl -uadmin:s3cret https://example.com");
        assert_eq!(r.auth.basic_username, "admin");
        assert_eq!(r.auth.basic_password, "s3cret");
    }

    #[test]
    fn user_flag_without_colon_is_username_only() {
        let r = parse("curl -u alice https://example.com");
        assert_eq!(r.auth.auth_type, crate::types::AuthType::Basic);
        assert_eq!(r.auth.basic_username, "alice");
        assert_eq!(r.auth.basic_password, "");
    }

    #[test]
    fn url_flag_supports_separate_and_equals_forms() {
        assert_eq!(parse("curl --url https://a.example").url, "https://a.example");
        assert_eq!(parse("curl --url=https://b.example").url, "https://b.example");
    }

    #[test]
    fn multiple_urls_are_rejected_as_ambiguous() {
        assert_eq!(
            parse_error("curl https://first.example https://second.example"),
            CurlImportError::MultipleUrls
        );
        assert_eq!(
            parse_error("curl https://first.example --url https://second.example"),
            CurlImportError::MultipleUrls
        );
    }

    #[test]
    fn line_continuations_and_flattened_backslashes() {
        let cmd = "curl -X POST \\\n  -H 'A: 1' \\\n  https://example.com";
        let r = parse(cmd);
        assert_eq!(r.method, HttpMethod::POST);
        assert_eq!(r.url, "https://example.com");
        // Same command flattened to one line (single-line input paste).
        let flat = "curl -X POST \\ -H 'A: 1' \\ https://example.com";
        let r = parse(flat);
        assert_eq!(r.method, HttpMethod::POST);
        assert_eq!(r.url, "https://example.com");
    }

    #[test]
    fn cookie_as_header_is_kept() {
        // Cookie passed the -H way still works (regression guard).
        let r = parse("curl 'https://example.com/' -H 'Cookie: sid=abc123; keepLogin=true'");
        assert_eq!(
            r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("cookie")),
            Some(&("Cookie".to_string(), "sid=abc123; keepLogin=true".to_string()))
        );
    }

    #[test]
    fn cookie_flag_becomes_cookie_header() {
        // Browser DevTools "Copy as cURL (bash)" emits cookies via -b, not -H.
        // The $ characters mirror real GA cookie values inside single quotes.
        let value = "sid=abc123; _ga=GA1.1.x$o2$g1$t123; keepLogin=true";
        let r = parse(&format!("curl 'https://example.com/' -b '{value}'"));
        assert_eq!(
            r.headers,
            vec![("Cookie".to_string(), value.to_string())],
            "parsed headers = {:?}",
            r.headers
        );
    }

    #[test]
    fn cookie_long_and_attached_forms() {
        // --cookie=..., --cookie <v>, and attached -b<v> all land as a Cookie header.
        assert_eq!(
            parse("curl --cookie 'a=1' https://example.com").headers,
            vec![("Cookie".to_string(), "a=1".to_string())]
        );
        assert_eq!(
            parse("curl --cookie=a=1 https://example.com").headers,
            vec![("Cookie".to_string(), "a=1".to_string())]
        );
        assert_eq!(
            parse("curl -ba=1 https://example.com").headers,
            vec![("Cookie".to_string(), "a=1".to_string())]
        );
    }

    #[test]
    fn cookie_flag_before_url_does_not_hijack_the_url() {
        // -b consumes its own value, so the cookie string is not mistaken for the URL.
        let r = parse("curl -b 'sid=abc; k=v' https://example.com/api");
        assert_eq!(r.url, "https://example.com/api");
        assert_eq!(r.headers, vec![("Cookie".to_string(), "sid=abc; k=v".to_string())]);
    }

    #[test]
    fn cookie_jar_filename_is_not_treated_as_data() {
        // A -b argument without '=' is a cookie-jar filename in curl; we can't load
        // it, so it must not become a bogus Cookie header.
        let r = parse("curl -b cookies.txt https://example.com");
        assert!(r.headers.is_empty(), "headers = {:?}", r.headers);
        assert_eq!(r.url, "https://example.com");
    }

    #[test]
    fn safe_valueless_options_are_ignored_explicitly() {
        let r = parse("curl -s -L --compressed https://example.com");
        assert_eq!(r.url, "https://example.com");
        assert_eq!(r.method, HttpMethod::GET);
        assert_eq!(
            parse("curl --silent --show-error --location https://example.com").url,
            "https://example.com"
        );
    }

    #[test]
    fn proxy_option_cannot_redirect_a_credential_bearing_import() {
        assert_eq!(
            parse_error(
                "curl --proxy http://proxy.example -H 'Authorization: Bearer secret' https://api.example"
            ),
            CurlImportError::UnsupportedOption("--proxy".to_string())
        );
        assert_eq!(
            parse_error(
                "curl --proxy http://proxy.example -H 'Authorization: Bearer secret' --url https://api.example"
            ),
            CurlImportError::UnsupportedOption("--proxy".to_string())
        );
    }

    #[test]
    fn output_option_operand_cannot_become_the_url() {
        assert_eq!(
            parse_error("curl -o out.json https://api.example"),
            CurlImportError::UnsupportedOption("-o".to_string())
        );
    }

    #[test]
    fn unknown_options_are_rejected_in_all_forms() {
        assert_eq!(
            parse_error("curl --future-option value https://api.example"),
            CurlImportError::UnsupportedOption("--future-option".to_string())
        );
        assert_eq!(
            parse_error("curl --future-option=value https://api.example"),
            CurlImportError::UnsupportedOption("--future-option".to_string())
        );
        assert_eq!(
            parse_error("curl --future-flag https://api.example"),
            CurlImportError::UnsupportedOption("--future-flag".to_string())
        );
    }

    #[test]
    fn supported_options_with_missing_values_are_rejected() {
        assert_eq!(
            parse_error("curl https://example.com -H"),
            CurlImportError::MissingOptionValue("-H".to_string())
        );
        assert_eq!(
            parse_error("curl --url"),
            CurlImportError::MissingOptionValue("--url".to_string())
        );
    }
}
