//! Shared constants of the entry screens (T-38a-045, T-38a-043). Upstream: `EntryScreenConstants.kt` (the three enums)
//! and `DotSeparatorText.kt` (the separator labels). Pure: the widgets read these values.

/// The narrowest the left info pane of the two-pane entry layouts gets (its width at the two-pane breakpoint).
pub const LEFT_PANE_WIDTH: i32 = 360;
/// The share of the page width the left info pane takes, so the pane and its cover grow with the window.
pub const LEFT_PANE_SHARE: f64 = 0.3;
/// The widest the chapter and episode rows (and the count row above them) get: wider than a page's column (`ui::PAGE_WIDTH`)
/// so a big window is not mostly margin, narrower than a whole screen so a row's download button stays near its title.
pub const LIST_MAX_WIDTH: i32 = 960;
/// The window width from which the entry screens show two panes (info on the left, list on the right).
pub const TWO_PANE_MIN_WIDTH: i32 = 900;
/// The window size assumed before the page has a real one: the app's default size.
pub const FALLBACK_WIDTH: i32 = 1180;

/// The width of the left info pane for a page `width` wide: [`LEFT_PANE_SHARE`] of it, never under [`LEFT_PANE_WIDTH`]. In
/// 20 px steps, so a window being resized rebuilds the header only now and then.
pub fn left_pane_width(width: i32) -> i32 {
    let share = (f64::from(width) * LEFT_PANE_SHARE).round() as i32;
    (share / 20 * 20).max(LEFT_PANE_WIDTH)
}

/// True when a window of this width gets the two-pane entry layout.
pub fn two_pane(window_width: i32) -> bool {
    window_width >= TWO_PANE_MIN_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_left_pane_is_a_share_of_the_page_and_never_under_its_minimum() {
        assert_eq!(left_pane_width(TWO_PANE_MIN_WIDTH), LEFT_PANE_WIDTH);
        assert_eq!(left_pane_width(1200), 360);
        assert_eq!(left_pane_width(1600), 480);
        assert_eq!(left_pane_width(2560), 760, "no upper cap: a wide window gets a wide pane");
        assert_eq!(left_pane_width(1610), 480, "20 px steps");
    }
}
