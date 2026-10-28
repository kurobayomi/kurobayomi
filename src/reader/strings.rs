//! Reader strings keyed by the upstream message keys (`p1-behavior.md` section 20, T-43a-061). Each one reads the string
//! table, so it follows the active language; the arguments of the formatted ones are plain function parameters.
//! Strings marked "text not re-read" in section 20 are kept as the spec gives them and must be verified.

use kuro_i18n::{Arg, MsgKey, tr, tr_plural, trf};

pub fn page_list_empty_error() -> String {
    tr(MsgKey::page_list_empty_error)
}

pub fn loader_not_implemented_error() -> String {
    tr(MsgKey::loader_not_implemented_error)
}

/// A literal in `ReaderActivity.kt:181`, not a localised key.
pub fn unknown_err() -> String {
    tr(MsgKey::reader_unknown_err)
}

pub fn unknown_error() -> String {
    tr(MsgKey::unknown_error)
}

/// The downloaded-only card text (E-LIST-2 proposal). The card shows it as is, without the load-failure prefix.
pub fn chapter_not_downloaded() -> String {
    tr(MsgKey::reader_chapter_not_downloaded)
}

pub fn download_notifier_title_error() -> String {
    tr(MsgKey::download_notifier_title_error)
}

pub fn loading() -> String {
    tr(MsgKey::loading)
}

pub fn on() -> String {
    tr(MsgKey::on)
}

pub fn off() -> String {
    tr(MsgKey::off)
}

pub fn retry() -> String {
    tr(MsgKey::action_retry)
}

pub fn close() -> String {
    tr(MsgKey::action_close)
}


pub fn transition_previous() -> String {
    tr(MsgKey::transition_previous)
}

pub fn transition_next() -> String {
    tr(MsgKey::transition_next)
}

pub fn transition_finished() -> String {
    tr(MsgKey::transition_finished)
}

pub fn transition_current() -> String {
    tr(MsgKey::transition_current)
}

pub fn transition_no_previous() -> String {
    tr(MsgKey::transition_no_previous)
}

pub fn transition_no_next() -> String {
    tr(MsgKey::transition_no_next)
}

/// `transition_pages_loading`: the status line while the adjacent chapter loads (B§11.3).
pub fn transition_pages_loading() -> String {
    tr(MsgKey::transition_pages_loading)
}

pub fn label_downloaded() -> String {
    tr(MsgKey::label_downloaded)
}


pub fn picture_saved() -> String {
    tr(MsgKey::picture_saved)
}

pub fn cover_updated() -> String {
    tr(MsgKey::cover_updated)
}

pub fn notification_first_add_to_library() -> String {
    tr(MsgKey::notification_first_add_to_library)
}

pub fn notification_cover_update_failed() -> String {
    tr(MsgKey::notification_cover_update_failed)
}

pub fn set_as_cover() -> String {
    tr(MsgKey::set_as_cover)
}

pub fn confirm_set_image_as_cover() -> String {
    tr(MsgKey::confirm_set_image_as_cover)
}

pub fn action_ok() -> String {
    tr(MsgKey::action_ok)
}

pub fn action_cancel() -> String {
    tr(MsgKey::action_cancel)
}

pub fn action_save() -> String {
    tr(MsgKey::action_save)
}

pub fn action_share() -> String {
    tr(MsgKey::action_share)
}

/// The button of the share notice: shows the shared picture in the file manager (T-43a-056).
pub fn action_show_in_file_manager() -> String {
    tr(MsgKey::reader_action_show_in_file_manager)
}

pub fn action_copy_to_clipboard() -> String {
    tr(MsgKey::action_copy_to_clipboard)
}


pub fn action_bookmark() -> String {
    tr(MsgKey::action_bookmark)
}

pub fn action_remove_bookmark() -> String {
    tr(MsgKey::action_remove_bookmark)
}

pub fn action_open_in_browser() -> String {
    tr(MsgKey::action_open_in_browser)
}

pub fn action_next_chapter() -> String {
    tr(MsgKey::action_next_chapter)
}

pub fn action_previous_chapter() -> String {
    tr(MsgKey::action_previous_chapter)
}

pub fn viewer() -> String {
    tr(MsgKey::viewer)
}

/// The chapter list's title and the top bar button's tooltip.
pub fn chapters() -> String {
    tr(MsgKey::chapters)
}

pub fn action_filter_bookmarked() -> String {
    tr(MsgKey::action_filter_bookmarked)
}

/// The chapter list's search box placeholder.
pub fn chapter_list_search() -> String {
    tr(MsgKey::reader_chapter_list_search)
}

/// The button that returns to the chapter read before a jump from the chapter list.
pub fn chapter_list_back(chapter: &str) -> String {
    trf(MsgKey::reader_chapter_list_back, &[("chapter", Arg::Str(chapter))])
}

pub fn pref_crop_borders() -> String {
    tr(MsgKey::pref_crop_borders)
}

pub fn action_settings() -> String {
    tr(MsgKey::action_settings)
}


pub fn label_default() -> String {
    tr(MsgKey::label_default)
}

/// PROPOSAL (E-BRIGHT-2): shown when a positive custom brightness is asked for, since there is no backlight control.
pub fn custom_brightness_unavailable() -> String {
    tr(MsgKey::reader_custom_brightness_unavailable)
}

/// The accessible name and tooltip of the top bar overflow button (the Kotlin content description, in English).
pub fn overflow_menu() -> String {
    tr(MsgKey::player_sheets_more_title)
}

pub fn menu_reader_settings() -> String {
    tr(MsgKey::reader_menu_reader_settings)
}


pub fn name_collision_suffix(n: u32) -> String {
    format!(" ({n})")
}

pub fn pref_category_reading_mode() -> String {
    tr(MsgKey::pref_category_reading_mode)
}

pub fn custom_filter() -> String {
    tr(MsgKey::custom_filter)
}

pub fn pager_viewer() -> String {
    tr(MsgKey::pager_viewer)
}

pub fn webtoon_viewer() -> String {
    tr(MsgKey::webtoon_viewer)
}

pub fn pref_viewer_nav() -> String {
    tr(MsgKey::pref_viewer_nav)
}

pub fn pref_read_with_tapping_inverted() -> String {
    tr(MsgKey::pref_read_with_tapping_inverted)
}

pub fn pref_image_scale_type() -> String {
    tr(MsgKey::pref_image_scale_type)
}

pub fn pref_zoom_start() -> String {
    tr(MsgKey::pref_zoom_start)
}

pub fn pref_landscape_zoom() -> String {
    tr(MsgKey::pref_landscape_zoom)
}

pub fn pref_navigate_pan() -> String {
    tr(MsgKey::pref_navigate_pan)
}

pub fn pref_dual_page_invert() -> String {
    tr(MsgKey::pref_dual_page_invert)
}

pub fn pref_page_rotate() -> String {
    tr(MsgKey::pref_page_rotate)
}

pub fn pref_page_rotate_invert() -> String {
    tr(MsgKey::pref_page_rotate_invert)
}

pub fn pref_webtoon_side_padding() -> String {
    tr(MsgKey::pref_webtoon_side_padding)
}

pub fn pref_double_tap_zoom() -> String {
    tr(MsgKey::pref_double_tap_zoom)
}

pub fn pref_webtoon_disable_zoom_out() -> String {
    tr(MsgKey::pref_webtoon_disable_zoom_out)
}

pub fn pref_show_page_number() -> String {
    tr(MsgKey::pref_show_page_number)
}

pub fn pref_fullscreen() -> String {
    tr(MsgKey::pref_fullscreen)
}

/// The help line that records the dropped cutout setting on Linux (T-43b-050). The key stays in the store.
pub fn cutout_dropped_note() -> String {
    tr(MsgKey::reader_cutout_dropped_note)
}

pub fn pref_keep_screen_on() -> String {
    tr(MsgKey::pref_keep_screen_on)
}

pub fn pref_read_with_long_tap() -> String {
    tr(MsgKey::pref_read_with_long_tap)
}

pub fn pref_always_show_chapter_transition() -> String {
    tr(MsgKey::pref_always_show_chapter_transition)
}

pub fn pref_page_transitions() -> String {
    tr(MsgKey::pref_page_transitions)
}

pub fn pref_skip_read_chapters() -> String {
    tr(MsgKey::pref_skip_read_chapters)
}

pub fn pref_skip_filtered_chapters() -> String {
    tr(MsgKey::pref_skip_filtered_chapters)
}

pub fn pref_skip_dupe_chapters() -> String {
    tr(MsgKey::pref_skip_dupe_chapters)
}

pub fn pref_preserve_reading_position() -> String {
    tr(MsgKey::pref_preserve_reading_position)
}

pub fn pref_read_with_volume_keys() -> String {
    tr(MsgKey::pref_read_with_volume_keys)
}

pub fn pref_read_with_volume_keys_inverted() -> String {
    tr(MsgKey::pref_read_with_volume_keys_inverted)
}

/// Under the volume key rows: the desktop can take the media keys before the reader sees them (E-25 caveat, the
/// catalogue's drop note). PageUp, PageDown and Space always page, so nothing is lost by it.
pub fn volume_keys_media_caveat() -> String {
    tr(MsgKey::reader_volume_keys_media_caveat)
}

pub fn pref_double_tap_anim_speed() -> String {
    tr(MsgKey::pref_double_tap_anim_speed)
}

pub fn double_tap_anim_speed_0() -> String {
    tr(MsgKey::double_tap_anim_speed_0)
}

pub fn double_tap_anim_speed_normal() -> String {
    tr(MsgKey::double_tap_anim_speed_normal)
}

pub fn double_tap_anim_speed_fast() -> String {
    tr(MsgKey::double_tap_anim_speed_fast)
}

pub fn pref_hide_threshold() -> String {
    tr(MsgKey::pref_hide_threshold)
}

pub fn pref_highest() -> String {
    tr(MsgKey::pref_highest)
}

pub fn pref_high() -> String {
    tr(MsgKey::pref_high)
}

pub fn pref_low() -> String {
    tr(MsgKey::pref_low)
}

pub fn pref_lowest() -> String {
    tr(MsgKey::pref_lowest)
}

/// Kurobayomi only (T-43a-039): the switch of the crash restore.
pub fn pref_restore_after_crash() -> String {
    tr(MsgKey::reader_pref_restore_after_crash)
}

pub fn pref_flash_page() -> String {
    tr(MsgKey::pref_flash_page)
}

pub fn pref_flash_duration() -> String {
    tr(MsgKey::pref_flash_duration)
}

pub fn pref_flash_page_interval() -> String {
    tr(MsgKey::pref_flash_page_interval)
}

pub fn pref_flash_with() -> String {
    tr(MsgKey::pref_flash_with)
}

pub fn pref_flash_style_black() -> String {
    tr(MsgKey::pref_flash_style_black)
}

pub fn pref_flash_style_white() -> String {
    tr(MsgKey::pref_flash_style_white)
}

pub fn pref_flash_style_white_black() -> String {
    tr(MsgKey::pref_flash_style_white_black)
}

pub fn pref_custom_brightness() -> String {
    tr(MsgKey::pref_custom_brightness)
}

pub fn pref_custom_color_filter() -> String {
    tr(MsgKey::pref_custom_color_filter)
}

// The four colour labels were not re-read from strings.xml; verify before release (T-43a-061).
pub fn color_filter_r_value() -> String {
    tr(MsgKey::color_filter_r_value)
}

pub fn color_filter_g_value() -> String {
    tr(MsgKey::color_filter_g_value)
}

pub fn color_filter_b_value() -> String {
    tr(MsgKey::color_filter_b_value)
}

pub fn color_filter_a_value() -> String {
    tr(MsgKey::color_filter_a_value)
}

pub fn pref_color_filter_mode() -> String {
    tr(MsgKey::pref_color_filter_mode)
}

pub fn filter_mode_multiply() -> String {
    tr(MsgKey::filter_mode_multiply)
}

pub fn filter_mode_screen() -> String {
    tr(MsgKey::filter_mode_screen)
}

pub fn filter_mode_overlay() -> String {
    tr(MsgKey::filter_mode_overlay)
}

pub fn filter_mode_lighten() -> String {
    tr(MsgKey::filter_mode_lighten)
}

pub fn filter_mode_darken() -> String {
    tr(MsgKey::filter_mode_darken)
}

pub fn pref_grayscale() -> String {
    tr(MsgKey::pref_grayscale)
}

pub fn pref_inverted_colors() -> String {
    tr(MsgKey::pref_inverted_colors)
}

pub fn tapping_inverted_none() -> String {
    tr(MsgKey::tapping_inverted_none)
}

pub fn tapping_inverted_horizontal() -> String {
    tr(MsgKey::tapping_inverted_horizontal)
}

pub fn tapping_inverted_vertical() -> String {
    tr(MsgKey::tapping_inverted_vertical)
}

pub fn tapping_inverted_both() -> String {
    tr(MsgKey::tapping_inverted_both)
}

pub fn l_nav() -> String {
    tr(MsgKey::l_nav)
}

pub fn kindlish_nav() -> String {
    tr(MsgKey::kindlish_nav)
}

pub fn edge_nav() -> String {
    tr(MsgKey::edge_nav)
}

pub fn right_and_left_nav() -> String {
    tr(MsgKey::right_and_left_nav)
}

pub fn disabled_nav() -> String {
    tr(MsgKey::disabled_nav)
}

pub fn scale_type_fit_screen() -> String {
    tr(MsgKey::scale_type_fit_screen)
}

pub fn scale_type_stretch() -> String {
    tr(MsgKey::scale_type_stretch)
}

pub fn scale_type_fit_width() -> String {
    tr(MsgKey::scale_type_fit_width)
}

pub fn scale_type_fit_height() -> String {
    tr(MsgKey::scale_type_fit_height)
}

pub fn scale_type_original_size() -> String {
    tr(MsgKey::scale_type_original_size)
}

pub fn scale_type_smart_fit() -> String {
    tr(MsgKey::scale_type_smart_fit)
}

pub fn zoom_start_automatic() -> String {
    tr(MsgKey::zoom_start_automatic)
}

pub fn zoom_start_left() -> String {
    tr(MsgKey::zoom_start_left)
}

pub fn zoom_start_right() -> String {
    tr(MsgKey::zoom_start_right)
}

pub fn zoom_start_center() -> String {
    tr(MsgKey::zoom_start_center)
}


// The four paged names match the base `strings.xml` of the upstream app (G-34 closed).
pub fn left_to_right_viewer() -> String {
    tr(MsgKey::left_to_right_viewer)
}

pub fn right_to_left_viewer() -> String {
    tr(MsgKey::right_to_left_viewer)
}

pub fn vertical_viewer() -> String {
    tr(MsgKey::vertical_viewer)
}

pub fn vertical_plus_viewer() -> String {
    tr(MsgKey::vertical_plus_viewer)
}


/// `Source not installed: %1$s` (`source_not_installed`).
pub fn source_not_installed(source: &str) -> String {
    trf(MsgKey::source_not_installed, &[("arg1", Arg::Str(source))])
}

/// `Failed to load pages: %s`, the initial load error card (T-43a-024).
pub fn failed_to_load_pages(message: &str) -> String {
    trf(MsgKey::transition_pages_error, &[("arg1", Arg::Str(message))])
}

/// `Requested chapter of id %d not found in chapter list` (`ReaderViewModel.kt:158-159`).
pub fn requested_chapter_not_found(id: i64) -> String {
    trf(MsgKey::reader_requested_chapter_not_found, &[("arg1", Arg::Int(id))])
}

/// `missing_chapters_warning` (plural): one and other forms.
pub fn missing_chapters_warning(count: i32) -> String {
    tr_plural(MsgKey::missing_chapters_warning, i64::from(count), &[])
}

/// `pref_pages` (plural): "1 page" or "%1$s pages".
pub fn pages(count: i32) -> String {
    tr_plural(MsgKey::pref_pages, i64::from(count), &[])
}

/// `share_page_info`: "manga: chapter, page n".
pub fn share_page_info(manga: &str, chapter: &str, page: i32) -> String {
    trf(MsgKey::share_page_info, &[("arg1", Arg::Str(manga)), ("arg2", Arg::Str(chapter)), ("arg3", Arg::Int(i64::from(page)))])
}

/// `On` or `Off`, shown by the crop toast and the reading-mode toast.
pub fn on_off(value: bool) -> String {
    if value { on() } else { off() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reading_mode_names_match_the_base_strings() {
        assert_eq!(vertical_plus_viewer(), "Continuous vertical");
        assert_eq!(webtoon_viewer(), "Webtoon");
    }

    #[test]
    fn the_menu_entries_for_the_moved_bottom_bar_actions() {
        assert_eq!(menu_reader_settings(), "Reader settings…");
        assert_eq!(pref_crop_borders(), "Crop borders");
    }

    #[test]
    fn proposal_messages_match_the_spec_text() {
        assert_eq!(chapter_not_downloaded(), "The chapter is not downloaded. Turn off downloaded-only mode or download it first.");
        assert_eq!(unknown_error(), "Unknown error");
        assert_eq!(name_collision_suffix(2), " (2)");
        assert_eq!(name_collision_suffix(10), " (10)");
    }

    #[test]
    fn transition_and_error_strings_match_the_upstream_text() {
        assert_eq!(page_list_empty_error(), "No pages found");
        assert_eq!(transition_no_next(), "There's no next chapter");
        assert_eq!(source_not_installed("Foo"), "Source not installed: Foo");
        assert_eq!(failed_to_load_pages("No pages found"), "Failed to load pages: No pages found");
    }

    #[test]
    fn missing_chapters_warning_has_one_and_other_forms() {
        assert!(missing_chapters_warning(1).starts_with("Skipping 1 chapter,"));
        assert!(missing_chapters_warning(3).starts_with("Skipping 3 chapters,"));
    }

    #[test]
    fn plural_and_format_strings_follow_the_spec() {
        assert_eq!(pages(1), "1 page");
        assert_eq!(pages(4), "4 pages");
        assert_eq!(share_page_info("M", "C 1", 2), "M: C 1, page 2");
    }

    #[test]
    fn on_off_is_always_one_of_two_words() {
        assert_eq!(on_off(true), "On");
        assert_eq!(on_off(false), "Off");
    }
}
