//! Standard HTTP request header names for the header-key typeahead.
//!
//! Pure functions and a static table, so the whole matching rule is unit-tested
//! without a GPUI window. The UI layer wraps `suggest` in a
//! `gpui_component::input::CompletionProvider`; keeping the logic here means it
//! survives unchanged if that mechanism is ever swapped for a hand-rolled popup.

/// Common HTTP request header names offered as completions.
///
/// Byte-sorted and deduplicated — `suggest` returns candidates in table order, so
/// the table's order *is* the menu's order. The six headers that already own
/// dedicated rows in the editor (`Cache-Control`, `Content-Type`, `Accept`,
/// `User-Agent`, `Connection`, `Content-Length`) are deliberately absent: they
/// cannot be typed into a custom row without sending a duplicate header on the
/// wire. See `PredefinedHeader` in `types.rs`.
pub const HEADER_NAMES: &[&str] = &[
    "Accept-Charset",
    "Accept-Encoding",
    "Accept-Language",
    "Access-Control-Request-Headers",
    "Access-Control-Request-Method",
    "Age",
    "Allow",
    "Authorization",
    "Content-Disposition",
    "Content-Encoding",
    "Content-Language",
    "Content-Location",
    "Content-MD5",
    "Content-Range",
    "Cookie",
    "Date",
    "ETag",
    "Expect",
    "Expires",
    "Forwarded",
    "From",
    "Host",
    "If-Match",
    "If-Modified-Since",
    "If-None-Match",
    "If-Range",
    "If-Unmodified-Since",
    "Keep-Alive",
    "Last-Modified",
    "Link",
    "Location",
    "Max-Forwards",
    "Origin",
    "Pragma",
    "Prefer",
    "Proxy-Authorization",
    "Range",
    "Referer",
    "Retry-After",
    "Server",
    "TE",
    "Trailer",
    "Transfer-Encoding",
    "Upgrade",
    "Via",
    "WWW-Authenticate",
    "Warning",
    "X-Api-Key",
    "X-CSRF-Token",
    "X-Correlation-Id",
    "X-Forwarded-For",
    "X-Forwarded-Host",
    "X-Forwarded-Proto",
    "X-Frame-Options",
    "X-HTTP-Method-Override",
    "X-Request-Id",
    "X-Requested-With",
];

/// Header names whose completion is a case-insensitive prefix match on `prefix`.
///
/// An empty `prefix` yields nothing: the trailing blank header row is always
/// present, so suggesting the entire table whenever it takes focus would fire a
/// large menu at a user who is merely tabbing past.
pub fn suggest(prefix: &str) -> Vec<&'static str> {
    if prefix.is_empty() {
        return Vec::new();
    }

    HEADER_NAMES
        .iter()
        .copied()
        .filter(|name| starts_with_ignore_ascii_case(name, prefix))
        .collect()
}

fn starts_with_ignore_ascii_case(name: &str, prefix: &str) -> bool {
    name.len() >= prefix.len()
        && name.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names that own dedicated rows in the editor and must never be suggested.
    const PREDEFINED: &[&str] = &[
        "Cache-Control",
        "Content-Type",
        "Accept",
        "User-Agent",
        "Connection",
        "Content-Length",
    ];

    #[test]
    fn suggests_authorization_for_au() {
        assert!(suggest("Au").contains(&"Authorization"));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let canonical = suggest("Au");
        assert_eq!(suggest("au"), canonical);
        assert_eq!(suggest("AU"), canonical);
        assert_eq!(suggest("aU"), canonical);
    }

    #[test]
    fn empty_prefix_suggests_nothing() {
        assert!(suggest("").is_empty());
    }

    #[test]
    fn matching_is_prefix_not_substring() {
        // "Type" appears inside "Content-Type" but is not a prefix of anything.
        assert!(suggest("Type").is_empty());
        assert!(suggest("Encoding").is_empty());
    }

    #[test]
    fn unknown_prefix_is_empty_and_does_not_panic() {
        assert!(suggest("Zzz").is_empty());
        assert!(suggest("X-No-Such-Header-Really").is_empty());
    }

    #[test]
    fn prefix_longer_than_any_name_is_empty() {
        let longest = HEADER_NAMES.iter().map(|n| n.len()).max().unwrap();
        assert!(suggest(&"A".repeat(longest + 1)).is_empty());
    }

    #[test]
    fn x_dash_yields_only_x_dash_names() {
        let results = suggest("X-");
        assert!(results.len() > 1, "expected several X- headers, got {:?}", results);
        assert!(results.iter().all(|name| name.starts_with("X-")));
    }

    #[test]
    fn predefined_headers_are_never_suggested() {
        for predefined in PREDEFINED {
            assert!(
                !HEADER_NAMES.contains(predefined),
                "{predefined} is in the table but owns a dedicated row"
            );
            // Also unreachable through its own name as a query.
            assert!(
                !suggest(predefined).contains(predefined),
                "{predefined} was suggested for its own name"
            );
        }
    }

    #[test]
    fn table_is_sorted_and_deduplicated() {
        for pair in HEADER_NAMES.windows(2) {
            assert!(
                pair[0] < pair[1],
                "table is unsorted or contains a duplicate: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn accept_prefix_excludes_the_predefined_accept_but_keeps_its_relatives() {
        let results = suggest("accept");
        assert!(!results.contains(&"Accept"));
        assert!(results.contains(&"Accept-Encoding"));
        assert!(results.contains(&"Accept-Language"));
    }
}
