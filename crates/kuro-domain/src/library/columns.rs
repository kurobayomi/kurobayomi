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

/// Column width of each cover size (Small, Medium, Large), in pixels. Medium is the default.
pub const COVER_SIZE_COLUMN_PX: [i32; 3] = [120, COLUMN_WIDTH_PX, 210];
/// The stored cover size that means Medium.
pub const COVER_SIZE_MEDIUM: i32 = 1;

/// Column width of a stored cover size; any other stored value reads as Medium.
pub fn column_width(cover_size: i32) -> i32 {
    usize::try_from(cover_size).ok().and_then(|i| COVER_SIZE_COLUMN_PX.get(i)).copied().unwrap_or(COLUMN_WIDTH_PX)
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
    fn each_cover_size_has_its_column_width_and_unknown_sizes_read_as_medium() {
        assert_eq!([column_width(0), column_width(1), column_width(2)], [120, 160, 210]);
        assert_eq!(column_width(COVER_SIZE_MEDIUM), COLUMN_WIDTH_PX);
        assert_eq!(column_width(-1), COLUMN_WIDTH_PX);
        assert_eq!(column_width(3), COLUMN_WIDTH_PX);
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
