//! Reading mode and crop-borders state of one manga (`ReadingMode.kt`, `SetMangaViewerFlags.kt`,
//! `ReaderViewModel.kt:669-749`). T-43a-004, T-43a-036, T-43a-037.
//!
//! Writes merge under the mask so bits outside the field survive, as `Flag` does upstream. A viewer word restored
//! from a backup keeps whatever other bits it carries; only the reading-mode bits are read and written here.

use kuro_prefs::flags::{Flag, with_flag};
use kuro_prefs::reader::{READING_MODE_MASK, ReadingMode};

/// The stored default reading mode that means Adaptive: the title's tags pick the strip or pages, and pages turn in the
/// adaptive direction (`pref_adaptive_direction`). It is the unset default (user, 2026-11-02); the stored default of
/// `pref_default_reading_mode_key` must say the same (`kuro-prefs`). A value that is no mode reads as Adaptive too.
pub const DEFAULT_READING_MODE_PREF: i32 = 0;

/// The pages mode of the adaptive direction: left to right for 1, else right to left (the default, 2).
pub fn adaptive_pages(direction: i32) -> ReadingMode {
    if direction == 1 { ReadingMode::LeftToRight } else { ReadingMode::RightToLeft }
}

/// The reading mode a manga's viewer flags select. Only the mode bits are read.
pub fn reading_mode_of(viewer_flags: i64) -> ReadingMode {
    ReadingMode::from_preference((viewer_flags & i64::from(READING_MODE_MASK)) as i32)
}

/// The tags that mark a title read as one vertical strip (webtoons, manhwa, manhua), compared without case.
const VERTICAL_TAGS: [&str; 6] = ["webtoon", "webtoons", "manhwa", "manhua", "long strip", "longstrip"];

/// The mode a title's tags call for, when they call for one: a webtoon, manhwa, manhua or long-strip title reads as
/// the vertical strip. Anything else is `None` and follows the default.
pub fn content_mode(genres: &[String]) -> Option<ReadingMode> {
    genres.iter().any(|g| VERTICAL_TAGS.contains(&g.trim().to_lowercase().as_str())).then_some(ReadingMode::Webtoon)
}

/// `getMangaReadingMode(resolveDefault = true)`: the manga's own mode, else the default reading mode. A fixed default
/// is used as it is; the Adaptive one (or a value that is no mode) takes the mode the title's tags call for (`content`),
/// else pages in `direction` (`adaptive_pages`). Never returns [`ReadingMode::Default`].
pub fn resolve_reading_mode(viewer_flags: i64, content: Option<ReadingMode>, default_pref: i32, direction: i32) -> ReadingMode {
    match reading_mode_of(viewer_flags) {
        ReadingMode::Default => match ReadingMode::from_preference(default_pref) {
            ReadingMode::Default => content.unwrap_or_else(|| adaptive_pages(direction)),
            mode => mode,
        },
        mode => mode,
    }
}

/// Writes a reading mode into the viewer flags, keeping every bit outside [`READING_MODE_MASK`].
pub fn set_reading_mode(viewer_flags: i64, mode: ReadingMode) -> i64 {
    with_flag(viewer_flags, Flag::new(i64::from(mode.flag_value()), i64::from(READING_MODE_MASK)))
}

/// Which crop-borders preference a reading mode uses (`ReaderViewModel.toggleCropBorders`, `:742-749`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropPref {
    Pager,
    Webtoon,
}

/// The crop preference for `mode`: pager modes use `crop_borders`, everything else `crop_borders_webtoon`.
pub fn crop_pref_for(mode: ReadingMode) -> CropPref {
    if ReadingMode::is_pager_type(mode.flag_value()) { CropPref::Pager } else { CropPref::Webtoon }
}

/// The result of flipping the crop toggle: which preference to write and its new value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropToggle {
    pub pref: CropPref,
    pub value: bool,
}

/// Flips the crop toggle for the current mode. `crop_pager` and `crop_webtoon` are the current values of the two
/// preferences; only the one for the current mode changes.
pub fn toggle_crop(mode: ReadingMode, crop_pager: bool, crop_webtoon: bool) -> CropToggle {
    match crop_pref_for(mode) {
        CropPref::Pager => CropToggle { pref: CropPref::Pager, value: !crop_pager },
        CropPref::Webtoon => CropToggle { pref: CropPref::Webtoon, value: !crop_webtoon },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_mode_values_zero_to_five_round_trip_under_the_mask() {
        for code in 0..=5 {
            let mode = ReadingMode::from_preference(code);
            assert_eq!(mode.flag_value(), code);
            assert_eq!(reading_mode_of(i64::from(code)), mode);
        }
        assert_eq!(READING_MODE_MASK, 0x07);
    }

    #[test]
    #[allow(non_snake_case)] // the name is an acceptance test id
    fn reading_mode_fromPreference_unknown_is_default() {
        assert_eq!(ReadingMode::from_preference(0x06), ReadingMode::Default);
        assert_eq!(reading_mode_of(0x06), ReadingMode::Default);
        assert_eq!(ReadingMode::from_preference(0x38), ReadingMode::Default);
    }

    #[test]
    fn mask_set_flag_preserves_other_bits() {
        let flags = 0x18 | 0x100 | 0x3;
        let after_mode = set_reading_mode(flags, ReadingMode::Webtoon);
        assert_eq!(after_mode, 0x18 | 0x100 | 0x4);
    }

    #[test]
    fn manga_mode_zero_resolves_to_preference_and_raw_is_kept_otherwise() {
        assert_eq!(resolve_reading_mode(0, None, DEFAULT_READING_MODE_PREF, 2), ReadingMode::RightToLeft, "adaptive, right to left");
        assert_eq!(resolve_reading_mode(0, None, 1, 2), ReadingMode::LeftToRight, "a fixed default is used as it is");
        assert_eq!(resolve_reading_mode(0x3, None, DEFAULT_READING_MODE_PREF, 2), ReadingMode::Vertical);
    }

    #[test]
    fn e21_resolve_never_returns_default_even_with_a_corrupt_global() {
        assert_eq!(resolve_reading_mode(0, None, 0, 1), ReadingMode::LeftToRight, "adaptive, left to right");
        assert_eq!(resolve_reading_mode(0, None, 9, 2), ReadingMode::RightToLeft, "no mode reads as adaptive");
        assert_eq!(resolve_reading_mode(0, None, 0, 7), ReadingMode::RightToLeft, "an unknown direction is right to left");
        assert_ne!(resolve_reading_mode(0, None, 0, 2), ReadingMode::Default);
    }

    #[test]
    fn e20b_writing_a_mode_keeps_the_bits_outside_the_mode_field() {
        assert_eq!(set_reading_mode(0x38, ReadingMode::RightToLeft), 0x3A);
        assert_eq!(reading_mode_of(set_reading_mode(0x38, ReadingMode::Webtoon) as i64), ReadingMode::Webtoon);
    }

    #[test]
    fn crop_toggle_flips_only_the_preference_for_the_viewer_type() {
        let pager = toggle_crop(ReadingMode::RightToLeft, false, false);
        assert_eq!(pager, CropToggle { pref: CropPref::Pager, value: true });
        let webtoon = toggle_crop(ReadingMode::Webtoon, false, true);
        assert_eq!(webtoon, CropToggle { pref: CropPref::Webtoon, value: false });
        assert_eq!(crop_pref_for(ReadingMode::ContinuousVertical), CropPref::Webtoon);
    }

    #[test]
    fn a_webtoon_title_reads_vertically_unless_it_has_a_mode_of_its_own() {
        let tags = |list: &[&str]| list.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
        assert_eq!(content_mode(&tags(&["Action", "Manhwa"])), Some(ReadingMode::Webtoon));
        assert_eq!(content_mode(&tags(&["Long Strip"])), Some(ReadingMode::Webtoon), "case and spaces do not matter");
        assert_eq!(content_mode(&tags(&["Action", "Shounen"])), None, "other manga keeps the default");
        let webtoon = content_mode(&tags(&["Webtoon"]));
        assert_eq!(resolve_reading_mode(0, webtoon, DEFAULT_READING_MODE_PREF, 2), ReadingMode::Webtoon, "adaptive follows the tags");
        assert_eq!(resolve_reading_mode(0x1, webtoon, DEFAULT_READING_MODE_PREF, 2), ReadingMode::LeftToRight, "the title's own mode wins");
        assert_eq!(resolve_reading_mode(0, webtoon, 2, 1), ReadingMode::RightToLeft, "a fixed default ignores the tags");
        assert_eq!(resolve_reading_mode(0, None, DEFAULT_READING_MODE_PREF, 1), ReadingMode::LeftToRight, "no tag: pages in the adaptive direction");
    }
}
