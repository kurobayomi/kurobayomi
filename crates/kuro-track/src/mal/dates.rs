//! MAL date conversion. Wire dates are
//! `yyyy-MM-dd` in the local calendar. Local fields are epoch milliseconds at local midnight, and 0 means unset.

use chrono::{DateTime, Local, NaiveDate, TimeDelta, TimeZone};

/// Current time in epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Current time in epoch seconds.
pub fn now_secs() -> i64 {
    now_ms() / 1000
}

/// The form value for an epoch-millisecond field. 0 is `Some("")`, which clears the date on MAL (E-DATE-1).
/// `None` only when the value is outside chrono's range.
pub fn to_mal_date(epoch_ms: i64) -> Option<String> {
    if epoch_ms == 0 {
        return Some(String::new());
    }
    let utc = DateTime::from_timestamp_millis(epoch_ms)?;
    Some(utc.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

/// Parses `YYYY-MM-DD`, `YYYY-MM` (day 1) or `YYYY` (January 1) and returns local midnight in epoch ms (D-08).
/// Anything else, including impossible days such as `2020-02-30`, returns `None` with a warning (E-DATE-3).
pub fn parse_mal_date(text: &str) -> Option<i64> {
    let Some(date) = parse_parts(text) else {
        tracing::warn!(date = text, "MAL date is not a valid date, ignored");
        return None;
    };
    local_midnight(date).map(|dt| dt.timestamp_millis())
}

fn parse_parts(text: &str) -> Option<NaiveDate> {
    let mut parts = text.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = match parts.next() {
        Some(m) => m.parse().ok()?,
        None => 1,
    };
    let day: u32 = match parts.next() {
        Some(d) => d.parse().ok()?,
        None => 1,
    };
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Local midnight of `date`. When midnight does not exist that day (a DST gap at 00:00), the first instant after
/// the gap is used (E-DATE-4).
fn local_midnight(date: NaiveDate) -> Option<DateTime<Local>> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&midnight).earliest().or_else(|| {
        Local
            .from_local_datetime(&(midnight + TimeDelta::hours(1)))
            .earliest()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const DST_CHILD_ENV: &str = "KURO_MAL_DST_CHILD";

    #[test]
    fn date_zero_is_empty_clear() {
        assert_eq!(to_mal_date(0), Some(String::new()));
    }

    #[test]
    fn date_roundtrip_local() {
        let ms = parse_mal_date("2021-01-05").expect("valid date");
        assert_eq!(to_mal_date(ms).as_deref(), Some("2021-01-05"));
    }

    #[test]
    fn date_partial_year_month_and_year() {
        let ym = parse_mal_date("2020-05").expect("YYYY-MM accepted");
        assert_eq!(to_mal_date(ym).as_deref(), Some("2020-05-01"));
        let y = parse_mal_date("2020").expect("YYYY accepted");
        assert_eq!(to_mal_date(y).as_deref(), Some("2020-01-01"));
    }

    #[test]
    fn date_invalid_is_absent() {
        assert_eq!(parse_mal_date("2020-02-30"), None);
        assert_eq!(parse_mal_date("hello"), None);
        assert_eq!(parse_mal_date(""), None);
    }

    #[test]
    fn date_roundtrip_identity_1970_to_2099() {
        // 1970-01-01 is skipped: in UTC its local midnight is epoch 0, which means "unset".
        let mut day = NaiveDate::from_ymd_opt(1970, 1, 2).unwrap();
        let end = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        while day <= end {
            let text = day.format("%Y-%m-%d").to_string();
            let ms = parse_mal_date(&text).unwrap_or_else(|| panic!("{text} parses"));
            assert_eq!(to_mal_date(ms).as_deref(), Some(text.as_str()));
            day = day.succ_opt().unwrap();
        }
    }

    /// Runs the check in a child process with `TZ` set to a zone where local midnight does not exist on
    /// 2021-09-05 (Chile starts DST at 00:00). The parent only spawns the child.
    #[test]
    fn dst_gap_midnight_is_valid_instant() {
        if std::env::var_os(DST_CHILD_ENV).is_some() {
            let ms = parse_mal_date("2021-09-05")
                .expect("a valid instant on a day whose midnight is skipped");
            assert_eq!(to_mal_date(ms).as_deref(), Some("2021-09-05"));
            return;
        }
        if !std::path::Path::new("/usr/share/zoneinfo/America/Santiago").exists() {
            eprintln!("zoneinfo missing, DST gap check skipped");
            return;
        }
        let exe = std::env::current_exe().expect("test binary path");
        let out = Command::new(exe)
            .args([
                "--exact",
                "mal::dates::tests::dst_gap_midnight_is_valid_instant",
                "--nocapture",
            ])
            .env("TZ", "America/Santiago")
            .env(DST_CHILD_ENV, "1")
            .output()
            .expect("child test runs");
        assert!(
            out.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
