//! The migrate option bits of both catalogues (`MangaMigrationFlags.kt`, `AnimeMigrationFlags.kt`; T-40a-015, T-41a-032):
//! which options the migrate dialog offers and starts selected, and the guard that keeps one migration from starting twice
//! (T-40a-052). Pure: the dialog widgets are the callers'. Tracking data is always copied and has no bit; the manga
//! tracking bit 0x04 is the anime background bit, which a manga never offers. The choice is not stored (the setting is
//! cut), so a dialog starts from every offered option.

use std::cell::Cell;

/// The chapters of a manga or the episodes of an anime.
pub const ITEMS: u32 = 0x01;
pub const CATEGORIES: u32 = 0x02;
pub const CUSTOM_BACKGROUND: u32 = 0x04;
pub const CUSTOM_COVER: u32 = 0x08;
pub const DELETE_DOWNLOADED: u32 = 0x10;

/// The options in display order: the chapters or episodes (only when the old title lists them), categories, custom
/// cover, custom background, and delete downloaded, each of the last three only when the old entry has one.
pub fn options(items: bool, has_custom_cover: bool, has_custom_background: bool, has_downloads: bool) -> Vec<u32> {
    let mut offered = Vec::new();
    if items {
        offered.push(ITEMS);
    }
    offered.push(CATEGORIES);
    if has_custom_cover {
        offered.push(CUSTOM_COVER);
    }
    if has_custom_background {
        offered.push(CUSTOM_BACKGROUND);
    }
    if has_downloads {
        offered.push(DELETE_DOWNLOADED);
    }
    offered
}

/// The options a dialog starts with: the offered ones whose bit is set in the saved bitmap. The saved default `i32::MAX`
/// selects all of them.
pub fn preselected(saved: i32, offered: &[u32]) -> Vec<u32> {
    offered.iter().copied().filter(|bit| (saved as u32) & bit != 0).collect()
}

/// Keeps one migration from starting twice (E-40-90). `begin` is true for the first request only, until `finish` ends
/// the run. A failed run finishes too, so the dialog can be used again.
#[derive(Default)]
pub struct RunGuard {
    running: Cell<bool>,
}

impl RunGuard {
    pub fn begin(&self) -> bool {
        !self.running.replace(true)
    }

    pub fn finish(&self) {
        self.running.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bits_and_their_option_order_match_the_data_layer() {
        assert_eq!((ITEMS, CATEGORIES, CUSTOM_BACKGROUND, CUSTOM_COVER, DELETE_DOWNLOADED), (0x01, 0x02, 0x04, 0x08, 0x10));
        assert_eq!(options(true, true, true, true), [ITEMS, CATEGORIES, CUSTOM_COVER, CUSTOM_BACKGROUND, DELETE_DOWNLOADED]);
        assert_eq!(options(false, false, false, false), [CATEGORIES], "episodes only for an episodes title");
        assert_eq!(options(false, false, false, true), [CATEGORIES, DELETE_DOWNLOADED], "delete only when downloads exist");
        assert_eq!(options(true, true, false, true), [ITEMS, CATEGORIES, CUSTOM_COVER, DELETE_DOWNLOADED], "a manga has no background");
    }

    #[test]
    fn the_saved_default_selects_every_offered_option() {
        let all = options(true, true, true, true);
        assert_eq!(preselected(i32::MAX, &all), all, "T55: 0x7FFFFFFF selects all five");
        assert_eq!(preselected(0, &all), Vec::<u32>::new(), "a saved zero selects nothing");
        assert_eq!(preselected(CATEGORIES as i32, &all), [CATEGORIES]);
    }

    #[test]
    fn a_second_migrate_request_is_ignored_until_the_run_finishes() {
        let guard = RunGuard::default();
        assert!(guard.begin(), "the first request starts");
        assert!(!guard.begin(), "E-40-90: a second request is ignored");
        guard.finish();
        assert!(guard.begin(), "a failed or finished run lets the dialog start again");
    }
}
