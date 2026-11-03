//! T2-CODEC-01 to T2-CODEC-08: the flag codec and the display-mode tokens.

use kuro_domain::library::flags::{contains, merge};
use kuro_platform::kind::Kind;
use kuro_platform::sort::{Direction, LibrarySort, SORT_MASK, SortType};
use kuro_prefs::library::LibraryDisplayMode;

const KINDS: [Kind; 2] = [Kind::Manga, Kind::Anime];

fn sort(ty: SortType, dir: Direction) -> LibrarySort {
    LibrarySort { ty, dir }
}

#[test]
fn t2_codec_01_counts() {
    assert_eq!(LibraryDisplayMode::ALL.len(), 4);
    assert_eq!(SortType::types(Kind::Manga).len(), 10);
    assert_eq!(SortType::types(Kind::Anime).len(), 11);
    assert_eq!(Direction::ALL.len(), 2);
}

#[test]
fn t2_codec_02_merge_keeps_the_bits_outside_the_mask() {
    let date_added_asc = sort(SortType::DateAdded, Direction::Ascending);
    assert_eq!(date_added_asc.flag(), 0b0101_1100);
    // bits 0, 1 and 7 are outside the sort mask and must survive
    let old = 0b1000_0011;
    assert_eq!(merge(old, date_added_asc.flag(), SORT_MASK), 0b1101_1111);
    assert_eq!(date_added_asc.apply_to(old), 0b1101_1111);
    assert_eq!(date_added_asc.apply_to(0), 0b0101_1100);
    assert!(contains(0b1101_1111, date_added_asc.flag(), SORT_MASK));
}

#[test]
fn t2_codec_03_decode_table() {
    let (alpha_asc, alpha_desc) = (sort(SortType::Alphabetical, Direction::Ascending), sort(SortType::Alphabetical, Direction::Descending));
    assert_eq!(LibrarySort::deserialize(Kind::Manga, ""), alpha_asc);
    assert_eq!(LibrarySort::deserialize(Kind::Anime, "UNSEEN_COUNT,DESCENDING"), sort(SortType::UnreadCount, Direction::Descending));
    assert_eq!(LibrarySort::deserialize(Kind::Manga, "FOO,ASCENDING"), alpha_asc);
    assert_eq!(LibrarySort::deserialize(Kind::Manga, "LAST_READ"), alpha_asc, "no comma reads the default");
    assert_eq!(LibrarySort::from_flags(Kind::Manga, Some(0)), alpha_desc);
    assert_eq!(LibrarySort::from_flags(Kind::Manga, Some(0x40)), alpha_asc);
    assert_eq!(LibrarySort::from_flags(Kind::Manga, Some(124)), sort(SortType::Random, Direction::Ascending));
    assert_eq!(LibrarySort::from_flags(Kind::Anime, Some(48)), sort(SortType::AiringTime, Direction::Descending));
    assert_eq!(LibrarySort::from_flags(Kind::Manga, Some(48)), alpha_desc, "no manga type has the bits 110000");
}

#[test]
fn t2_codec_04_merge_with_a_base_is_not_the_base() {
    let base = 0b0000_1100;
    let merged = sort(SortType::DateAdded, Direction::Ascending).apply_to(base);
    assert_eq!(merged, 0b0101_1100);
    assert_ne!(merged, base);
}

#[test]
fn t2_codec_05_default_flags() {
    assert_eq!(LibrarySort::DEFAULT.flag(), 0b0100_0000);
}

#[test]
fn t2_codec_06_every_type_and_direction_round_trips_through_flags() {
    for kind in KINDS {
        for ty in SortType::types(kind) {
            for dir in Direction::ALL {
                let original = sort(*ty, dir);
                let decoded = LibrarySort::from_flags(kind, Some(original.flag()));
                assert_eq!(decoded, original, "{kind:?} {ty:?} {dir:?}");
            }
        }
    }
}

#[test]
fn t2_codec_07_every_token_round_trips() {
    for kind in KINDS {
        for ty in SortType::types(kind) {
            for dir in Direction::ALL {
                let original = sort(*ty, dir);
                assert_eq!(LibrarySort::deserialize(kind, &original.serialize(kind)), original, "{kind:?} {ty:?} {dir:?}");
            }
        }
    }
    assert_eq!(LibrarySort::deserialize(Kind::Manga, "RANDOM,ASCENDING"), sort(SortType::Random, Direction::Ascending));
}

#[test]
fn t2_codec_08_display_mode_tokens() {
    for (mode, token) in [
        (LibraryDisplayMode::CompactGrid, "COMPACT_GRID"),
        (LibraryDisplayMode::ComfortableGrid, "COMFORTABLE_GRID"),
        (LibraryDisplayMode::CoverOnlyGrid, "COVER_ONLY_GRID"),
        (LibraryDisplayMode::List, "LIST"),
    ] {
        assert_eq!(mode.serialize(), token);
        assert_eq!(LibraryDisplayMode::deserialize(token), mode);
    }
    assert_eq!(LibraryDisplayMode::deserialize("SPIRAL"), LibraryDisplayMode::CompactGrid);
}
