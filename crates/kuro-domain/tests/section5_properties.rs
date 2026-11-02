//! Property tests of the section 5 accessors and the flag algebra (T-08a-040). Each property runs 256 cases.

use kuro_platform::kind::Kind;
use kuro_domain::library::flags::{contains, merge};
use kuro_platform::sort::{Direction, LibrarySort, SortType};
use kuro_prefs::library::{LibraryDisplayMode, LibraryPrefs, MANGA_SORT_TOKENS};
use kuro_prefs::store::PrefStore;
use kuro_prefs::tristate::TriState;
use kuro_prefs::values::PrefValue;
use proptest::prelude::*;

const DISPLAY_KEY: &str = "pref_display_mode_library";

fn tristate(index: usize) -> TriState {
    [TriState::DISABLED, TriState::ENABLED_IS, TriState::ENABLED_NOT][index % 3]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn merge_replaces_exactly_the_masked_bits(value: i64, flag: i64, mask: i64) {
        let merged = merge(value, flag, mask);
        prop_assert_eq!(merged & mask, flag & mask);
        prop_assert_eq!(merged & !mask, value & !mask);
    }

    #[test]
    fn a_merged_flag_is_contained_under_its_mask(value: i64, flag: i64, mask: i64) {
        prop_assert!(contains(merge(value, flag, mask), flag & mask, mask));
    }

    #[test]
    fn every_sort_round_trips_through_its_stored_text_and_its_flags(kind_is_anime: bool, ty in 0usize..11, dir in 0usize..2) {
        let kind = if kind_is_anime { Kind::Anime } else { Kind::Manga };
        let types = SortType::types(kind);
        let sort = LibrarySort { ty: types[ty % types.len()], dir: Direction::ALL[dir] };
        prop_assert_eq!(LibrarySort::deserialize(kind, &sort.serialize(kind)), sort);
        prop_assert_eq!(LibrarySort::from_flags(kind, Some(sort.flag())), sort);
    }

    #[test]
    fn every_display_mode_is_stored_under_its_spelling(index in 0usize..4) {
        let mode = LibraryDisplayMode::ALL[index];
        let prefs = LibraryPrefs::new(PrefStore::in_memory());
        let _ = prefs.display_mode.set(mode);
        prop_assert_eq!(prefs.display_mode.get(), mode);
        prop_assert_eq!(prefs.store().get_raw(DISPLAY_KEY), Some(PrefValue::Str(mode.serialize().to_owned())));
    }

    #[test]
    fn an_unknown_display_mode_reads_the_default_and_stays_stored(text in "[A-Za-z_ ]{0,24}") {
        prop_assume!(LibraryDisplayMode::from_serialized(&text).is_none());
        let prefs = LibraryPrefs::new(PrefStore::in_memory());
        prefs.store().set_raw(DISPLAY_KEY, PrefValue::Str(text.clone()));
        prop_assert_eq!(prefs.display_mode.get(), LibraryDisplayMode::CompactGrid);
        prop_assert_eq!(prefs.store().get_raw(DISPLAY_KEY), Some(PrefValue::Str(text)));
    }

    #[test]
    fn a_tristate_filter_is_off_or_the_predicate_or_its_negation(index in 0usize..3, predicate: bool) {
        let state = tristate(index);
        let expected = match state {
            TriState::DISABLED => true,
            TriState::ENABLED_IS => predicate,
            _ => !predicate,
        };
        prop_assert_eq!(state.apply_filter(|| predicate), expected);
    }

    #[test]
    fn a_tristate_round_trips_through_the_library_prefs(index in 0usize..3) {
        let prefs = LibraryPrefs::new(PrefStore::in_memory());
        let state = tristate(index);
        let _ = prefs.filter_unread.set(state);
        prop_assert_eq!(prefs.filter_unread.get(), state);
    }

    #[test]
    fn integer_accessors_keep_what_they_are_given(int32: i32, int64: i64) {
        let prefs = LibraryPrefs::new(PrefStore::in_memory());
        let _ = prefs.new_manga_updates_count.set(int32);
        prop_assert_eq!(prefs.new_manga_updates_count.get(), int32);
        let _ = prefs.last_updated_timestamp.set(int64);
        prop_assert_eq!(prefs.last_updated_timestamp.get(), int64);
    }

    #[test]
    fn a_known_manga_sort_token_with_a_direction_is_accepted(token in 0usize..MANGA_SORT_TOKENS.len(), descending: bool) {
        let direction = if descending { "DESCENDING" } else { "ASCENDING" };
        let text = format!("{},{direction}", MANGA_SORT_TOKENS[token]);
        let prefs = LibraryPrefs::new(PrefStore::in_memory());
        let _ = prefs.manga_sorting_mode.set(kuro_prefs::library::SortSetting(text.clone()));
        prop_assert_eq!(prefs.manga_sorting_mode.get().0, text);
    }
}
