//! Human-readable formatting helpers for UI display (sizes, durations,
//! relative timestamps). Pure functions so every display rule is unit-tested.

use chrono::{DateTime, Utc};

/// Render a value with at most two decimals, trimming trailing zeros
/// ("1.50" -> "1.5", "1.00" -> "1").
fn trim2(value: f64) -> String {
    let s = format!("{:.2}", value);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Format a byte count for display: "532 B", "1.5 KB", "5.15 MB", "3 GB".
/// At most two decimals, trailing zeros trimmed.
pub fn format_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b < KB {
        format!("{} B", bytes)
    } else if b < MB {
        format!("{} KB", trim2(b / KB))
    } else if b < GB {
        format!("{} MB", trim2(b / MB))
    } else {
        format!("{} GB", trim2(b / GB))
    }
}

/// Format a duration for display: "245 ms", "1.52 s", "1 m 30 s".
/// At most two decimals on seconds, trailing zeros trimmed.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{} ms", ms)
    } else if ms < 60_000 {
        format!("{} s", trim2(ms as f64 / 1_000.0))
    } else {
        let mut minutes = ms / 60_000;
        let mut seconds = ((ms % 60_000) as f64 / 1_000.0).round() as u64;
        if seconds == 60 {
            minutes += 1;
            seconds = 0;
        }
        format!("{} m {} s", minutes, seconds)
    }
}

/// Format an RFC 3339 timestamp relative to `now`: "just now", "5 min ago",
/// "1 hour ago", "3 days ago". Unparseable input is returned unchanged.
pub fn format_relative_time(timestamp: &str, now: DateTime<Utc>) -> String {
    let Ok(then) = DateTime::parse_from_rfc3339(timestamp) else {
        return timestamp.to_string();
    };
    let elapsed = now.signed_duration_since(then);

    let plural = |n: i64, unit: &str| {
        format!("{} {}{} ago", n, unit, if n == 1 { "" } else { "s" })
    };

    if elapsed.num_seconds() < 60 {
        "just now".to_string()
    } else if elapsed.num_minutes() < 60 {
        format!("{} min ago", elapsed.num_minutes())
    } else if elapsed.num_hours() < 24 {
        plural(elapsed.num_hours(), "hour")
    } else {
        plural(elapsed.num_days(), "day")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    // ===== format_size =====

    #[test]
    fn size_bytes_shown_plain() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(532), "532 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn size_kilobytes_trim_trailing_zeros() {
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1259), "1.23 KB");
    }

    #[test]
    fn size_megabytes_and_gigabytes() {
        assert_eq!(format_size(1024 * 1024), "1 MB");
        assert_eq!(format_size(5_400_000), "5.15 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3 GB");
    }

    // ===== format_duration_ms =====

    #[test]
    fn duration_millis_shown_plain() {
        assert_eq!(format_duration_ms(0), "0 ms");
        assert_eq!(format_duration_ms(245), "245 ms");
        assert_eq!(format_duration_ms(999), "999 ms");
    }

    #[test]
    fn duration_seconds_trim_trailing_zeros() {
        assert_eq!(format_duration_ms(1000), "1 s");
        assert_eq!(format_duration_ms(1520), "1.52 s");
        assert_eq!(format_duration_ms(30_100), "30.1 s");
    }

    #[test]
    fn duration_minutes_with_whole_seconds() {
        assert_eq!(format_duration_ms(60_000), "1 m 0 s");
        assert_eq!(format_duration_ms(90_000), "1 m 30 s");
        assert_eq!(format_duration_ms(61_000), "1 m 1 s");
    }

    #[test]
    fn duration_seconds_rounding_carries_into_minutes() {
        // 119_999 ms would round to "1 m 60 s" without a carry.
        assert_eq!(format_duration_ms(119_999), "2 m 0 s");
    }

    // ===== format_relative_time =====

    fn ts(now: DateTime<Utc>, ago: Duration) -> String {
        (now - ago).to_rfc3339()
    }

    #[test]
    fn relative_under_a_minute_is_just_now() {
        let now = Utc::now();
        assert_eq!(format_relative_time(&ts(now, Duration::seconds(30)), now), "just now");
        assert_eq!(format_relative_time(&ts(now, Duration::seconds(0)), now), "just now");
    }

    #[test]
    fn relative_future_timestamp_is_just_now() {
        // Clock skew: a timestamp slightly in the future must not underflow.
        let now = Utc::now();
        assert_eq!(format_relative_time(&ts(now, Duration::seconds(-5)), now), "just now");
    }

    #[test]
    fn relative_minutes() {
        let now = Utc::now();
        assert_eq!(format_relative_time(&ts(now, Duration::minutes(1)), now), "1 min ago");
        assert_eq!(format_relative_time(&ts(now, Duration::minutes(5)), now), "5 min ago");
        assert_eq!(format_relative_time(&ts(now, Duration::minutes(59)), now), "59 min ago");
    }

    #[test]
    fn relative_hours_pluralized() {
        let now = Utc::now();
        assert_eq!(format_relative_time(&ts(now, Duration::minutes(90)), now), "1 hour ago");
        assert_eq!(format_relative_time(&ts(now, Duration::hours(5)), now), "5 hours ago");
    }

    #[test]
    fn relative_days_pluralized() {
        let now = Utc::now();
        assert_eq!(format_relative_time(&ts(now, Duration::hours(25)), now), "1 day ago");
        assert_eq!(format_relative_time(&ts(now, Duration::days(3)), now), "3 days ago");
    }

    #[test]
    fn relative_unparseable_returned_unchanged() {
        let now = Utc::now();
        assert_eq!(format_relative_time("not-a-date", now), "not-a-date");
    }
}
