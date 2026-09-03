//! Pure functions for URL and query parameter handling.
//!
//! This module contains stateless, side-effect-free functions for parsing and building URLs
//! with query parameters. These functions are designed to be easily testable.

use std::borrow::Cow;

use url::Url;

/// Represents a query parameter with its enabled state.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryParam {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

impl QueryParam {
    pub fn new(key: impl Into<String>, value: impl Into<String>, enabled: bool) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled,
        }
    }
}

/// Syntactic URL pieces used while the user may still be typing an incomplete
/// URL. Splitting does not require the input to be a valid absolute URL.
struct UrlParts<'a> {
    base: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split_url(url: &str) -> UrlParts<'_> {
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (url, None),
    };
    let (base, query) = match before_fragment.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (before_fragment, None),
    };

    UrlParts {
        base,
        query,
        fragment,
    }
}

/// Extract the URL without its query string while preserving any fragment.
///
/// # Examples
/// ```
/// assert_eq!(extract_base_url("https://example.com/api?foo=bar"), "https://example.com/api");
/// assert_eq!(extract_base_url("https://example.com/api?foo=bar#result"), "https://example.com/api#result");
/// assert_eq!(extract_base_url("https://example.com/api"), "https://example.com/api");
/// assert_eq!(extract_base_url(""), "");
/// ```
pub fn extract_base_url(url: &str) -> String {
    let parts = split_url(url);
    match parts.fragment {
        Some(fragment) => format!("{}#{fragment}", parts.base),
        None => parts.base.to_string(),
    }
}

/// Parse query parameters from a URL string.
///
/// Returns a list of (key, value) pairs. All returned params are considered "enabled".
/// Returns an empty Vec if:
/// - URL is empty
/// - URL has no query string
///
/// Parsing is deliberately syntactic so partial/template URLs behave exactly
/// like complete absolute URLs while they are edited.
///
/// # Examples
/// ```
/// let params = parse_query_params("https://example.com?foo=bar&baz=qux");
/// assert_eq!(params, vec![("foo".to_string(), "bar".to_string()), ("baz".to_string(), "qux".to_string())]);
/// ```
pub fn parse_query_params(url: &str) -> Vec<(String, String)> {
    split_url(url)
        .query
        .map(parse_query_string)
        .unwrap_or_default()
}

fn parse_query_string(query: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// Build a URL by replacing its query parameters while preserving its
/// fragment. Template tokens stay literal so send-time substitution can still
/// recognize them; every other key/value byte is percent-encoded.
///
/// Only enabled params with non-empty keys are included in the query string.
/// Keys and values are URL-encoded.
///
/// # Arguments
/// * `url` - A complete or partial URL; its existing query is replaced
/// * `params` - List of query parameters with enabled state
///
/// # Examples
/// ```
/// let params = vec![
///     QueryParam::new("foo", "bar", true),
///     QueryParam::new("disabled", "value", false),
///     QueryParam::new("baz", "qux", true),
/// ];
/// let url = build_url_with_params("https://example.com/api", &params);
/// assert_eq!(url, "https://example.com/api?foo=bar&baz=qux");
/// ```
pub fn build_url_with_params(url: &str, params: &[QueryParam]) -> String {
    let parts = split_url(url);
    build_url_parts(parts.base, parts.fragment, params, true)
}

fn build_url_parts(
    base: &str,
    fragment: Option<&str>,
    params: &[QueryParam],
    preserve_templates: bool,
) -> String {
    let param_parts: Vec<String> = params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
        .map(|p| {
            format!(
                "{}={}",
                encode_query_component(&p.key, preserve_templates),
                encode_query_component(&p.value, preserve_templates)
            )
        })
        .collect();

    let mut result = base.to_string();
    if !param_parts.is_empty() {
        result.push('?');
        result.push_str(&param_parts.join("&"));
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    result
}

fn encode_query_component(value: &str, preserve_templates: bool) -> String {
    if !preserve_templates {
        return urlencoding::encode(value).into_owned();
    }

    let mut encoded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find("{{") {
        encoded.push_str(urlencoding::encode(&rest[..open]).as_ref());
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            encoded.push_str(urlencoding::encode(&rest[open..]).as_ref());
            return encoded;
        };
        let token_end = open + 2 + close + 2;
        encoded.push_str(&rest[open..token_end]);
        rest = &rest[token_end..];
    }
    encoded.push_str(urlencoding::encode(rest).as_ref());
    encoded
}

/// Resolve all URL template-bearing pieces and encode the resulting query
/// keys/values exactly once. An input without a literal query is resolved as a
/// whole so a `{{base_url}}` variable may itself supply a complete URL.
pub fn resolve_url_for_send(url: &str, mut resolve: impl FnMut(&str) -> String) -> String {
    let parts = split_url(url);
    let Some(query) = parts.query else {
        return resolve(url);
    };

    let base = resolve(parts.base);
    let fragment = parts.fragment.map(&mut resolve);
    let params = parse_query_string(query)
        .into_iter()
        .map(|(key, value)| QueryParam::new(resolve(&key), resolve(&value), true))
        .collect::<Vec<_>>();

    build_url_parts(&base, fragment.as_deref(), &params, false)
}

/// Add the default HTTP scheme when absent, then parse and normalize with the
/// URL library. Only HTTP(S) URLs with a host are accepted.
pub fn normalize_http_url(url: &str) -> Option<Url> {
    let candidate = if has_explicit_scheme(url) {
        Cow::Borrowed(url)
    } else {
        Cow::Owned(format!("http://{url}"))
    };
    let parsed = Url::parse(&candidate).ok()?;
    (matches!(parsed.scheme(), "http" | "https") && parsed.has_host()).then_some(parsed)
}

fn has_explicit_scheme(url: &str) -> bool {
    let Some(separator) = url.find("://") else {
        return false;
    };
    let scheme = &url[..separator];
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

/// Compare two lists of query parameters (ignoring empty trailing entries).
///
/// Returns true if the params are equivalent (same keys and values in order).
pub fn params_equal(params1: &[(String, String)], params2: &[(String, String)]) -> bool {
    // Filter out empty entries for comparison
    let filtered1: Vec<_> = params1
        .iter()
        .filter(|(k, v)| !k.is_empty() || !v.is_empty())
        .collect();
    let filtered2: Vec<_> = params2
        .iter()
        .filter(|(k, v)| !k.is_empty() || !v.is_empty())
        .collect();

    filtered1 == filtered2
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ extract_base_url tests ============

    #[test]
    fn test_extract_base_url_with_query() {
        assert_eq!(
            extract_base_url("https://example.com/api?foo=bar"),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_extract_base_url_without_query() {
        assert_eq!(
            extract_base_url("https://example.com/api"),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_extract_base_url_empty() {
        assert_eq!(extract_base_url(""), "");
    }

    #[test]
    fn test_extract_base_url_only_query() {
        assert_eq!(extract_base_url("?foo=bar"), "");
    }

    #[test]
    fn test_extract_base_url_preserves_fragment_after_query() {
        assert_eq!(
            extract_base_url("https://example.com/api?foo=bar#result"),
            "https://example.com/api#result"
        );
        assert_eq!(
            extract_base_url("https://example.com/api#result"),
            "https://example.com/api#result"
        );
    }

    // ============ parse_query_params tests ============

    #[test]
    fn test_parse_empty_url() {
        assert_eq!(parse_query_params(""), Vec::<(String, String)>::new());
    }

    #[test]
    fn test_parse_url_without_query() {
        assert_eq!(
            parse_query_params("https://example.com/api"),
            Vec::<(String, String)>::new()
        );
    }

    #[test]
    fn test_parse_url_with_single_param() {
        assert_eq!(
            parse_query_params("https://example.com?foo=bar"),
            vec![("foo".to_string(), "bar".to_string())]
        );
    }

    #[test]
    fn test_parse_url_with_multiple_params() {
        assert_eq!(
            parse_query_params("https://example.com?foo=bar&baz=qux"),
            vec![
                ("foo".to_string(), "bar".to_string()),
                ("baz".to_string(), "qux".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_url_with_encoded_params() {
        assert_eq!(
            parse_query_params("https://example.com?name=hello%20world&key=a%26b"),
            vec![
                ("name".to_string(), "hello world".to_string()),
                ("key".to_string(), "a&b".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_url_with_empty_value() {
        assert_eq!(
            parse_query_params("https://example.com?foo=&bar=baz"),
            vec![
                ("foo".to_string(), "".to_string()),
                ("bar".to_string(), "baz".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_partial_url_with_query() {
        // Incomplete URL that can't be parsed by Url::parse
        assert_eq!(
            parse_query_params("example.com?foo=bar"),
            vec![("foo".to_string(), "bar".to_string())]
        );
    }

    #[test]
    fn test_parse_just_query_string() {
        assert_eq!(
            parse_query_params("?foo=bar&baz=qux"),
            vec![
                ("foo".to_string(), "bar".to_string()),
                ("baz".to_string(), "qux".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_query_stops_before_fragment() {
        assert_eq!(
            parse_query_params("https://example.com/path?q=value#fragment?not=a-query"),
            vec![("q".to_string(), "value".to_string())]
        );
    }

    #[test]
    fn test_parse_preserves_templates_duplicates_empty_values_and_unicode() {
        assert_eq!(
            parse_query_params(
                "partial/{{path}}?tag=one&token={{token}}&tag=two&empty=&name=%E4%BD%A0%E5%A5%BD#result"
            ),
            vec![
                ("tag".to_string(), "one".to_string()),
                ("token".to_string(), "{{token}}".to_string()),
                ("tag".to_string(), "two".to_string()),
                ("empty".to_string(), "".to_string()),
                ("name".to_string(), "你好".to_string()),
            ]
        );
    }

    // ============ build_url_with_params tests ============

    #[test]
    fn test_build_url_empty_params() {
        assert_eq!(
            build_url_with_params("https://example.com/api", &[]),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_build_url_with_enabled_params() {
        let params = vec![
            QueryParam::new("foo", "bar", true),
            QueryParam::new("baz", "qux", true),
        ];
        assert_eq!(
            build_url_with_params("https://example.com/api", &params),
            "https://example.com/api?foo=bar&baz=qux"
        );
    }

    #[test]
    fn test_build_url_with_disabled_params() {
        let params = vec![
            QueryParam::new("foo", "bar", true),
            QueryParam::new("disabled", "value", false),
            QueryParam::new("baz", "qux", true),
        ];
        assert_eq!(
            build_url_with_params("https://example.com/api", &params),
            "https://example.com/api?foo=bar&baz=qux"
        );
    }

    #[test]
    fn test_build_url_all_disabled() {
        let params = vec![
            QueryParam::new("foo", "bar", false),
            QueryParam::new("baz", "qux", false),
        ];
        assert_eq!(
            build_url_with_params("https://example.com/api", &params),
            "https://example.com/api"
        );
    }

    #[test]
    fn test_build_url_with_empty_key() {
        let params = vec![
            QueryParam::new("foo", "bar", true),
            QueryParam::new("", "ignored", true), // Empty key should be skipped
            QueryParam::new("baz", "qux", true),
        ];
        assert_eq!(
            build_url_with_params("https://example.com/api", &params),
            "https://example.com/api?foo=bar&baz=qux"
        );
    }

    #[test]
    fn test_build_url_with_special_chars() {
        let params = vec![
            QueryParam::new("name", "hello world", true),
            QueryParam::new("special", "a&b=c", true),
        ];
        assert_eq!(
            build_url_with_params("https://example.com/api", &params),
            "https://example.com/api?name=hello%20world&special=a%26b%3Dc"
        );
    }

    #[test]
    fn test_build_url_empty_base() {
        let params = vec![QueryParam::new("foo", "bar", true)];
        assert_eq!(build_url_with_params("", &params), "?foo=bar");
    }

    #[test]
    fn test_build_keeps_templates_substitutable_and_query_before_fragment() {
        let params = vec![
            QueryParam::new("q", "prefix-{{ token }}-suffix", true),
            QueryParam::new("tag", "one", true),
            QueryParam::new("tag", "two", true),
            QueryParam::new("empty", "", true),
        ];
        assert_eq!(
            build_url_with_params("{{base_url}}/search?old=value#results", &params),
            "{{base_url}}/search?q=prefix-{{ token }}-suffix&tag=one&tag=two&empty=#results"
        );
    }

    #[test]
    fn test_build_encodes_unicode_without_losing_order_or_duplicates() {
        let params = vec![
            QueryParam::new("名称", "你好 世界", true),
            QueryParam::new("名称", "再见", true),
        ];
        assert_eq!(
            build_url_with_params("https://example.test/#片段", &params),
            "https://example.test/?%E5%90%8D%E7%A7%B0=%E4%BD%A0%E5%A5%BD%20%E4%B8%96%E7%95%8C&%E5%90%8D%E7%A7%B0=%E5%86%8D%E8%A7%81#片段"
        );
    }

    #[test]
    fn test_resolve_url_encodes_substituted_query_exactly_once() {
        let result = resolve_url_for_send(
            "https://example.test/search?q={{term}}&existing=hello%20world#results",
            |part| part.replace("{{term}}", "a b&c/你好"),
        );
        assert_eq!(
            result,
            "https://example.test/search?q=a%20b%26c%2F%E4%BD%A0%E5%A5%BD&existing=hello%20world#results"
        );
    }

    #[test]
    fn test_resolve_partial_url_uses_deterministic_syntactic_split() {
        let result = resolve_url_for_send("{{base_url}}/search?q={{term}}#{{section}}", |part| {
            part.replace("{{base_url}}", "HTTPS://example.test")
                .replace("{{term}}", "hello world")
                .replace("{{section}}", "results")
        });
        assert_eq!(
            result,
            "HTTPS://example.test/search?q=hello%20world#results"
        );
    }

    #[test]
    fn test_normalize_http_url_accepts_uppercase_and_defaults_missing_scheme() {
        let uppercase = normalize_http_url("HTTPS://Example.Test/path").unwrap();
        assert_eq!(uppercase.as_str(), "https://example.test/path");

        let defaulted = normalize_http_url("example.test/path").unwrap();
        assert_eq!(defaulted.as_str(), "http://example.test/path");

        let scheme_in_query =
            normalize_http_url("example.test/callback?next=https://other.test").unwrap();
        assert_eq!(
            scheme_in_query.as_str(),
            "http://example.test/callback?next=https://other.test"
        );

        assert!(normalize_http_url("ftp://example.test/file").is_none());
        assert!(normalize_http_url("HTTPS://").is_none());
    }

    // ============ params_equal tests ============

    #[test]
    fn test_params_equal_same() {
        let params1 = vec![
            ("foo".to_string(), "bar".to_string()),
            ("baz".to_string(), "qux".to_string()),
        ];
        let params2 = vec![
            ("foo".to_string(), "bar".to_string()),
            ("baz".to_string(), "qux".to_string()),
        ];
        assert!(params_equal(&params1, &params2));
    }

    #[test]
    fn test_params_equal_different() {
        let params1 = vec![("foo".to_string(), "bar".to_string())];
        let params2 = vec![("foo".to_string(), "different".to_string())];
        assert!(!params_equal(&params1, &params2));
    }

    #[test]
    fn test_params_equal_ignores_empty() {
        let params1 = vec![
            ("foo".to_string(), "bar".to_string()),
            ("".to_string(), "".to_string()), // Empty entry
        ];
        let params2 = vec![("foo".to_string(), "bar".to_string())];
        assert!(params_equal(&params1, &params2));
    }

    #[test]
    fn test_params_equal_both_empty() {
        let params1: Vec<(String, String)> = vec![];
        let params2: Vec<(String, String)> = vec![];
        assert!(params_equal(&params1, &params2));
    }

    #[test]
    fn test_params_equal_order_matters() {
        let params1 = vec![
            ("foo".to_string(), "bar".to_string()),
            ("baz".to_string(), "qux".to_string()),
        ];
        let params2 = vec![
            ("baz".to_string(), "qux".to_string()),
            ("foo".to_string(), "bar".to_string()),
        ];
        assert!(!params_equal(&params1, &params2));
    }
}
