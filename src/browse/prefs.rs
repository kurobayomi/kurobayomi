//! Browse preferences (T-40a-032, T-40b-005, T-41b-043). The keys live in the one settings file with the other
//! preferences: `[sources]` for the source keys and the library key of the entry kind for the default category, so each
//! kind's pages read their own. The display mode, the migrate flags, the grid columns and the hide-in-library switch are
//! cut from the settings and hard-coded (see `misc::diet_defaults`).

use kuro_prefs::library::{LibraryDisplayMode, LibraryPrefs};
use kuro_prefs::pref::Pref;
use kuro_prefs::source::SourcePrefs;
use kuro_prefs::store::PrefStore;

use kuro_platform::kind::Kind;

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// The typed browse preferences of one kind. Each field reads and writes one stored key.
pub struct BrowsePrefs {
    /// `[sources]`: the upstream source keys (languages, pins, disabled sources and the migrate sorting).
    pub sources: SourcePrefs,
    /// `default_category` or `default_anime_category` (owned by the library screen): -1 asks, 0 is uncategorised, above 0
    /// a category id.
    pub default_category: Pref<i32>,
    /// `library_cover_size`: the cover size the Library grids use too.
    pub cover_size: Pref<i32>,
}

impl BrowsePrefs {
    pub fn new(store: PrefStore, kind: Kind) -> Self {
        let library = LibraryPrefs::new(store.clone());
        let default_category = match kind {
            Kind::Manga => library.default_manga_category.clone(),
            Kind::Anime => library.default_anime_category.clone(),
        };
        Self { sources: SourcePrefs::new(store), default_category, cover_size: library.cover_size.clone() }
    }

    /// The display mode of the source grids: the compact grid, since the setting is cut.
    pub fn display_mode(&self) -> LibraryDisplayMode {
        LibraryDisplayMode::CompactGrid
    }

    /// `browse_hide_in_library_items`: cut from the settings, so a browse page never leaves the library's entries out.
    pub fn hide_in_library(&self) -> bool {
        crate::misc::diet_defaults::BROWSE_HIDE_IN_LIBRARY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::source::default_enabled_languages;
    use kuro_prefs::toml_backend::TomlBackend;

    fn prefs() -> BrowsePrefs {
        BrowsePrefs::new(PrefStore::in_memory(), Kind::Manga)
    }

    #[test]
    fn defaults_match_the_upstream_settings() {
        let p = prefs();
        assert_eq!(p.display_mode(), LibraryDisplayMode::CompactGrid);
        assert!(!p.sources.show_nsfw_source.get(), "adult titles are hidden by default");
        assert!(p.sources.enabled_languages.get().contains("all"));
        assert_eq!(p.default_category.get(), -1);
        assert!(!p.hide_in_library(), "the hide switch is cut, so nothing is hidden");
    }

    #[test]
    fn each_kind_reads_its_own_category_key() {
        let keys = [(Kind::Manga, "default_category"), (Kind::Anime, "default_anime_category")];
        for (kind, category) in keys {
            let store = PrefStore::in_memory();
            let p = BrowsePrefs::new(store.clone(), kind);
            assert_eq!(p.default_category.key(), category, "{kind:?}");
            assert_eq!(p.default_category.get(), -1, "{kind:?}: ask");
            p.default_category.set(4).expect("set");
            assert_eq!(store.get_raw(category), Some(kuro_prefs::values::PrefValue::Int(4)), "{kind:?}: the kind's own key is written");
        }
    }

    #[test]
    fn c_and_posix_locales_default_to_english() {
        for locale in ["C", "POSIX", "C.UTF-8"] {
            let langs = default_enabled_languages(locale);
            assert!(langs.contains("en"), "{locale}");
            assert!(!langs.contains("c") && !langs.contains("posix"), "{locale}");
        }
    }

    #[test]
    fn unknown_keys_in_the_settings_file_survive_a_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("config");
        let state = dir.path().join("state");
        std::fs::create_dir_all(&config).expect("config dir");
        std::fs::create_dir_all(&state).expect("state dir");
        std::fs::write(kuro_prefs::toml_backend::config_file(&config), "[sources]\nfuture_key = \"kept\"\n").expect("seed");
        {
            let backend = TomlBackend::open(&config, &state).expect("open");
            let store = PrefStore::new(Box::new(backend)).expect("store");
            let p = BrowsePrefs::new(store.clone(), Kind::Manga);
            p.default_category.set(3).expect("set");
            store.flush().expect("flush");
        }
        let text = std::fs::read_to_string(kuro_prefs::toml_backend::config_file(&config)).expect("read");
        assert!(text.contains("future_key"), "unknown key dropped:\n{text}");
    }
}
