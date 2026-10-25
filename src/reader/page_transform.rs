//! Page transforms of the decode worker (T-44a-050, RD §5.3, B§9.1, B§9.2). Pure: pixels in, pixels out, no GTK.
//!
//! The decision (`pager_intent`, `webtoon_intent`) is made on the GTK thread from the settings; [`apply`] runs on the
//! worker before the pyramid is built. A rotate has priority over a split. A normal pager page splits only when it is
//! wide, which the worker learns from the decoded size, so the intent carries `only_if_wide` and the result says
//! whether the page was changed (a changed normal page asks the viewer for its insert page).

use crate::reader::page_decode::DecodedPage;

/// What the worker does to a decoded page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageTransform {
    None,
    /// Rotates a wide page by 90 degrees, or by -90 when `invert` is set. A page that is not wide is kept.
    RotateWide { invert: bool },
    /// The left half of the page.
    SplitLeft,
    /// The right half of the page.
    SplitRight,
    /// Webtoon split: a wide page becomes two halves one above the other, the `upper_right` half on top. A page that
    /// is not wide is kept.
    SplitAndMerge { upper_right: bool },
}

/// A transform and the condition it applies under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DecodeIntent {
    pub transform: PageTransform,
    /// The split applies only to a wide source (a normal pager page). An insert page splits whatever it is given.
    pub only_if_wide: bool,
}

impl DecodeIntent {
    pub const NONE: Self = Self { transform: PageTransform::None, only_if_wide: false };

    /// Whether this intent splits a page into halves.
    pub fn is_split(&self) -> bool {
        matches!(self.transform, PageTransform::SplitLeft | PageTransform::SplitRight | PageTransform::SplitAndMerge { .. })
    }
}

impl Default for DecodeIntent {
    fn default() -> Self {
        Self::NONE
    }
}

/// The dual-page settings and the viewer family a page is decoded under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SplitSettings {
    pub split: bool,
    pub invert: bool,
    pub rotate: bool,
    pub rotate_invert: bool,
    /// The pager reads left to right, which picks the side of a split (B§9.1 table). Unused by the webtoon.
    pub l2r: bool,
    /// A webtoon page: a split merges the halves (B§9.2) and there are no insert pages.
    pub webtoon: bool,
}

/// `ImageUtil.isWideImage`: strictly wider than tall.
pub fn is_wide(width: u32, height: u32) -> bool {
    width > height
}

/// The transform for a page (B§9.1, B§9.2). `is_insert` marks the second half of a pager split.
pub fn intent(s: SplitSettings, is_insert: bool) -> DecodeIntent {
    if s.rotate {
        return DecodeIntent { transform: PageTransform::RotateWide { invert: s.rotate_invert }, only_if_wide: false };
    }
    if !s.split {
        return DecodeIntent::NONE;
    }
    if s.webtoon {
        return DecodeIntent { transform: PageTransform::SplitAndMerge { upper_right: !s.invert }, only_if_wide: true };
    }
    // The side before inversion: the insert is the half the normal page did not show.
    let right = s.l2r == is_insert;
    let right = if s.invert { !right } else { right };
    let transform = if right { PageTransform::SplitRight } else { PageTransform::SplitLeft };
    DecodeIntent { transform, only_if_wide: !is_insert }
}

/// Applies the intent. Returns the page and whether it was changed.
pub fn apply(page: DecodedPage, intent: DecodeIntent) -> (DecodedPage, bool) {
    let wide = is_wide(page.width, page.height);
    if intent.only_if_wide && !wide {
        return (page, false);
    }
    match intent.transform {
        PageTransform::None => (page, false),
        PageTransform::RotateWide { invert } if wide => (rotate(&page, invert), true),
        PageTransform::RotateWide { .. } => (page, false),
        PageTransform::SplitLeft => (split_in_half(&page, false), true),
        PageTransform::SplitRight => (split_in_half(&page, true), true),
        PageTransform::SplitAndMerge { upper_right } if wide => (split_and_merge(&page, upper_right), true),
        PageTransform::SplitAndMerge { .. } => (page, false),
    }
}

/// `splitInHalf`: `width / 2` columns, from the right edge or the left edge. For an odd width the right half starts at
/// `width - width / 2`, as upstream does.
pub fn split_in_half(page: &DecodedPage, right: bool) -> DecodedPage {
    let half = page.width / 2;
    let x0 = if right { page.width - half } else { 0 };
    crop(page, x0, half)
}

/// `splitAndMerge`: `width / 2` by `2 * height`. The upper cell takes the right half when `upper_right`, else the left.
pub fn split_and_merge(page: &DecodedPage, upper_right: bool) -> DecodedPage {
    let half = page.width / 2;
    let right = crop(page, page.width - half, half);
    let left = crop(page, 0, half);
    let (upper, lower) = if upper_right { (right, left) } else { (left, right) };
    let mut rgba = upper.rgba;
    rgba.extend_from_slice(&lower.rgba);
    DecodedPage { width: half, height: page.height * 2, rgba }
}

/// A quarter turn: 90 degrees clockwise, or counter-clockwise when `invert` (`postRotate(-90)`).
pub fn rotate(page: &DecodedPage, invert: bool) -> DecodedPage {
    let Some(img) = image::RgbaImage::from_raw(page.width, page.height, page.rgba.clone()) else {
        return DecodedPage { width: 0, height: 0, rgba: Vec::new() };
    };
    let out = if invert { image::imageops::rotate270(&img) } else { image::imageops::rotate90(&img) };
    DecodedPage { width: out.width(), height: out.height(), rgba: out.into_raw() }
}

/// Columns `x0..x0 + width` of every row.
fn crop(page: &DecodedPage, x0: u32, width: u32) -> DecodedPage {
    let (row, from, len) = (page.width as usize * 4, x0 as usize * 4, width as usize * 4);
    let mut rgba = Vec::with_capacity(len * page.height as usize);
    for y in 0..page.height as usize {
        rgba.extend_from_slice(&page.rgba[y * row + from..y * row + from + len]);
    }
    DecodedPage { width, height: page.height, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page whose pixel at column `x` has red `x` and green `y`.
    fn grid(width: u32, height: u32) -> DecodedPage {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        DecodedPage { width, height, rgba }
    }

    fn columns(page: &DecodedPage) -> Vec<u8> {
        (0..page.width as usize).map(|x| page.rgba[x * 4]).collect()
    }

    fn settings(edit: impl FnOnce(&mut SplitSettings)) -> SplitSettings {
        let mut s = SplitSettings { split: true, l2r: true, ..Default::default() };
        edit(&mut s);
        s
    }

    #[test]
    fn wide_is_strictly_wider_than_tall() {
        assert!(is_wide(101, 100));
        assert!(!is_wide(100, 100));
        assert!(!is_wide(99, 100));
    }

    #[test]
    fn a_split_takes_half_the_width_from_the_chosen_edge() {
        let page = grid(8, 2);
        assert_eq!(columns(&split_in_half(&page, false)), [0, 1, 2, 3]);
        assert_eq!(columns(&split_in_half(&page, true)), [4, 5, 6, 7]);
        assert_eq!(split_in_half(&page, true).height, 2);
    }

    #[test]
    fn an_odd_width_loses_the_middle_column_to_the_right_half() {
        let page = grid(5, 1);
        assert_eq!(columns(&split_in_half(&page, false)), [0, 1]);
        assert_eq!(columns(&split_in_half(&page, true)), [3, 4], "right starts at width - width / 2");
    }

    #[test]
    fn a_2000_by_1000_page_splits_into_two_squares() {
        let page = grid(2000, 1000);
        assert!(is_wide(page.width, page.height));
        let left = split_in_half(&page, false);
        let right = split_in_half(&page, true);
        assert_eq!((left.width, left.height), (1000, 1000));
        assert_eq!((right.width, right.height), (1000, 1000));
    }

    #[test]
    fn apply_leaves_a_page_that_is_not_wide_when_the_intent_says_so() {
        let tall = grid(2, 4);
        let (out, changed) = apply(tall.clone(), DecodeIntent { transform: PageTransform::SplitLeft, only_if_wide: true });
        assert!(!changed);
        assert_eq!(out, tall);
        let (out, changed) = apply(tall.clone(), DecodeIntent { transform: PageTransform::SplitLeft, only_if_wide: false });
        assert!(changed, "an insert page splits whatever it is given");
        assert_eq!(out.width, 1);
        assert_eq!(apply(tall.clone(), DecodeIntent::NONE), (tall, false));
    }

    #[test]
    fn the_split_side_follows_the_viewer_and_the_page_kind() {
        // B§9.1 table: L2R normal LEFT, L2R insert RIGHT, not L2R normal RIGHT, not L2R insert LEFT.
        let side = |l2r, is_insert| intent(settings(|s| s.l2r = l2r), is_insert).transform;
        assert_eq!(side(true, false), PageTransform::SplitLeft);
        assert_eq!(side(true, true), PageTransform::SplitRight);
        assert_eq!(side(false, false), PageTransform::SplitRight);
        assert_eq!(side(false, true), PageTransform::SplitLeft);
    }

    #[test]
    fn only_a_normal_pager_page_checks_for_width() {
        assert!(intent(settings(|_| {}), false).only_if_wide);
        assert!(!intent(settings(|_| {}), true).only_if_wide);
    }

    #[test]
    fn split_off_means_no_transform() {
        assert_eq!(intent(settings(|s| s.split = false), false), DecodeIntent::NONE);
    }
}
