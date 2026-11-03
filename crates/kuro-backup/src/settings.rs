//! Automatic backup settings rules (area 21, T-21a-031, T-21a-032, T-21a-037, T-21a-039). The caller reads and
//! writes the config; this module decides the values.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};

/// The six interval values the Settings list offers, in list order (B-04, E-13).
pub const ALLOWED_INTERVAL_HOURS: [u32; 6] = [0, 6, 12, 24, 48, 168];
pub const DEFAULT_INTERVAL_HOURS: u32 = 12;
const AUTO_DIR_NAME: &str = "autobackup";
const APP_DIR_NAME: &str = "kurobayomi";
pub const LAST_AUTO_PREFIX: &str = "Last automatically backed up: ";

/// Stored value to allowed value. Anything outside the list, including a negative number, loads as the default (E-13).
pub fn normalize_interval(stored: i64) -> u32 {
    u32::try_from(stored)
        .ok()
        .filter(|h| ALLOWED_INTERVAL_HOURS.contains(h))
        .unwrap_or(DEFAULT_INTERVAL_HOURS)
}

/// The setter accepts only the six values (E-13).
pub fn accept_interval(hours: u32) -> Option<u32> {
    ALLOWED_INTERVAL_HOURS.contains(&hours).then_some(hours)
}

/// Settings list labels (B-04). `None` for a value outside the list.
pub fn interval_label(hours: u32) -> Option<&'static str> {
    match hours {
        0 => Some("Off"),
        6 => Some("Every 6 hours"),
        12 => Some("Every 12 hours"),
        24 => Some("Daily"),
        48 => Some("Every 2 days"),
        168 => Some("Weekly"),
        _ => None,
    }
}

/// The value of `state.backup.enabled_at_epoch_ms` after an interval change (E-08). Set only when the interval goes
/// from 0 to a non-zero value; `None` keeps the stored value.
pub fn enabled_at_on_change(old_hours: u32, new_hours: u32, now_ms: i64) -> Option<i64> {
    (old_hours == 0 && new_hours != 0).then_some(now_ms)
}

/// One-time migration for the version that adds automatic backups (Aniyomi migration 84f): a stored interval of 0
/// becomes 12. Returns the value to store, or `None` when nothing changes. The caller runs this once per version.
pub fn migrate_disabled_interval(stored: Option<i64>) -> Option<u32> {
    (stored == Some(0)).then_some(DEFAULT_INTERVAL_HOURS)
}

/// `<base>/autobackup` when a storage base folder is set, else `<XDG data home>/kurobayomi/autobackup` (T-21a-037).
pub fn automatic_dir(base: Option<&Path>, data_home: &Path) -> PathBuf {
    match base {
        Some(base) => base.join(AUTO_DIR_NAME),
        None => data_home.join(APP_DIR_NAME).join(AUTO_DIR_NAME),
    }
}

/// Text for the "last automatic backup" info line (T-21a-031, B-10). Zero means never.
pub fn last_auto_info(last_ms: i64, now_ms: i64) -> String {
    format!("{LAST_AUTO_PREFIX}{}", relative_time(last_ms, now_ms))
}

/// Relative time buckets. A time in the future reads as "Just now" (E-31).
pub fn relative_time(last_ms: i64, now_ms: i64) -> String {
    if last_ms == 0 {
        return "Never".into();
    }
    let secs = (now_ms.saturating_sub(last_ms) / 1000).max(0);
    match secs {
        0..60 => "Just now".into(),
        60..3600 => plural(secs / 60, "minute"),
        3600..86_400 => plural(secs / 3600, "hour"),
        _ => match DateTime::<Utc>::from_timestamp_millis(last_ms) {
            Some(utc) => utc.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(),
            None => "Never".into(),
        },
    }
}

fn plural(n: i64, unit: &str) -> String {
    if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_loads_only_allowed_values() {
        for h in ALLOWED_INTERVAL_HOURS {
            assert_eq!(normalize_interval(i64::from(h)), h);
        }
        assert_eq!(normalize_interval(3), 12, "E-13: 3 hours loads as 12");
        assert_eq!(normalize_interval(-6), 12);
        assert_eq!(normalize_interval(i64::MAX), 12);
    }

    #[test]
    fn setter_rejects_other_values() {
        assert_eq!(accept_interval(24), Some(24));
        assert_eq!(accept_interval(3), None);
    }

    #[test]
    fn labels_match_the_list() {
        let labels: Vec<_> = ALLOWED_INTERVAL_HOURS.iter().map(|h| interval_label(*h).unwrap()).collect();
        assert_eq!(labels, ["Off", "Every 6 hours", "Every 12 hours", "Daily", "Every 2 days", "Weekly"]);
        assert_eq!(interval_label(3), None);
    }

    #[test]
    fn enabling_stamps_enable_time_only_from_zero() {
        assert_eq!(enabled_at_on_change(0, 12, 500), Some(500));
        assert_eq!(enabled_at_on_change(12, 6, 500), None);
        assert_eq!(enabled_at_on_change(12, 0, 500), None);
    }

    #[test]
    fn migration_turns_zero_into_twelve() {
        assert_eq!(migrate_disabled_interval(Some(0)), Some(12));
        assert_eq!(migrate_disabled_interval(Some(24)), None);
        assert_eq!(migrate_disabled_interval(None), None);
    }

    #[test]
    fn automatic_dir_default_and_base() {
        let data = Path::new("/home/u/.local/share");
        assert_eq!(automatic_dir(None, data), PathBuf::from("/home/u/.local/share/kurobayomi/autobackup"));
        assert_eq!(automatic_dir(Some(Path::new("/mnt/bk")), data), PathBuf::from("/mnt/bk/autobackup"));
    }

    #[test]
    fn last_auto_text_buckets() {
        let now = 10_000_000_000i64;
        assert_eq!(last_auto_info(0, now), "Last automatically backed up: Never");
        assert_eq!(relative_time(now - 30_000, now), "Just now");
        assert_eq!(relative_time(now + 5_000, now), "Just now", "future timestamp reads as now");
        assert_eq!(relative_time(now - 60_000, now), "1 minute ago");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5 minutes ago");
        assert_eq!(relative_time(now - 3_600_000, now), "1 hour ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3 hours ago");
        assert!(relative_time(now - 2 * 86_400_000, now).len() == 16, "date form yyyy-MM-dd HH:mm");
    }
}
