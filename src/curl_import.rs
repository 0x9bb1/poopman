//! Parse a pasted `curl …` command line into a [`RequestData`].
//!
//! Deliberately lenient: unknown flags are skipped, and a backslash followed
//! by whitespace is treated as a token break (a multi-line command pasted
//! into the single-line URL input arrives with `\<newline>` flattened to
//! `\<space>`; POSIX "escaped space" semantics would corrupt it).

use crate::types::{AuthConfig, AuthType, BodyType, FormDataRow, FormDataValue, HttpMethod, RawSubtype, RequestData};

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
fn flag_value(tokens: &[String], i: &mut usize, short: &str, long: &str) -> Option<String> {
    let tok = &tokens[*i];
    if let Some(rest) = tok.strip_prefix(long) {
        if rest.is_empty() {
            *i += 1;
            return tokens.get(*i).cloned();
        }
        if let Some(v) = rest.strip_prefix('=') {
            return Some(v.to_string());
        }
        return None; // e.g. "--headers" is not "--header"
    }
    if !short.is_empty()
        && let Some(rest) = tok.strip_prefix(short)
    {
        if rest.is_empty() {
            *i += 1;
            return tokens.get(*i).cloned();
        }
        return Some(rest.to_string()); // attached: -XPOST
    }
    None
}

/// Does this token invoke the given flag (exact, attached, or `=` form)?
fn matches_flag(tok: &str, short: &str, long: &str) -> bool {
    tok == short
        || tok == long
        || (!short.is_empty() && tok.starts_with(short) && tok.len() > short.len() && !tok.starts_with(long))
        || tok.starts_with(&format!("{}=", long))
}

pub fn parse_curl(input: &str) -> Option<RequestData> {
    let tokens = tokenize(input);
    if tokens.first().map(String::as_str) != Some("curl") {
        return None;
    }

    let mut method: Option<HttpMethod> = None;
    let mut url = String::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data_parts: Vec<String> = Vec::new();
    let mut form_rows: Vec<FormDataRow> = Vec::new();
    let mut auth = AuthConfig::default();

    let mut i = 1;
    while i < tokens.len() {
        let tok = tokens[i].clone();
        if matches_flag(&tok, "-X", "--request") {
            if let Some(v) = flag_value(&tokens, &mut i, "-X", "--request")
                && let Some(m) = HttpMethod::from_str(&v)
            {
                method = Some(m);
            }
        } else if matches_flag(&tok, "-H", "--header") {
            if let Some(v) = flag_value(&tokens, &mut i, "-H", "--header")
                && let Some((k, val)) = v.split_once(':')
            {
                headers.push((k.trim().to_string(), val.trim().to_string()));
            }
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
            if let Some(v) = flag_value(&tokens, &mut i, "", long) {
                data_parts.push(v);
            }
        } else if matches_flag(&tok, "-d", "--data") {
            if let Some(v) = flag_value(&tokens, &mut i, "-d", "--data") {
                data_parts.push(v);
            }
        } else if matches_flag(&tok, "-F", "--form") {
            if let Some(v) = flag_value(&tokens, &mut i, "-F", "--form")
                && let Some((k, val)) = v.split_once('=')
            {
                let value = match val.strip_prefix('@') {
                    Some(path) => FormDataValue::File { path: path.to_string() },
                    None => FormDataValue::Text(val.to_string()),
                };
                form_rows.push(FormDataRow { enabled: true, key: k.to_string(), value });
            }
        } else if matches_flag(&tok, "-b", "--cookie") {
            if let Some(v) = flag_value(&tokens, &mut i, "-b", "--cookie") {
                // curl reads the -b argument as cookie data when it contains '=',
                // otherwise as a cookie-jar filename (which we cannot load).
                // Browser "Copy as cURL" always emits cookie data as a -b string.
                if v.contains('=') {
                    headers.push(("Cookie".to_string(), v));
                }
            }
        } else if matches_flag(&tok, "-u", "--user") {
            if let Some(v) = flag_value(&tokens, &mut i, "-u", "--user") {
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
            }
        } else if matches_flag(&tok, "", "--url") {
            if let Some(v) = flag_value(&tokens, &mut i, "", "--url")
                && url.is_empty()
            {
                url = v;
            }
        } else if !tok.starts_with('-') && url.is_empty() {
            url = tok;
        }
        // Unknown flags fall through and are skipped.
        i += 1;
    }

    if url.is_empty() {
        return None;
    }

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

    Some(RequestData { method, url, headers, body, auth })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BodyType, FormDataValue, HttpMethod, RawSubtype};

    fn parse(s: &str) -> RequestData {
        parse_curl(s).expect("should parse")
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
        assert!(parse_curl("wget https://example.com").is_none());
        assert!(parse_curl("").is_none());
        assert!(parse_curl("curl").is_none()); // no URL
        assert!(parse_curl("https://example.com").is_none());
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
    fn url_flag_and_bare_url_first_wins() {
        assert_eq!(parse("curl --url https://a.example").url, "https://a.example");
        let r = parse("curl https://first.example https://second.example");
        assert_eq!(r.url, "https://first.example");
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
    fn unknown_flags_are_skipped() {
        let r = parse("curl -s -L --compressed https://example.com");
        assert_eq!(r.url, "https://example.com");
        assert_eq!(r.method, HttpMethod::GET);
    }
}
