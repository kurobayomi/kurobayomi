//! The MAL tracker operations. Manga and anime share the
//! same rules; each has its own functions because the list fields differ. Pure functions (`advance_*`,
//! `*_put_form`, `apply_*`) carry the rules, and the async methods only add the network call.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;
use url::Url;

use super::api::MalApi;
use super::auth::MalAuth;
use super::consts::{API_BASE, BUILT_IN_CLIENT_ID, OAUTH_BASE};
use super::dates::{now_ms, to_mal_date};
use super::dto::{AnimeListStatus, MangaListStatus};
use super::error::MalError;
use super::pkce::{authorize_url, new_verifier};
use super::redirect::CodeOutcome;
use super::score::{SCORE_LIST, score_from_label, wire_score};
use super::search::remote_date;
use super::status::{anime_wire, from_wire, manga_wire};
use super::store::{CLIENT_ID_ENV, MalSettings, TokenStore, resolve_client_id};
use crate::keyed_lock::KeyedLocks;
use crate::messages::MsgKey;
use crate::model::{AnimeTrack, MangaTrack};

const READING: i64 = 1;
const WATCHING: i64 = 11;
const COMPLETED: i64 = 2;
const REREADING: i64 = 7;
const REWATCHING: i64 = 17;
const PLAN_TO_READ: i64 = 6;
const PLAN_TO_WATCH: i64 = 16;

/// Mirrors the Kotlin `MyAnimeList` class with the Kurobayomi storage and the explicit `Result`s.
pub struct MalTracker {
    api: MalApi,
    auth: Arc<MalAuth>,
    settings: Arc<MalSettings>,
    /// `None` when no client id is configured. Login is then refused with [`MalError::NoClientId`].
    client_id: Option<String>,
    /// Verifier of the login in progress. A new attempt replaces it, so the old redirect fails (E-LOGIN-4).
    pending: Mutex<Option<String>>,
    /// Logged-in state for the watchers of `Tracker::login_state`. Published by `sync_login_state`.
    login_tx: watch::Sender<bool>,
    /// Toast keys raised by refreshes (D-10). The GUI drains them with `take_notices`.
    notices: Mutex<Vec<MsgKey>>,
    /// The score picker labels, in index order.
    scores: Vec<String>,
    /// One lock per anime title, held across each PUT, so two writes to one title never overlap (T-29a-055).
    anime_puts: KeyedLocks,
    /// One lock per manga title, held across each PUT (T-29a-055).
    manga_puts: KeyedLocks,
}

impl MalTracker {
    /// The tracker with the real endpoints and the client id from `KUROBAYOMI_MAL_CLIENT_ID` or the config file
    /// (T-29a-040), else the app's built-in one.
    pub fn from_config(config_dir: &Path, tokens: Arc<dyn TokenStore>) -> Result<Self, MalError> {
        let file_id = MalSettings::open(config_dir)?.client_id();
        let env_id = std::env::var(CLIENT_ID_ENV).ok();
        let client_id = resolve_client_id(file_id.as_deref(), env_id.as_deref()).or_else(|| Some(BUILT_IN_CLIENT_ID.to_owned()));
        Self::new(config_dir, tokens, client_id)
    }

    /// The tracker with the real endpoints and the given client id (`None` refuses login).
    pub fn new(
        config_dir: &Path,
        tokens: Arc<dyn TokenStore>,
        client_id: Option<String>,
    ) -> Result<Self, MalError> {
        Self::with_endpoints(config_dir, tokens, client_id, OAUTH_BASE, API_BASE)
    }

    pub fn with_endpoints(
        config_dir: &Path,
        tokens: Arc<dyn TokenStore>,
        client_id: Option<String>,
        oauth_base: &str,
        api_base: &str,
    ) -> Result<Self, MalError> {
        let settings = Arc::new(MalSettings::open(config_dir)?);
        let refresh_url = Url::parse(&format!("{}/token", oauth_base.trim_end_matches('/')))?;
        let auth = Arc::new(MalAuth::new(
            tokens,
            settings.clone(),
            refresh_url,
            client_id.clone().unwrap_or_default(),
        )?);
        let api = MalApi::with_bases(
            auth.clone(),
            client_id.clone().unwrap_or_default(),
            oauth_base,
            api_base,
        )?;
        let logged_in = !settings.username().is_empty() && auth.has_token();
        let pending = settings.pending_verifier();
        Ok(Self {
            api,
            auth,
            settings,
            client_id,
            pending: Mutex::new(pending),
            login_tx: watch::channel(logged_in).0,
            notices: Mutex::new(Vec::new()),
            scores: SCORE_LIST.iter().map(|s| (*s).to_owned()).collect(),
            anime_puts: KeyedLocks::default(),
            manga_puts: KeyedLocks::default(),
        })
    }

    pub fn api(&self) -> &MalApi {
        &self.api
    }

    /// The settings file, for tests that set the username directly.
    #[cfg(test)]
    pub(crate) fn settings_for_test(&self) -> &MalSettings {
        &self.settings
    }

    pub fn auth(&self) -> &MalAuth {
        &self.auth
    }

    /// A username and a token are both needed (`BaseTracker.kt:32-34`).
    pub fn is_logged_in(&self) -> bool {
        !self.settings.username().is_empty() && self.auth.has_token()
    }

    pub fn username(&self) -> String {
        self.settings.username()
    }

    /// Clears the username, the token and the auth-expired flag. Sends no request.
    pub fn logout(&self) -> Result<(), MalError> {
        self.auth.clear_token()?;
        self.settings.set_username("")?;
        self.settings.set_auth_expired(false)?;
        self.sync_login_state();
        Ok(())
    }

    /// The client id, or [`MalError::NoClientId`] when none is configured.
    pub fn client_id(&self) -> Result<&str, MalError> {
        self.client_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or(MalError::NoClientId)
    }

    /// Starts a browser login: a new verifier replaces any earlier one, and the authorise URL is returned for the
    /// caller to open. Fails before any state changes when no client id is set.
    pub fn begin_login(&self) -> Result<Url, MalError> {
        let client_id = self.client_id()?;
        let verifier = new_verifier();
        let url = authorize_url(client_id, &verifier);
        self.settings.set_pending_verifier(Some(&verifier))?;
        *self.pending_slot() = Some(verifier);
        Ok(url)
    }

    /// True while a login waits for its redirect or paste (the Settings row shows "Waiting for MyAnimeList...").
    pub fn login_in_progress(&self) -> bool {
        self.pending_slot().is_some()
    }

    /// Drops the verifier of the login in progress. The Cancel button of the login dialog calls this.
    pub fn cancel_login(&self) {
        *self.pending_slot() = None;
        if let Err(e) = self.settings.set_pending_verifier(None) {
            tracing::warn!(error = %e, "MAL pending login could not be cleared from the settings file");
        }
    }

    /// Ends the login attempt with what the redirect or the paste carried. A code completes the login. A denial
    /// or a timeout keeps an existing login (D-04) and drops the verifier.
    pub async fn finish_login(&self, outcome: CodeOutcome) -> Result<(), MalError> {
        match outcome {
            CodeOutcome::Code(code) => self.complete_login(&code).await,
            CodeOutcome::Denied => {
                self.cancel_login();
                Err(MalError::LoginNotCompleted)
            }
            CodeOutcome::TimedOut => {
                self.cancel_login();
                Err(MalError::LoginTimedOut)
            }
        }
    }

    /// Exchanges `code` with the verifier of the login in progress.
    pub async fn complete_login(&self, code: &str) -> Result<(), MalError> {
        let verifier = self.pending_slot().take().ok_or(MalError::NoPendingLogin)?;
        if let Err(e) = self.settings.set_pending_verifier(None) {
            tracing::warn!(error = %e, "MAL pending login could not be cleared from the settings file");
        }
        self.login_with_code(code, &verifier).await
    }

    /// Completes a browser login (`MyAnimeList.kt:251-260`). The verifier is the one `begin_login` made for the
    /// authorise URL. On a failed exchange an existing login is kept. Without one, the state is cleared. A failed
    /// user lookup clears the state too, as upstream does (D-03 returns the error to the caller).
    pub async fn login_with_code(&self, code: &str, verifier: &str) -> Result<(), MalError> {
        self.client_id()?;
        let result = self.exchange_and_store(code, verifier).await;
        self.sync_login_state();
        result
    }

    async fn exchange_and_store(&self, code: &str, verifier: &str) -> Result<(), MalError> {
        let token = match self.api.exchange_code(code, verifier).await {
            Ok(token) => token,
            Err(e) => {
                if !self.is_logged_in() {
                    self.logout()?;
                }
                return Err(e);
            }
        };
        self.auth.set_token(token)?;
        // The new token supersedes an earlier expiry, and the user lookup below must not be refused by it.
        self.settings.set_auth_expired(false)?;
        let name = match self.api.current_user().await {
            Ok(name) => name,
            Err(e) => {
                self.logout()?;
                return Err(e);
            }
        };
        if name.trim().is_empty() {
            self.logout()?;
            return Err(MalError::NotAuthenticated);
        }
        self.settings.set_username(&name)?;
        self.settings.set_auth_expired(false)
    }

    /// Publishes the logged-in state when it changed. Expiry is found inside API calls, so the tracker calls this
    /// after each operation.
    pub fn sync_login_state(&self) {
        let now = self.is_logged_in();
        self.login_tx.send_if_modified(|logged_in| {
            if *logged_in == now {
                false
            } else {
                *logged_in = now;
                true
            }
        });
    }

    pub fn login_watch(&self) -> watch::Receiver<bool> {
        self.login_tx.subscribe()
    }

    /// The score picker labels (`MyAnimeList.kt:49-51`).
    pub fn score_labels(&self) -> &[String] {
        &self.scores
    }

    /// Toasts raised since the last call (D-10). Drained by the GUI after each refresh.
    pub fn take_notices(&self) -> Vec<MsgKey> {
        std::mem::take(&mut *self.notices_slot())
    }

    fn notify(&self, key: MsgKey) {
        self.notices_slot().push(key);
    }

    fn pending_slot(&self) -> MutexGuard<'_, Option<String>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn notices_slot(&self) -> MutexGuard<'_, Vec<MsgKey>> {
        self.notices.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `refresh` for manga (`MyAnimeList.kt:242`). A title on the list is returned as it is. A title the website no
    /// longer lists is added again with the local values, and a notice is raised (D-10).
    pub async fn refresh_manga(&self, track: &mut MangaTrack) -> Result<(), MalError> {
        if self.find_manga_list_item(track).await? {
            return Ok(());
        }
        self.put_manga(track).await?;
        self.notify(MsgKey::MalReAdded);
        Ok(())
    }

    /// `refresh` for anime, the same rule as [`MalTracker::refresh_manga`].
    pub async fn refresh_anime(&self, track: &mut AnimeTrack) -> Result<(), MalError> {
        if self.find_anime_list_item(track).await? {
            return Ok(());
        }
        self.put_anime(track).await?;
        self.notify(MsgKey::MalReAdded);
        Ok(())
    }

    /// `api.findListItem` for manga (`MyAnimeListApi.kt:251-267`). Sets `total_chapters` in every case. Returns
    /// `false` when the title is not on the user's list, and then the track is left with its local progress.
    pub async fn find_manga_list_item(&self, track: &mut MangaTrack) -> Result<bool, MalError> {
        let entry = self.api.manga_list_entry(track.remote_id).await?;
        track.total_chapters = entry.num_chapters;
        match entry.my_list_status {
            Some(status) => {
                apply_manga_status(&status, track);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// `api.findListItem` for anime (`MyAnimeListApi.kt:269-285`).
    pub async fn find_anime_list_item(&self, track: &mut AnimeTrack) -> Result<bool, MalError> {
        let entry = self.api.anime_list_entry(track.remote_id).await?;
        track.total_episodes = entry.num_episodes;
        match entry.my_list_status {
            Some(status) => {
                apply_anime_status(&status, track);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// `updateItem` for manga: PUT the local state and apply the reply (E12, `MyAnimeListApi.kt:181-206`).
    pub async fn put_manga(&self, track: &mut MangaTrack) -> Result<(), MalError> {
        let _held = self.manga_puts.hold(track.remote_id).await;
        let form = manga_put_form(track)?;
        let reply = self.api.put_manga(track.remote_id, &form).await?;
        apply_manga_status(&reply, track);
        Ok(())
    }

    /// `updateItem` for anime (E13, `MyAnimeListApi.kt:208-233`).
    pub async fn put_anime(&self, track: &mut AnimeTrack) -> Result<(), MalError> {
        let _held = self.anime_puts.hold(track.remote_id).await;
        let form = anime_put_form(track)?;
        let reply = self.api.put_anime(track.remote_id, &form).await?;
        apply_anime_status(&reply, track);
        Ok(())
    }

    /// DELETE the remote entry (`MyAnimeList.kt:161-167`). A 404 comes back as `Http(404)` for the caller to log.
    pub async fn delete_manga(&self, remote_id: i64) -> Result<(), MalError> {
        self.api.delete_list_item("manga", remote_id).await
    }

    pub async fn delete_anime(&self, remote_id: i64) -> Result<(), MalError> {
        self.api.delete_list_item("anime", remote_id).await
    }

    /// `update(track, didReadChapter)` for manga: progress rules, then the PUT (`MyAnimeList.kt:125-141`).
    pub async fn update_manga(
        &self,
        track: &mut MangaTrack,
        did_read: bool,
    ) -> Result<(), MalError> {
        advance_manga(track, did_read, now_ms());
        self.put_manga(track).await
    }

    /// `update(track, didWatchEpisode)` for anime (`MyAnimeList.kt:143-159`).
    pub async fn update_anime(
        &self,
        track: &mut AnimeTrack,
        did_watch: bool,
    ) -> Result<(), MalError> {
        advance_anime(track, did_watch, now_ms());
        self.put_anime(track).await
    }

    /// `setRemoteMangaStatus`: a completed status with a known total sets progress to the total (`MangaTracker.kt`).
    pub async fn set_manga_status(
        &self,
        track: &mut MangaTrack,
        status: i64,
    ) -> Result<(), MalError> {
        track.status = status;
        if status == COMPLETED && track.total_chapters != 0 {
            track.last_chapter_read = track.total_chapters as f64;
        }
        self.update_manga(track, false).await
    }

    /// `setRemoteLastChapterRead` (`MangaTracker.kt:74-89`).
    pub async fn set_manga_last_chapter_read(
        &self,
        track: &mut MangaTrack,
        chapters: i64,
    ) -> Result<(), MalError> {
        if track.last_chapter_read == 0.0 && chapters > 0 && track.status != REREADING {
            track.status = READING;
        }
        track.last_chapter_read = chapters as f64;
        if track.total_chapters != 0 && track.last_chapter_read as i64 == track.total_chapters {
            track.status = COMPLETED;
            track.finish_date = now_ms();
        }
        self.update_manga(track, false).await
    }

    /// `setRemoteScore`. A label outside the list is refused and the score stays (D-12).
    pub async fn set_manga_score(
        &self,
        track: &mut MangaTrack,
        label: &str,
    ) -> Result<(), MalError> {
        track.score =
            score_from_label(label).ok_or_else(|| MalError::UnknownScore(label.to_owned()))?;
        self.update_manga(track, false).await
    }

    /// `setRemoteAnimeStatus` (`AnimeTracker.kt`).
    pub async fn set_anime_status(
        &self,
        track: &mut AnimeTrack,
        status: i64,
    ) -> Result<(), MalError> {
        track.status = status;
        if status == COMPLETED && track.total_episodes != 0 {
            track.last_episode_seen = track.total_episodes as f64;
        }
        self.update_anime(track, false).await
    }

    /// `setRemoteLastEpisodeSeen` (`AnimeTracker.kt:75-88`).
    pub async fn set_anime_last_episode_seen(
        &self,
        track: &mut AnimeTrack,
        episodes: i64,
    ) -> Result<(), MalError> {
        if track.last_episode_seen == 0.0 && episodes > 0 && track.status != REWATCHING {
            track.status = WATCHING;
        }
        track.last_episode_seen = episodes as f64;
        if track.total_episodes != 0 && track.last_episode_seen as i64 == track.total_episodes {
            track.status = COMPLETED;
            track.finish_date = now_ms();
        }
        self.update_anime(track, false).await
    }

    /// `setRemoteScore` for anime.
    pub async fn set_anime_score(
        &self,
        track: &mut AnimeTrack,
        label: &str,
    ) -> Result<(), MalError> {
        track.score =
            score_from_label(label).ok_or_else(|| MalError::UnknownScore(label.to_owned()))?;
        self.update_anime(track, false).await
    }

    /// `bind` for manga (`MyAnimeList.kt:169-187`). A title on the list keeps its remote status, and reading
    /// locally moves a non-rereading title to Reading (D-16). A title not on the list is created.
    pub async fn bind_manga(&self, track: &mut MangaTrack, has_read: bool) -> Result<(), MalError> {
        if self.find_manga_list_item(track).await? {
            if track.status != COMPLETED {
                let is_rereading = track.status == REREADING;
                if !is_rereading && has_read {
                    track.status = READING;
                }
            }
            self.update_manga(track, false).await
        } else {
            track.status = if has_read { READING } else { PLAN_TO_READ };
            track.score = 0.0;
            self.put_manga(track).await
        }
    }

    /// `bind` for anime (`MyAnimeList.kt:189-207`).
    pub async fn bind_anime(&self, track: &mut AnimeTrack, has_seen: bool) -> Result<(), MalError> {
        if self.find_anime_list_item(track).await? {
            if track.status != COMPLETED {
                let is_rewatching = track.status == REWATCHING;
                if !is_rewatching && has_seen {
                    track.status = WATCHING;
                }
            }
            self.update_anime(track, false).await
        } else {
            track.status = if has_seen { WATCHING } else { PLAN_TO_WATCH };
            track.score = 0.0;
            self.put_anime(track).await
        }
    }
}

/// Progress rules for a manga update (`MyAnimeList.kt:125-141`). The completion branch runs before the rereading
/// check, so a re-read that reaches the last chapter becomes Completed.
pub fn advance_manga(track: &mut MangaTrack, did_read: bool, now: i64) {
    if track.status == COMPLETED || !did_read {
        return;
    }
    if track.total_chapters > 0 && track.last_chapter_read as i64 == track.total_chapters {
        track.status = COMPLETED;
        track.finish_date = now;
    } else if track.status != REREADING {
        track.status = READING;
        if track.last_chapter_read == 1.0 {
            track.start_date = now;
        }
    }
}

/// Progress rules for an anime update (`MyAnimeList.kt:143-159`).
pub fn advance_anime(track: &mut AnimeTrack, did_watch: bool, now: i64) {
    if track.status == COMPLETED || !did_watch {
        return;
    }
    if track.total_episodes > 0 && track.last_episode_seen as i64 == track.total_episodes {
        track.status = COMPLETED;
        track.finish_date = now;
    } else if track.status != REWATCHING {
        track.status = WATCHING;
        if track.last_episode_seen == 1.0 {
            track.start_date = now;
        }
    }
}

/// The manga PUT form in source order (`MyAnimeListApi.kt:181-206`). Codes outside the manga list are refused.
pub fn manga_put_form(track: &MangaTrack) -> Result<Vec<(&'static str, String)>, MalError> {
    let status = manga_wire(track.status).ok_or(MalError::UnknownStatus(track.status))?;
    let mut form = vec![
        ("status", status.to_owned()),
        ("is_rereading", (track.status == REREADING).to_string()),
        ("score", wire_score(track.score)),
        (
            "num_chapters_read",
            (track.last_chapter_read.trunc() as i64).to_string(),
        ),
    ];
    push_date(&mut form, "start_date", track.start_date);
    push_date(&mut form, "finish_date", track.finish_date);
    Ok(form)
}

/// The anime PUT form in source order (`MyAnimeListApi.kt:208-233`). Code 1 is sent as `watching`.
pub fn anime_put_form(track: &AnimeTrack) -> Result<Vec<(&'static str, String)>, MalError> {
    let status = anime_wire(track.status).ok_or(MalError::UnknownStatus(track.status))?;
    let mut form = vec![
        ("status", status.to_owned()),
        ("is_rewatching", (track.status == REWATCHING).to_string()),
        ("score", wire_score(track.score)),
        (
            "num_watched_episodes",
            (track.last_episode_seen.trunc() as i64).to_string(),
        ),
    ];
    push_date(&mut form, "start_date", track.start_date);
    push_date(&mut form, "finish_date", track.finish_date);
    Ok(form)
}

/// Adds a date key unless the conversion fails. 0 becomes an empty value, which clears the date on MAL.
fn push_date(form: &mut Vec<(&'static str, String)>, key: &'static str, epoch_ms: i64) {
    if let Some(text) = to_mal_date(epoch_ms) {
        form.push((key, text));
    }
}

/// Applies a manga list status to the track (`parseMangaItem`, `MyAnimeListApi.kt:344-353`). Absent dates keep
/// the local value.
pub fn apply_manga_status(status: &MangaListStatus, track: &mut MangaTrack) {
    track.status = if status.is_rereading {
        REREADING
    } else {
        from_wire(Some(&status.status))
    };
    track.last_chapter_read = status.num_chapters_read;
    track.score = status.score as f64;
    if let Some(start) = remote_date(status.start_date.as_deref()) {
        track.start_date = start;
    }
    if let Some(finish) = remote_date(status.finish_date.as_deref()) {
        track.finish_date = finish;
    }
}

/// Applies an anime list status (`parseAnimeItem`, `MyAnimeListApi.kt:355-364`).
pub fn apply_anime_status(status: &AnimeListStatus, track: &mut AnimeTrack) {
    track.status = if status.is_rewatching {
        REWATCHING
    } else {
        from_wire(Some(&status.status))
    };
    track.last_episode_seen = status.num_episodes_watched;
    track.score = status.score as f64;
    if let Some(start) = remote_date(status.start_date.as_deref()) {
        track.start_date = start;
    }
    if let Some(finish) = remote_date(status.finish_date.as_deref()) {
        track.finish_date = finish;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mal::status::MalStatus;

    fn manga(status: i64, read: f64, total: i64) -> MangaTrack {
        MangaTrack {
            id: None,
            manga_id: 1,
            tracker_id: crate::model::TrackerId::MYANIMELIST,
            remote_id: 5,
            library_id: None,
            title: "t".into(),
            last_chapter_read: read,
            total_chapters: total,
            status,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    fn anime(status: i64, seen: f64, total: i64) -> AnimeTrack {
        AnimeTrack {
            id: None,
            anime_id: 1,
            tracker_id: crate::model::TrackerId::MYANIMELIST,
            remote_id: 5,
            library_id: None,
            title: "t".into(),
            last_episode_seen: seen,
            total_episodes: total,
            status,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    #[test]
    fn update_completes_on_last_chapter() {
        let mut t = manga(MalStatus::Reading.code(), 10.0, 10);
        advance_manga(&mut t, true, 99);
        assert_eq!(t.status, COMPLETED);
        assert_eq!(t.finish_date, 99);
    }

    #[test]
    fn update_rereading_reaching_last_chapter_completes() {
        let mut t = manga(REREADING, 10.0, 10);
        advance_manga(&mut t, true, 7);
        assert_eq!(t.status, COMPLETED);
    }

    #[test]
    fn update_rereading_no_status_change() {
        let mut t = manga(REREADING, 3.0, 10);
        advance_manga(&mut t, true, 7);
        assert_eq!(t.status, REREADING);
        assert_eq!(t.start_date, 0);
    }

    #[test]
    fn update_started_date_on_chapter_one() {
        let mut t = manga(MalStatus::PlanToRead.code(), 1.0, 10);
        advance_manga(&mut t, true, 42);
        assert_eq!(t.status, READING);
        assert_eq!(t.start_date, 42);
    }

    #[test]
    fn update_unknown_total_never_completes() {
        let mut t = manga(READING, 5.0, 0);
        advance_manga(&mut t, true, 1);
        assert_eq!(t.status, READING);
    }

    #[test]
    fn update_without_read_flag_changes_nothing() {
        let mut t = manga(MalStatus::OnHold.code(), 1.0, 10);
        advance_manga(&mut t, false, 1);
        assert_eq!(t.status, MalStatus::OnHold.code());
    }

    #[test]
    fn update_anime_watch_rules() {
        let mut t = anime(WATCHING, 12.0, 12);
        advance_anime(&mut t, true, 5);
        assert_eq!(t.status, COMPLETED);
        assert_eq!(t.finish_date, 5);

        let mut r = anime(REWATCHING, 4.0, 12);
        advance_anime(&mut r, true, 5);
        assert_eq!(r.status, REWATCHING);

        let mut first = anime(PLAN_TO_WATCH, 1.0, 12);
        advance_anime(&mut first, true, 8);
        assert_eq!(first.status, WATCHING);
        assert_eq!(first.start_date, 8);
    }

    #[test]
    fn manga_form_order_and_truncation() {
        let mut t = manga(READING, 12.7, 20);
        t.score = 8.0;
        let form = manga_put_form(&t).unwrap();
        let keys: Vec<&str> = form.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            [
                "status",
                "is_rereading",
                "score",
                "num_chapters_read",
                "start_date",
                "finish_date"
            ]
        );
        assert_eq!(form[3].1, "12");
        assert_eq!(form[2].1, "8.0");
    }

    #[test]
    fn form_dates_empty_clears_and_zero_only_when_set() {
        let mut t = manga(READING, 0.0, 0);
        t.start_date = 0;
        let form = manga_put_form(&t).unwrap();
        assert!(form.contains(&("start_date", String::new())));
        assert!(form.contains(&("finish_date", String::new())));
    }

    #[test]
    fn form_refuses_codes_outside_list() {
        assert!(manga_put_form(&manga(11, 0.0, 0)).is_err());
        assert!(anime_put_form(&anime(2, 0.0, 0)).is_ok());
        assert!(anime_put_form(&anime(99, 0.0, 0)).is_err());
    }

    #[test]
    fn anime_form_uses_plain_keys_and_code_one_is_watching() {
        let form = anime_put_form(&anime(READING, 3.9, 12)).unwrap();
        let keys: Vec<&str> = form.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            [
                "status",
                "is_rewatching",
                "score",
                "num_watched_episodes",
                "start_date",
                "finish_date"
            ]
        );
        assert_eq!(form[0].1, "watching");
        assert_eq!(form[3].1, "3");
    }

    #[test]
    fn apply_status_keeps_absent_dates_and_reads_rereading_flag() {
        let status = MangaListStatus {
            is_rereading: true,
            status: "reading".into(),
            num_chapters_read: 3.0,
            score: 7,
            start_date: None,
            finish_date: Some("2021-03-01".into()),
        };
        let mut t = manga(READING, 0.0, 10);
        t.start_date = 55;
        apply_manga_status(&status, &mut t);
        assert_eq!(t.status, REREADING);
        assert_eq!(t.last_chapter_read, 3.0);
        assert_eq!(t.score, 7.0);
        assert_eq!(t.start_date, 55);
        assert!(t.finish_date > 0);
    }
}
