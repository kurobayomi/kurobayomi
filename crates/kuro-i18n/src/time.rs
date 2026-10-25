//! Relative dates and times and the short duration (plan
//! 51-i18n T-51a-016, T-51a-025). Thresholds follow `plans/51-i18n/p1-behavior.md`
//! section 6. Every function takes `now` or `today` as an argument, so tests use a fixed clock.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};

use crate::date::DateFormat;
use crate::{Arg, MsgKey, tr, tr_plural, trf};

/// How dates are shown: relative (`relative_time_v2`) or absolute, with the absolute format and
/// the locale its month names come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateStyle<'a> {
    pub relative: bool,
    pub format: DateFormat,
    pub locale: &'a str,
}

impl DateStyle<'_> {
    fn absolute(&self, date: NaiveDate) -> String {
        self.format.format(date, self.locale)
    }
}

/// `LocalDate.toRelativeString`: calendar-day differences against `today`.
pub fn relative_date(date: NaiveDate, today: NaiveDate, style: DateStyle<'_>) -> String {
    if !style.relative {
        return style.absolute(date);
    }
    let difference = (today - date).num_days();
    if difference < -7 {
        style.absolute(date)
    } else if difference < 0 {
        tr_plural(MsgKey::upcoming_relative_time, difference.abs(), &[])
    } else if difference < 1 {
        tr(MsgKey::relative_time_today)
    } else if difference < 7 {
        tr_plural(MsgKey::relative_time, difference, &[])
    } else {
        style.absolute(date)
    }
}

/// `LocalDateTime.toRelativeString` for chapter and episode release times. The day test uses
/// whole days between the two times; the day count in the text uses calendar days.
pub fn relative_datetime(when: NaiveDateTime, now: NaiveDateTime, style: DateStyle<'_>) -> String {
    if !style.relative {
        return style.absolute(when.date());
    }
    let time_difference = (now - when).num_days();
    let date_difference = (now.date() - when.date()).num_days();
    if time_difference < -7 {
        style.absolute(when.date())
    } else if time_difference < 0 {
        tr_plural(MsgKey::upcoming_relative_time, date_difference.abs(), &[])
    } else if time_difference < 1 {
        let hour_difference = (now - when).num_hours();
        if hour_difference < 0 {
            tr_plural(
                MsgKey::upcoming_relative_time_hours,
                hour_difference.abs(),
                &[],
            )
        } else if hour_difference < 1 {
            let minute_difference = (now - when).num_minutes();
            if minute_difference < 0 {
                tr_plural(
                    MsgKey::upcoming_relative_time_minutes,
                    minute_difference.abs(),
                    &[],
                )
            } else if minute_difference == 0 {
                tr(MsgKey::relative_time_now)
            } else {
                tr_plural(MsgKey::relative_time_minutes, minute_difference, &[])
            }
        } else {
            tr_plural(MsgKey::relative_time_hours, hour_difference, &[])
        }
    } else if time_difference < 7 {
        tr_plural(MsgKey::relative_time, date_difference, &[])
    } else {
        style.absolute(when.date())
    }
}

/// `relativeTimeSpanString` for the last-update text, from epoch milliseconds: `Never` at or
/// below zero, `Just now` under a minute, otherwise the relative datetime text (always relative).
pub fn relative_time_span(epoch_millis: i64, now_millis: i64, style: DateStyle<'_>) -> String {
    if epoch_millis <= 0 {
        return tr(MsgKey::relative_time_span_never);
    }
    if now_millis - epoch_millis < 60_000 {
        return tr(MsgKey::updates_last_update_info_just_now);
    }
    let when = local_naive(epoch_millis);
    let now = local_naive(now_millis);
    relative_datetime(
        when,
        now,
        DateStyle {
            relative: true,
            ..style
        },
    )
}

/// The system-zone wall clock for epoch milliseconds (chrono reads the zone from the OS).
fn local_naive(epoch_millis: i64) -> NaiveDateTime {
    DateTime::from_timestamp_millis(epoch_millis)
        .map(|utc| utc.with_timezone(&Local).naive_local())
        .unwrap_or_default()
}

/// `Duration.toDurationString` (T-51a-025): `1d 3h`, `2m 30s`, `45s`. Day, hour, minute and
/// second parts follow the rules in behaviour section 6.3. An empty result is `fallback`.
pub fn duration_short(total_seconds: i64, fallback: &str) -> String {
    let days = total_seconds / 86_400;
    let rest = total_seconds % 86_400;
    let hours = rest / 3_600;
    let minutes = rest % 3_600 / 60;
    let seconds = rest % 60;
    let mut parts = Vec::new();
    if days != 0 {
        parts.push(trf(MsgKey::day_short, &[("arg1", Arg::Int(days))]));
    }
    if hours != 0 {
        parts.push(trf(MsgKey::hour_short, &[("arg1", Arg::Int(hours))]));
    }
    if minutes != 0 && (days == 0 || hours == 0) {
        parts.push(trf(MsgKey::minute_short, &[("arg1", Arg::Int(minutes))]));
    }
    if seconds != 0 && days == 0 && hours == 0 {
        parts.push(trf(MsgKey::seconds_short, &[("arg1", Arg::Int(seconds))]));
    }
    if parts.is_empty() {
        fallback.to_string()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate};

    use super::*;

    /// Fixed clock: 9 October 2026, 12:00. Tests never read the system clock.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 9)
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .expect("valid fixed clock")
    }

    fn today() -> NaiveDate {
        now().date()
    }

    fn style(relative: bool) -> DateStyle<'static> {
        DateStyle {
            relative,
            format: DateFormat::IsoDate,
            locale: "en",
        }
    }

    fn days_before(n: i64) -> NaiveDate {
        today() - Duration::days(n)
    }

    #[test]
    fn relative_date_thresholds_t1_to_t8() {
        // T1: -8 days (a date 8 days ahead) is absolute.
        assert_eq!(
            relative_date(days_before(-8), today(), style(true)),
            "2026-10-17"
        );
        // T2: one day ahead.
        assert_eq!(
            relative_date(days_before(-1), today(), style(true)),
            "Tomorrow"
        );
        // T3: three days ahead.
        assert_eq!(
            relative_date(days_before(-3), today(), style(true)),
            "In 3 days"
        );
        // T4: today.
        assert_eq!(relative_date(today(), today(), style(true)), "Today");
        // T5: yesterday.
        assert_eq!(
            relative_date(days_before(1), today(), style(true)),
            "Yesterday"
        );
        // T6, T7: two and six days ago.
        assert_eq!(
            relative_date(days_before(2), today(), style(true)),
            "2 days ago"
        );
        assert_eq!(
            relative_date(days_before(6), today(), style(true)),
            "6 days ago"
        );
        // T8: seven days ago is absolute.
        assert_eq!(
            relative_date(days_before(7), today(), style(true)),
            "2026-10-02"
        );
    }

    #[test]
    fn relative_off_gives_the_absolute_date() {
        // relative_time_v2 false yields the absolute date.
        assert_eq!(relative_date(today(), today(), style(false)), "2026-10-09");
        assert_eq!(relative_datetime(now(), now(), style(false)), "2026-10-09");
    }

    #[test]
    fn relative_datetime_hour_branch_t9() {
        // T9: 18 hours before the fixed clock is within the day, so the hour count is used.
        let when = now() - Duration::hours(18);
        assert_eq!(relative_datetime(when, now(), style(true)), "18 hours ago");
    }

    #[test]
    fn relative_datetime_minute_and_now_t10_to_t13() {
        // T10: same minute.
        assert_eq!(relative_datetime(now(), now(), style(true)), "Now");
        // T11: one hour before.
        assert_eq!(
            relative_datetime(now() - Duration::hours(1), now(), style(true)),
            "1 hour ago"
        );
        // T12: one minute before.
        assert_eq!(
            relative_datetime(now() - Duration::minutes(1), now(), style(true)),
            "1 minute ago"
        );
        // T13: one minute ahead.
        assert_eq!(
            relative_datetime(now() + Duration::minutes(1), now(), style(true)),
            "In 1 minute"
        );
    }

    #[test]
    fn future_times_truncate_toward_zero_like_java_between() {
        // Twelve hours ahead is zero whole days, so the hour branch applies (not "Tomorrow").
        assert_eq!(
            relative_datetime(now() + Duration::hours(12), now(), style(true)),
            "In 12 hours"
        );
        // Thirty hours ahead is one whole day: time difference -1, calendar difference -1.
        assert_eq!(
            relative_datetime(now() + Duration::hours(30), now(), style(true)),
            "Tomorrow"
        );
    }

    #[test]
    fn relative_datetime_day_thresholds() {
        // Nine days ahead is absolute; six days back uses the calendar-day count.
        assert_eq!(
            relative_datetime(now() + Duration::days(9), now(), style(true)),
            "2026-10-18"
        );
        assert_eq!(
            relative_datetime(now() - Duration::days(6), now(), style(true)),
            "6 days ago"
        );
        assert_eq!(
            relative_datetime(now() - Duration::days(8), now(), style(true)),
            "2026-10-01"
        );
    }

    #[test]
    fn duration_short_rules_t14_to_t19() {
        // T14: zero gives the fallback.
        assert_eq!(duration_short(0, "fallback"), "fallback");
        // T15: 1 day 5 minutes: minutes shown because hours is zero.
        assert_eq!(duration_short(86_400 + 300, "x"), "1d 5m");
        // T16: 1 day 3 hours 5 minutes: minutes hidden.
        assert_eq!(duration_short(86_400 + 3 * 3_600 + 300, "x"), "1d 3h");
        // T17: 45 seconds.
        assert_eq!(duration_short(45, "x"), "45s");
        // T18: 2 minutes 30 seconds.
        assert_eq!(duration_short(150, "x"), "2m 30s");
        // T19: 1 hour 30 seconds: seconds hidden because hours is set.
        assert_eq!(duration_short(3_600 + 30, "x"), "1h");
    }

    #[test]
    fn never_and_just_now_t20_t21() {
        let now_ms = now().and_utc().timestamp_millis();
        // T20: epoch 0 is Never.
        assert_eq!(relative_time_span(0, now_ms, style(true)), "Never");
        // T21: 30 seconds ago is Just now.
        assert_eq!(
            relative_time_span(now_ms - 30_000, now_ms, style(true)),
            "Just now"
        );
    }

    #[test]
    fn five_minutes_ago_uses_the_catalog_plural_t22() {
        // T22: the catalog text (relative_time_minutes), not the framework text.
        let now_ms = now().and_utc().timestamp_millis();
        assert_eq!(
            relative_time_span(now_ms - 5 * 60_000, now_ms, style(false)),
            "5 minutes ago"
        );
        assert_eq!(
            relative_datetime(now() - Duration::minutes(5), now(), style(true)),
            "5 minutes ago"
        );
    }
}
