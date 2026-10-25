//! The action row under an entry header (T-38a-017, T-39a-026) and the next update label (T-38a-018). Upstream:
//! `MangaInfoHeader.kt:163-240` (manga) and `AnimeInfoHeader.kt` (anime). The rules are here; `action_row_view` draws them.
//!
//! The row has four equal buttons: library, interval, tracking and Browser. An inactive button uses the foreground at
//! 0.38 alpha (`INACTIVE_ALPHA`), an active one the primary colour.

use kuro_domain::entries::fetch_interval::next_update_days;
use kuro_i18n::MsgKey;

/// The colour of a button: the primary colour when it is active, the dimmed foreground otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Inactive,
}

/// The alpha of the foreground of an inactive button (`LocalContentColor` at 0.38).
pub const INACTIVE_ALPHA: f64 = 0.38;

/// The widget opacity of a button in `tone`: full for the primary colour, `INACTIVE_ALPHA` otherwise.
pub fn tone_alpha(tone: Tone) -> f64 {
    match tone {
        Tone::Primary => 1.0,
        Tone::Inactive => INACTIVE_ALPHA,
    }
}

/// The library button: "In library" with a filled heart for a favorite, "Add to library" with an outlined one otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibraryButton {
    pub label: MsgKey,
    pub filled: bool,
    pub tone: Tone,
}

pub fn library(favorite: bool) -> LibraryButton {
    if favorite {
        LibraryButton { label: MsgKey::in_library, filled: true, tone: Tone::Primary }
    } else {
        LibraryButton { label: MsgKey::add_to_library, filled: false, tone: Tone::Inactive }
    }
}

/// A long press on the library button opens the category editor only when the entry is a favorite (E-13).
pub fn library_long_press_opens_categories(favorite: bool) -> bool {
    favorite
}

/// The next update, as the interval button shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextUpdate {
    /// No next update (or a completed entry): "N/A".
    NotApplicable,
    /// Due today, or already past: "Soon".
    Soon,
    /// Whole days left: "1 day", "N days".
    Days(i64),
}

/// `days = max(0, trunc((next - now) in days))` (`next_update_days`). A null next update, or a completed entry, is not applicable.
/// The whole-day count truncates toward zero, so +36 h is one day and +23 h is "Soon" (T-38a-018).
pub fn next_update(next_ms: Option<i64>, now_ms: i64, completed: bool) -> NextUpdate {
    match next_update_days((!completed).then_some(next_ms).flatten(), now_ms) {
        None => NextUpdate::NotApplicable,
        Some(0) => NextUpdate::Soon,
        Some(days) => NextUpdate::Days(days),
    }
}

/// The interval button: the primary colour when the entry has its own interval (`fetch_interval < 0`).
pub fn interval_tone(user_interval: bool) -> Tone {
    if user_interval { Tone::Primary } else { Tone::Inactive }
}

/// The interval dialog opens only for a favorite (E-14).
pub fn interval_click_opens_dialog(favorite: bool) -> bool {
    favorite
}

/// The tracking button text: "Tracking" with no tracker, "N trackers" otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingLabel {
    Tracking,
    Trackers(i64),
}

pub fn tracking_label(count: i64) -> TrackingLabel {
    if count == 0 { TrackingLabel::Tracking } else { TrackingLabel::Trackers(count) }
}

/// The tracking button is primary once a tracker is bound, dimmed while none is.
pub fn tracking_tone(count: i64) -> Tone {
    if count == 0 { Tone::Inactive } else { Tone::Primary }
}

/// Whether the row shows the tracking button at all. Anime shows it only when a tracking callback exists; manga always has one.
pub fn tracking_visible(has_callback: bool) -> bool {
    has_callback
}

/// The Browser button (the title in the system browser) is shown only for an HTTP source (E-26).
pub fn webview_visible(http_source: bool) -> bool {
    http_source
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    #[test]
    fn next_update_counts_whole_days_and_truncates() {
        let now = 1_000 * DAY;
        assert_eq!(next_update(Some(now + DAY + DAY / 2), now, false), NextUpdate::Days(1), "+36 h");
        assert_eq!(next_update(Some(now + 3 * DAY + 5), now, false), NextUpdate::Days(3));
    }

    #[test]
    fn next_update_under_a_day_or_in_the_past_is_soon() {
        let now = 1_000 * DAY;
        assert_eq!(next_update(Some(now + 23 * DAY / 24), now, false), NextUpdate::Soon, "+23 h");
        assert_eq!(next_update(Some(now - 2 * DAY), now, false), NextUpdate::Soon, "past");
        assert_eq!(next_update(Some(now), now, false), NextUpdate::Soon);
    }

    #[test]
    fn next_update_is_not_applicable_without_a_date_or_for_a_completed_entry() {
        let now = 1_000 * DAY;
        assert_eq!(next_update(None, now, false), NextUpdate::NotApplicable);
        assert_eq!(next_update(Some(now + 5 * DAY), now, true), NextUpdate::NotApplicable, "completed");
    }

    #[test]
    fn library_button_switches_label_icon_and_colour_with_the_favorite_state() {
        let fav = library(true);
        assert_eq!((fav.label, fav.filled, fav.tone), (MsgKey::in_library, true, Tone::Primary));
        let not = library(false);
        assert_eq!((not.label, not.filled, not.tone), (MsgKey::add_to_library, false, Tone::Inactive));
    }

    #[test]
    fn an_inactive_button_is_drawn_at_038_alpha_and_an_active_one_at_full() {
        assert_eq!(tone_alpha(Tone::Inactive), 0.38);
        assert_eq!(tone_alpha(Tone::Primary), 1.0);
    }

    #[test]
    fn library_long_press_and_interval_click_need_a_favorite() {
        assert!(library_long_press_opens_categories(true));
        assert!(!library_long_press_opens_categories(false));
        assert!(interval_click_opens_dialog(true));
        assert!(!interval_click_opens_dialog(false));
    }

    #[test]
    fn interval_button_is_primary_only_for_a_user_interval() {
        assert_eq!(interval_tone(true), Tone::Primary);
        assert_eq!(interval_tone(false), Tone::Inactive);
    }

    #[test]
    fn tracking_label_counts_trackers() {
        assert_eq!(tracking_label(0), TrackingLabel::Tracking);
        assert_eq!(tracking_label(2), TrackingLabel::Trackers(2));
        assert_eq!(tracking_tone(0), Tone::Inactive);
        assert_eq!(tracking_tone(1), Tone::Primary);
    }

    #[test]
    fn tracking_and_webview_visibility_follow_the_callback_and_the_source() {
        assert!(!tracking_visible(false));
        assert!(tracking_visible(true));
        assert!(webview_visible(true));
        assert!(!webview_visible(false));
    }
}
