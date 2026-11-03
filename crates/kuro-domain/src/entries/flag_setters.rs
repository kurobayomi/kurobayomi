//! Flag setters of the three entry screens (`SetMangaChapterFlags`, `SetAnimeEpisodeFlags`, `SetAnimeSeasonFlags`).
//! Each setter writes one group of bits. Plain groups are single masked UPDATEs in `kuro-data`; the sort-or-flip
//! and set-all setters read and write the word inside one writer transaction. Pure state functions are separate
//! so the vectors can be checked without a database.

use kuro_data::DataResult;
use kuro_data::entry::EntryRepo;
use kuro_db::flags::{item, season as se};
use kuro_platform::kind::Kind;

use super::season_display::SeasonDisplayMode;
use crate::flags::set_flag;

/// Sort-or-flip state machine. When `flag` is the sort already in use, the direction bit
/// flips; otherwise `flag` becomes the sort, ascending. `sorting_mask` and `dir_mask` are the layout of the word;
/// `asc` is the bit that means ascending.
pub fn sort_or_flip(current: i64, flag: i64, sorting_mask: i64, dir_mask: i64, asc: i64) -> i64 {
    if current & sorting_mask == flag {
        let descending = current & dir_mask == 0;
        let order = if descending { asc } else { 0 };
        set_flag(current, order, dir_mask)
    } else {
        set_flag(set_flag(current, flag, sorting_mask), asc, dir_mask)
    }
}

/// Overlay booleans of a season (bit set = shown).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SeasonOverlay {
    Downloaded,
    Unseen,
    Local,
    Language,
    Continue,
}

impl SeasonOverlay {
    pub const fn mask(self) -> i64 {
        match self {
            Self::Downloaded => se::OVERLAY_DOWNLOADED_MASK,
            Self::Unseen => se::OVERLAY_UNSEEN_MASK,
            Self::Local => se::OVERLAY_LOCAL_MASK,
            Self::Language => se::OVERLAY_LANG_MASK,
            Self::Continue => se::OVERLAY_CONT_MASK,
        }
    }
}

/// Sets exactly the overlay bit: `on` sets it, `off` clears it, no other bit moves.
pub fn overlay_flag(current: i64, overlay: SeasonOverlay, on: bool) -> i64 {
    set_flag(current, if on { overlay.mask() } else { 0 }, overlay.mask())
}

/// Grid size is clamped to `0..=15` (four bits).
pub fn clamp_grid_size(size: i64) -> i64 {
    size.clamp(0, 15)
}

/// The item set-all of either kind: the word built from 0 by applying the nine groups in the documented order (unread,
/// downloaded, bookmarked, fillermarked, sorting, direction, display, previews, summaries), in the unified layout
/// (`kuro_db::flags::item`). A manga word passes 0 for the three anime-only groups.
#[allow(clippy::too_many_arguments)]
pub fn item_all(
    unread: i64,
    downloaded: i64,
    bookmarked: i64,
    fillermarked: i64,
    sorting: i64,
    direction: i64,
    display: i64,
    previews: i64,
    summaries: i64,
) -> i64 {
    let word = set_flag(0, unread, item::UNDONE_MASK);
    let word = set_flag(word, downloaded, item::DOWNLOADED_MASK);
    let word = set_flag(word, bookmarked, item::BOOKMARKED_MASK);
    let word = set_flag(word, fillermarked, item::FILLERMARKED_MASK);
    let word = set_flag(word, sorting, item::SORTING_MASK);
    let word = set_flag(word, direction, item::SORT_DIR_MASK);
    let word = set_flag(word, display, item::DISPLAY_MASK);
    let word = set_flag(word, previews, item::PREVIEWS_MASK);
    set_flag(word, summaries, item::SUMMARIES_MASK)
}

/// The season set-all inputs, in the order of `SetAnimeSeasonFlags.awaitSetAllFlags`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SeasonAll {
    pub download: i64,
    pub unseen: i64,
    pub started: i64,
    pub completed: i64,
    pub bookmarked: i64,
    pub fillermarked: i64,
    pub sorting: i64,
    pub direction: i64,
    pub grid_mode: i64,
    pub grid_size: i64,
    pub overlays: [bool; 5],
    pub display: i64,
}

/// Word of the season set-all, applied from 0.
pub fn season_all(a: SeasonAll) -> i64 {
    let word = set_flag(0, a.download, se::DOWNLOADED_MASK);
    let word = set_flag(word, a.unseen, se::UNSEEN_MASK);
    let word = set_flag(word, a.started, se::STARTED_MASK);
    let word = set_flag(word, a.completed, se::COMPLETED_MASK);
    let word = set_flag(word, a.bookmarked, se::BOOKMARKED_MASK);
    let word = set_flag(word, a.fillermarked, se::FILLERMARKED_MASK);
    let word = set_flag(word, a.sorting, se::SORT_MASK);
    let word = set_flag(word, a.direction, se::SORT_DIR_MASK);
    let word = set_flag(word, a.grid_mode << se::GRID_DISPLAY_MODE_SHIFT, se::GRID_DISPLAY_MODE_MASK);
    let word = set_flag(word, clamp_grid_size(a.grid_size) << se::GRID_DISPLAY_SIZE_SHIFT, se::GRID_DISPLAY_SIZE_MASK);
    let overlays = [SeasonOverlay::Downloaded, SeasonOverlay::Unseen, SeasonOverlay::Local, SeasonOverlay::Language, SeasonOverlay::Continue];
    let word = overlays.iter().zip(a.overlays).fold(word, |w, (o, on)| overlay_flag(w, *o, on));
    set_flag(word, a.display, se::DISPLAY_MODE_MASK)
}

// The setters. Each one is a single function over the shared `EntryRepo`.

/// The groups of the item word a user can set. The last three exist for anime only.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ItemFlagGroup {
    Undone,
    Downloaded,
    Bookmarked,
    Fillermarked,
    Display,
    Previews,
    Summaries,
}

impl ItemFlagGroup {
    pub const fn mask(self) -> i64 {
        match self {
            Self::Undone => item::UNDONE_MASK,
            Self::Downloaded => item::DOWNLOADED_MASK,
            Self::Bookmarked => item::BOOKMARKED_MASK,
            Self::Fillermarked => item::FILLERMARKED_MASK,
            Self::Display => item::DISPLAY_MASK,
            Self::Previews => item::PREVIEWS_MASK,
            Self::Summaries => item::SUMMARIES_MASK,
        }
    }

    pub const fn anime_only(self) -> bool {
        matches!(self, Self::Fillermarked | Self::Previews | Self::Summaries)
    }
}

/// Sets one group of the item word (unified layout) in one masked statement, so two setters on different groups never
/// overwrite each other (EC-23). `Ok(false)` for an entry that does not exist or a group its kind lacks.
pub fn set_item_flag(repo: &EntryRepo, id: i64, group: ItemFlagGroup, flag: i64) -> DataResult<bool> {
    if group.anime_only() && !matches!(repo.by_id(id), Ok(e) if e.kind == Kind::Anime) {
        return Ok(false);
    }
    repo.set_item_flags_masked(id, flag, group.mask())
}

/// Sort-or-flip of the item list (unified layout), read and written in one writer transaction.
pub fn set_item_sort_or_flip(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.update_item_flags(id, |cur| sort_or_flip(cur, flag, item::SORTING_MASK, item::SORT_DIR_MASK, item::SORT_ASC))
}

/// Overwrites the whole item word (see [`item_all`]).
pub fn set_item_all(repo: &EntryRepo, id: i64, word: i64) -> DataResult<bool> {
    repo.update_item_flags(id, |_| word)
}

// Season (anime parent) setters.

/// Sets the season downloaded filter.
pub fn set_season_downloaded(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::DOWNLOADED_MASK)
}

/// Sets the season unseen filter.
pub fn set_season_unseen(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::UNSEEN_MASK)
}

/// Sets the season started filter.
pub fn set_season_started(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::STARTED_MASK)
}

/// Sets the season completed filter.
pub fn set_season_completed(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::COMPLETED_MASK)
}

/// Sets the season bookmarked filter.
pub fn set_season_bookmarked(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::BOOKMARKED_MASK)
}

/// Sets the season fillermarked filter.
pub fn set_season_fillermarked(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::FILLERMARKED_MASK)
}

/// Sort-or-flip of the season list (mask `0xE000`, direction `0x1`).
pub fn set_season_sort_or_flip(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.update_season_flags(id, |cur| sort_or_flip(cur, flag, se::SORT_MASK, se::SORT_DIR_MASK, se::SORT_ASC))
}

/// Grid layout of the season list (bits 16-17).
pub fn set_season_grid_mode(repo: &EntryRepo, id: i64, mode: SeasonDisplayMode) -> DataResult<bool> {
    repo.set_season_flags_masked(id, mode.to_i64() << se::GRID_DISPLAY_MODE_SHIFT, se::GRID_DISPLAY_MODE_MASK)
}

/// Grid size (bits 18-21), clamped to 0..=15.
pub fn set_season_grid_size(repo: &EntryRepo, id: i64, size: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, clamp_grid_size(size) << se::GRID_DISPLAY_SIZE_SHIFT, se::GRID_DISPLAY_SIZE_MASK)
}

/// Sets exactly one overlay bit.
pub fn set_season_overlay(repo: &EntryRepo, id: i64, overlay: SeasonOverlay, on: bool) -> DataResult<bool> {
    repo.set_season_flags_masked(id, if on { overlay.mask() } else { 0 }, overlay.mask())
}

/// Season display mode (`SEASON_DISPLAY_MODE`, one bit).
pub fn set_season_display(repo: &EntryRepo, id: i64, flag: i64) -> DataResult<bool> {
    repo.set_season_flags_masked(id, flag, se::DISPLAY_MODE_MASK)
}

/// Overwrites the whole season word with the 16 inputs.
pub fn set_season_all(repo: &EntryRepo, id: i64, all: SeasonAll) -> DataResult<bool> {
    let word = season_all(all);
    repo.update_season_flags(id, |_| word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_data::entry::Entry;

    #[test]
    fn sort_or_flip_vector_from_zero() {
        let mut w = 0;
        w = sort_or_flip(w, item::SORTING_NUMBER, item::SORTING_MASK, item::SORT_DIR_MASK, item::SORT_ASC);
        assert_eq!(w, 0x200 | 0x1, "new column forces ascending");
        w = sort_or_flip(w, item::SORTING_NUMBER, item::SORTING_MASK, item::SORT_DIR_MASK, item::SORT_ASC);
        assert_eq!(w, 0x200, "same column flips the direction");
    }

    #[test]
    fn a_new_column_forces_ascending_even_when_the_old_one_was_descending() {
        let w = sort_or_flip(0x200, item::SORTING_UPLOAD_DATE, item::SORTING_MASK, item::SORT_DIR_MASK, item::SORT_ASC);
        assert_eq!(w, 0x400 | 0x1);
    }

    #[test]
    fn item_vectors_reproduce_through_the_setters() {
        // 0x201 -> 0x200 (flip) -> 0x202 (unseen = SHOW_UNDONE) -> 0x242 (not bookmarked) -> 0x342 (not fillermarked)
        let mut w = sort_or_flip(0, item::SORTING_NUMBER, item::SORTING_MASK, item::SORT_DIR_MASK, item::SORT_ASC);
        assert_eq!(w, 0x201);
        w = sort_or_flip(w, item::SORTING_NUMBER, item::SORTING_MASK, item::SORT_DIR_MASK, item::SORT_ASC);
        assert_eq!(w, 0x200);
        w = set_flag(w, item::SHOW_UNDONE, item::UNDONE_MASK);
        assert_eq!(w, 0x202);
        w = set_flag(w, item::SHOW_NOT_BOOKMARKED, item::BOOKMARKED_MASK);
        assert_eq!(w, 0x242);
        w = set_flag(w, item::SHOW_NOT_FILLERMARKED, item::FILLERMARKED_MASK);
        assert_eq!(w, 0x342);
        // 0x101B42: number display, not summaries, not previews, number sort, not filler, not bookmarked, unseen.
        let end = item_all(item::SHOW_UNDONE, 0, item::SHOW_NOT_BOOKMARKED, item::SHOW_NOT_FILLERMARKED, item::SORTING_NUMBER, 0, item::DISPLAY_NUMBER, item::SHOW_NOT_PREVIEWS, item::SHOW_NOT_SUMMARIES);
        assert_eq!(end, 0x101B42);
    }

    #[test]
    fn season_grid_and_overlay_vector() {
        let all = SeasonAll {
            download: 0,
            unseen: 0,
            started: 0,
            completed: 0,
            bookmarked: 0,
            fillermarked: 0,
            sorting: 0,
            direction: 0,
            grid_mode: SeasonDisplayMode::List.to_i64(),
            grid_size: 5,
            overlays: [false, true, true, false, true],
            display: 0,
        };
        assert_eq!(season_all(all), 0x5970000);
    }

    #[test]
    fn overlay_setters_touch_exactly_their_bit() {
        assert_eq!(overlay_flag(0, SeasonOverlay::Downloaded, true), 0x400000);
        assert_eq!(overlay_flag(0x400000 | 0x7, SeasonOverlay::Downloaded, false), 0x7);
        assert_eq!(overlay_flag(0, SeasonOverlay::Unseen, true), 0x800000);
        assert_eq!(overlay_flag(0, SeasonOverlay::Local, true), 0x1000000);
        assert_eq!(overlay_flag(0, SeasonOverlay::Language, true), 0x2000000);
        assert_eq!(overlay_flag(0, SeasonOverlay::Continue, true), 0x4000000);
    }

    #[test]
    fn grid_size_is_clamped_to_four_bits() {
        assert_eq!(clamp_grid_size(-3), 0);
        assert_eq!(clamp_grid_size(99), 15);
        assert_eq!(clamp_grid_size(7), 7);
    }

    fn repo_with_one_entry(kind: Kind) -> (tempfile::TempDir, EntryRepo, i64) {
        let (dir, db) = crate::testdb::test_db();
        let repo = EntryRepo::new(db);
        let id = repo.insert(&Entry { favorite: true, source: 1, url: "u".into(), title: "T".into(), ..Entry::create(kind) }).unwrap();
        (dir, repo, id)
    }

    #[test]
    fn each_item_setter_writes_only_its_group() {
        for kind in Kind::ALL {
            let (_d, repo, id) = repo_with_one_entry(kind);
            assert!(set_item_flag(&repo, id, ItemFlagGroup::Undone, item::SHOW_UNDONE).unwrap());
            assert!(set_item_flag(&repo, id, ItemFlagGroup::Downloaded, item::SHOW_DOWNLOADED).unwrap());
            assert!(set_item_flag(&repo, id, ItemFlagGroup::Bookmarked, item::SHOW_NOT_BOOKMARKED).unwrap());
            assert!(set_item_flag(&repo, id, ItemFlagGroup::Display, item::DISPLAY_NUMBER).unwrap());
            assert_eq!(repo.by_id(id).unwrap().item_flags, 0x2 | 0x8 | 0x40 | 0x100000, "{kind}");
            assert!(set_item_flag(&repo, id, ItemFlagGroup::Undone, item::SHOW_DONE).unwrap());
            assert_eq!(repo.by_id(id).unwrap().item_flags, 0x4 | 0x8 | 0x40 | 0x100000, "only the unread group changed");
        }
    }

    #[test]
    fn the_anime_only_groups_do_nothing_for_manga() {
        let (_d, repo, manga) = repo_with_one_entry(Kind::Manga);
        for group in [ItemFlagGroup::Fillermarked, ItemFlagGroup::Previews, ItemFlagGroup::Summaries] {
            assert!(!set_item_flag(&repo, manga, group, group.mask()).unwrap(), "{group:?}");
        }
        assert_eq!(repo.by_id(manga).unwrap().item_flags, 0);
        let (_d, repo, anime) = repo_with_one_entry(Kind::Anime);
        assert!(set_item_flag(&repo, anime, ItemFlagGroup::Fillermarked, item::SHOW_NOT_FILLERMARKED).unwrap());
        assert_eq!(repo.by_id(anime).unwrap().item_flags, item::SHOW_NOT_FILLERMARKED);
    }

    #[test]
    fn item_sort_or_flip_and_set_all_persist() {
        for kind in Kind::ALL {
            let (_d, repo, id) = repo_with_one_entry(kind);
            set_item_sort_or_flip(&repo, id, item::SORTING_NUMBER).unwrap();
            assert_eq!(repo.by_id(id).unwrap().item_flags, 0x201);
            set_item_sort_or_flip(&repo, id, item::SORTING_NUMBER).unwrap();
            assert_eq!(repo.by_id(id).unwrap().item_flags, 0x200);
            set_item_all(&repo, id, item_all(item::SHOW_UNDONE, 0, 0, 0, item::SORTING_ALPHABET, item::SORT_ASC, 0, 0, 0)).unwrap();
            assert_eq!(repo.by_id(id).unwrap().item_flags, 0x2 | 0x600 | 0x1, "{kind}");
        }
    }

    #[test]
    fn a_missing_row_gives_false_not_an_error() {
        let (_d, repo, _id) = repo_with_one_entry(Kind::Manga);
        assert!(!set_item_flag(&repo, 9_999, ItemFlagGroup::Undone, item::SHOW_UNDONE).unwrap());
        assert!(!set_item_sort_or_flip(&repo, 9_999, item::SORTING_NUMBER).unwrap());
    }

    #[test]
    fn concurrent_setters_on_different_groups_both_persist() {
        for kind in Kind::ALL {
            let (_d, repo, id) = repo_with_one_entry(kind);
            let repo = std::sync::Arc::new(repo);
            std::thread::scope(|scope| {
                let a = repo.clone();
                let b = repo.clone();
                scope.spawn(move || {
                    for _ in 0..50 {
                        set_item_flag(&a, id, ItemFlagGroup::Undone, item::SHOW_UNDONE).unwrap();
                    }
                });
                scope.spawn(move || {
                    for _ in 0..50 {
                        set_item_flag(&b, id, ItemFlagGroup::Bookmarked, item::SHOW_BOOKMARKED).unwrap();
                    }
                });
            });
            let word = repo.by_id(id).unwrap().item_flags;
            assert_eq!(word & (item::UNDONE_MASK | item::BOOKMARKED_MASK), item::SHOW_UNDONE | item::SHOW_BOOKMARKED, "{kind}");
        }
    }

    #[test]
    fn season_setters_round_trip_through_the_database() {
        let (_d, repo, id) = repo_with_one_entry(Kind::Anime);
        set_season_grid_mode(&repo, id, SeasonDisplayMode::List).unwrap();
        set_season_grid_size(&repo, id, 5).unwrap();
        set_season_overlay(&repo, id, SeasonOverlay::Unseen, true).unwrap();
        set_season_overlay(&repo, id, SeasonOverlay::Local, true).unwrap();
        set_season_overlay(&repo, id, SeasonOverlay::Continue, true).unwrap();
        assert_eq!(repo.by_id(id).unwrap().season_flags(), 0x5970000);
        set_season_overlay(&repo, id, SeasonOverlay::Local, false).unwrap();
        assert_eq!(repo.by_id(id).unwrap().season_flags(), 0x5970000 & !0x1000000);
    }
}
