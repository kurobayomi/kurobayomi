//! MyAnimeList behind the tracker traits (T-29a-048). The registry, the track sheet and the login rows hold the
//! tracker as `Arc<dyn Tracker>`, so every trait method is a thin call into [`MalTracker`]. The capability flags
//! (T-29a-028), the logged-in state (T-29a-027) and the failure logs (T-29a-035) are decided here.

use async_trait::async_trait;
use tokio::sync::watch;

use super::error::MalError;
use super::score::display_score;
use super::status::{ANIME_LIST, MANGA_LIST, MalStatus, anime_label, manga_label};
use super::tracker::MalTracker;
use crate::error::{TrackError, redact};
use crate::messages::MsgKey;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::tracker::{
    AnimeTracker, DeletableAnimeTracker, DeletableMangaTracker, LogoRef, MangaTracker, Tracker,
};

/// Display name (`MyAnimeList.kt`).
pub const NAME: &str = "MyAnimeList";
/// Logo tile colour `#2E51A2` (`Color.rgb(46, 81, 162)`, T-29a-030).
pub const LOGO_RGB: (u8, u8, u8) = (46, 81, 162);

impl From<MalError> for TrackError {
    /// The error text is the toast. Refusals with their own UI text use their key.
    fn from(e: MalError) -> Self {
        match e {
            MalError::LoginNotCompleted => TrackError::Key(MsgKey::MalLoginNotCompleted),
            MalError::NoClientId => TrackError::Key(MsgKey::MalNoClientId),
            other => TrackError::Key(MsgKey::Raw(other.to_string())),
        }
    }
}

/// The toast of a finished browser login (T-29a-051). A success shows nothing, as the
/// source does. A refusal shows "Login was not completed" and keeps the login (D-04). Any other failure shows its own
/// text.
pub fn login_toast(result: &Result<(), MalError>) -> Option<MsgKey> {
    match result {
        Ok(()) => None,
        Err(MalError::LoginNotCompleted) => Some(MsgKey::MalLoginNotCompleted),
        Err(e) => Some(MsgKey::Raw(e.to_string())),
    }
}

/// Logs a failed progress or refresh write (T-29a-035). Only the error text is logged, and it carries no token.
fn log_update_failure(e: &MalError, remote_id: i64) {
    tracing::error!(
        error = %redact(&e.to_string()),
        "Failed to update remote track data id={remote_id}"
    );
}

/// Logs a failed remote DELETE. A 404 means the title was already gone (E-DEL-1). No toast is shown for either
/// (T-29a-031).
fn log_delete_failure(result: Result<(), MalError>, remote_id: i64) {
    match result {
        Ok(()) => {}
        Err(MalError::Http(404)) => {
            tracing::warn!(remote_id, "MAL entry was not on the list when removed");
        }
        Err(e) => {
            tracing::error!(
                remote_id,
                error = %redact(&e.to_string()),
                "Failed to remove the MAL entry"
            );
        }
    }
}

#[async_trait]
impl Tracker for MalTracker {
    fn id(&self) -> TrackerId {
        TrackerId::MYANIMELIST
    }

    fn name(&self) -> &str {
        NAME
    }

    fn logo(&self) -> LogoRef {
        LogoRef::IconName("myanimelist")
    }

    fn logo_color(&self) -> u32 {
        let (r, g, b) = LOGO_RGB;
        0xFF00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
    }

    fn setup_message(&self) -> Option<MsgKey> {
        self.client_id().err().map(|_| MsgKey::MalNoClientId)
    }

    /// Reading dates are sent and shown (`MyAnimeList.kt`, T-29a-028).
    fn supports_reading_dates(&self) -> bool {
        true
    }

    /// MAL has no private tracking (T-29a-028, T-29a-050).
    fn supports_private_tracking(&self) -> bool {
        false
    }

    fn completion_status(&self) -> i64 {
        MalStatus::Completed.code()
    }

    fn score_list(&self) -> &[String] {
        self.score_labels()
    }

    /// The browser login passes the authorisation code in `password`. The verifier comes from the login that
    /// [`MalTracker::begin_login`] started (E-LOGIN-4). `username` is not used.
    async fn login(&self, _username: &str, password: &str) -> Result<(), TrackError> {
        let result = self.complete_login(password).await;
        self.sync_login_state();
        result.map_err(TrackError::from)
    }

    fn logout(&self) -> Result<(), TrackError> {
        MalTracker::logout(self).map_err(TrackError::from)
    }

    fn is_logged_in(&self) -> bool {
        MalTracker::is_logged_in(self)
    }

    /// True when the token was refused or the refresh returned 401 (T-29a-027). The row stays checked.
    fn auth_expired(&self) -> bool {
        self.auth().expired()
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.login_watch()
    }

    /// MyAnimeList signs in through the browser, so typed credentials are not stored.
    fn save_credentials(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        Err(TrackError::Message(
            "MyAnimeList signs in through the browser".into(),
        ))
    }

    fn username(&self) -> String {
        MalTracker::username(self)
    }

    /// MAL keeps no password. The browser login leaves nothing to show.
    fn password(&self) -> String {
        String::new()
    }

    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        Some(self)
    }

    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }

    fn as_deletable_anime(&self) -> Option<&dyn DeletableAnimeTracker> {
        Some(self)
    }

    fn as_deletable_manga(&self) -> Option<&dyn DeletableMangaTracker> {
        Some(self)
    }
}

#[async_trait]
impl AnimeTracker for MalTracker {
    fn status_list(&self) -> Vec<i64> {
        ANIME_LIST.iter().map(|s| s.code()).collect()
    }

    fn watching_status(&self) -> i64 {
        MalStatus::Watching.code()
    }

    fn rewatching_status(&self) -> i64 {
        MalStatus::Rewatching.code()
    }

    /// The label text itself. MAL has no message keys, so the GUI shows this English text.
    fn status_label(&self, status: i64) -> Option<&'static str> {
        anime_label(status)
    }

    fn display_score(&self, t: &AnimeTrack) -> String {
        display_score(t.score)
    }

    async fn update(
        &self,
        t: &AnimeTrack,
        did_watch_episode: bool,
    ) -> Result<AnimeTrack, TrackError> {
        let mut next = t.clone();
        let result = self.update_anime(&mut next, did_watch_episode).await;
        self.sync_login_state();
        if let Err(e) = &result {
            log_update_failure(e, t.remote_id);
        }
        result.map_err(TrackError::from).map(|()| next)
    }

    async fn bind(&self, t: &mut AnimeTrack, has_seen: bool) -> Result<(), TrackError> {
        let result = self.bind_anime(t, has_seen).await;
        self.sync_login_state();
        result.map_err(TrackError::from)
    }

    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        let result = self.api().search_anime(query).await;
        self.sync_login_state();
        result.map_err(TrackError::from)
    }

    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        let mut next = t.clone();
        let result = self.refresh_anime(&mut next).await;
        self.sync_login_state();
        if let Err(e) = &result {
            log_update_failure(e, t.remote_id);
        }
        result.map_err(TrackError::from).map(|()| next)
    }
}

#[async_trait]
impl MangaTracker for MalTracker {
    fn status_list(&self) -> Vec<i64> {
        MANGA_LIST.iter().map(|s| s.code()).collect()
    }

    fn reading_status(&self) -> i64 {
        MalStatus::Reading.code()
    }

    fn rereading_status(&self) -> i64 {
        MalStatus::Rereading.code()
    }

    /// The label text itself. See the anime form.
    fn status_label(&self, status: i64) -> Option<&'static str> {
        manga_label(status)
    }

    fn display_score(&self, t: &MangaTrack) -> String {
        display_score(t.score)
    }

    async fn update(
        &self,
        t: &MangaTrack,
        did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        let mut next = t.clone();
        let result = self.update_manga(&mut next, did_read_chapter).await;
        self.sync_login_state();
        if let Err(e) = &result {
            log_update_failure(e, t.remote_id);
        }
        result.map_err(TrackError::from).map(|()| next)
    }

    async fn bind(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), TrackError> {
        let result = self.bind_manga(t, has_read).await;
        self.sync_login_state();
        result.map_err(TrackError::from)
    }

    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        let result = self.api().search_manga(query).await;
        self.sync_login_state();
        result.map_err(TrackError::from)
    }

    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let mut next = t.clone();
        let result = self.refresh_manga(&mut next).await;
        self.sync_login_state();
        if let Err(e) = &result {
            log_update_failure(e, t.remote_id);
        }
        result.map_err(TrackError::from).map(|()| next)
    }
}

/// The local row is removed by the caller whatever the remote answer (`AnimeTrackInfoDialog.kt:833-866`). A failure
/// is logged here and never returned, so no toast shows.
#[async_trait]
impl DeletableAnimeTracker for MalTracker {
    async fn delete(&self, t: &AnimeTrack) -> Result<(), TrackError> {
        let result = self.delete_anime(t.remote_id).await;
        self.sync_login_state();
        log_delete_failure(result, t.remote_id);
        Ok(())
    }
}

#[async_trait]
impl DeletableMangaTracker for MalTracker {
    async fn delete(&self, t: &MangaTrack) -> Result<(), TrackError> {
        let result = self.delete_manga(t.remote_id).await;
        self.sync_login_state();
        log_delete_failure(result, t.remote_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::mal::store::SecretTokenStore;

    fn tracker_without_client_id(dir: &std::path::Path) -> MalTracker {
        let secrets = crate::fakes::MemSecrets::new();
        let tokens: Arc<dyn crate::mal::store::TokenStore> =
            Arc::new(SecretTokenStore::new(secrets));
        MalTracker::new(dir, tokens, None).expect("tracker builds")
    }

    #[test]
    fn capability_flags_and_identity_are_the_mal_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        let t = tracker_without_client_id(dir.path());
        assert_eq!(t.id(), TrackerId::MYANIMELIST);
        assert_eq!(t.id().0, 1);
        assert_eq!(t.name(), "MyAnimeList");
        assert!(t.supports_reading_dates(), "T-29a-028");
        assert!(!t.supports_private_tracking(), "T-29a-028, T-29a-050");
        assert_eq!(t.completion_status(), 2);
        assert_eq!(t.score_list().len(), 11);
        assert_eq!(t.score_list().first().map(String::as_str), Some("0"));
        assert_eq!(t.score_list().last().map(String::as_str), Some("10"));
        assert!(t.as_deletable_anime().is_some() && t.as_deletable_manga().is_some());
        assert!(t.as_enhanced_anime().is_none() && t.as_enhanced_manga().is_none());
    }

    #[test]
    fn status_lists_and_labels_match_the_source_tables() {
        let dir = tempfile::tempdir().expect("tempdir");
        let t = tracker_without_client_id(dir.path());
        let anime = t.as_anime().expect("anime view");
        let manga = t.as_manga().expect("manga view");
        assert_eq!(anime.status_list(), vec![11, 2, 3, 4, 16, 17]);
        assert_eq!(anime.watching_status(), 11);
        assert_eq!(anime.rewatching_status(), 17);
        assert_eq!(anime.status_label(1), None, "code 1 has no anime label");
        assert_eq!(anime.status_label(16), Some("Plan to watch"));
        assert_eq!(manga.status_list(), vec![1, 2, 3, 4, 6, 7]);
        assert_eq!(manga.reading_status(), 1);
        assert_eq!(manga.rereading_status(), 7);
        assert_eq!(manga.status_label(7), Some("Rereading"));
        assert_eq!(manga.status_label(11), None);
    }

    #[test]
    fn logo_colour_is_the_mal_blue() {
        assert_eq!(LOGO_RGB, (46, 81, 162));
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            tracker_without_client_id(dir.path()).logo_color(),
            0xFF2E_51A2
        );
    }

    #[test]
    fn without_a_client_id_the_row_says_so_and_no_login_starts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let t = tracker_without_client_id(dir.path());
        assert!(matches!(t.setup_message(), Some(MsgKey::MalNoClientId)));
        assert!(matches!(t.begin_login(), Err(MalError::NoClientId)));
        assert!(
            !t.login_in_progress(),
            "nothing is pending when the start is refused"
        );
        assert!(matches!(
            TrackError::from(MalError::NoClientId),
            TrackError::Key(MsgKey::MalNoClientId)
        ));
    }

    #[test]
    fn a_login_toast_follows_the_outcome() {
        assert_eq!(
            login_toast(&Ok(())),
            None,
            "a browser success shows nothing"
        );
        assert_eq!(
            login_toast(&Err(MalError::LoginNotCompleted)),
            Some(MsgKey::MalLoginNotCompleted)
        );
        assert_eq!(
            login_toast(&Err(MalError::Http(400))),
            Some(MsgKey::Raw("HTTP error 400".into()))
        );
        assert_eq!(
            login_toast(&Err(MalError::RefreshFailed)),
            Some(MsgKey::Raw("MAL: Failed to refresh account token".into()))
        );
    }

    #[test]
    fn the_mal_error_reaches_the_toast_with_its_own_text() {
        let err: TrackError = MalError::Http(401).into();
        assert_eq!(err.to_string(), "HTTP error 401");
        assert_eq!(
            err.toast("MyAnimeList"),
            MsgKey::Raw("HTTP error 401".into())
        );
    }
}
