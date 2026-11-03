//! Message keys of the tracking flows. The core returns
//! keys; the GUI resolves them with the i18n crate. `english` gives the source's English text, used by the tests
//! and as the fallback. Every key maps to a Fluent id in the English catalogue, checked at build time.

use crate::mal::messages as mal_text;

/// The English catalogue. Included so that a missing file fails the build of this crate.
pub const EN_CATALOGUE: &str = include_str!("../../kuro-i18n/locales/en/kurobayomi.ftl");

/// Fluent ids of the user-visible strings of behaviour section 18, all present in the English catalogue.
pub const SECTION_18_IDS: &[&str] = &[
    "pref_category_tracking",
    "tracking_guide",
    "tracking_info",
    "services",
    "enhanced_services",
    "enhanced_tracking_info",
    "enhanced_services_not_installed",
    "login_title",
    "email",
    "username",
    "password",
    "login",
    "loading",
    "login_success",
    "logout_title",
    "logout",
    "logout_success",
    "action_close",
    "action_cancel",
    "action_ok",
    "action_remove",
    "lock_always",
    "default_category_summary",
    "lock_never",
    "pref_auto_update_manga_sync",
    "pref_track_on_add_library",
    "pref_show_next_episode_airing_time",
    "pref_auto_update_manga_on_mark_read",
    "track_error",
    "confirm_tracker_update",
    "confirm_tracker_update_anime",
    "trackers_updated_summary_manga",
    "trackers_updated_summary_anime",
    "action_search_hint",
    "no_results_found",
    "unknown_error",
    "action_track",
    "action_toggle_private_on",
    "action_toggle_private_off",
    "tracked_privately",
    "add_tracking",
    "error_no_match",
    "action_open_in_browser",
    "action_copy_link",
    "action_copy_to_clipboard",
    "label_more",
    "status",
    "track_status",
    "track_type",
    "label_started",
    "score",
    "chapters",
    "episodes",
    "track_started_reading_date",
    "track_finished_reading_date",
    "track_remove_date_conf_title",
    "track_remove_start_date_conf_text",
    "track_remove_finish_date_conf_text",
    "track_delete_title",
    "track_delete_text",
    "track_delete_remote_text",
    "copied_to_clipboard",
];

/// One user-visible message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgKey {
    /// `"%1$s error: %2$s"`.
    TrackerError { tracker: String, detail: String },
    /// "Logged in".
    LoginSuccess,
    /// "Login failed", shown when the failure carries no message (defect D14).
    LoginFailed,
    /// "You are now logged out".
    LogoutSuccess,
    /// "Trackers updated to episode %d".
    TrackersUpdatedAnime(i32),
    /// "Trackers updated to chapter %d".
    TrackersUpdatedManga(i32),
    /// "Update trackers to episode %d?".
    ConfirmUpdateAnime(i32),
    /// "Update trackers to chapter %d?".
    ConfirmUpdateManga(i32),
    /// "No match found", the enhanced match failure.
    NoMatchFound,
    /// "No results found", an empty search.
    NoResultsFound,
    /// "Unknown error", a search failure with no message.
    UnknownError,
    /// "Copied to clipboard:\n%1$s". The text is already truncated, see [`MsgKey::copied`].
    CopiedToClipboard(String),
    /// "Could not remove the local tracking". Shown when the local delete fails after the remote step (E-39).
    LocalRemovalFailed,
    /// "No link stored for this entry". The tracker page cannot be opened or copied (E-43).
    NoTrackingLink,
    /// "N tracking updates pending": the parked-queue toast and the sheet's pending badge (T-27a-071).
    PendingUpdates(usize),
    /// "<name> logged in" (proposal key, part 2).
    TrackerLoggedIn(String),
    /// "<name> login failed".
    TrackerLoginFailed(String),
    /// "<name> login cancelled".
    TrackerLoginCancelled(String),
    /// "Unrecognized login link".
    UnrecognizedLoginLink,
    /// "<name> is not configured".
    TrackerNotConfigured(String),
    /// "Login expired, start again".
    LoginExpired,
    /// "No tracking data for this tracker".
    NoTrackingData,
    /// "Cancel" (`action_cancel`).
    ActionCancel,
    /// "Remove" (`action_remove`).
    ActionRemove,
    /// "OK" (`action_ok`).
    ActionOk,
    /// "Remove date?".
    RemoveDateTitle,
    /// "This will remove your previously selected start date from %s".
    RemoveStartDateText(String),
    /// "This will remove your previously selected finish date from %s".
    RemoveFinishDateText(String),
    /// "Remove %s tracking?".
    TrackDeleteTitle(String),
    /// "This will remove the tracking locally."
    TrackDeleteText,
    /// "Also remove from %s".
    TrackDeleteRemoteText(String),
    /// "Start date".
    StartDate,
    /// "Finish date".
    FinishDate,
    /// "Status" (the status row title).
    Status,
    /// "Score".
    Score,
    /// "Chapters" (manga progress title).
    Chapters,
    /// "Episodes" (anime progress title).
    Episodes,
    /// "You are not logged in to AniList" (T-28a-045).
    AniListNotAuthenticated,
    /// "Session expired, log in again". Also the subtitle of the AniList login row (T-28a-048).
    AniListSessionExpired,
    /// "Kitsu session expired. Log in again." The subtitle of an expired Kitsu login row (T-30a-031, Proposal text).
    KitsuSessionExpired,
    /// "This entry is not on your AniList list".
    AniListNotInList,
    /// "AniList rate limit reached, try again in a minute".
    AniListRateLimited,
    /// "No network connection to AniList".
    AniListOffline,
    /// "Unknown AniList status".
    AniListUnknownStatus,
    /// "AniList login failed".
    AniListLoginFailed,
    /// "Could not remove the entry from AniList".
    AniListRemoteDeleteFailed,
    /// "Logged in to AniList" (T-28a-048).
    AniListLoggedIn,
    /// "Logging in…", the AniList login row while the Viewer query runs.
    AniListLoggingIn,
    /// "AniList login must be renewed", after the legacy score preference forced a logout (T-28a-013).
    AniListReloginRequired,
    /// "Re-added to MyAnimeList": a refresh found the title missing and added it again (D-10).
    MalReAdded,
    /// "Waiting for MyAnimeList...": the Settings row while the loopback login waits (T-29a-047).
    MalWaitingForLogin,
    /// "Set a MyAnimeList client id to log in": the Settings row when no client id is configured (T-29a-040).
    MalNoClientId,
    /// "Log in again": the Settings row when the MAL login expired (E-UI-2, T-29a-047).
    MalLogInAgain,
    /// "Login was not completed": consent refused or no code (D-04, T-29a-051).
    MalLoginNotCompleted,
    /// An enhanced-tracker failure shown through its `track.enhanced.*` key (T-33a-022). `arg` fills `%s`, empty
    /// when the text has none. Use [`MsgKey::enhanced`].
    Enhanced { key: &'static str, arg: String },
    /// "HTTP %d, check website in WebView" (`exception_http`), a server answer other than success (T-31a-041).
    HttpStatus(u16),
    /// `track_error` with a fixed detail key instead of free text (T-31a-041, design section 10).
    TrackerDetail {
        tracker: String,
        detail: TrackDetail,
    },
    /// Raw text of a failure, shown as is.
    Raw(String),
}

/// The fixed detail of a [`MsgKey::TrackerDetail`] toast. Each variant is a message of its own, so no English
/// text from an error type reaches the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackDetail {
    NotAuthenticated,
    TooManyEntries,
    NotFoundManga,
    NotFoundAnime,
    UnknownStatus,
    NoLibraryId,
    /// The answer of the server could not be read.
    InvalidResponse,
    /// The request did not reach the server.
    Network,
    SecretStorage,
}

impl TrackDetail {
    /// English text of the detail, used by the tests and as the fallback.
    pub fn english(self) -> &'static str {
        match self {
            TrackDetail::NotAuthenticated => "Not authenticated",
            TrackDetail::TooManyEntries => "Too many entries in response",
            TrackDetail::NotFoundManga => "Could not find manga",
            TrackDetail::NotFoundAnime => "Could not find anime",
            TrackDetail::UnknownStatus => "Unknown status",
            TrackDetail::NoLibraryId => "Missing library id",
            TrackDetail::InvalidResponse => "Invalid response",
            TrackDetail::Network => "Network error",
            TrackDetail::SecretStorage => "Secret storage is unavailable",
        }
    }
}

impl MsgKey {
    pub fn enhanced(key: &'static str, arg: &str) -> Self {
        MsgKey::Enhanced {
            key,
            arg: arg.to_owned(),
        }
    }

    /// The copy toast for `content`, truncated to 50 characters (`truncateCenter(50)`).
    pub fn copied(content: &str) -> Self {
        MsgKey::CopiedToClipboard(truncate_center(content, 50))
    }

    /// The Fluent id of the message. `None` for raw text and for the two messages that have no catalogue id yet
    /// (`LoginFailed`, `LocalRemovalFailed`, `NoTrackingLink`); the GUI shows [`english`] for those until the i18n
    /// lane adds them.
    pub fn resource_id(&self) -> Option<&'static str> {
        Some(match self {
            MsgKey::TrackerError { .. } => "track_error",
            MsgKey::LoginSuccess => "login_success",
            MsgKey::LogoutSuccess => "logout_success",
            MsgKey::TrackersUpdatedAnime(_) => "trackers_updated_summary_anime",
            MsgKey::TrackersUpdatedManga(_) => "trackers_updated_summary_manga",
            MsgKey::ConfirmUpdateAnime(_) => "confirm_tracker_update_anime",
            MsgKey::ConfirmUpdateManga(_) => "confirm_tracker_update",
            MsgKey::NoMatchFound => "error_no_match",
            MsgKey::NoResultsFound => "no_results_found",
            MsgKey::UnknownError => "unknown_error",
            MsgKey::CopiedToClipboard(_) => "copied_to_clipboard",
            MsgKey::ActionCancel => "action_cancel",
            MsgKey::ActionRemove => "action_remove",
            MsgKey::ActionOk => "action_ok",
            MsgKey::RemoveDateTitle => "track_remove_date_conf_title",
            MsgKey::RemoveStartDateText(_) => "track_remove_start_date_conf_text",
            MsgKey::RemoveFinishDateText(_) => "track_remove_finish_date_conf_text",
            MsgKey::TrackDeleteTitle(_) => "track_delete_title",
            MsgKey::TrackDeleteText => "track_delete_text",
            MsgKey::TrackDeleteRemoteText(_) => "track_delete_remote_text",
            MsgKey::StartDate => "track_started_reading_date",
            MsgKey::FinishDate => "track_finished_reading_date",
            MsgKey::Status => "status",
            MsgKey::Score => "score",
            MsgKey::Chapters => "chapters",
            MsgKey::Episodes => "episodes",
            MsgKey::HttpStatus(_) => "exception_http",
            MsgKey::LoginFailed
            | MsgKey::LocalRemovalFailed
            | MsgKey::NoTrackingLink
            | MsgKey::PendingUpdates(_)
            | MsgKey::TrackerLoggedIn(_)
            | MsgKey::TrackerLoginFailed(_)
            | MsgKey::TrackerLoginCancelled(_)
            | MsgKey::UnrecognizedLoginLink
            | MsgKey::TrackerNotConfigured(_)
            | MsgKey::LoginExpired
            | MsgKey::NoTrackingData
            | MsgKey::AniListNotAuthenticated
            | MsgKey::AniListSessionExpired
            | MsgKey::KitsuSessionExpired
            | MsgKey::AniListNotInList
            | MsgKey::AniListRateLimited
            | MsgKey::AniListOffline
            | MsgKey::AniListUnknownStatus
            | MsgKey::AniListLoginFailed
            | MsgKey::AniListRemoteDeleteFailed
            | MsgKey::AniListLoggedIn
            | MsgKey::AniListLoggingIn
            | MsgKey::AniListReloginRequired
            | MsgKey::MalNoClientId
            | MsgKey::MalReAdded
            | MsgKey::MalWaitingForLogin
            | MsgKey::MalLogInAgain
            | MsgKey::MalLoginNotCompleted
            | MsgKey::TrackerDetail { .. }
            | MsgKey::Enhanced { .. }
            | MsgKey::Raw(_) => return None,
        })
    }
}

/// `truncateCenter`: when longer than `count`, keeps the start and the end with "..." in the middle. The pieces
/// are `floor((count - 3) / 2)` characters each, so the result is `2 * piece + 3` characters.
pub fn truncate_center(text: &str, count: usize) -> String {
    let len = text.chars().count();
    if len <= count {
        return text.to_owned();
    }
    let piece = count.saturating_sub(3) / 2;
    let head: String = text.chars().take(piece).collect();
    let tail: String = text.chars().skip(len - piece).collect();
    format!("{head}...{tail}")
}

/// English text of a message key.
pub fn english(key: &MsgKey) -> String {
    match key {
        MsgKey::TrackerError { tracker, detail } => format!("{tracker} error: {detail}"),
        MsgKey::LoginSuccess => "Logged in".into(),
        MsgKey::LoginFailed => "Login failed".into(),
        MsgKey::LogoutSuccess => "You are now logged out".into(),
        MsgKey::TrackersUpdatedAnime(n) => format!("Trackers updated to episode {n}"),
        MsgKey::TrackersUpdatedManga(n) => format!("Trackers updated to chapter {n}"),
        MsgKey::ConfirmUpdateAnime(n) => format!("Update trackers to episode {n}?"),
        MsgKey::ConfirmUpdateManga(n) => format!("Update trackers to chapter {n}?"),
        MsgKey::NoMatchFound => "No match found".into(),
        MsgKey::NoResultsFound => "No results found".into(),
        MsgKey::UnknownError => "Unknown error".into(),
        MsgKey::CopiedToClipboard(text) => format!("Copied to clipboard:\n{text}"),
        MsgKey::LocalRemovalFailed => "Could not remove the local tracking".into(),
        MsgKey::PendingUpdates(n) => format!("{n} tracking updates pending"),
        MsgKey::NoTrackingLink => "No link stored for this entry".into(),
        MsgKey::TrackerLoggedIn(name) => format!("{name} logged in"),
        MsgKey::TrackerLoginFailed(name) => format!("{name} login failed"),
        MsgKey::TrackerLoginCancelled(name) => format!("{name} login cancelled"),
        MsgKey::UnrecognizedLoginLink => "Unrecognized login link".into(),
        MsgKey::TrackerNotConfigured(name) => format!("{name} is not configured"),
        MsgKey::LoginExpired => "Login expired, start again".into(),
        MsgKey::NoTrackingData => "No tracking data for this tracker".into(),
        MsgKey::ActionCancel => "Cancel".into(),
        MsgKey::ActionRemove => "Remove".into(),
        MsgKey::ActionOk => "OK".into(),
        MsgKey::RemoveDateTitle => "Remove date?".into(),
        MsgKey::RemoveStartDateText(name) => {
            format!("This will remove your previously selected start date from {name}")
        }
        MsgKey::RemoveFinishDateText(name) => {
            format!("This will remove your previously selected finish date from {name}")
        }
        MsgKey::TrackDeleteTitle(name) => format!("Remove {name} tracking?"),
        MsgKey::TrackDeleteText => "This will remove the tracking locally.".into(),
        MsgKey::TrackDeleteRemoteText(name) => format!("Also remove from {name}"),
        MsgKey::StartDate => "Start date".into(),
        MsgKey::FinishDate => "Finish date".into(),
        MsgKey::Status => "Status".into(),
        MsgKey::Score => "Score".into(),
        MsgKey::Chapters => "Chapters".into(),
        MsgKey::Episodes => "Episodes".into(),
        MsgKey::AniListNotAuthenticated => "You are not logged in to AniList".into(),
        MsgKey::AniListSessionExpired => "Session expired, log in again".into(),
        MsgKey::KitsuSessionExpired => "Kitsu session expired. Log in again.".into(),
        MsgKey::AniListNotInList => "This entry is not on your AniList list".into(),
        MsgKey::AniListRateLimited => "AniList rate limit reached, try again in a minute".into(),
        MsgKey::AniListOffline => "No network connection to AniList".into(),
        MsgKey::AniListUnknownStatus => "Unknown AniList status".into(),
        MsgKey::AniListLoginFailed => "AniList login failed".into(),
        MsgKey::AniListRemoteDeleteFailed => "Could not remove the entry from AniList".into(),
        MsgKey::AniListLoggedIn => "Logged in to AniList".into(),
        MsgKey::AniListLoggingIn => "Logging in\u{2026}".into(),
        MsgKey::AniListReloginRequired => "AniList login must be renewed".into(),
        MsgKey::MalReAdded => mal_text::RE_ADDED.into(),
        MsgKey::MalWaitingForLogin => mal_text::WAITING_FOR_LOGIN.into(),
        MsgKey::MalNoClientId => mal_text::NO_CLIENT_ID.into(),
        MsgKey::MalLogInAgain => mal_text::LOG_IN_AGAIN.into(),
        MsgKey::MalLoginNotCompleted => mal_text::LOGIN_NOT_COMPLETED.into(),
        MsgKey::Enhanced { key, arg } => enhanced_text(key, arg),
        MsgKey::HttpStatus(code) => format!("HTTP {code}, check website in WebView"),
        MsgKey::TrackerDetail { tracker, detail } => {
            format!("{tracker} error: {}", detail.english())
        }
        MsgKey::Raw(text) => text.clone(),
    }
}

/// English text of a `track.enhanced.*` key.
fn enhanced_text(key: &str, arg: &str) -> String {
    match key {
        "track.enhanced.unauthorized" => "Unauthorized / api key not valid".into(),
        "track.enhanced.server_error" => "Error fetching JWT token".into(),
        "track.enhanced.login_required" => "Please log in through the extension".into(),
        "track.enhanced.search_unsupported" => "Search is not available for this tracker".into(),
        "track.enhanced.missing_fragment" => "Tracking link is incomplete".into(),
        "track.enhanced.chapter_missing" => format!("Chapter {arg} is not on the server"),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_texts_match_the_source_strings() {
        assert_eq!(english(&MsgKey::LoginFailed), "Login failed");
        assert_eq!(
            english(&MsgKey::ConfirmUpdateManga(12)),
            "Update trackers to chapter 12?"
        );
        assert_eq!(
            english(&MsgKey::TrackersUpdatedAnime(3)),
            "Trackers updated to episode 3"
        );
        assert_eq!(
            english(&MsgKey::TrackerError {
                tracker: "AniList".into(),
                detail: "down".into()
            }),
            "AniList error: down"
        );
    }

    #[test]
    fn tr_27_060_copy_toast_truncates_to_fifty_characters_in_the_middle() {
        let long: String = (0..60).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let toast = english(&MsgKey::copied(&long));
        let body = toast.strip_prefix("Copied to clipboard:\n").unwrap();
        assert_eq!(body.chars().count(), 49, "23 + '...' + 23");
        assert!(body.contains("..."));
        assert!(body.starts_with(&long[..23]) && body.ends_with(&long[long.len() - 23..]));
        assert_eq!(truncate_center("short", 50), "short");
    }

    #[test]
    fn tr_27b_030_every_english_text_has_its_placeholders_filled() {
        for key in [
            MsgKey::TrackerLoggedIn("AniList".into()),
            MsgKey::TrackerLoginFailed("AniList".into()),
            MsgKey::TrackerLoginCancelled("AniList".into()),
            MsgKey::TrackerNotConfigured("AniList".into()),
            MsgKey::RemoveStartDateText("AniList".into()),
            MsgKey::RemoveFinishDateText("AniList".into()),
            MsgKey::TrackDeleteTitle("AniList".into()),
            MsgKey::TrackDeleteRemoteText("AniList".into()),
            MsgKey::TrackerError {
                tracker: "AniList".into(),
                detail: "down".into(),
            },
        ] {
            let text = english(&key);
            assert!(!text.contains('%') && !text.contains("{}"), "{text}");
            assert!(text.contains("AniList") || text.contains("down"), "{text}");
        }
        assert_eq!(
            english(&MsgKey::TrackerLoggedIn("Kitsu".into())),
            "Kitsu logged in"
        );
        assert_eq!(
            english(&MsgKey::TrackDeleteTitle("Bangumi".into())),
            "Remove Bangumi tracking?"
        );
        assert_eq!(english(&MsgKey::LoginExpired), "Login expired, start again");
    }

    #[test]
    fn tr_27_078_every_key_id_is_a_section_18_id_in_the_english_catalogue() {
        let keys = [
            MsgKey::TrackerError {
                tracker: String::new(),
                detail: String::new(),
            },
            MsgKey::LoginSuccess,
            MsgKey::LoginFailed,
            MsgKey::LogoutSuccess,
            MsgKey::TrackersUpdatedAnime(1),
            MsgKey::TrackersUpdatedManga(1),
            MsgKey::ConfirmUpdateAnime(1),
            MsgKey::ConfirmUpdateManga(1),
            MsgKey::NoMatchFound,
            MsgKey::NoResultsFound,
            MsgKey::UnknownError,
            MsgKey::copied("x"),
            MsgKey::LocalRemovalFailed,
            MsgKey::NoTrackingLink,
            MsgKey::PendingUpdates(2),
            MsgKey::ActionCancel,
            MsgKey::ActionRemove,
            MsgKey::ActionOk,
            MsgKey::RemoveDateTitle,
            MsgKey::RemoveStartDateText(String::new()),
            MsgKey::RemoveFinishDateText(String::new()),
            MsgKey::TrackDeleteTitle(String::new()),
            MsgKey::TrackDeleteText,
            MsgKey::TrackDeleteRemoteText(String::new()),
            MsgKey::StartDate,
            MsgKey::FinishDate,
            MsgKey::Status,
            MsgKey::Score,
            MsgKey::Chapters,
            MsgKey::Episodes,
        ];
        for key in &keys {
            let Some(id) = key.resource_id() else {
                assert!(matches!(
                    key,
                    MsgKey::LoginFailed
                        | MsgKey::LocalRemovalFailed
                        | MsgKey::NoTrackingLink
                        | MsgKey::PendingUpdates(_)
                        | MsgKey::TrackerLoggedIn(_)
                        | MsgKey::TrackerLoginFailed(_)
                        | MsgKey::TrackerLoginCancelled(_)
                        | MsgKey::UnrecognizedLoginLink
                        | MsgKey::TrackerNotConfigured(_)
                        | MsgKey::LoginExpired
                        | MsgKey::NoTrackingData
                        | MsgKey::AniListNotAuthenticated
                        | MsgKey::AniListSessionExpired
                        | MsgKey::KitsuSessionExpired
                        | MsgKey::AniListNotInList
                        | MsgKey::AniListRateLimited
                        | MsgKey::AniListOffline
                        | MsgKey::AniListUnknownStatus
                        | MsgKey::AniListLoginFailed
                        | MsgKey::AniListRemoteDeleteFailed
                        | MsgKey::AniListLoggedIn
                        | MsgKey::AniListLoggingIn
                        | MsgKey::AniListReloginRequired
                        | MsgKey::MalNoClientId
                        | MsgKey::MalReAdded
                        | MsgKey::MalWaitingForLogin
                        | MsgKey::MalLogInAgain
                        | MsgKey::MalLoginNotCompleted
                ));
                continue;
            };
            assert!(SECTION_18_IDS.contains(&id), "{id} is not in section 18");
            assert!(
                EN_CATALOGUE.contains(&format!("\n{id} =")),
                "{id} missing from the English catalogue"
            );
        }
        assert_eq!(MsgKey::Raw("x".into()).resource_id(), None);
    }

    #[test]
    fn t_31a_041_http_status_and_detail_keys_read_their_texts() {
        assert_eq!(
            english(&MsgKey::HttpStatus(503)),
            "HTTP 503, check website in WebView"
        );
        assert_eq!(
            MsgKey::HttpStatus(503).resource_id(),
            Some("exception_http")
        );
        assert!(EN_CATALOGUE.contains("\nexception_http ="));
        assert_eq!(
            english(&MsgKey::TrackerDetail {
                tracker: "Bangumi".into(),
                detail: TrackDetail::NotFoundAnime
            }),
            "Bangumi error: Could not find anime"
        );
    }

    #[test]
    fn mal_keys_read_the_mal_text_table() {
        assert_eq!(
            english(&MsgKey::MalWaitingForLogin),
            "Waiting for MyAnimeList..."
        );
        assert_eq!(english(&MsgKey::MalLogInAgain), "Log in again");
        assert_eq!(
            english(&MsgKey::KitsuSessionExpired),
            "Kitsu session expired. Log in again."
        );
        assert_eq!(
            english(&MsgKey::MalNoClientId),
            "Set a MyAnimeList client id to log in"
        );
        assert_eq!(english(&MsgKey::MalReAdded), "Re-added to MyAnimeList");
        assert_eq!(
            english(&MsgKey::MalLoginNotCompleted),
            "Login was not completed"
        );
    }

    #[test]
    fn tr_27_078_section_18_ids_exist_in_the_english_catalogue() {
        for id in SECTION_18_IDS {
            let line = format!("\n{id} =");
            assert!(
                EN_CATALOGUE.contains(&line),
                "{id} missing from kuro-i18n en catalogue"
            );
        }
    }
}
