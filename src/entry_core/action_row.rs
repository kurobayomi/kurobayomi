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

/// Whether the row shows the next-update button. A finished title has no next update, so it has no button: it read
/// "N/A" and said nothing (user, 2026-11-20). A title still coming out shows it when there is an estimate, and also
/// when there is none but the title is in the library, where the button opens the dialog that sets the interval. An
/// anime the score site lists as on air (`on_air`) shows its weekly time whatever the source says of it.
pub fn interval_visible(next: NextUpdate, favorite: bool, completed: bool, on_air: bool) -> bool {
    on_air || (!completed && (next != NextUpdate::NotApplicable || favorite))
}

/// When an anime on air is broadcast each week, on the clock of the user: the weekday and the minute of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirTime {
    pub weekday: chrono::Weekday,
    pub minute: u32,
}

const DAY_MINUTES: i32 = 24 * 60;

/// The weekly slot `utc_week_minute` (minutes after Monday 00:00 UTC, `kuro_scores::provider::AirSlot`) on a clock
/// `offset_minutes` east of UTC. The day may be the one before or after the UTC day.
pub fn air_time(utc_week_minute: u32, offset_minutes: i32) -> AirTime {
    let local = (utc_week_minute as i32 + offset_minutes).rem_euclid(7 * DAY_MINUTES);
    let weekday = match local / DAY_MINUTES {
        0 => chrono::Weekday::Mon,
        1 => chrono::Weekday::Tue,
        2 => chrono::Weekday::Wed,
        3 => chrono::Weekday::Thu,
        4 => chrono::Weekday::Fri,
        5 => chrono::Weekday::Sat,
        _ => chrono::Weekday::Sun,
    };
    AirTime { weekday, minute: (local % DAY_MINUTES) as u32 }
}

/// The weekly time as the button writes it under its count: "Wed 14:00". The weekday is chrono's short name, as on the
/// Upcoming calendar.
pub fn air_text(air: AirTime) -> String {
    format!("{} {:02}:{:02}", air.weekday, air.minute / 60, air.minute % 60)
}

/// The days to the next weekly broadcast, in the words the button uses for every title ("Soon", "N days"): an anime
/// and a manga read alike (user, 2026-11-20). `now_weekday` and `now_minute` are the weekday and the minute of the day
/// on the clock `air` is on. Whole days, cut like the estimate: within a day is "Soon", and a slot that has just
/// passed counts to next week's. The site does not say when a title is on a break, so the count is to the slot.
pub fn next_air(air: AirTime, now_weekday: chrono::Weekday, now_minute: u32) -> NextUpdate {
    let slot = air.weekday.num_days_from_monday() as i32 * DAY_MINUTES + air.minute as i32;
    let now = now_weekday.num_days_from_monday() as i32 * DAY_MINUTES + now_minute as i32;
    match (slot - now).rem_euclid(7 * DAY_MINUTES) / DAY_MINUTES {
        0 => NextUpdate::Soon,
        days => NextUpdate::Days(i64::from(days)),
    }
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
    fn the_next_update_button_is_shown_only_when_it_has_something_to_say_or_to_set() {
        assert!(!interval_visible(NextUpdate::NotApplicable, true, true, false), "finished: no next update, in the library or not");
        assert!(!interval_visible(NextUpdate::Days(3), false, true, false), "a finished title has none, whatever was estimated");
        assert!(interval_visible(NextUpdate::Days(3), false, false, false));
        assert!(interval_visible(NextUpdate::Soon, false, false, false));
        assert!(interval_visible(NextUpdate::NotApplicable, true, false, false), "no estimate yet: the library title can still set one");
        assert!(!interval_visible(NextUpdate::NotApplicable, false, false, false), "nothing to say and nothing to set");
        assert!(interval_visible(NextUpdate::NotApplicable, false, true, true), "on air on the score site: its weekly time is shown");
    }

    #[test]
    fn a_weekly_slot_is_written_on_the_users_clock_and_the_day_follows() {
        use chrono::Weekday;
        // Wednesday 13:00 UTC (22:00 in Japan)
        let slot = 2 * 24 * 60 + 13 * 60;
        assert_eq!(air_time(slot, 0), AirTime { weekday: Weekday::Wed, minute: 13 * 60 });
        assert_eq!(air_text(air_time(slot, 60)), "Wed 14:00", "an hour east of UTC");
        assert_eq!(air_text(air_time(slot, 9 * 60)), "Wed 22:00", "in Japan");
        assert_eq!(air_text(air_time(slot, -8 * 60)), "Wed 05:00");
        assert_eq!(air_text(air_time(slot, 12 * 60)), "Thu 01:00", "far east: the next day");
        // Monday 01:30 UTC, seen from the west, is still Sunday: the week wraps
        assert_eq!(air_text(air_time(90, -5 * 60)), "Sun 20:30");
        // Sunday 23:00 UTC, seen from the east, is Monday already
        assert_eq!(air_text(air_time(6 * 24 * 60 + 23 * 60, 2 * 60)), "Mon 01:00");
        // a zone of a half hour
        assert_eq!(air_text(air_time(slot, 5 * 60 + 30)), "Wed 18:30");
    }

    #[test]
    fn the_days_to_the_weekly_slot_are_counted_like_the_estimate() {
        use chrono::Weekday;
        let wednesday_two_pm = AirTime { weekday: Weekday::Wed, minute: 14 * 60 };
        assert_eq!(next_air(wednesday_two_pm, Weekday::Fri, 10 * 60), NextUpdate::Days(5), "Friday morning: next Wednesday");
        assert_eq!(next_air(wednesday_two_pm, Weekday::Tue, 13 * 60), NextUpdate::Days(1), "+25 h");
        assert_eq!(next_air(wednesday_two_pm, Weekday::Tue, 15 * 60), NextUpdate::Soon, "+23 h");
        assert_eq!(next_air(wednesday_two_pm, Weekday::Wed, 13 * 60), NextUpdate::Soon, "in an hour");
        assert_eq!(next_air(wednesday_two_pm, Weekday::Wed, 14 * 60), NextUpdate::Soon, "now");
        assert_eq!(next_air(wednesday_two_pm, Weekday::Wed, 14 * 60 + 1), NextUpdate::Days(6), "a minute ago: next week's");
        // across the end of the week
        let monday_one_am = AirTime { weekday: Weekday::Mon, minute: 60 };
        assert_eq!(next_air(monday_one_am, Weekday::Sun, 23 * 60), NextUpdate::Soon, "in two hours");
        assert_eq!(next_air(monday_one_am, Weekday::Sat, 0), NextUpdate::Days(2));
    }

    #[test]
    fn tracking_and_webview_visibility_follow_the_callback_and_the_source() {
        assert!(!tracking_visible(false));
        assert!(tracking_visible(true));
        assert!(webview_visible(true));
        assert!(!webview_visible(false));
    }
}
