//! Shared row and dialog pieces. The row and page metrics follow the app's spacing scale (4, 8, 12, 16, 24;
//! docs/design/other-tabs.md) and match `crate::ui::row`, so settings rows and the other boxed lists look alike. The
//! flash timings come from Aniyomi's `Constants.kt`.

pub mod dialog;
pub mod edit_text;
pub mod group;
pub mod info;
pub mod list;
pub mod multi_select;
pub mod row;
pub mod slider;
pub mod switch;
pub mod text;
pub mod theme_mode;

pub const PREF_HORIZONTAL_PADDING: i32 = crate::ui::ROW_PAD_H;
pub const PREF_VERTICAL_PADDING: i32 = crate::ui::ROW_PAD_V;
pub const TRAILING_WIDGET_BUFFER: i32 = 12;
pub const PREF_MIN_HEIGHT: i32 = crate::ui::ROW_MIN_HEIGHT;
pub const GROUP_SPACER: i32 = 24;
pub const GROUP_HEADER_BOTTOM: i32 = 12;
pub const ICON_END_MARGIN: i32 = 12;
pub const SUBTITLE_MAX_LINES: i32 = 10;
pub const TITLE_MAX_LINES: i32 = 2;
pub const CHECK_ROW_MIN: i32 = 48;
pub const DIALOG_WIDTH: i32 = 360;
pub const PAGE_MAX_WIDTH: i32 = crate::ui::PAGE_WIDTH;
pub const FLASH_DELAY_MS: u64 = 600;
pub const FLASH_STEP_MS: u64 = 200;
pub const FLASH_TOGGLES: u32 = 10;
pub const FLASH_TOTAL_MS: u64 = 3000;
pub const HIGHLIGHT_SCROLL_DELAY_MS: u64 = 500;
pub const SLIDER_ROW_PADDING: i32 = 12;
pub const TRACKER_CHECK_SIZE: i32 = 32;

/// Where Help in the app menu leads: the front page of the website's help.
pub const HELP_URL: &str = kuro_platform::links::URL_HELP_HOME;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_url_and_restore_title_are_the_exact_strings() {
        assert_eq!(HELP_URL, "https://kurobayomi.github.io/help");
        assert_eq!(super::super::strings::label_backup(), "Backup and restore");
    }
}
