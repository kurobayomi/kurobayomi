//! Backup domain (area 09 section 1): backup preference values (`prefs`) and the backup flag tags (`flags`). The backup
//! format and restore live in the backup areas (area 21 and area 22), not in this crate.

pub mod flags;
pub mod prefs;

pub use flags::{BackupFlag, BackupFlags};
pub use prefs::BackupPrefs;
