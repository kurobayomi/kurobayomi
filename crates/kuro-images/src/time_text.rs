//! Duration and relative update time text for the stats and entry screens.

use chrono::{Local, TimeZone};

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;

/// Short duration text such as `1d 1h`, `1h 1m`, `5m 30s` (`Duration.toDurationString`).
/// Days show alone with hours. Minutes show unless days and hours both count. Seconds show only when there are no
/// days and no hours. A zero duration gives `fallback`.
pub fn duration_text(total_seconds: i64, fallback: &str) -> String {
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::with_capacity(4);
    if days != 0 {
        parts.push(format!("{days}d"));
    }
    if hours != 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes != 0 && (days == 0 || hours == 0) {
        parts.push(format!("{minutes}m"));
    }
    if seconds != 0 && days == 0 && hours == 0 {
        parts.push(format!("{seconds}s"));
    }
    if parts.is_empty() {
        fallback.to_owned()
    } else {
        parts.join(" ")
    }
}

/// Relative time of an update, from epoch milliseconds. A non-positive timestamp is
/// `Never`. Anything less than a minute old, including future timestamps, is `Just now`. Older values show
/// minutes, then hours, then days, up to one week, and an absolute local date after that. The date format is
/// English and the wording is an approximation of Android's `DateUtils` (open item O-7).
pub fn relative_time_text(epoch_ms: i64, now_ms: i64) -> String {
    if epoch_ms <= 0 {
        return "Never".to_owned();
    }
    let diff = now_ms - epoch_ms;
    if diff < MINUTE_MS {
        return "Just now".to_owned();
    }
    if diff < HOUR_MS {
        return count_text(diff / MINUTE_MS, "minute");
    }
    if diff < DAY_MS {
        return count_text(diff / HOUR_MS, "hour");
    }
    if diff < WEEK_MS {
        return count_text(diff / DAY_MS, "day");
    }
    match Local.timestamp_millis_opt(epoch_ms).single() {
        Some(time) => time.format("%b %-d, %Y").to_string(),
        None => "Never".to_owned(),
    }
}

fn count_text(count: i64, unit: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_examples_from_the_spec() {
        assert_eq!(duration_text(90_061, "none"), "1d 1h");
        assert_eq!(duration_text(3_660, "none"), "1h 1m");
        assert_eq!(duration_text(3_630, "none"), "1h");
        assert_eq!(duration_text(330, "none"), "5m 30s");
        assert_eq!(duration_text(0, "none"), "none");
    }

    #[test]
    fn duration_drops_minutes_only_when_days_and_hours_both_count() {
        // Days and minutes without hours: minutes stay.
        assert_eq!(duration_text(86_400 + 120, "x"), "1d 2m");
        // Days and hours: minutes go.
        assert_eq!(duration_text(86_400 + 3_600 + 120, "x"), "1d 1h");
    }

    #[test]
    fn duration_shows_seconds_only_without_days_and_hours() {
        assert_eq!(duration_text(45, "x"), "45s");
        assert_eq!(duration_text(2 * 86_400 + 5, "x"), "2d");
    }

    #[test]
    fn duration_fallback_is_used_only_when_empty() {
        assert_eq!(duration_text(0, "–"), "–");
        assert_eq!(duration_text(1, "–"), "1s");
    }

    #[test]
    fn non_positive_timestamp_is_never() {
        assert_eq!(relative_time_text(0, 1_000_000_000), "Never");
        assert_eq!(relative_time_text(-5, 1_000_000_000), "Never");
    }

    #[test]
    fn under_a_minute_and_future_timestamps_are_just_now() {
        let now = 1_700_000_000_000;
        assert_eq!(relative_time_text(now, now), "Just now");
        assert_eq!(relative_time_text(now - 59_999, now), "Just now");
        assert_eq!(relative_time_text(now + 3_600_000, now), "Just now");
    }

    #[test]
    fn relative_phrases_count_each_unit() {
        let now = 1_700_000_000_000;
        assert_eq!(relative_time_text(now - MINUTE_MS, now), "1 minute ago");
        assert_eq!(
            relative_time_text(now - 5 * MINUTE_MS, now),
            "5 minutes ago"
        );
        assert_eq!(relative_time_text(now - 2 * HOUR_MS, now), "2 hours ago");
        assert_eq!(relative_time_text(now - DAY_MS, now), "1 day ago");
        assert_eq!(relative_time_text(now - 6 * DAY_MS, now), "6 days ago");
    }

    #[test]
    fn older_than_a_week_is_an_absolute_date() {
        let now = 1_700_000_000_000;
        let old = now - WEEK_MS - DAY_MS;
        let text = relative_time_text(old, now);
        let expected = Local
            .timestamp_millis_opt(old)
            .single()
            .unwrap()
            .format("%b %-d, %Y")
            .to_string();
        assert_eq!(text, expected);
        assert!(text.ends_with(|c: char| c.is_ascii_digit()));
    }
}
