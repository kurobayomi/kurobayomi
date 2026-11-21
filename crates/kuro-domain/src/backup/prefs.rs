//! Backup preferences (`BackupPreferences.kt`, T-09a-052): the flags. Keys and defaults are the ones registered in
//! `kuro_prefs::registry`. The automatic backup and its interval were cut by the user on 2026-11-21.

use kuro_prefs::pref::Pref;
use kuro_prefs::store::PrefStore;

use super::flags::BackupFlags;

/// Typed handles for the backup preferences.
#[derive(Clone)]
pub struct BackupPrefs {
    /// `backup_flags`. Kept for format parity; see the module note in `flags`.
    pub flags: Pref<BackupFlags>,
}

impl BackupPrefs {
    pub fn new(store: &PrefStore) -> Self {
        Self { flags: store.pref("backup_flags", BackupFlags::default()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_default_to_the_four_upstream_tags() {
        let store = PrefStore::in_memory();
        let prefs = BackupPrefs::new(&store);
        assert_eq!(prefs.flags.get(), BackupFlags::default());
        assert_eq!(prefs.flags.key(), "backup_flags");
    }
}
