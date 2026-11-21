//! `[storage]` preferences (Aniyomi `StoragePreferences`, T-02a-024) and the backup flag strings.
//!
//! There is no `[backup]` section: `backup_flags` was cut by the settings diet, and the automatic backup with its
//! `backup_interval` and app-state stamps was cut by the user on 2026-11-21 (all in `DROPPED_KEYS`). The flag strings
//! exist so an import can tell which parts an Aniyomi backup preference named.

use crate::registry::define_prefs;

pub const FLAG_CATEGORIES: &str = "1";
pub const FLAG_CHAPTERS: &str = "2";
pub const FLAG_HISTORY: &str = "4";
pub const FLAG_TRACK: &str = "8";
pub const FLAG_SETTINGS: &str = "10";
pub const FLAG_EXT_SETTINGS: &str = "20";
pub const FLAG_EXTENSIONS: &str = "40";

define_prefs! {
    /// `[storage]` section: the app-state base storage folder, empty until the user picks one (the default folder
    /// comes from `kuro_platform::paths::Paths`), and the one size limit of every cache in megabytes.
    pub struct StoragePrefs in "storage" {
        base_storage_directory: String = "__APP_STATE_storage_dir" => String::new();
        cache_size_mb: i32 = "cache_size_mb" => 500;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::PrefStore;
    use crate::testutil::assert_section_registered;

    #[test]
    fn keys_and_defaults_are_the_plan_ones() {
        assert!(StoragePrefs::new(PrefStore::in_memory()).base_storage_directory.get().is_empty());
        assert_eq!(StoragePrefs::new(PrefStore::in_memory()).cache_size_mb.get(), 500);
    }

    #[test]
    fn every_field_is_registered_in_its_table() {
        let storage = StoragePrefs::new(PrefStore::in_memory());
        assert_section_registered(StoragePrefs::TABLE, StoragePrefs::FIELDS, &storage.defaults());
    }

    #[test]
    fn the_flag_strings_are_the_aniyomi_hex_digits() {
        assert_eq!([FLAG_CATEGORIES, FLAG_CHAPTERS, FLAG_HISTORY, FLAG_TRACK, FLAG_SETTINGS, FLAG_EXT_SETTINGS, FLAG_EXTENSIONS], ["1", "2", "4", "8", "10", "20", "40"]);
    }
}
