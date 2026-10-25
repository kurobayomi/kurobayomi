//! `[sources]` preferences (Aniyomi `SourcePreferences`, T-10a-017). The keys and defaults are the ones registered in
//! `registry`; the test below compares every field with its row. Commented-out SY keys (`eh_enable_source_blacklist`,
//! `sources_tab_*`) are not fields here.

use std::collections::BTreeSet;

use crate::library::LibraryDisplayMode;
use crate::pref_enum;
use crate::registry::define_prefs;

pref_enum! {
    /// `SetMigrateSorting.Mode`: how the migration source list is ordered.
    pub enum MigrationSortingMode { ALPHABETICAL, TOTAL }
}

pref_enum! {
    /// `SetMigrateSorting.Direction`.
    pub enum MigrationDirection { ASCENDING, DESCENDING }
}

/// The preference namespace of one source: `source_<decimal id>` (T-16a-034, EC-PR-01).
pub fn source_namespace(source_id: i64) -> String {
    format!("source_{source_id}")
}

/// The stored key of one setting of a source: `<namespace>.<key>`. A read of an unset key returns the caller's default
/// and writes nothing; only a set writes a row.
pub fn source_setting_key(source_id: i64, key: &str) -> String {
    format!("{}.{key}", source_namespace(source_id))
}

define_prefs! {
    /// `[sources]` section: the keys of Aniyomi `SourcePreferences`, minus the derived `animeext_updates_count` (T-19a-064).
    pub struct SourcePrefs in "sources" {
        // the look of every source's listing, kept apart from the library's as in Aniyomi
        display_mode: LibraryDisplayMode = "pref_display_mode_catalogue" => LibraryDisplayMode::CompactGrid;
        enabled_languages: BTreeSet<String> = "source_languages" => kuro_i18n::source_names::default_enabled_languages(&system_locale());
        // off by default (Aniyomi: on): adult titles stay hidden until the user turns them on
        show_nsfw_source: bool = "show_nsfw_source" => false;
        anime_extension_repos: BTreeSet<String> = "anime_extension_repos" => BTreeSet::new();
        manga_extension_repos: BTreeSet<String> = "extension_repos" => BTreeSet::new();
        trusted_extensions: BTreeSet<String> = "__APP_STATE_trusted_extensions" => BTreeSet::new();
        // Aniyomi's disabled sources (decimal ids), as a restore writes them: the app reads them once, turns on every
        // known source but these, and clears them
        disabled_anime_sources: BTreeSet<String> = "hidden_anime_catalogues" => BTreeSet::new();
        disabled_manga_sources: BTreeSet<String> = "hidden_catalogues" => BTreeSet::new();
        // the repository sources that are on, by their entry id in the repository index; a source not in it is off
        enabled_sources: BTreeSet<String> = "enabled_sources" => BTreeSet::new();
        // "<source id>=<entry id>": the repository entry each loaded source came from, so a source that is off now is
        // still known by its id
        source_keys: BTreeSet<String> = "__APP_STATE_source_keys" => BTreeSet::new();
    }
}

/// The device language: the language part of the resolved UI locale of `kuro-i18n`, not an environment read (T-51a-035).
pub fn system_locale() -> String {
    kuro_i18n::source_names::device_language()
}

pub use kuro_i18n::source_names::default_enabled_languages;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::REGISTRY;
    use crate::store::PrefStore;

    #[test]
    fn every_field_is_a_registered_sources_key_with_the_same_name() {
        assert_eq!(SourcePrefs::TABLE, "sources");
        assert_eq!(SourcePrefs::FIELDS.len(), 10, "the settings diet kept nine keys, and the source view came back");
        for (field, key) in SourcePrefs::FIELDS {
            let row = REGISTRY.iter().find(|m| m.key == *key).unwrap_or_else(|| panic!("{field}: {key} not registered"));
            assert_eq!(row.table, "sources", "{key}");
        }
    }

    #[test]
    fn the_app_state_keys_carry_the_prefix() {
        let key = |field: &str| SourcePrefs::FIELDS.iter().find(|(f, _)| *f == field).expect("field").1;
        assert_eq!(key("trusted_extensions"), "__APP_STATE_trusted_extensions");
        assert!(!SourcePrefs::FIELDS.iter().any(|(_, k)| k.starts_with("eh_") || k.starts_with("sources_tab")));
    }

    #[test]
    fn defaults_match_the_registry_and_the_plan() {
        let store = PrefStore::in_memory();
        let prefs = SourcePrefs::new(store);
        assert!(!prefs.show_nsfw_source.get(), "adult titles are hidden by default");
        assert_eq!(prefs.display_mode.key(), "pref_display_mode_catalogue");
        assert_eq!(prefs.display_mode.get(), LibraryDisplayMode::CompactGrid);
        assert!(prefs.disabled_manga_sources.get().is_empty());
        assert!(prefs.enabled_languages.get().contains("en"));
        assert!(prefs.enabled_languages.get().contains("all"));
    }

    #[test]
    fn a_set_of_source_ids_round_trips_through_its_decimal_strings() {
        let store = PrefStore::in_memory();
        let prefs = SourcePrefs::new(store.clone());
        let ids: BTreeSet<String> = ["4294967296", "7"].iter().map(|s| (*s).to_owned()).collect();
        prefs.disabled_manga_sources.set(ids.clone()).unwrap();
        assert_eq!(prefs.disabled_manga_sources.get(), ids, "T-17a-015: {{4294967296, 7}} reads back equal");
        let reopened = SourcePrefs::new(store);
        assert_eq!(reopened.disabled_manga_sources.get(), ids, "and survives a new handle on the same store");
    }

    #[test]
    fn every_default_of_the_kept_keys_holds_on_an_empty_store() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        assert!(prefs.trusted_extensions.get().is_empty());
        assert!(prefs.enabled_sources.get().is_empty() && prefs.source_keys.get().is_empty());
        assert!(prefs.disabled_anime_sources.get().is_empty());
        assert!(prefs.manga_extension_repos.get().is_empty() && prefs.anime_extension_repos.get().is_empty());
    }

    #[test]
    fn the_default_languages_apply_only_while_the_key_is_unset() {
        let store = PrefStore::in_memory();
        let prefs = SourcePrefs::new(store.clone());
        let stored: BTreeSet<String> = ["fr".to_owned()].into_iter().collect();
        prefs.enabled_languages.set(stored.clone()).unwrap();
        let reopened = SourcePrefs::new(store);
        assert_eq!(reopened.enabled_languages.get(), stored, "a stored value wins over the system default");
    }

    #[test]
    fn default_languages_add_the_system_subtag_and_an_unusable_locale_gives_en() {
        let set = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<_>>();
        assert_eq!(default_enabled_languages("pt_BR.UTF-8"), set(&["all", "en", "pt"]));
        assert_eq!(default_enabled_languages("ja_JP.UTF-8"), set(&["all", "en", "ja"]));
        assert_eq!(default_enabled_languages("pt_BR@euro"), set(&["all", "en", "pt"]));
        assert_eq!(default_enabled_languages("zh-Hant_TW"), set(&["all", "en", "zh"]));
        assert_eq!(default_enabled_languages("JA"), set(&["all", "en", "ja"]));
        assert_eq!(default_enabled_languages("C"), set(&["all", "en"]));
        assert_eq!(default_enabled_languages("POSIX"), set(&["all", "en"]));
        assert_eq!(default_enabled_languages(""), set(&["all", "en"]));
        assert_eq!(default_enabled_languages("12345"), set(&["all", "en"]));
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;
    use crate::store::PrefStore;
    use crate::values::PrefValue;

    #[test]
    fn the_namespace_is_the_source_id_in_decimal() {
        assert_eq!(source_namespace(4_851_548_778_462_321_757), "source_4851548778462321757");
        assert_eq!(source_setting_key(7, "nsfw"), "source_7.nsfw");
    }

    #[test]
    fn a_read_of_an_unset_source_setting_gives_the_default_and_writes_no_row() {
        let store = PrefStore::in_memory();
        let pref = store.pref(source_setting_key(9, "quality"), false);
        assert!(!pref.get());
        assert!(store.all().is_empty(), "a read never creates a row");
        assert_eq!(store.get_raw(&source_setting_key(9, "quality")), None);
    }

    #[test]
    fn a_written_source_setting_persists_under_its_namespace() {
        let store = PrefStore::in_memory();
        let pref = store.pref(source_setting_key(9, "quality"), false);
        pref.set(true).unwrap();
        assert!(pref.get());
        assert_eq!(store.get_raw("source_9.quality"), Some(PrefValue::Bool(true)));
        assert_eq!(store.get_raw("source_10.quality"), None, "another source's namespace is untouched");
    }
}

