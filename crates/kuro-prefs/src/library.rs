//! The `[library]` preferences: display, sort, columns, filters, badges, categories and counters.
//!
//! Keys and defaults come from the registry. The per-entry chapter, episode and season templates and the
//! "set as default" writers are cut (settings diet). Enum-valued keys have typed codecs: an unknown stored value reads as the default and the
//! stored text is kept, as Aniyomi's `getEnum` and `getObject` do.

use std::borrow::Cow;
use crate::category::category_keys;
use crate::pref::Pref;
use crate::PrefError;
use crate::registry;
use crate::registry::define_prefs;
use kuro_platform::kind::Kind;
use kuro_platform::sort::LibrarySort;
use crate::store::PrefStore;
use crate::tristate::TriState;
use crate::values::{Decoded, PrefType, PrefValue};

define_prefs! {
    /// `[library]` section: the keys of Aniyomi `LibraryPreferences` the settings diet kept.
    /// The per-tracker filter families are methods on this type.
    pub struct LibraryPrefs in "library" {
        display_mode: LibraryDisplayMode = "pref_display_mode_library" => LibraryDisplayMode::CompactGrid;
        manga_sorting_mode: SortSetting<false> = "library_sorting_mode" => SortSetting("ALPHABETICAL,ASCENDING".to_owned());
        anime_sorting_mode: SortSetting<true> = "animelib_sorting_mode" => SortSetting("ALPHABETICAL,ASCENDING".to_owned());
        last_updated_timestamp: i64 = "__APP_STATE_library_update_last_timestamp" => 0;
        auto_update_interval: i32 = "pref_library_update_interval_key" => 0;
        filter_downloaded_anime: TriState = "pref_filter_animelib_downloaded_v2" => TriState::DISABLED;
        filter_downloaded_manga: TriState = "pref_filter_library_downloaded_v2" => TriState::DISABLED;
        filter_unseen: TriState = "pref_filter_animelib_unread_v2" => TriState::DISABLED;
        filter_unread: TriState = "pref_filter_library_unread_v2" => TriState::DISABLED;
        filter_completed_anime: TriState = "pref_filter_animelib_completed_v2" => TriState::DISABLED;
        filter_completed_manga: TriState = "pref_filter_library_completed_v2" => TriState::DISABLED;
        new_manga_updates_count: i32 = "library_unread_updates_count" => 0;
        new_anime_updates_count: i32 = "library_unseen_updates_count" => 0;
        default_anime_category: i32 = "default_anime_category" => -1;
        default_manga_category: i32 = "default_category" => -1;
        last_used_anime_category: i32 = "__APP_STATE_last_used_anime_category" => 0;
        last_used_manga_category: i32 = "__APP_STATE_last_used_category" => 0;
        cover_size: i32 = "library_cover_size" => 160;
    }
}

/// `LibraryDisplayMode` (`LibraryDisplayMode.kt`). One key serves both libraries and is stored as the
/// spelling of [`LibraryDisplayMode::serialize`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum LibraryDisplayMode {
    CompactGrid,
    ComfortableGrid,
    List,
    CoverOnlyGrid,
}

impl LibraryDisplayMode {
    /// Menu order of the four modes.
    pub const ALL: [LibraryDisplayMode; 4] = [Self::CompactGrid, Self::ComfortableGrid, Self::List, Self::CoverOnlyGrid];
    pub const DEFAULT: Self = Self::CompactGrid;

    /// The stored spelling.
    pub const fn serialize(self) -> &'static str {
        match self {
            Self::CompactGrid => "COMPACT_GRID",
            Self::ComfortableGrid => "COMFORTABLE_GRID",
            Self::List => "LIST",
            Self::CoverOnlyGrid => "COVER_ONLY_GRID",
        }
    }

    /// The mode stored as `stored`, or `None` for any other text (lower case and empty included).
    pub fn from_serialized(stored: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.serialize() == stored)
    }

    /// Kotlin `deserialize`: the mode stored as `stored`, and [`LibraryDisplayMode::DEFAULT`] for any other text.
    pub fn deserialize(stored: &str) -> Self {
        Self::from_serialized(stored).unwrap_or(Self::DEFAULT)
    }
}

impl PrefType for LibraryDisplayMode {
    fn encode(&self) -> PrefValue {
        PrefValue::Str(self.serialize().to_owned())
    }

    fn decode(v: &PrefValue) -> Decoded<Self> {
        match v {
            PrefValue::Str(s) => Self::from_serialized(s).map_or(Decoded::Invalid, Decoded::Ok),
            _ => Decoded::WrongType,
        }
    }
}

/// Type tokens of the manga library sort (`MangaLibrarySort.Serializer`).
pub const MANGA_SORT_TOKENS: &[&str] = &[
    "ALPHABETICAL",
    "LAST_READ",
    "LAST_MANGA_UPDATE",
    "UNREAD_COUNT",
    "TOTAL_CHAPTERS",
    "LATEST_CHAPTER",
    "CHAPTER_FETCH_DATE",
    "DATE_ADDED",
    "TRACKER_MEAN",
    "RANDOM",
];

/// Type tokens of the anime library sort (`AnimeLibrarySort.Serializer`).
pub const ANIME_SORT_TOKENS: &[&str] = &[
    "ALPHABETICAL",
    "LAST_SEEN",
    "LAST_ANIME_UPDATE",
    "UNSEEN_COUNT",
    "TOTAL_EPISODES",
    "LATEST_EPISODE",
    "EPISODE_FETCH_DATE",
    "DATE_ADDED",
    "TRACKER_MEAN",
    "AIRING_TIME",
    "RANDOM",
];

/// A stored `<TYPE>,<DIRECTION>` library sort such as `LAST_READ,DESCENDING`. `ANIME` picks the type tokens of
/// that kind. Text with an unknown type, an unknown direction or no comma is `Invalid`: the default is read and
/// the stored text stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortSetting<const ANIME: bool>(pub String);

impl<const ANIME: bool> PrefType for SortSetting<ANIME> {
    fn encode(&self) -> PrefValue {
        PrefValue::Str(self.0.clone())
    }

    /// Read like Kotlin's `getObject`: the text is parsed and the value comes back in canonical
    /// form (an unknown type reads as the default type, see [`kuro_platform::sort::LibrarySort::parse`]).
    /// The stored text is not rewritten by a read. Empty text or text without a comma is `Invalid`.
    fn decode(v: &PrefValue) -> Decoded<Self> {
        let PrefValue::Str(s) = v else {
            return Decoded::WrongType;
        };
        let kind = Kind::from_anime(ANIME);
        LibrarySort::parse(kind, s).map_or(Decoded::Invalid, |sort| Decoded::Ok(Self(sort.serialize(kind))))
    }
}

/// A handle for a family key, with its default taken from the registry row for that key.
/// Panics when the key is not registered or its default does not decode as `T`. Both are
/// programming errors, and the test that builds [`LibraryPrefs`] catches them.
fn reg<T: PrefType>(store: &PrefStore, key: impl Into<Cow<'static, str>>) -> Pref<T> {
    let key = key.into();
    let (meta, _) =
        registry::lookup(&key).unwrap_or_else(|| panic!("unregistered preference key {key}"));
    let default = match T::decode(&meta.default.value()) {
        Decoded::Ok(v) => v,
        _ => panic!("registry default of {key} does not match the handle type"),
    };
    store.pref(key, default)
}

impl LibraryPrefs {
    /// The default category of `kind` (`-1` asks every time).
    pub fn default_category(&self, kind: Kind) -> &Pref<i32> {
        match kind {
            Kind::Manga => &self.default_manga_category,
            Kind::Anime => &self.default_anime_category,
        }
    }

    pub fn last_used_category(&self, kind: Kind) -> &Pref<i32> {
        match kind {
            Kind::Manga => &self.last_used_manga_category,
            Kind::Anime => &self.last_used_anime_category,
        }
    }

    pub fn new_updates_count(&self, kind: Kind) -> &Pref<i32> {
        match kind {
            Kind::Manga => &self.new_manga_updates_count,
            Kind::Anime => &self.new_anime_updates_count,
        }
    }

    pub fn filter_downloaded(&self, kind: Kind) -> &Pref<TriState> {
        match kind {
            Kind::Manga => &self.filter_downloaded_manga,
            Kind::Anime => &self.filter_downloaded_anime,
        }
    }

    /// The "unread" filter of manga, the "unseen" filter of anime.
    pub fn filter_unread(&self, kind: Kind) -> &Pref<TriState> {
        match kind {
            Kind::Manga => &self.filter_unread,
            Kind::Anime => &self.filter_unseen,
        }
    }

    pub fn filter_completed(&self, kind: Kind) -> &Pref<TriState> {
        match kind {
            Kind::Manga => &self.filter_completed_manga,
            Kind::Anime => &self.filter_completed_anime,
        }
    }

    pub fn filter_tracked(&self, kind: Kind, tracker_id: i32) -> Pref<TriState> {
        match kind {
            Kind::Manga => self.filter_tracked_manga(tracker_id),
            Kind::Anime => self.filter_tracked_anime(tracker_id),
        }
    }

    /// The library sort of `kind` as canonical `<TYPE>,<DIRECTION>` text (the default when the stored text is invalid).
    pub fn sorting_mode(&self, kind: Kind) -> String {
        match kind {
            Kind::Manga => self.manga_sorting_mode.get().0,
            Kind::Anime => self.anime_sorting_mode.get().0,
        }
    }

    /// Stores `text` as the library sort of `kind` (written as given, like the old per-kind fields).
    pub fn set_sorting_mode(&self, kind: Kind, text: &str) -> Result<(), PrefError> {
        match kind {
            Kind::Manga => self.manga_sorting_mode.set(SortSetting(text.to_owned())),
            Kind::Anime => self.anime_sorting_mode.set(SortSetting(text.to_owned())),
        }
    }

    pub fn filter_tracked_anime(&self, tracker_id: i32) -> Pref<TriState> {
        reg(
            &self.store,
            format!("pref_filter_animelib_tracked_{tracker_id}_v2"),
        )
    }

    pub fn filter_tracked_manga(&self, tracker_id: i32) -> Pref<TriState> {
        reg(
            &self.store,
            format!("pref_filter_library_tracked_{tracker_id}_v2"),
        )
    }
}

/// `LibraryPreferences` companion constants: the device restrictions of library updates.
pub const DEVICE_ONLY_ON_WIFI: &str = "wifi";
pub const DEVICE_NETWORK_NOT_METERED: &str = "network_not_metered";
pub const DEVICE_CHARGING: &str = "ac";

/// Entry restrictions of library updates (manga and anime share the key).
pub const ENTRY_NON_COMPLETED: &str = "manga_ongoing";
pub const ENTRY_HAS_UNVIEWED: &str = "manga_fully_read";
pub const ENTRY_NON_VIEWED: &str = "manga_started";
pub const ENTRY_OUTSIDE_RELEASE_PERIOD: &str = "manga_outside_release_period";

/// Values of the "mark duplicate as read" multi-select preferences.
pub const MARK_DUPLICATE_CHAPTER_READ_NEW: &str = "new";
pub const MARK_DUPLICATE_CHAPTER_READ_EXISTING: &str = "existing";
pub const MARK_DUPLICATE_EPISODE_SEEN_NEW: &str = "new_episode";
pub const MARK_DUPLICATE_EPISODE_SEEN_EXISTING: &str = "existing_episode";

pub const DEFAULT_MANGA_CATEGORY_PREF_KEY: &str = category_keys(Kind::Manga).default;
pub const DEFAULT_ANIME_CATEGORY_PREF_KEY: &str = category_keys(Kind::Anime).default;
pub const LIBRARY_UPDATE_MANGA_CATEGORIES_PREF_KEY: &str = category_keys(Kind::Manga).update;
pub const LIBRARY_UPDATE_ANIME_CATEGORIES_PREF_KEY: &str = category_keys(Kind::Anime).update;
pub const LIBRARY_UPDATE_MANGA_CATEGORIES_EXCLUDE_PREF_KEY: &str = category_keys(Kind::Manga).update_exclude;
pub const LIBRARY_UPDATE_ANIME_CATEGORIES_EXCLUDE_PREF_KEY: &str = category_keys(Kind::Anime).update_exclude;

/// The six keys that hold category ids (`categoryPreferenceKeys`). Backup restore remaps them.
pub fn category_preference_keys() -> [&'static str; 6] {
    [
        DEFAULT_MANGA_CATEGORY_PREF_KEY,
        DEFAULT_ANIME_CATEGORY_PREF_KEY,
        LIBRARY_UPDATE_MANGA_CATEGORIES_PREF_KEY,
        LIBRARY_UPDATE_ANIME_CATEGORIES_PREF_KEY,
        LIBRARY_UPDATE_MANGA_CATEGORIES_EXCLUDE_PREF_KEY,
        LIBRARY_UPDATE_ANIME_CATEGORIES_EXCLUDE_PREF_KEY,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::assert_section_registered;
    use crate::values::PrefValue;

    #[test]
    fn constructing_resolves_every_key_in_the_registry() {
        let lib = LibraryPrefs::new(PrefStore::in_memory());
        assert_eq!(lib.display_mode.key(), "pref_display_mode_library");
        assert_eq!(lib.manga_sorting_mode.key(), "library_sorting_mode");
        assert_eq!(lib.anime_sorting_mode.key(), "animelib_sorting_mode");
        assert_eq!(
            lib.last_updated_timestamp.key(),
            "__APP_STATE_library_update_last_timestamp"
        );
    }

    #[test]
    fn every_field_is_registered_under_the_library_table() {
        let lib = LibraryPrefs::new(PrefStore::in_memory());
        assert_eq!(LibraryPrefs::TABLE, "library");
        assert_eq!(LibraryPrefs::FIELDS.len(), 18, "the settings diet kept 17 keys here, the cover size came back, plus the tracked-filter families");
        assert_section_registered(LibraryPrefs::TABLE, LibraryPrefs::FIELDS, &lib.defaults());
    }

    #[test]
    fn defaults_match_the_registry() {
        let lib = LibraryPrefs::new(PrefStore::in_memory());
        assert_eq!(lib.auto_update_interval.get(), 0);
        assert_eq!(lib.new_manga_updates_count.get(), 0);
        assert_eq!(lib.new_anime_updates_count.get(), 0);
        assert_eq!(lib.default_manga_category.get(), -1);
        assert_eq!(lib.default_anime_category.get(), -1);
        assert_eq!(lib.last_used_manga_category.get(), 0);
        assert_eq!(lib.display_mode.get(), LibraryDisplayMode::CompactGrid);
        assert_eq!(lib.manga_sorting_mode.get().0, "ALPHABETICAL,ASCENDING");
        assert_eq!(lib.filter_downloaded_manga.get(), TriState::DISABLED);
        assert_eq!(lib.filter_completed_anime.get(), TriState::DISABLED);
    }

    #[test]
    fn the_token_lists_are_the_platform_codec_tokens() {
        assert_eq!(kuro_platform::sort::tokens(Kind::Manga), MANGA_SORT_TOKENS);
        assert_eq!(kuro_platform::sort::tokens(Kind::Anime), ANIME_SORT_TOKENS);
    }

    #[test]
    fn category_keys_cover_every_old_list() {
        let old: std::collections::BTreeSet<&str> = [
            "default_category", "library_update_categories", "library_update_categories_exclude",
            "remove_exclude_categories", "download_new_categories", "download_new_categories_exclude",
            "default_anime_category", "animelib_update_categories", "animelib_update_categories_exclude",
            "remove_exclude_anime_categories", "download_new_anime_categories", "download_new_anime_categories_exclude",
        ]
        .into_iter()
        .collect();
        let new: std::collections::BTreeSet<&str> =
            Kind::ALL.into_iter().flat_map(|k| category_keys(k).all()).collect();
        assert_eq!(new, old);
        let six: std::collections::BTreeSet<&str> = category_preference_keys().into_iter().collect();
        assert!(six.is_subset(&old));
        let download: std::collections::BTreeSet<&str> = crate::download::CATEGORY_SET_KEYS.into_iter().collect();
        assert!(download.is_subset(&old));
        assert_eq!(crate::category::category_id_set_keys().len(), 10);
    }

    #[test]
    fn sorting_mode_accessor_matches_the_registry_keys() {
        let store = PrefStore::in_memory();
        let lib = LibraryPrefs::new(store.clone());
        for (kind, key, text) in [
            (Kind::Manga, "library_sorting_mode", "LAST_READ,DESCENDING"),
            (Kind::Anime, "animelib_sorting_mode", "AIRING_TIME,ASCENDING"),
        ] {
            assert_eq!(lib.sorting_mode(kind), "ALPHABETICAL,ASCENDING");
            lib.set_sorting_mode(kind, text).unwrap();
            assert_eq!(store.get_raw(key), Some(PrefValue::Str(text.into())));
            assert_eq!(lib.sorting_mode(kind), text);
        }
        // an unknown type reads as Alphabetical, and an unparsable text reads as the default; the stored text stays
        lib.set_sorting_mode(Kind::Manga, "AIRING_TIME,ASCENDING").unwrap();
        assert_eq!(lib.sorting_mode(Kind::Manga), "ALPHABETICAL,ASCENDING");
        lib.set_sorting_mode(Kind::Manga, "RANDOM").unwrap();
        assert_eq!(lib.sorting_mode(Kind::Manga), "ALPHABETICAL,ASCENDING");
        assert_eq!(store.get_raw("library_sorting_mode"), Some(PrefValue::Str("RANDOM".into())));
    }

    #[test]
    fn the_kind_accessors_pick_the_per_kind_fields() {
        let lib = LibraryPrefs::new(PrefStore::in_memory());
        assert_eq!(lib.default_category(Kind::Manga).key(), "default_category");
        assert_eq!(lib.default_category(Kind::Anime).key(), "default_anime_category");
        assert_eq!(lib.last_used_category(Kind::Anime).key(), "__APP_STATE_last_used_anime_category");
        assert_eq!(lib.new_updates_count(Kind::Manga).key(), "library_unread_updates_count");
        assert_eq!(lib.filter_downloaded(Kind::Anime).key(), "pref_filter_animelib_downloaded_v2");
        assert_eq!(lib.filter_unread(Kind::Manga).key(), "pref_filter_library_unread_v2");
        assert_eq!(lib.filter_unread(Kind::Anime).key(), "pref_filter_animelib_unread_v2");
        assert_eq!(lib.filter_completed(Kind::Manga).key(), "pref_filter_library_completed_v2");
        assert_eq!(lib.filter_tracked(Kind::Anime, 3).key(), "pref_filter_animelib_tracked_3_v2");
    }

    #[test]
    fn tracked_filters_use_the_tracker_id_in_the_key() {
        let store = PrefStore::in_memory();
        let lib = LibraryPrefs::new(store.clone());
        let p = lib.filter_tracked_manga(7);
        assert_eq!(p.key(), "pref_filter_library_tracked_7_v2");
        assert_eq!(p.get(), TriState::DISABLED);
        p.set(TriState::ENABLED_NOT).unwrap();
        assert_eq!(
            store.get_raw("pref_filter_library_tracked_7_v2"),
            Some(PrefValue::Str("ENABLED_NOT".into()))
        );
        assert_eq!(
            lib.filter_tracked_anime(7).key(),
            "pref_filter_animelib_tracked_7_v2"
        );
    }

    #[test]
    fn a_filter_stored_as_another_kind_is_reset() {
        let store = PrefStore::in_memory();
        let lib = LibraryPrefs::new(store.clone());
        store.set_raw("pref_filter_library_downloaded_v2", PrefValue::Int(1));
        assert_eq!(lib.filter_downloaded_manga.get(), TriState::DISABLED);
        assert!(!lib.filter_downloaded_manga.is_set());
    }

    /// Table of `p1-data-and-config.md` section 5 (the 19 rows the settings diet kept). Columns: index, key, namespace, Rust field, kind
    /// (`bool`, `int`, `long`, `str`, `set`) and the default in stored form (a set is comma-separated, empty for `{}`).
    const SECTION_5: &str = include_str!("../tests/fixtures/library_prefs_table.tsv");

    fn section_5_rows() -> Vec<Vec<&'static str>> {
        SECTION_5
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split('\t').collect())
            .collect()
    }

    /// The stored key of a section 5 row: app-state keys carry the `__APP_STATE_` prefix.
    fn stored_key(row: &[&str]) -> String {
        if row[2] == "app-state" {
            format!("__APP_STATE_{}", row[1])
        } else {
            row[1].to_owned()
        }
    }

    fn fixture_value(kind: &str, text: &str) -> PrefValue {
        match kind {
            "bool" => PrefValue::Bool(text.parse().unwrap()),
            "int" => PrefValue::Int(text.parse().unwrap()),
            "long" => PrefValue::Long(text.parse().unwrap()),
            "str" => PrefValue::Str(text.to_owned()),
            "set" => PrefValue::Set(text.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect()),
            other => panic!("unknown fixture kind {other}"),
        }
    }

    fn kind_name(kind: registry::Kind) -> &'static str {
        match kind {
            registry::Kind::Bool => "bool",
            registry::Kind::Int => "int",
            registry::Kind::Long => "long",
            registry::Kind::Float => "float",
            registry::Kind::Str => "str",
            registry::Kind::Set => "set",
        }
    }

    #[test]
    fn section_5_table_has_19_accessors_registered_with_their_namespace() {
        let rows = section_5_rows();
        assert_eq!(rows.len(), 19);
        let fields: Vec<&str> = LibraryPrefs::FIELDS.iter().map(|(_, key)| *key).collect();
        for row in &rows {
            let stored = stored_key(row);
            let (meta, _) = registry::lookup(&stored).unwrap_or_else(|| panic!("{stored} not registered"));
            let expected = if row[2] == "app-state" { "AppState" } else { "Normal" };
            assert_eq!(format!("{:?}", meta.ns), expected, "namespace of {stored}");
            assert!(fields.contains(&stored.as_str()) || row[1].contains("{id}"), "{stored} has no LibraryPrefs field");
        }
        let app_state: Vec<&str> = rows.iter().filter(|r| r[2] == "app-state").map(|r| r[1]).collect();
        assert_eq!(app_state, ["library_update_last_timestamp", "last_used_anime_category", "last_used_category"]);
    }

    #[test]
    fn section_5_rows_match_key_kind_and_default() {
        let lib = LibraryPrefs::new(PrefStore::in_memory());
        let handle_defaults = lib.defaults();
        for row in section_5_rows() {
            let stored = stored_key(&row);
            let (meta, _) = registry::lookup(&stored).unwrap_or_else(|| panic!("{stored} not registered"));
            let expected = fixture_value(row[4], row[5]);
            assert_eq!(kind_name(meta.kind), row[4], "registry kind of {stored}");
            assert_eq!(meta.default.value(), expected, "registry default of {stored}");
            if !stored.contains("{id}") {
                let (_, handle) = handle_defaults
                    .iter()
                    .find(|(key, _)| *key == stored)
                    .unwrap_or_else(|| panic!("{stored} has no LibraryPrefs handle"));
                assert_eq!(handle, &expected, "LibraryPrefs default and kind of {stored}");
            }
        }
    }

    #[test]
    fn the_category_key_list_follows_the_table() {
        assert_eq!(category_preference_keys().len(), 6);
        assert!(category_preference_keys().contains(&"default_category"));
    }

    #[test]
    fn the_dropped_swipe_keys_have_no_accessor_and_no_registry_row() {
        let fields: Vec<&str> = LibraryPrefs::FIELDS.iter().map(|(_, key)| *key).collect();
        for key in [
            "pref_chapter_swipe_end_action",
            "pref_chapter_swipe_start_action",
            "pref_episode_swipe_end_action",
            "pref_episode_swipe_start_action",
        ] {
            assert!(!fields.contains(&key), "{key} has a LibraryPrefs field");
            assert!(registry::lookup(key).is_none(), "{key} is registered");
            assert!(registry::is_dropped(key), "{key} is not in DROPPED_KEYS");
        }
    }

    #[test]
    fn known_enum_names_decode_to_their_variants() {
        let store = PrefStore::in_memory();
        let lib = LibraryPrefs::new(store.clone());
        store.set_raw("pref_filter_library_downloaded_v2", PrefValue::Str("ENABLED_NOT".into()));
        assert_eq!(lib.filter_downloaded_manga.get(), TriState::ENABLED_NOT);
        store.set_raw("pref_display_mode_library", PrefValue::Str("COVER_ONLY_GRID".into()));
        assert_eq!(lib.display_mode.get(), LibraryDisplayMode::CoverOnlyGrid);
    }

    #[test]
    fn an_unknown_tristate_name_reads_the_default_and_stay_stored() {
        let store = PrefStore::in_memory();
        let lib = LibraryPrefs::new(store.clone());
        store.set_raw("pref_filter_library_downloaded_v2", PrefValue::Str("BANANA".into()));
        assert_eq!(lib.filter_downloaded_manga.get(), TriState::DISABLED, "TriState BANANA reads as DISABLED");
        assert_eq!(store.get_raw("pref_filter_library_downloaded_v2"), Some(PrefValue::Str("BANANA".into())));
    }

    #[test]
    fn unknown_display_mode_and_sort_strings_read_the_default_and_stay_stored() {
        let store = PrefStore::in_memory();
        let lib = LibraryPrefs::new(store.clone());
        store.set_raw("pref_display_mode_library", PrefValue::Str("list".into()));
        assert_eq!(lib.display_mode.get(), LibraryDisplayMode::CompactGrid, "lower case reads as compact");
        assert_eq!(store.get_raw("pref_display_mode_library"), Some(PrefValue::Str("list".into())));

        // Edge cases E3 to E5: an unknown type keeps the direction, only the exact word ASCENDING is ascending,
        // and text without a comma or empty text reads as the default.
        let cases = [
            ("Banana,ASCENDING", "ALPHABETICAL,ASCENDING"),
            ("LAST_READ,SIDEWAYS", "LAST_READ,DESCENDING"),
            ("LAST_READ", "ALPHABETICAL,ASCENDING"),
            ("AIRING_TIME,DESCENDING", "ALPHABETICAL,DESCENDING"),
            ("", "ALPHABETICAL,ASCENDING"),
        ];
        for (stored, expected) in cases {
            store.set_raw("library_sorting_mode", PrefValue::Str(stored.into()));
            assert_eq!(lib.manga_sorting_mode.get().0, expected, "manga sort {stored:?}");
            assert_eq!(store.get_raw("library_sorting_mode"), Some(PrefValue::Str(stored.into())));
        }
        store.set_raw("animelib_sorting_mode", PrefValue::Str("AIRING_TIME,DESCENDING".into()));
        assert_eq!(lib.anime_sorting_mode.get().0, "AIRING_TIME,DESCENDING", "airing time is an anime sort");
        store.set_raw("library_sorting_mode", PrefValue::Str("LAST_READ,DESCENDING".into()));
        assert_eq!(lib.manga_sorting_mode.get().0, "LAST_READ,DESCENDING");
    }

    #[test]
    fn display_mode_text_that_is_unknown_reads_compact_grid() {
        assert_eq!(LibraryDisplayMode::deserialize("COVER_ONLY_GRID"), LibraryDisplayMode::CoverOnlyGrid);
        assert_eq!(LibraryDisplayMode::deserialize("cover"), LibraryDisplayMode::CompactGrid);
        assert_eq!(LibraryDisplayMode::deserialize(""), LibraryDisplayMode::CompactGrid);
    }
}
