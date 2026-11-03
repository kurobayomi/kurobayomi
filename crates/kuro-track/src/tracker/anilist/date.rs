//! AniList fuzzy dates and the epoch-millisecond values kept in the track rows.
//! Dates are local calendar days: AniList receives year, month and day, and the local track
//! stores local midnight of that day. Time of day is lost on purpose.

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use serde::Serialize;

/// The `FuzzyDateInput` sent in update mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FuzzyDateInput {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// Epoch milliseconds of local midnight on `year-month-day`. `0` when the date does not exist (Feb 30).
/// When midnight falls in a DST gap, 01:00 local is used.
pub fn local_day_ms(year: i32, month: u32, day: u32) -> i64 {
    let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
        return 0;
    };
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).earliest())
        .or_else(|| {
            date.and_hms_opt(1, 0, 0)
                .and_then(|naive| Local.from_local_datetime(&naive).earliest())
        });
    midnight.map_or(0, |dt| dt.timestamp_millis())
}

/// AniList fuzzy date to epoch milliseconds. A partial date (year only, missing month or day) is `0`.
pub fn fuzzy_to_ms(year: Option<i32>, month: Option<i32>, day: Option<i32>) -> i64 {
    match (year, month, day) {
        (Some(y), Some(m), Some(d)) => match (u32::try_from(m), u32::try_from(d)) {
            (Ok(m), Ok(d)) => local_day_ms(y, m, d),
            _ => 0,
        },
        _ => 0,
    }
}

/// Epoch milliseconds to the `FuzzyDateInput` of the local calendar day. `0` means no date (`None`).
pub fn create_date(ms: i64) -> Option<FuzzyDateInput> {
    if ms == 0 {
        return None;
    }
    let local = Local.timestamp_millis_opt(ms).single()?.date_naive();
    Some(FuzzyDateInput {
        year: local.year(),
        month: local.month(),
        day: local.day(),
    })
}

/// `yyyy-MM-dd` of the local day of `ms`, or `""` for `0` (search results, `start_date`).
pub fn format_ymd(ms: i64) -> String {
    if ms == 0 {
        return String::new();
    }
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Current time in epoch milliseconds.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn t_28a_023_year_only_and_invalid_dates_are_zero() {
        assert_eq!(fuzzy_to_ms(Some(2024), None, None), 0, "test 25");
        assert_eq!(fuzzy_to_ms(Some(2024), Some(2), Some(30)), 0, "test 26");
        assert_eq!(fuzzy_to_ms(None, Some(2), Some(3)), 0);
    }

    #[test]
    fn t_28a_023_fuzzy_date_is_local_midnight() -> TestResult {
        let ms = fuzzy_to_ms(Some(2025), Some(3), Some(10));
        let expected = Local
            .with_ymd_and_hms(2025, 3, 10, 0, 0, 0)
            .earliest()
            .ok_or("no local midnight")?
            .timestamp_millis();
        assert_eq!(ms, expected);
        assert_eq!(format_ymd(ms), "2025-03-10");
        Ok(())
    }

    #[test]
    fn t_28a_023_create_date_zero_is_none_and_ms_becomes_local_ymd() {
        assert_eq!(create_date(0), None, "test 27");
        let ms = fuzzy_to_ms(Some(2025), Some(3), Some(10)) + 23 * 3_600_000 + 1_800_000;
        assert_eq!(
            create_date(ms),
            Some(FuzzyDateInput {
                year: 2025,
                month: 3,
                day: 10
            }),
            "test 28, time of day dropped"
        );
    }

    #[test]
    fn t_28a_050_round_trip_keeps_the_day_under_the_test_zone() {
        for (y, m, d) in [(2025, 3, 10), (2024, 2, 29), (1999, 12, 31)] {
            let ms = local_day_ms(y, m, d);
            assert_eq!(
                create_date(ms),
                Some(FuzzyDateInput {
                    year: y,
                    month: m as u32,
                    day: d as u32
                })
            );
        }
    }

    /// Picking a date and reading it back keeps the calendar day under each zone (T-28a-050). The parent spawns one
    /// child process per zone, because `TZ` is read once per process. The days include the New York DST changes.
    #[test]
    fn t_28a_050_round_trip_holds_under_new_york_and_tokyo() {
        const CHILD: &str = "KUROBAYOMI_DATE_ZONE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            for (y, m, d) in [
                (2025, 3, 9),
                (2025, 3, 10),
                (2025, 11, 2),
                (2025, 11, 3),
                (2025, 1, 1),
                (2024, 2, 29),
                (1999, 12, 31),
            ] {
                let ms = local_day_ms(y, m, d);
                assert_eq!(
                    create_date(ms),
                    Some(FuzzyDateInput {
                        year: y,
                        month: m,
                        day: d
                    }),
                    "{y}-{m}-{d} in {}",
                    std::env::var("TZ").unwrap_or_default()
                );
                assert_eq!(format_ymd(ms), format!("{y:04}-{m:02}-{d:02}"));
            }
            return;
        }
        let exe = std::env::current_exe().expect("test binary path");
        for zone in ["America/New_York", "Asia/Tokyo"] {
            let out = std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "tracker::anilist::date::tests::t_28a_050_round_trip_holds_under_new_york_and_tokyo",
                    "--nocapture",
                ])
                .env("TZ", zone)
                .env(CHILD, zone)
                .output()
                .expect("child test runs");
            assert!(
                out.status.success(),
                "{zone}: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
    }

    #[test]
    fn t_28a_023_format_ymd_of_zero_is_empty() {
        assert_eq!(format_ymd(0), "");
    }
}
