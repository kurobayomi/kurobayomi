//! Kitsu tracker: identity, capability flags, status tables (T-30a-001, T-30a-002), the REST client, the Algolia
//! search and the tracker traits in [`tracker::KitsuTracker`].

pub mod api;
pub mod auth;
pub mod dates;
pub mod dialog;
pub mod dto;
pub mod error;
pub mod import;
#[cfg(test)]
mod fixture_tests;
pub mod rules;
pub mod score;
pub mod search;
pub mod status;
pub mod tracker;
#[cfg(test)]
mod wire_tests;

use kuro_platform::kind::Kind;
pub use tracker::KitsuTracker;

use crate::model::TrackerId;

/// Registry id of Kitsu (`TrackerManager.kt`).
pub const TRACKER_ID: TrackerId = TrackerId::KITSU;
pub const NAME: &str = "Kitsu";

/// Kitsu library status codes (`Kitsu.kt`). Manga and anime use different codes for "reading" and "planned".
pub const READING: i64 = 1;
pub const WATCHING: i64 = 11;
pub const COMPLETED: i64 = 2;
pub const ON_HOLD: i64 = 3;
pub const DROPPED: i64 = 4;
pub const PLAN_TO_READ: i64 = 5;
pub const PLAN_TO_WATCH: i64 = 15;

/// Manga status list in source order (`status_list(Manga)`).
pub const MANGA_STATUSES: [i64; 5] = [READING, COMPLETED, ON_HOLD, DROPPED, PLAN_TO_READ];
/// Anime status list in source order (`status_list(Anime)`).
pub const ANIME_STATUSES: [i64; 5] = [WATCHING, PLAN_TO_WATCH, COMPLETED, ON_HOLD, DROPPED];

/// The status list of one kind.
pub fn status_list(kind: Kind) -> &'static [i64] {
    match kind {
        Kind::Manga => &MANGA_STATUSES,
        Kind::Anime => &ANIME_STATUSES,
    }
}

/// The label of a status of `kind`. `None` for a code of the other kind, or any unknown code (KT-10).
pub fn status_label(kind: Kind, status: i64) -> Option<&'static str> {
    Some(match (kind, status) {
        (Kind::Manga, READING) => "Currently reading",
        (Kind::Manga, PLAN_TO_READ) => "Want to read",
        (Kind::Anime, WATCHING) => "Currently watching",
        (Kind::Anime, PLAN_TO_WATCH) => "Want to watch",
        (_, COMPLETED) => "Completed",
        (_, ON_HOLD) => "On hold",
        (_, DROPPED) => "Dropped",
        _ => return None,
    })
}

/// Kitsu has no rereading status.
pub fn rereading_status() -> Option<i64> {
    None
}

/// Kitsu has no rewatching status.
pub fn rewatching_status() -> Option<i64> {
    None
}

/// The status a finished title gets (`completion_status`).
pub fn completion_status() -> i64 {
    COMPLETED
}

/// Reading and watching dates are shown and sent (the track sheet shows date rows for Kitsu only).
pub fn supports_reading_dates() -> bool {
    true
}

/// The private button is shown for Kitsu (`supports_private_tracking`).
pub fn supports_private_tracking() -> bool {
    true
}

/// Kitsu can delete a library entry for both kinds.
pub fn is_deletable(_kind: Kind) -> bool {
    true
}

/// Logged in only when both the username and the user id are set (T-30a-001).
pub fn is_logged_in(username: &str, user_id: &str) -> bool {
    !username.is_empty() && !user_id.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_30a_001_identity_and_capability_flags() {
        assert_eq!(TRACKER_ID.0, 3);
        assert_eq!(NAME, "Kitsu");
        assert!(is_deletable(Kind::Manga) && is_deletable(Kind::Anime));
        assert!(supports_reading_dates());
        assert!(supports_private_tracking());
        assert_eq!(completion_status(), 2);
        assert_eq!(rereading_status(), None);
        assert_eq!(rewatching_status(), None);
    }

    #[test]
    fn t_30a_001_logged_in_needs_a_username_and_a_user_id() {
        assert!(is_logged_in("ann", "42"));
        assert!(!is_logged_in("", "42"));
        assert!(!is_logged_in("ann", ""));
        assert!(!is_logged_in("", ""));
    }

    #[test]
    fn t_30a_002_status_constants_have_the_source_codes() {
        assert_eq!(
            [
                READING,
                WATCHING,
                COMPLETED,
                ON_HOLD,
                DROPPED,
                PLAN_TO_READ,
                PLAN_TO_WATCH
            ],
            [1, 11, 2, 3, 4, 5, 15]
        );
    }

    #[test]
    fn t_30a_002_status_lists_keep_the_source_order() {
        assert_eq!(status_list(Kind::Manga), &[1, 2, 3, 4, 5]);
        assert_eq!(status_list(Kind::Anime), &[11, 15, 2, 3, 4]);
    }

    #[test]
    fn t_30a_002_labels_follow_the_kind_and_reject_other_codes() {
        assert_eq!(status_label(Kind::Manga, 1), Some("Currently reading"));
        assert_eq!(status_label(Kind::Manga, 5), Some("Want to read"));
        assert_eq!(status_label(Kind::Anime, 11), Some("Currently watching"));
        assert_eq!(status_label(Kind::Anime, 15), Some("Want to watch"));
        assert_eq!(status_label(Kind::Anime, 2), Some("Completed"));
        assert_eq!(status_label(Kind::Manga, 3), Some("On hold"));
        assert_eq!(status_label(Kind::Anime, 4), Some("Dropped"));
        assert_eq!(
            status_label(Kind::Manga, 11),
            None,
            "anime code under manga"
        );
        assert_eq!(status_label(Kind::Anime, 5), None, "manga code under anime");
        assert_eq!(status_label(Kind::Manga, 99), None);
    }
}
