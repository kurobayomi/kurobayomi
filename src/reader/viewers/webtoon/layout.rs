//! Strip geometry for the webtoon (T-44a-032, T-44a-034, B§7.1, B§7.3, TW §1). Pixels are integers. `offsets` are
//! prefix sums, so a lookup is a binary search and a height change is O(n).

use std::ops::Range;

/// The space under each page in gapped mode, in logical pixels (15 dp, E-38). Continuous mode adds none.
pub const GAP_PX: i32 = 15;

/// The gap under each page for the strip's mode: [`GAP_PX`] when gapped, 0 otherwise (E-38).
pub fn gap_px(gapped: bool) -> i32 {
    if gapped { GAP_PX } else { 0 }
}

/// The heights of every item in the strip and their prefix sums.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StripLayout {
    heights: Vec<i32>,
    /// `offsets[i]` is the top of item `i`; `offsets[len]` is the total height.
    offsets: Vec<i64>,
}

impl StripLayout {
    pub fn new(heights: Vec<i32>) -> Self {
        let mut layout = Self { heights, offsets: Vec::new() };
        layout.recompute();
        layout
    }

    fn recompute(&mut self) {
        self.offsets.clear();
        self.offsets.push(0);
        for h in &self.heights {
            let last = *self.offsets.last().unwrap_or(&0);
            self.offsets.push(last + i64::from(*h));
        }
    }

    pub fn len(&self) -> usize {
        self.heights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    pub fn height(&self, index: usize) -> i32 {
        self.heights.get(index).copied().unwrap_or(0)
    }

    #[cfg(test)]
    /// Changes one item's height (a decode finished, or a placeholder was resized).
    pub fn set_height(&mut self, index: usize, height: i32) {
        if let Some(h) = self.heights.get_mut(index) {
            *h = height.max(0);
            self.recompute();
        }
    }

    /// The top of item `index` in strip space.
    pub fn offset(&self, index: usize) -> i64 {
        self.offsets.get(index).copied().unwrap_or(0)
    }

    pub fn total_height(&self) -> i64 {
        *self.offsets.last().unwrap_or(&0)
    }

    /// The scroll distance of taps and keys, and the extra space realised on each side: `viewport * 3 / 4`.
    pub fn scroll_distance(viewport: i32) -> i32 {
        viewport * 3 / 4
    }

    /// The items to realise for a scroll position `scroll` in a viewport of `viewport` pixels (B§7.1). An item is
    /// realised when it overlaps the viewport grown by [`Self::scroll_distance`] on both sides.
    pub fn realised(&self, scroll: i64, viewport: i32) -> Range<usize> {
        let extra = i64::from(Self::scroll_distance(viewport));
        let (low, high) = (scroll - extra, scroll + i64::from(viewport) + extra);
        let first = self.first_ending_after(low);
        let end = self.offsets[..self.heights.len()].partition_point(|&top| top < high);
        first..end.max(first)
    }

    /// The first item whose bottom is below `position` (index 0 before the strip has any items).
    pub fn first_visible(&self, scroll: i64) -> Option<usize> {
        (!self.heights.is_empty()).then(|| self.first_ending_after(scroll))
    }

    /// First item whose bottom edge lies beyond `y`, clamped to the last item.
    fn first_ending_after(&self, y: i64) -> usize {
        let bottoms = &self.offsets[1..];
        bottoms.partition_point(|&bottom| bottom <= y).min(self.heights.len().saturating_sub(1))
    }

    /// The bottom-most realised item whose bottom edge is inside the viewport, or whose top is above it (B§7.3).
    /// `None` when no realised item qualifies, including before the first layout (`viewport == 0`). Only items that
    /// reach into the viewport count, as only those are children of upstream's list: a page that ends exactly at the
    /// top of the view is off screen, or a jump to the top of a tall page would report the page before it.
    pub fn last_end_visible(&self, scroll: i64, viewport: i32, realised: Range<usize>) -> Option<usize> {
        if viewport <= 0 {
            return None;
        }
        let (parent_start, parent_end) = (scroll, scroll + i64::from(viewport));
        realised.rev().find(|&i| {
            let (start, end) = (self.offset(i), self.offset(i) + i64::from(self.height(i)));
            end > parent_start && (end <= parent_end || start < parent_start)
        })
    }

    /// Whether a scroll range ending at `upper` holds the whole strip. Right after the strip changes size the
    /// range is stale, and a scroll to an item would be clamped to it.
    pub fn fits_range(&self, upper: f64) -> bool {
        upper >= self.total_height() as f64
    }

    /// The scroll position that puts item `index` at the top of the viewport (B§7.5).
    pub fn scroll_to(&self, index: usize) -> i64 {
        self.offset(index)
    }
}

/// A page that is not decoded yet and has no known size is as tall as a typical manga page: this many times its width.
pub const PLACEHOLDER_RATIO: f64 = 1.4;

/// The height of a page of source size `src` (width, height) shown `width` pixels wide. At least 1.
pub fn scaled_height(src: (u32, u32), width: i32) -> i32 {
    if src.0 == 0 {
        return 1;
    }
    ((f64::from(width.max(0)) * f64::from(src.1) / f64::from(src.0)).round() as i32).max(1)
}

/// The height a page takes in the strip (the gap not included). A known size wins, then the height of the previous
/// decoded page (already at this width), then [`PLACEHOLDER_RATIO`] times the width. Never 0, so an undecoded page
/// cannot make the strip short and let one wheel notch land in the next chapter.
pub fn reserved_height(known: Option<(u32, u32)>, previous: Option<i32>, width: i32) -> i32 {
    match (known, previous) {
        (Some(src), _) => scaled_height(src, width),
        (None, Some(h)) => h.max(1),
        (None, None) => ((f64::from(width.max(0)) * PLACEHOLDER_RATIO).round() as i32).max(1),
    }
}

/// How far to move the scroll position so the item that holds the top of the viewport stays where it is: the shift of
/// that item's top between the old layout (`old_index`) and the new one (`new_index`). Items below it, and the item
/// itself growing, change nothing; pages above it that grew or shrank shift it.
pub fn anchor_delta(old: &StripLayout, old_index: usize, new: &StripLayout, new_index: usize) -> i64 {
    new.offset(new_index) - old.offset(old_index)
}

/// One scroll input never moves more than a viewport.
pub fn clamp_step(delta: f64, viewport: i32) -> f64 {
    let max = f64::from(viewport.max(0));
    delta.clamp(-max, max)
}

/// The distance of a wheel scroll of `dy` notches: the usual `viewport^(2/3)` per notch, clamped by [`clamp_step`].
pub fn wheel_delta(dy: f64, viewport: i32) -> f64 {
    clamp_step(dy * f64::from(viewport.max(0)).powf(2.0 / 3.0), viewport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_laid_out_before_it_has_a_size_realises_nothing_and_reports_no_current_item() {
        // The first sync runs before the hidden strip is allocated: every placeholder height is 0 (T-44a-032).
        let unsized_strip = StripLayout::new(vec![0; 5]);
        let range = unsized_strip.realised(0, 0);
        assert_eq!(unsized_strip.last_end_visible(0, 0, range), None);
        // Once the viewport is known the placeholders fill it, items realise and a current item exists.
        let sized = StripLayout::new(vec![600; 5]);
        let range = sized.realised(0, 600);
        assert!(!range.is_empty());
        assert_eq!(sized.last_end_visible(0, 600, range), Some(0));
    }

    #[test]
    fn a_scroll_range_holds_the_strip_only_once_it_has_grown_to_it() {
        let strip = StripLayout::new(vec![600; 5]);
        assert!(!strip.fits_range(0.0));
        assert!(!strip.fits_range(2999.0));
        assert!(strip.fits_range(3000.0));
    }

    #[test]
    fn an_undecoded_page_reserves_height_from_its_size_then_the_previous_page_then_the_width() {
        assert_eq!(reserved_height(Some((800, 2400)), Some(500), 400), 1200, "a known size wins");
        assert_eq!(reserved_height(None, Some(900), 400), 900, "the previous decoded page");
        assert_eq!(reserved_height(None, None, 500), 700, "1.4 times the width");
        assert_eq!(reserved_height(None, None, 0), 1, "never 0");
        assert_eq!(scaled_height((40, 60), 400), 600);
        assert_eq!(scaled_height((0, 60), 400), 1);
    }

    #[test]
    fn anchoring_follows_pages_above_the_viewport_and_ignores_the_one_at_the_top() {
        let old = StripLayout::new(vec![700, 700, 700]);
        // item 0 decodes taller: item 1 (holding the viewport top) moves down by 300
        let grown_above = StripLayout::new(vec![1000, 700, 700]);
        assert_eq!(anchor_delta(&old, 1, &grown_above, 1), 300);
        // item 1 itself resizes: its top stays, so nothing moves
        let grown_at_top = StripLayout::new(vec![700, 1500, 700]);
        assert_eq!(anchor_delta(&old, 1, &grown_at_top, 1), 0);
        // a page above shrinks
        let shrunk = StripLayout::new(vec![400, 700, 700]);
        assert_eq!(anchor_delta(&old, 1, &shrunk, 1), -300);
        // the first item at the top of the strip never shifts
        assert_eq!(anchor_delta(&old, 0, &grown_above, 0), 0);
    }

    #[test]
    fn one_scroll_input_never_moves_more_than_a_viewport() {
        assert_eq!(clamp_step(5000.0, 800), 800.0);
        assert_eq!(clamp_step(-5000.0, 800), -800.0);
        assert_eq!(clamp_step(300.0, 800), 300.0);
        assert_eq!(clamp_step(300.0, 0), 0.0);
        assert!((wheel_delta(1.0, 1000) - 100.0).abs() < 1e-6, "1000^(2/3) is 100");
        assert_eq!(wheel_delta(1000.0, 1000), 1000.0, "a huge delta is clamped to the viewport");
        assert_eq!(wheel_delta(-1000.0, 1000), -1000.0);
    }

    /// A 50-item strip of 100 px items in a 400 px viewport.
    fn strip() -> StripLayout {
        StripLayout::new(vec![100; 50])
    }

    #[test]
    fn offsets_are_prefix_sums_and_a_height_change_shifts_later_items() {
        let mut s = StripLayout::new(vec![10, 20, 30]);
        assert_eq!((s.offset(0), s.offset(1), s.offset(2), s.total_height()), (0, 10, 30, 60));
        s.set_height(1, 50);
        assert_eq!((s.offset(2), s.total_height()), (60, 90));
    }

    #[test]
    fn realised_range_has_three_quarters_of_a_viewport_on_each_side() {
        let s = strip();
        // Viewport 400, extra 300. At scroll 0 the realised items reach offset 700: items 0..7.
        assert_eq!(s.realised(0, 400), 0..7);
        // At scroll 1000 the window is 700..1700: items 7..17.
        assert_eq!(s.realised(1000, 400), 7..17);
    }

    #[test]
    fn last_end_visible_is_the_bottom_most_item_whose_end_is_on_screen() {
        let s = strip();
        let realised = s.realised(0, 400);
        // At the top the viewport holds items 0..4 fully; item 3 ends at 400 and is the last one inside.
        assert_eq!(s.last_end_visible(0, 400, realised.clone()), Some(3));
        // In the middle: scroll 1050 shows 1050..1450; item 14 (1400..1500) ends below the viewport, so item 13.
        let mid = s.realised(1050, 400);
        assert_eq!(s.last_end_visible(1050, 400, mid), Some(13));
        // At the bottom of a 5000 px strip the last item's end is at the viewport end.
        let bottom = s.realised(4600, 400);
        assert_eq!(s.last_end_visible(4600, 400, bottom), Some(49));
    }

    #[test]
    fn a_page_that_ends_at_the_top_of_the_view_is_not_visible() {
        // 1000 px pages in a 600 px view, scrolled to the top of page 3: page 2 ends exactly at the top
        let s = StripLayout::new(vec![1000; 6]);
        assert_eq!(s.last_end_visible(3000, 600, s.realised(3000, 600)), None, "page 3 fills the view and ends below it");
        assert_eq!(s.last_end_visible(3500, 600, s.realised(3500, 600)), Some(3), "page 3's end is on screen");
    }

    #[test]
    fn last_end_visible_is_none_without_realised_items_or_before_layout() {
        let s = strip();
        assert_eq!(s.last_end_visible(0, 400, 0..0), None);
        assert_eq!(s.last_end_visible(0, 0, s.realised(0, 0)), None);
        assert_eq!(StripLayout::default().first_visible(0), None);
    }

    #[test]
    fn scroll_to_aligns_the_item_top_with_the_viewport_top() {
        assert_eq!(strip().scroll_to(7), 700);
    }

    #[test]
    fn scroll_distance_is_three_quarters_of_the_viewport_with_integer_division() {
        assert_eq!(StripLayout::scroll_distance(1080), 810);
        assert_eq!(StripLayout::scroll_distance(7), 5);
    }

    /// T-44a-079: the strip geometry and the current-item sequence of a scroll through a strip with cards, a page that
    /// failed (0.8 of the viewport) and a taller page, compared with the golden file.
    #[test]
    fn the_golden_webtoon_geometry_matches() {
        let viewport = 800;
        let heights = vec![200, 800, 640, 800, 200];
        let layout = StripLayout::new(heights.clone());
        let offsets: Vec<i64> = (0..=heights.len()).map(|i| layout.offset(i)).collect();
        assert_eq!(offsets, [0, 200, 1000, 1640, 2440, 2640], "prefix sums, worked out by hand");
        let scrolls = [0, 100, 400, 800, 1200, 1640, 2000, 2440];
        let steps: Vec<serde_json::Value> = scrolls
            .iter()
            .map(|&scroll| {
                let realised = layout.realised(scroll, viewport);
                serde_json::json!({
                    "scroll": scroll,
                    "realised": [realised.start, realised.end],
                    "first_visible": layout.first_visible(scroll),
                    "current": layout.last_end_visible(scroll, viewport, realised),
                })
            })
            .collect();
        let golden = serde_json::json!({
            "about": "heights are a card, a page, a failed page at 0.8 of the viewport, a page and a card; viewport 800",
            "viewport": viewport,
            "heights": heights,
            "offsets": offsets,
            "total": layout.total_height(),
            "scroll_distance": StripLayout::scroll_distance(viewport),
            "steps": steps,
        });
        crate::reader::golden::check("webtoon_geometry.json", &golden);
    }

    #[test]
    fn t033_gapped_mode_adds_15_px_under_each_page_and_continuous_adds_none() {
        assert_eq!(gap_px(true), 15);
        assert_eq!(gap_px(false), 0);
        let gap = gap_px(true);
        let layout = StripLayout::new(vec![100 + gap, 200 + gap]);
        assert_eq!(layout.offset(1), 115, "the second page starts after the first page and its gap");
        assert_eq!(layout.height(0) - gap, 100, "the row is the page without its gap");
    }
}
