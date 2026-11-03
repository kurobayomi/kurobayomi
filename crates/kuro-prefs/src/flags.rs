//! Integer bit layouts that default-template preferences and the database share.
//!
//! Values come from `Manga.kt` and `Anime.kt`. The
//! database area stores the same integers in `mangas.chapter_flags` and `animes.episode_flags`.
//! Season flags (section 3.3) are at the end of this file.

/// One field of a flag integer: the bits in `mask` hold `flag`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flag {
    pub flag: i64,
    pub mask: i64,
}

impl Flag {
    pub const fn new(flag: i64, mask: i64) -> Self {
        Self { flag, mask }
    }

    /// Whether `value` carries this flag (Kotlin `Long.contains(flag)`).
    pub const fn is_set_in(self, value: i64) -> bool {
        value & self.mask == self.flag
    }
}

/// Replaces the bits of `current` under `flag.mask` with `flag.flag` (Kotlin `Long + Flag`).
pub const fn with_flag(current: i64, flag: Flag) -> i64 {
    (current & !flag.mask) | (flag.flag & flag.mask)
}

pub const CHAPTER_SORT_DIR_MASK: i64 = 0x1;
pub const CHAPTER_SORT_DESC: i64 = 0x0;
pub const CHAPTER_SORT_ASC: i64 = 0x1;

pub const CHAPTER_UNREAD_MASK: i64 = 0x6;
pub const CHAPTER_SHOW_ALL_UNREAD: i64 = 0x0;
pub const CHAPTER_SHOW_UNREAD: i64 = 0x2;
pub const CHAPTER_SHOW_READ: i64 = 0x4;

pub const CHAPTER_DOWNLOADED_MASK: i64 = 0x18;
pub const CHAPTER_SHOW_DOWNLOADED: i64 = 0x8;
pub const CHAPTER_SHOW_NOT_DOWNLOADED: i64 = 0x10;

pub const CHAPTER_BOOKMARKED_MASK: i64 = 0x60;
pub const CHAPTER_SHOW_BOOKMARKED: i64 = 0x20;
pub const CHAPTER_SHOW_NOT_BOOKMARKED: i64 = 0x40;

pub const CHAPTER_SORTING_MASK: i64 = 0x300;
pub const CHAPTER_SORTING_SOURCE: i64 = 0x0;
pub const CHAPTER_SORTING_NUMBER: i64 = 0x100;
pub const CHAPTER_SORTING_UPLOAD_DATE: i64 = 0x200;
pub const CHAPTER_SORTING_ALPHABET: i64 = 0x300;

pub const CHAPTER_DISPLAY_MASK: i64 = 0x100000;
pub const CHAPTER_DISPLAY_NAME: i64 = 0x0;
pub const CHAPTER_DISPLAY_NUMBER: i64 = 0x100000;

pub const EPISODE_SORT_DIR_MASK: i64 = 0x1;
pub const EPISODE_SORT_DESC: i64 = 0x0;
pub const EPISODE_SORT_ASC: i64 = 0x1;

pub const EPISODE_UNSEEN_MASK: i64 = 0x6;
pub const EPISODE_SHOW_ALL_UNSEEN: i64 = 0x0;
pub const EPISODE_SHOW_UNSEEN: i64 = 0x2;
pub const EPISODE_SHOW_SEEN: i64 = 0x4;

pub const EPISODE_DOWNLOADED_MASK: i64 = 0x18;
pub const EPISODE_SHOW_DOWNLOADED: i64 = 0x8;
pub const EPISODE_SHOW_NOT_DOWNLOADED: i64 = 0x10;

pub const EPISODE_BOOKMARKED_MASK: i64 = 0x60;
pub const EPISODE_SHOW_BOOKMARKED: i64 = 0x20;
pub const EPISODE_SHOW_NOT_BOOKMARKED: i64 = 0x40;

pub const EPISODE_FILLERMARKED_MASK: i64 = 0x180;
pub const EPISODE_SHOW_FILLERMARKED: i64 = 0x80;
pub const EPISODE_SHOW_NOT_FILLERMARKED: i64 = 0x100;

pub const EPISODE_SORTING_MASK: i64 = 0x600;
pub const EPISODE_SORTING_SOURCE: i64 = 0x0;
pub const EPISODE_SORTING_NUMBER: i64 = 0x200;
pub const EPISODE_SORTING_UPLOAD_DATE: i64 = 0x400;
pub const EPISODE_SORTING_ALPHABET: i64 = 0x600;

pub const EPISODE_PREVIEWS_MASK: i64 = 0x800;
pub const EPISODE_SHOW_PREVIEWS: i64 = 0x0;
pub const EPISODE_SHOW_NOT_PREVIEWS: i64 = 0x800;

pub const EPISODE_SUMMARIES_MASK: i64 = 0x1000;
pub const EPISODE_SHOW_SUMMARIES: i64 = 0x0;
pub const EPISODE_SHOW_NOT_SUMMARIES: i64 = 0x1000;

pub const EPISODE_DISPLAY_MASK: i64 = 0x100000;
pub const EPISODE_DISPLAY_NAME: i64 = 0x0;
pub const EPISODE_DISPLAY_NUMBER: i64 = 0x100000;

// Anime `season_flags` (`Anime.kt`). Bit set means on for the overlays.

pub const SEASON_SORT_DIR_MASK: i64 = 0x1;
pub const SEASON_SORT_DESC: i64 = 0x0;
pub const SEASON_SORT_ASC: i64 = 0x1;

pub const SEASON_DOWNLOADED_MASK: i64 = 0x6;
pub const SEASON_SHOW_DOWNLOADED: i64 = 0x2;
pub const SEASON_SHOW_NOT_DOWNLOADED: i64 = 0x4;

pub const SEASON_UNSEEN_MASK: i64 = 0x18;
pub const SEASON_SHOW_UNSEEN: i64 = 0x8;
pub const SEASON_SHOW_SEEN: i64 = 0x10;

pub const SEASON_STARTED_MASK: i64 = 0x60;
pub const SEASON_SHOW_STARTED: i64 = 0x20;
pub const SEASON_SHOW_NOT_STARTED: i64 = 0x40;

pub const SEASON_COMPLETED_MASK: i64 = 0x180;
pub const SEASON_SHOW_COMPLETED: i64 = 0x80;
pub const SEASON_SHOW_NOT_COMPLETED: i64 = 0x100;

pub const SEASON_BOOKMARKED_MASK: i64 = 0x600;
pub const SEASON_SHOW_BOOKMARKED: i64 = 0x200;
pub const SEASON_SHOW_NOT_BOOKMARKED: i64 = 0x400;

pub const SEASON_FILLERMARKED_MASK: i64 = 0x1800;
pub const SEASON_SHOW_FILLERMARKED: i64 = 0x800;
pub const SEASON_SHOW_NOT_FILLERMARKED: i64 = 0x1000;

pub const SEASON_SORT_MASK: i64 = 0xE000;
pub const SEASON_SORT_SOURCE: i64 = 0x0;
pub const SEASON_SORT_SEASON: i64 = 0x2000;
pub const SEASON_SORT_UPLOAD: i64 = 0x4000;
pub const SEASON_SORT_ALPHABET: i64 = 0x6000;
pub const SEASON_SORT_COUNT: i64 = 0x8000;
pub const SEASON_SORT_LAST_SEEN: i64 = 0xA000;
pub const SEASON_SORT_FETCHED: i64 = 0xC000;

/// Grid mode: 0 CompactGrid, 1 ComfortableGrid, 2 CoverOnlyGrid, 3 List. Shifted down by [`SEASON_GRID_DISPLAY_MODE_SHIFT`].
pub const SEASON_GRID_DISPLAY_MODE_MASK: i64 = 0x30000;
pub const SEASON_GRID_DISPLAY_MODE_SHIFT: u32 = 16;
/// Grid size: 0 is automatic. Shifted down by [`SEASON_GRID_DISPLAY_SIZE_SHIFT`].
pub const SEASON_GRID_DISPLAY_SIZE_MASK: i64 = 0x3C0000;
pub const SEASON_GRID_DISPLAY_SIZE_SHIFT: u32 = 18;

pub const SEASON_OVERLAY_DOWNLOADED_MASK: i64 = 0x400000;
pub const SEASON_OVERLAY_UNSEEN_MASK: i64 = 0x800000;
pub const SEASON_OVERLAY_LOCAL_MASK: i64 = 0x1000000;
pub const SEASON_OVERLAY_LANG_MASK: i64 = 0x2000000;
pub const SEASON_OVERLAY_CONT_MASK: i64 = 0x4000000;

pub const SEASON_DISPLAY_MODE_MASK: i64 = 0x8000000;
pub const SEASON_DISPLAY_MODE_SOURCE: i64 = 0x0;
pub const SEASON_DISPLAY_MODE_NUMBER: i64 = 0x8000000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_flag_replaces_only_the_masked_bits() {
        let sorted = with_flag(0x0, Flag::new(CHAPTER_SORTING_ALPHABET, CHAPTER_SORTING_MASK));
        assert_eq!(sorted, 0x300);
        let unread = with_flag(sorted, Flag::new(CHAPTER_SHOW_READ, CHAPTER_UNREAD_MASK));
        assert_eq!(unread, 0x304);
        assert_eq!(with_flag(unread, Flag::new(0, CHAPTER_UNREAD_MASK)), 0x300);
    }

    #[test]
    fn contains_compares_only_the_masked_bits() {
        let show_read = Flag::new(CHAPTER_SHOW_READ, CHAPTER_UNREAD_MASK);
        assert!(show_read.is_set_in(0x304));
        assert!(!show_read.is_set_in(0x302));
    }

    #[test]
    fn episode_masks_do_not_overlap_their_neighbours() {
        let masks = [
            EPISODE_SORT_DIR_MASK,
            EPISODE_UNSEEN_MASK,
            EPISODE_DOWNLOADED_MASK,
            EPISODE_BOOKMARKED_MASK,
            EPISODE_FILLERMARKED_MASK,
            EPISODE_SORTING_MASK,
            EPISODE_PREVIEWS_MASK,
            EPISODE_SUMMARIES_MASK,
            EPISODE_DISPLAY_MASK,
        ];
        let union = masks.iter().fold(0, |acc, m| acc | m);
        assert_eq!(union.count_ones(), masks.iter().map(|m| m.count_ones()).sum::<u32>());
    }
}
