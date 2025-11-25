//! Pure functions for URL and query parameter handling.
//!
//! This module contains stateless, side-effect-free functions for parsing and building URLs
//! with query parameters. These functions are designed to be easily testable.

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

/// Extract the base URL (without query string) from a URL string.
///
/// # Examples
/// ```
/// assert_eq!(extract_base_url("https://example.com/api?foo=bar"), "https://example.com/api");
/// assert_eq!(extract_base_url("https://example.com/api"), "https://example.com/api");
/// assert_eq!(extract_base_url(""), "");
/// ```
pub fn extract_base_url(url: &str) -> &str {
    if let Some(pos) = url.find('?') {
        &url[..pos]
    } else {
        url
    }
}

/// Parse query parameters from a URL string.
///
/// Returns a list of (key, value) pairs. All returned params are considered "enabled".
/// Returns an empty Vec if:
/// - URL is empty
/// - URL has no query string
/// - URL parsing fails and no manual query string found
///
/// # Examples
/// ```
/// let params = parse_query_params("https://example.com?foo=bar&baz=qux");
/// assert_eq!(params, vec![("foo".to_string(), "bar".to_string()), ("baz".to_string(), "qux".to_string())]);
/// ```
pub fn parse_query_params(url: &str) -> Vec<(String, String)> {
    if url.is_empty() {
        return Vec::new();
    }

    // Try to parse as a valid URL first
    if let Ok(parsed_url) = Url::parse(url) {
        return parsed_url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
    }

    // URL parsing failed, try to extract query string manually
    if let Some(query_start) = url.find('?') {
        let query = &url[query_start + 1..];
        let mut params = Vec::new();

        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            if let Some(eq_pos) = pair.find('=') {
                let key = urlencoding::decode(&pair[..eq_pos])
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let value = urlencoding::decode(&pair[eq_pos + 1..])
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if !key.is_empty() {
                    params.push((key, value));
                }
            } else {
                // Key without value (e.g., "?foo&bar=baz")
                let key = urlencoding::decode(pair)
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if !key.is_empty() {
                    params.push((key, String::new()));
                }
            }
        }

        return params;
    }

    // No query string found
    Vec::new()
}

/// Build a URL by combining a base URL with query parameters.
///
/// Only enabled params with non-empty keys are included in the query string.
/// Keys and values are URL-encoded.
///
/// # Arguments
/// * `base_url` - The base URL (without query string)
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
pub fn build_url_with_params(base_url: &str, params: &[QueryParam]) -> String {
    let param_parts: Vec<String> = params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
        .map(|p| {
            format!(
                "{}={}",
                urlencoding::encode(&p.key),
                urlencoding::encode(&p.value)
            )
        })
        .collect();

    if param_parts.is_empty() {
        base_url.to_string()
    } else {
        format!("{}?{}", base_url, param_parts.join("&"))
    }
}

/// Compare two lists of query parameters (ignoring empty trailing entries).
///
/// Returns true if the params are equivalent (same keys and values in order).
pub fn params_equal(
    params1: &[(String, String)],
    params2: &[(String, String)],
) -> bool {
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
