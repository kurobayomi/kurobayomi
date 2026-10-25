//! The player's localised string table (area 46 p1-behavior section 14, T-46a-060). One table holds
//! the English text of every key the player uses, with plural forms where the key needs one. The
//! Netflix countdown prefix is a key here (E-46-067), not a literal in the toast code.
//!
//! The text of `PlayerMessage` still comes from the `kuro-i18n` catalogue, which translators own.
//! This table is the English base the player is checked against: a key that the code uses must be
//! in it (see the tests in `errors.rs`).

/// One string. `other` is the plural form for counts other than one, `None` for a fixed string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: &'static str,
    pub text: &'static str,
    pub other: Option<&'static str>,
}

const fn fixed(key: &'static str, text: &'static str) -> Entry {
    Entry {
        key,
        text,
        other: None,
    }
}

/// Every key of section 14 that the player reads, plus the English fallbacks of the messages in
/// `errors.rs`. `%s` and `%d` slots are filled by the caller.
pub const TABLE: &[Entry] = &[
    fixed("player_speed", "%.2fx"),
    Entry {
        key: "seconds",
        text: "%d second",
        other: Some("%d seconds"),
    },
    fixed("numeric_chooser_value_too_big", "Value too big"),
    fixed("numeric_chooser_value_too_small", "Value too small"),
    fixed("display_mode_episode", "Episode %1$s"),
    fixed("episodes", "Episodes"),
    fixed("player_sheets_audio_delay_title", "Audio delay"),
    fixed("enable_auto_play", "Auto-play is on"),
    fixed("disable_auto_play", "Auto-play is off"),
    fixed("video_crop_screen", "Cropped to screen"),
    fixed("video_fit_screen", "Fit to screen"),
    fixed("video_stretch_screen", "Stretched to screen"),
    fixed("player_skip_action", "Skip %1$s"),
    fixed("player_aniskip_skip", "%s skipped"),
    fixed("player_aniskip_dontskip", "Don't skip"),
    fixed("player_aniskip_dontskip_toast", "Skipping %s in %d seconds"),
    fixed("player_aniskip_toast_prefix", "Skip Intro: "),
    fixed("player_intro_skipped", "%1$s skipped"),
    fixed("player_chapter_type_opening", "Opening"),
    fixed("player_chapter_type_ending", "Ending"),
    fixed("player_chapter_type_recap", "Recap"),
    fixed("player_chapter_type_mixedop", "Mixed opening"),
    fixed("no_prev_episode", "Previous Episode not found!"),
    fixed("no_next_episode", "Next Episode not found!"),
    fixed("player_sheets_filters_brightness", "Brightness"),
    fixed("player_sheets_sub_delay_title", "Subtitle delay"),
    fixed("player_sheets_sub_delay_delay", "Delay"),
    fixed("player_generic_unit_ms", "ms"),
    fixed("player_sheets_delay_set_as_default", "Set as default"),
    fixed("player_sheets_sub_delay_audio_sound_heard", "Sound heard"),
    fixed(
        "player_sheets_sub_delay_sound_sound_spotted",
        "Sound spotted",
    ),
    fixed(
        "player_sheets_sub_delay_subtitle_voice_heard",
        "Voice heard",
    ),
    fixed("player_sheets_sub_delay_subtitle_text_seen", "Text seen"),
    fixed("action_cancel", "Cancel"),
    fixed("action_ok", "OK"),
    fixed("toast_sleep_timer_ended", "Sleep timer ended"),
    fixed("no_hosters", "Hoster list is empty"),
    fixed("no_episode_loaded", "No episode loaded"),
    fixed("no_available_videos", "No available videos"),
    fixed(
        "select_hoster_from_empty_list",
        "Hoster selected from empty list",
    ),
    fixed("picture_saved", "Picture saved"),
    fixed("http_server_start_failure", "Failed to start http server"),
    fixed("http_server_is_running", "Http server is running"),
    fixed("torrentserver_is_running", "Torrent Server is running"),
    fixed("action_close", "Close"),
    fixed("action_stop", "Stop"),
    fixed("player_screenshot_failed", "Screenshot failed"),
];

/// The English text of a fixed key, or of the singular form of a plural key.
pub fn english(key: &str) -> Option<&'static str> {
    TABLE.iter().find(|e| e.key == key).map(|e| e.text)
}

/// The text for a count: the singular form for 1, the plural form otherwise, with `%d` replaced by
/// the count. `None` for an unknown key.
pub fn plural(key: &str, count: i64) -> Option<String> {
    let entry = TABLE.iter().find(|e| e.key == key)?;
    let template = if count == 1 {
        entry.text
    } else {
        entry.other.unwrap_or(entry.text)
    };
    Some(template.replace("%d", &count.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique() {
        for (i, entry) in TABLE.iter().enumerate() {
            assert!(
                TABLE[i + 1..].iter().all(|other| other.key != entry.key),
                "duplicate key {}",
                entry.key
            );
        }
    }

    #[test]
    fn countdown_prefix_is_a_key_with_its_english_text() {
        assert_eq!(english("player_aniskip_toast_prefix"), Some("Skip Intro: "));
    }

    #[test]
    fn plural_keys_pick_the_form_for_the_count() {
        assert_eq!(plural("seconds", 1).as_deref(), Some("1 second"));
        assert_eq!(plural("seconds", 0).as_deref(), Some("0 seconds"));
        assert_eq!(plural("seconds", 30).as_deref(), Some("30 seconds"));
        assert_eq!(plural("missing", 2), None);
    }
}
