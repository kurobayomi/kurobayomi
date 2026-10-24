//! Column count of the library grids (T-08a-043). The four preferences hold 0 (Auto) to 10. Auto fits 160 px columns to
//! the window, with at least two. The portrait preference applies below 900 px of width and the landscape one from 900 px.

/// The value that means "fit to the window".
pub const AUTO: i32 = 0;
/// Largest stored value.
pub const MAX_COLUMNS: i32 = 10;
/// Width of one column under Auto, in pixels.
pub const COLUMN_WIDTH_PX: i32 = 160;
/// Window width where the landscape preference takes over from the portrait one, in pixels.
pub const PORTRAIT_BREAKPOINT_PX: i32 = 900;

/// The cover size slider: the column width of the cover grids, in pixels. [`COLUMN_WIDTH_PX`] is the default.
pub const COVER_WIDTH_MIN_PX: i32 = 100;
pub const COVER_WIDTH_MAX_PX: i32 = 320;
/// The slider moves in steps of this many pixels.
pub const COVER_WIDTH_STEP_PX: i32 = 10;

/// The column width for a stored cover size, kept in the slider's range.
pub fn column_width(stored: i32) -> i32 {
    stored.clamp(COVER_WIDTH_MIN_PX, COVER_WIDTH_MAX_PX)
}

/// A slider position as the cover size to store: the nearest step, in range.
pub fn snap_cover_width(value: f64) -> i32 {
    column_width((value / f64::from(COVER_WIDTH_STEP_PX)).round() as i32 * COVER_WIDTH_STEP_PX)
}

/// A stored value kept in `0..=10`.
pub fn clamp_columns(stored: i32) -> i32 {
    stored.clamp(AUTO, MAX_COLUMNS)
}

/// Columns under Auto for a window `width_px` wide: `max(2, width / 160)`.
pub fn auto_columns(width_px: i32) -> i32 {
    (width_px / COLUMN_WIDTH_PX).max(2)
}

/// Columns to draw for a window `width_px` wide. `portrait` applies below the breakpoint and `landscape` at or above it.
/// A value of 0 is Auto.
pub fn columns_for_width(width_px: i32, portrait: i32, landscape: i32) -> i32 {
    let stored = if width_px < PORTRAIT_BREAKPOINT_PX { portrait } else { landscape };
    match clamp_columns(stored) {
        AUTO => auto_columns(width_px),
        n => n,
    }
}

/// The slider's value text: "Auto" at 0, otherwise the number.
pub fn slider_value_text(value: i32) -> String {
    match clamp_columns(value) {
        AUTO => "Auto".to_owned(),
        n => n.to_string(),
    }
}

/// The slider's caption: "List size" in list mode, "Items per row" for the grids.
pub fn slider_caption(list_mode: bool) -> &'static str {
    if list_mode { "List size" } else { "Items per row" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_the_window_over_160_with_at_least_two_columns() {
        assert_eq!(auto_columns(1000), 6);
        assert_eq!(auto_columns(320), 2);
        assert_eq!(auto_columns(100), 2);
    }

    #[test]
    fn the_cover_width_stays_in_the_slider_range_and_snaps_to_its_steps() {
        assert_eq!(column_width(COLUMN_WIDTH_PX), 160);
        assert_eq!((column_width(0), column_width(999)), (COVER_WIDTH_MIN_PX, COVER_WIDTH_MAX_PX));
        assert_eq!((snap_cover_width(163.0), snap_cover_width(166.0)), (160, 170));
        assert_eq!((snap_cover_width(40.0), snap_cover_width(400.0)), (100, 320));
    }

    #[test]
    fn the_portrait_key_is_used_below_900_and_the_landscape_key_from_900() {
        assert_eq!(columns_for_width(899, 3, 7), 3);
        assert_eq!(columns_for_width(900, 3, 7), 7);
        assert_eq!(columns_for_width(1200, 3, 7), 7);
    }

    #[test]
    fn zero_means_auto_for_the_chosen_key() {
        assert_eq!(columns_for_width(800, 0, 9), auto_columns(800));
        assert_eq!(columns_for_width(1000, 0, 0), 6);
    }

    #[test]
    fn stored_values_are_kept_in_range() {
        assert_eq!(clamp_columns(-4), 0);
        assert_eq!(clamp_columns(11), 10);
        assert_eq!(columns_for_width(1000, 99, 99), 10);
    }

    #[test]
    fn the_slider_text_and_caption_follow_the_mode() {
        assert_eq!(slider_value_text(0), "Auto");
        assert_eq!(slider_value_text(4), "4");
        assert_eq!(slider_caption(true), "List size");
        assert_eq!(slider_caption(false), "Items per row");
    }
}
