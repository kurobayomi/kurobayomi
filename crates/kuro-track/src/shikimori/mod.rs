//! Shikimori tracker (`Shikimori.kt`): identity, login by OAuth code, the library rules and the tracker traits.
//! The network and token state are in [`api`].

pub mod api;
pub mod auth;
pub mod config;
pub mod dto;
pub mod error;
pub mod registry;
pub mod search;
pub mod status;
#[cfg(test)]
mod tests;

use kuro_platform::kind::Kind;
use std::sync::Arc;

use async_trait::async_trait;
use kuro_platform::secrets::SecretStore;
use tokio::sync::watch;

use self::api::{Endpoints, LibEntry, OAuthClient, Rating, ShikimoriApi};
use self::error::ShikimoriError;
use self::status::ShikiStatus;
use crate::error::TrackError;
use crate::messages::MsgKey;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::tracker::{
    AnimeTracker, DeletableAnimeTracker, DeletableMangaTracker, LogoRef, MangaTracker, Tracker,
};

pub const NAME: &str = "Shikimori";
/// `Color.rgb(40, 40, 40)` (`Shikimori.kt`), `0xAARRGGBB`.
pub const LOGO_COLOR: u32 = 0xFF28_2828;
/// The 48 px logo (T-31a-047). The provider's mark: shipping it is the user's licence decision (OQ-3).
const LOGO_PNG: &[u8] = include_bytes!("../../assets/tracker/ic_tracker_shikimori.png");
const COMPLETED: i64 = 2;
const READING: i64 = 1;
const REREADING: i64 = 6;
const PLAN_TO_READ: i64 = 5;

/// The status after a read or watch event (`Shikimori.kt:77-89`, `91-103`). Only runs for a read event, and never
/// changes a completed entry. A reached total completes it. Otherwise the entry becomes reading, unless it is
/// rereading.
pub fn next_status_after_read(
    current: ShikiStatus,
    did_read: bool,
    progress: f64,
    total: i64,
) -> ShikiStatus {
    if current == ShikiStatus::Completed || !did_read {
        return current;
    }
    if progress as i64 == total && total > 0 {
        return ShikiStatus::Completed;
    }
    if current == ShikiStatus::Rereading {
        return current;
    }
    ShikiStatus::Reading
}

/// The status of a found entry after bind (`Shikimori.kt:119-122`): reading when there is local progress, unless the
/// entry is completed or rereading.
fn bind_found_status(remote: ShikiStatus, has_read: bool) -> ShikiStatus {
    if remote != ShikiStatus::Completed && remote != ShikiStatus::Rereading && has_read {
        ShikiStatus::Reading
    } else {
        remote
    }
}

fn status_of(v: i64) -> Result<ShikiStatus, ShikimoriError> {
    ShikiStatus::from_internal(v).ok_or_else(|| ShikimoriError::UnknownStatus(v.to_string()))
}

pub struct ShikimoriTracker {
    api: ShikimoriApi,
    labels: Vec<String>,
}

impl ShikimoriTracker {
    /// `client` is the user's OAuth client. Without it login is refused (D-OPEN-1). `redirect_uri` is the one the
    /// authorise URL and the code exchange both carry.
    pub fn new(
        endpoints: Endpoints,
        client: Option<OAuthClient>,
        redirect_uri: &str,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, ShikimoriError> {
        Ok(Self {
            api: ShikimoriApi::new(endpoints, client, redirect_uri, secrets)?,
            labels: (0..=10).map(|n| n.to_string()).collect(),
        })
    }

    pub fn production(
        client: Option<OAuthClient>,
        redirect_uri: &str,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, ShikimoriError> {
        Self::new(Endpoints::production(), client, redirect_uri, secrets)
    }

    pub fn api(&self) -> &ShikimoriApi {
        &self.api
    }

    /// The URL the browser opens for the login.
    pub fn auth_url(&self) -> Result<String, ShikimoriError> {
        self.api.auth_url()
    }

    /// Completes a login with the code of the redirect (`login(code)`, `Shikimori.kt:223-232`). Nothing is stored
    /// unless the token exchange and the user lookup both succeed, so a failed attempt leaves an existing login alone
    /// (RC-06). The username is the user id, the password the access token.
    pub async fn login_with_code(&self, code: &str) -> Result<(), TrackError> {
        let token = self.api.exchange_code(code).await?;
        let user = self.api.who_am_i(&token.access_token).await?;
        self.api
            .base()
            .save_oauth(&user.id.to_string(), &token.access_token, &token)?;
        self.api.tokens().login(token);
        Ok(())
    }

    /// The stored user id. Logged out, nothing is sent (T-31a-008).
    fn user_id(&self) -> Result<String, ShikimoriError> {
        let id = self.api.base().username();
        if id.is_empty() {
            return Err(ShikimoriError::NotAuthenticated);
        }
        Ok(id)
    }

    async fn write(
        &self,
        kind: Kind,
        target_id: i64,
        progress: f64,
        score: f64,
        status: ShikiStatus,
    ) -> Result<i64, ShikimoriError> {
        let rating = Rating {
            kind,
            user_id: self.user_id()?,
            target_id,
            progress,
            score,
            status,
        };
        self.api.upsert(&rating).await
    }

    /// The user's row for a catalogue record. `Ok(None)` when the user has none.
    async fn find(&self, kind: Kind, target_id: i64) -> Result<Option<LibEntry>, ShikimoriError> {
        let user = self.user_id()?;
        self.api.find_library(kind, target_id, &user).await
    }
}

#[async_trait]
impl Tracker for ShikimoriTracker {
    fn id(&self) -> TrackerId {
        TrackerId::SHIKIMORI
    }

    fn name(&self) -> &str {
        NAME
    }

    fn logo(&self) -> LogoRef {
        LogoRef::Embedded(LOGO_PNG)
    }

    fn logo_color(&self) -> u32 {
        LOGO_COLOR
    }

    /// The authorise URL for the configured client. `None` while the client is not configured (T-31a-037).
    fn browser_auth_url(&self) -> Option<String> {
        self.auth_url().ok()
    }

    /// The Settings row shows this while the OAuth client is not configured (T-31a-037).
    fn setup_message(&self) -> Option<MsgKey> {
        (!self.api.has_client()).then(|| MsgKey::TrackerNotConfigured(NAME.to_owned()))
    }

    fn completion_status(&self) -> i64 {
        COMPLETED
    }

    fn score_list(&self) -> &[String] {
        &self.labels
    }

    /// The password is the OAuth code of the redirect (`login(password)`).
    async fn login(&self, _username: &str, password: &str) -> Result<(), TrackError> {
        self.login_with_code(password).await
    }

    /// Credentials first, then the token (`Logout order`).
    fn logout(&self) -> Result<(), TrackError> {
        let result = self.api.base().logout();
        self.api.tokens().clear();
        result
    }

    fn is_logged_in(&self) -> bool {
        self.api.base().is_logged_in()
    }

    fn auth_expired(&self) -> bool {
        self.api.base().auth_expired()
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.api.base().login_state()
    }

    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.api.base().save_credentials(username, password)
    }

    fn username(&self) -> String {
        self.api.base().username()
    }

    fn password(&self) -> String {
        self.api.base().password()
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

/// The status list, shared by both kinds (`getStatusListManga` and `getStatusListAnime`).
fn status_list() -> Vec<i64> {
    ShikiStatus::ALL.iter().map(|s| *s as i64).collect()
}

#[async_trait]
impl MangaTracker for ShikimoriTracker {
    fn status_list(&self) -> Vec<i64> {
        status_list()
    }

    fn reading_status(&self) -> i64 {
        READING
    }

    fn rereading_status(&self) -> i64 {
        REREADING
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        status::label(Kind::Manga, status)
    }

    fn display_score(&self, t: &MangaTrack) -> String {
        (t.score as i64).to_string()
    }

    async fn update(
        &self,
        t: &MangaTrack,
        did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        let current = status_of(t.status)?;
        let status = next_status_after_read(
            current,
            did_read_chapter,
            t.last_chapter_read,
            t.total_chapters,
        );
        let library = self
            .write(
                Kind::Manga,
                t.remote_id,
                t.last_chapter_read,
                t.score,
                status,
            )
            .await?;
        let mut out = t.clone();
        out.status = status as i64;
        out.library_id = Some(library);
        Ok(out)
    }

    async fn bind(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), TrackError> {
        match self.find(Kind::Manga, t.remote_id).await? {
            Some(lib) => {
                t.last_chapter_read = lib.progress;
                t.score = lib.score;
                t.status = bind_found_status(lib.status, has_read) as i64;
                t.library_id = Some(lib.library_id);
            }
            None => {
                t.status = if has_read { READING } else { PLAN_TO_READ };
                t.score = 0.0;
            }
        }
        let library = self
            .write(
                Kind::Manga,
                t.remote_id,
                t.last_chapter_read,
                t.score,
                status_of(t.status)?,
            )
            .await?;
        t.library_id = Some(library);
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        let entries = self.api.search(Kind::Manga, query).await?;
        let base = self.api.base_url();
        Ok(entries
            .iter()
            .filter_map(|e| search::to_manga_search(base, e))
            .collect())
    }

    /// Copies progress, score and status, and the catalogue chapter count. Nothing is written (T-31a-013 refresh).
    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let lib = self
            .find(Kind::Manga, t.remote_id)
            .await?
            .ok_or(ShikimoriError::NotFound("manga"))?;
        let mut out = t.clone();
        out.library_id = Some(lib.library_id);
        out.last_chapter_read = lib.progress;
        out.score = lib.score;
        out.status = lib.status as i64;
        out.total_chapters = lib.total;
        Ok(out)
    }
}

#[async_trait]
impl AnimeTracker for ShikimoriTracker {
    fn status_list(&self) -> Vec<i64> {
        status_list()
    }

    fn watching_status(&self) -> i64 {
        READING
    }

    fn rewatching_status(&self) -> i64 {
        REREADING
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        status::label(Kind::Anime, status)
    }

    fn display_score(&self, t: &AnimeTrack) -> String {
        (t.score as i64).to_string()
    }

    async fn update(
        &self,
        t: &AnimeTrack,
        did_watch_episode: bool,
    ) -> Result<AnimeTrack, TrackError> {
        let current = status_of(t.status)?;
        let status = next_status_after_read(
            current,
            did_watch_episode,
            t.last_episode_seen,
            t.total_episodes,
        );
        let library = self
            .write(
                Kind::Anime,
                t.remote_id,
                t.last_episode_seen,
                t.score,
                status,
            )
            .await?;
        let mut out = t.clone();
        out.status = status as i64;
        out.library_id = Some(library);
        Ok(out)
    }

    async fn bind(&self, t: &mut AnimeTrack, has_seen: bool) -> Result<(), TrackError> {
        match self.find(Kind::Anime, t.remote_id).await? {
            Some(lib) => {
                t.last_episode_seen = lib.progress;
                t.score = lib.score;
                t.status = bind_found_status(lib.status, has_seen) as i64;
                t.library_id = Some(lib.library_id);
            }
            None => {
                t.status = if has_seen { READING } else { PLAN_TO_READ };
                t.score = 0.0;
            }
        }
        let library = self
            .write(
                Kind::Anime,
                t.remote_id,
                t.last_episode_seen,
                t.score,
                status_of(t.status)?,
            )
            .await?;
        t.library_id = Some(library);
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        let entries = self.api.search(Kind::Anime, query).await?;
        let base = self.api.base_url();
        Ok(entries
            .iter()
            .map(|e| search::to_anime_search(base, e))
            .collect())
    }

    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        let lib = self
            .find(Kind::Anime, t.remote_id)
            .await?
            .ok_or(ShikimoriError::NotFound("anime"))?;
        let mut out = t.clone();
        out.library_id = Some(lib.library_id);
        out.last_episode_seen = lib.progress;
        out.score = lib.score;
        out.status = lib.status as i64;
        out.total_episodes = lib.total;
        Ok(out)
    }
}

#[async_trait]
impl DeletableMangaTracker for ShikimoriTracker {
    async fn delete(&self, t: &MangaTrack) -> Result<(), TrackError> {
        let library = t.library_id.ok_or(ShikimoriError::NoLibraryId)?;
        Ok(self.api.delete(library).await?)
    }
}

#[async_trait]
impl DeletableAnimeTracker for ShikimoriTracker {
    async fn delete(&self, t: &AnimeTrack) -> Result<(), TrackError> {
        let library = t.library_id.ok_or(ShikimoriError::NoLibraryId)?;
        Ok(self.api.delete(library).await?)
    }
}
