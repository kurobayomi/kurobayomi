//! The ONE library sort codec (docs/design/unified-model.md section 1.1): types, flag layout and string codec for both kinds (`MangaLibrarySortMode.kt`,
//! `AnimeLibrarySortMode.kt`). One `SortType` serves both kinds: the flag numbers are shared, only the
//! stored tokens differ, and `AiringTime` exists for anime alone.

use crate::kind::Kind;

/// Bits 2 to 5: the sort type.
pub const SORT_TYPE_MASK: i64 = 0b0011_1100;
/// Bit 6: the direction (set = ascending).
pub const SORT_DIR_MASK: i64 = 0b0100_0000;
/// Both sort fields together (`0x7C`).
pub const SORT_MASK: i64 = SORT_TYPE_MASK | SORT_DIR_MASK;
/// Alphabetical, ascending: the seed of `categories.flags`.
pub const DEFAULT_FLAGS: i64 = SORT_DIR_MASK;

/// `(value & !mask) | (flag & mask)`.
const fn merge(value: i64, flag: i64, mask: i64) -> i64 {
    (value & !mask) | (flag & mask)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Direction {
    Ascending,
    Descending,
}

impl Direction {
    pub const ALL: [Direction; 2] = [Direction::Ascending, Direction::Descending];

    pub const fn flag(self) -> i64 {
        match self {
            Self::Ascending => SORT_DIR_MASK,
            Self::Descending => 0,
        }
    }

    pub const fn from_flags(flags: i64) -> Self {
        if flags & SORT_DIR_MASK != 0 { Self::Ascending } else { Self::Descending }
    }

    /// Stored token: `ASCENDING` or `DESCENDING`.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Ascending => "ASCENDING",
            Self::Descending => "DESCENDING",
        }
    }

    /// Exact match on `ASCENDING`; anything else is `Descending` (upstream `if ... else`).
    pub fn from_token(token: &str) -> Self {
        if token == "ASCENDING" { Self::Ascending } else { Self::Descending }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum SortType {
    Alphabetical,
    /// `LastRead` for manga, `LastSeen` for anime.
    LastActivity,
    /// `LastUpdate` (manga) / `LastAnimeUpdate` (anime).
    LastUpdate,
    /// `UnreadCount` (manga) / `UnseenCount` (anime).
    UnreadCount,
    /// `TotalChapters` / `TotalEpisodes`.
    TotalItems,
    /// `LatestChapter` / `LatestEpisode`.
    LatestItem,
    /// `ChapterFetchDate` / `EpisodeFetchDate`.
    ItemFetchDate,
    DateAdded,
    TrackerMean,
    /// Anime only.
    AiringTime,
    Random,
}

const MANGA_TYPES: [SortType; 10] = [
    SortType::Alphabetical,
    SortType::LastActivity,
    SortType::LastUpdate,
    SortType::UnreadCount,
    SortType::TotalItems,
    SortType::LatestItem,
    SortType::ItemFetchDate,
    SortType::DateAdded,
    SortType::TrackerMean,
    SortType::Random,
];

const ANIME_TYPES: [SortType; 11] = [
    SortType::Alphabetical,
    SortType::LastActivity,
    SortType::LastUpdate,
    SortType::UnreadCount,
    SortType::TotalItems,
    SortType::LatestItem,
    SortType::ItemFetchDate,
    SortType::DateAdded,
    SortType::TrackerMean,
    SortType::AiringTime,
    SortType::Random,
];

impl SortType {
    /// The types of one kind, in declaration order (`MangaLibrarySort.types` / `AnimeLibrarySort.types`).
    pub const fn types(kind: Kind) -> &'static [SortType] {
        match kind {
            Kind::Manga => &MANGA_TYPES,
            Kind::Anime => &ANIME_TYPES,
        }
    }

    /// The type's bits inside `SORT_TYPE_MASK`.
    pub const fn flag(self) -> i64 {
        match self {
            Self::Alphabetical => 0b0000_0000,
            Self::LastActivity => 0b0000_0100,
            Self::LastUpdate => 0b0000_1000,
            Self::UnreadCount => 0b0000_1100,
            Self::TotalItems => 0b0001_0000,
            Self::LatestItem => 0b0001_0100,
            Self::ItemFetchDate => 0b0001_1000,
            Self::DateAdded => 0b0001_1100,
            Self::TrackerMean => 0b0010_0000,
            Self::AiringTime => 0b0011_0000,
            Self::Random => 0b0011_1100,
        }
    }

    /// `AiringTime` is anime only; every other type is valid for both kinds.
    pub const fn valid_for(self, kind: Kind) -> bool {
        !matches!((self, kind), (Self::AiringTime, Kind::Manga))
    }

    /// Stored token of this type for `kind`, or `None` when the kind does not have the type.
    pub const fn token(self, kind: Kind) -> Option<&'static str> {
        Some(match (kind, self) {
            (_, Self::Alphabetical) => "ALPHABETICAL",
            (_, Self::DateAdded) => "DATE_ADDED",
            (_, Self::TrackerMean) => "TRACKER_MEAN",
            (_, Self::Random) => "RANDOM",
            (Kind::Manga, Self::LastActivity) => "LAST_READ",
            (Kind::Manga, Self::LastUpdate) => "LAST_MANGA_UPDATE",
            (Kind::Manga, Self::UnreadCount) => "UNREAD_COUNT",
            (Kind::Manga, Self::TotalItems) => "TOTAL_CHAPTERS",
            (Kind::Manga, Self::LatestItem) => "LATEST_CHAPTER",
            (Kind::Manga, Self::ItemFetchDate) => "CHAPTER_FETCH_DATE",
            (Kind::Manga, Self::AiringTime) => return None,
            (Kind::Anime, Self::LastActivity) => "LAST_SEEN",
            (Kind::Anime, Self::LastUpdate) => "LAST_ANIME_UPDATE",
            (Kind::Anime, Self::UnreadCount) => "UNSEEN_COUNT",
            (Kind::Anime, Self::TotalItems) => "TOTAL_EPISODES",
            (Kind::Anime, Self::LatestItem) => "LATEST_EPISODE",
            (Kind::Anime, Self::ItemFetchDate) => "EPISODE_FETCH_DATE",
            (Kind::Anime, Self::AiringTime) => "AIRING_TIME",
        })
    }

    /// Exact, case-sensitive token lookup for `kind`.
    pub fn from_token(kind: Kind, token: &str) -> Option<Self> {
        Self::types(kind).iter().copied().find(|t| t.token(kind) == Some(token))
    }

    /// `valueOf(flag)`: the type whose bits match `flags & SORT_TYPE_MASK`, or `Alphabetical` when none does
    /// (the unassigned bit patterns).
    pub fn from_flags(kind: Kind, flags: i64) -> Self {
        Self::types(kind).iter().copied().find(|t| t.flag() == flags & SORT_TYPE_MASK).unwrap_or(Self::Alphabetical)
    }
}

/// A sort type and direction: the value of `MangaLibrarySort` / `AnimeLibrarySort`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LibrarySort {
    pub ty: SortType,
    pub dir: Direction,
}

impl LibrarySort {
    pub const DEFAULT: Self = Self { ty: SortType::Alphabetical, dir: Direction::Ascending };

    /// `type + direction`: the flag word of this sort alone (`MangaLibrarySort.flag`).
    pub const fn flag(self) -> i64 {
        merge(self.ty.flag(), self.dir.flag(), SORT_DIR_MASK)
    }

    /// `valueOf(flags)`: `None` is the default sort; otherwise the type and direction are decoded.
    pub fn from_flags(kind: Kind, flags: Option<i64>) -> Self {
        match flags {
            None => Self::DEFAULT,
            Some(f) => Self { ty: SortType::from_flags(kind, f), dir: Direction::from_flags(f) },
        }
    }

    pub fn is_ascending(self) -> bool {
        self.dir == Direction::Ascending
    }

    /// `category.flags + type + direction`: replaces the sort bits (`0x7C`) and keeps every other bit.
    pub const fn apply_to(self, category_flags: i64) -> i64 {
        merge(merge(category_flags, self.ty.flag(), SORT_TYPE_MASK), self.dir.flag(), SORT_DIR_MASK)
    }

    /// `"<TYPE>,<DIRECTION>"`, for example `LAST_READ,ASCENDING`.
    pub fn serialize(self, kind: Kind) -> String {
        let token = self.ty.token(kind).unwrap_or("ALPHABETICAL");
        format!("{token},{}", self.dir.token())
    }

    /// Lenient decode (never fails). A string with no comma (`"LAST_READ"`) is the default sort, as
    /// upstream's `values[1]` exception is caught.
    pub fn deserialize(kind: Kind, stored: &str) -> Self {
        if stored.is_empty() {
            return Self::DEFAULT;
        }
        let mut parts = stored.split(',');
        let token = parts.next().unwrap_or_default();
        let Some(direction) = parts.next() else {
            return Self::DEFAULT;
        };
        Self {
            ty: SortType::from_token(kind, token).unwrap_or(SortType::Alphabetical),
            dir: Direction::from_token(direction),
        }
    }

    /// Strict decode for the preference decoder: `None` for empty text or text without a comma; an unknown type is
    /// Alphabetical and only the exact word `ASCENDING` is ascending.
    pub fn parse(kind: Kind, stored: &str) -> Option<Self> {
        let (token, direction) = stored.split_once(',')?;
        Some(Self {
            ty: SortType::from_token(kind, token).unwrap_or(SortType::Alphabetical),
            dir: Direction::from_token(direction),
        })
    }
}

/// The stored tokens of one kind, in declaration order.
pub fn tokens(kind: Kind) -> Vec<&'static str> {
    SortType::types(kind).iter().filter_map(|t| t.token(kind)).collect()
}

/// Flags of a stored sort such as `RANDOM,ASCENDING`; text without a direction gives [`DEFAULT_FLAGS`].
pub fn flags_from_text(kind: Kind, text: &str) -> i64 {
    LibrarySort::deserialize(kind, text).flag()
}

/// The stored text of a sort's flags, the inverse of [`flags_from_text`].
pub fn text_from_flags(kind: Kind, flags: i64) -> String {
    LibrarySort::from_flags(kind, Some(flags)).serialize(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::{Anime, Manga};

    #[test]
    fn type_counts_and_flag_numbers_follow_the_table() {
        assert_eq!(SortType::types(Manga).len(), 10);
        assert_eq!(SortType::types(Anime).len(), 11);
        let flags: Vec<i64> = SortType::types(Anime).iter().map(|t| t.flag()).collect();
        assert_eq!(flags, vec![0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C, 0x20, 0x30, 0x3C]);
        assert_eq!(LibrarySort { ty: SortType::Random, dir: Direction::Ascending }.flag(), 0x7C);
        assert_eq!(LibrarySort { ty: SortType::Random, dir: Direction::Descending }.flag(), 0x3C);
    }

    #[test]
    fn airing_time_exists_for_anime_only() {
        assert!(!SortType::AiringTime.valid_for(Manga));
        assert!(SortType::AiringTime.valid_for(Anime));
        assert_eq!(SortType::AiringTime.token(Manga), None);
    }

    #[test]
    fn unassigned_patterns_decode_to_alphabetical() {
        for f in [0x24, 0x28, 0x2C, 0x30, 0x34, 0x38] {
            assert_eq!(SortType::from_flags(Manga, f), SortType::Alphabetical, "manga {f:#x}");
        }
        for f in [0x24, 0x28, 0x2C, 0x34, 0x38] {
            assert_eq!(SortType::from_flags(Anime, f), SortType::Alphabetical, "anime {f:#x}");
        }
        assert_eq!(SortType::from_flags(Anime, 0x30), SortType::AiringTime);
        assert_eq!(SortType::from_flags(Manga, 0x30), SortType::Alphabetical);
    }

    #[test]
    fn decode_worked_examples() {
        let dec = |k, f| LibrarySort::from_flags(k, Some(f));
        assert_eq!(dec(Manga, 0), LibrarySort { ty: SortType::Alphabetical, dir: Direction::Descending });
        assert_eq!(dec(Manga, 64), LibrarySort::DEFAULT);
        assert_eq!(dec(Manga, 0x3C), LibrarySort { ty: SortType::Random, dir: Direction::Descending });
        assert_eq!(dec(Manga, 255), LibrarySort { ty: SortType::Random, dir: Direction::Ascending });
        assert_eq!(dec(Manga, 128), LibrarySort { ty: SortType::Alphabetical, dir: Direction::Descending });
        assert_eq!(dec(Anime, 0x30).ty, SortType::AiringTime);
        assert_eq!(LibrarySort::from_flags(Manga, None), LibrarySort::DEFAULT);
    }

    #[test]
    fn serialize_and_deserialize_follow_the_upstream_tokens() {
        assert_eq!(LibrarySort::DEFAULT.serialize(Manga), "ALPHABETICAL,ASCENDING");
        let last_read = LibrarySort { ty: SortType::LastActivity, dir: Direction::Descending };
        assert_eq!(last_read.serialize(Manga), "LAST_READ,DESCENDING");
        assert_eq!(last_read.serialize(Anime), "LAST_SEEN,DESCENDING");
        assert_eq!(LibrarySort::deserialize(Manga, "LAST_READ"), LibrarySort::DEFAULT);
        assert_eq!(LibrarySort::deserialize(Manga, "LAST_SEEN,ASCENDING"), LibrarySort::DEFAULT);
        assert_eq!(
            LibrarySort::deserialize(Manga, "ALPHABETICAL,ASCENDING"),
            LibrarySort::DEFAULT
        );
        assert_eq!(
            LibrarySort::deserialize(Manga, "LAST_SEEN,ASCENDING"),
            LibrarySort { ty: SortType::Alphabetical, dir: Direction::Ascending }
        );
        assert_eq!(
            LibrarySort::deserialize(Manga, "UNREAD_COUNT,BOGUS"),
            LibrarySort { ty: SortType::UnreadCount, dir: Direction::Descending }
        );
        assert_eq!(LibrarySort::deserialize(Manga, ""), LibrarySort::DEFAULT);
        assert_eq!(LibrarySort::deserialize(Manga, "last_read,ASCENDING").ty, SortType::Alphabetical, "tokens are upper case");
    }

    #[test]
    fn every_token_round_trips_for_its_kind() {
        for kind in Kind::ALL {
            for ty in SortType::types(kind).iter().copied() {
                for dir in Direction::ALL {
                    let sort = LibrarySort { ty, dir };
                    assert_eq!(LibrarySort::deserialize(kind, &sort.serialize(kind)), sort, "{kind:?} {ty:?}");
                }
            }
        }
    }

    #[test]
    fn apply_to_replaces_only_the_sort_bits() {
        let date_added_asc = LibrarySort { ty: SortType::DateAdded, dir: Direction::Ascending };
        assert_eq!(date_added_asc.apply_to(0x81), 0xDD);
        assert_eq!(date_added_asc.apply_to(0x0), 0x5C);
        assert_eq!(date_added_asc.apply_to(0x1), 0x5D, "bit 0 (chapter sort direction) is kept");
    }

    #[test]
    fn the_flag_helpers_match_the_old_manga_codec_vectors() {
        assert_eq!(DEFAULT_FLAGS, 0x40);
        assert_eq!(flags_from_text(Manga, ""), DEFAULT_FLAGS);
        assert_eq!(flags_from_text(Manga, "RANDOM"), DEFAULT_FLAGS, "no direction");
        assert_eq!(text_from_flags(Manga, flags_from_text(Manga, "NOPE,ASCENDING")), "ALPHABETICAL,ASCENDING");
        assert_eq!(text_from_flags(Manga, flags_from_text(Manga, "DATE_ADDED,OTHER")), "DATE_ADDED,DESCENDING");
        assert_eq!(text_from_flags(Manga, 0b0011_1000), "ALPHABETICAL,DESCENDING");
        assert_eq!(flags_from_text(Anime, "AIRING_TIME,ASCENDING"), 0x30 | 0x40);
        assert_eq!(flags_from_text(Manga, "AIRING_TIME,ASCENDING"), 0x40, "anime only");
        for kind in Kind::ALL {
            for token in tokens(kind) {
                for dir in ["ASCENDING", "DESCENDING"] {
                    let s = format!("{token},{dir}");
                    assert_eq!(text_from_flags(kind, flags_from_text(kind, &s)), s);
                }
            }
        }
    }

    #[test]
    fn tokens_lists_one_per_type_and_parse_is_strict() {
        assert_eq!(tokens(Manga).len(), 10);
        assert_eq!(tokens(Anime).len(), 11);
        assert_eq!(tokens(Manga)[1], "LAST_READ");
        assert_eq!(tokens(Anime)[1], "LAST_SEEN");
        assert_eq!(LibrarySort::parse(Manga, ""), None);
        assert_eq!(LibrarySort::parse(Manga, "ALPHABETICAL"), None);
        assert_eq!(LibrarySort::parse(Manga, "FOO,DESCENDING").unwrap().ty, SortType::Alphabetical);
        assert_eq!(LibrarySort::parse(Manga, "LAST_READ,asc").unwrap().dir, Direction::Descending);
        for kind in Kind::ALL {
            for ty in SortType::types(kind).iter().copied() {
                for dir in Direction::ALL {
                    let s = LibrarySort { ty, dir };
                    assert_eq!(LibrarySort::parse(kind, &s.serialize(kind)), Some(s));
                }
            }
        }
    }
}
