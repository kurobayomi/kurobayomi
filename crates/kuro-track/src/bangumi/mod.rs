//! Bangumi tracker (`Bangumi.kt`): identity, login by OAuth code, the collection rules and the tracker traits.
//! The network and token state are in [`api`].

pub mod api;
pub mod auth;
pub mod dto;
pub mod error;
pub mod search;
pub mod status;
#[cfg(test)]
mod tests;

use kuro_platform::kind::Kind;
use std::sync::Arc;

use async_trait::async_trait;
use kuro_platform::secrets::SecretStore;
use kuro_prefs::pref::Pref;
use tokio::sync::watch;

use self::api::{BangumiApi, CollectionWrite, Endpoints};
use self::dto::BgmEpisode;
use self::error::BangumiError;
use self::status::BgmStatus;
use crate::error::TrackError;
use crate::messages::MsgKey;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::shikimori::api::OAuthClient;
use crate::tracker::{AnimeTracker, LogoRef, MangaTracker, Tracker};

pub const NAME: &str = "Bangumi";
/// `Color.rgb(240, 145, 153)` (`Bangumi.kt:150`), `0xAARRGGBB`.
pub const LOGO_COLOR: u32 = 0xFFF0_9199;
/// The 48 px logo (T-31a-047). The provider's mark: shipping it is the user's licence decision (OQ-3).
const LOGO_PNG: &[u8] = include_bytes!("../../assets/tracker/ic_tracker_bangumi.png");
const COMPLETED: i64 = 2;
const READING: i64 = 3;
/// Bangumi has no rereading or rewatching status (`Bangumi.kt:182-184`, both `-1`).
const NO_REPEAT_STATUS: i64 = -1;

/// The status after a read or watch event (`Bangumi.kt:56-68`, `70-82`). Only runs for a read event. A reached total
/// completes the entry, and any other status becomes reading. Completed entries never change. There is no rereading
/// guard, because Bangumi has no rereading.
pub fn next_status_after_read(
    current: BgmStatus,
    did_read: bool,
    progress: f64,
    total: i64,
) -> BgmStatus {
    if current == BgmStatus::Completed || !did_read {
        return current;
    }
    if progress as i64 == total && total > 0 {
        BgmStatus::Completed
    } else {
        BgmStatus::Reading
    }
}

/// The status of a found entry after bind (`Bangumi.kt:93-95`): reading when there is local progress, unless the
/// entry is completed.
fn bind_found_status(remote: BgmStatus, has_read: bool) -> BgmStatus {
    if remote != BgmStatus::Completed && has_read {
        BgmStatus::Reading
    } else {
        remote
    }
}

fn status_of(v: i64) -> Result<BgmStatus, BangumiError> {
    BgmStatus::from_internal(v).ok_or_else(|| BangumiError::UnknownStatus(v.to_string()))
}

/// The status list, shared by both kinds (`getStatusListManga` and `getStatusListAnime`).
fn status_list() -> Vec<i64> {
    BgmStatus::PICKER_ORDER.iter().map(|s| *s as i64).collect()
}

/// Where the episode-progress switch comes from: a fixed value, or the user's setting read on each write.
enum EpisodeSwitch {
    Fixed(bool),
    Setting(Pref<bool>),
}

impl EpisodeSwitch {
    fn is_on(&self) -> bool {
        match self {
            EpisodeSwitch::Fixed(on) => *on,
            EpisodeSwitch::Setting(pref) => pref.get(),
        }
    }
}

/// Bangumi's episode-based progress for anime (T-31a-044, D7) is off until it is verified on a live account. The
/// default is the parity write of `ep_status`.
pub struct BangumiTracker {
    api: BangumiApi,
    labels: Vec<String>,
    episode_progress: EpisodeSwitch,
}

/// The ids of the first `progress` main episodes, in episode order (T-31a-044). Specials (`type` other than 0) are
/// not counted, and a progress above the episode count marks every main episode.
fn first_main_episodes(episodes: &[BgmEpisode], progress: i64) -> Vec<i64> {
    let mut main: Vec<&BgmEpisode> = episodes.iter().filter(|e| e.kind == 0).collect();
    main.sort_by(|a, b| a.sort.total_cmp(&b.sort));
    main.iter()
        .take(progress.max(0) as usize)
        .map(|e| e.id)
        .collect()
}

impl BangumiTracker {
    /// `client` is the user's OAuth client. Without it login is refused (D-OPEN-1). `redirect_uri` is the one the
    /// authorise URL and the code exchange both carry.
    pub fn new(
        endpoints: Endpoints,
        client: Option<OAuthClient>,
        redirect_uri: &str,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, BangumiError> {
        Ok(Self {
            api: BangumiApi::new(endpoints, client, redirect_uri, secrets)?,
            labels: (0..=10).map(|n| n.to_string()).collect(),
            episode_progress: EpisodeSwitch::Fixed(false),
        })
    }

    /// Turns the episode-based anime progress on or off (T-31a-044). The setting is off by default.
    pub fn with_episode_progress(mut self, on: bool) -> Self {
        self.episode_progress = EpisodeSwitch::Fixed(on);
        self
    }

    /// Reads the episode-based anime progress from the user's setting (`TrackPrefs::bangumi_episode_progress`) on
    /// every write, so a change applies without a restart (T-31a-044).
    pub fn with_episode_progress_setting(mut self, setting: Pref<bool>) -> Self {
        self.episode_progress = EpisodeSwitch::Setting(setting);
        self
    }

    /// True while the anime progress is still the unverified `ep_status` write, so the UI says so (D7).
    pub fn anime_progress_unverified(&self) -> bool {
        !self.episode_progress.is_on()
    }

    /// Writes the anime progress through the episode endpoints (T-31a-044). Does nothing unless the setting is on, or
    /// when no episode is watched.
    async fn write_episode_progress(
        &self,
        subject_id: i64,
        progress: f64,
    ) -> Result<(), BangumiError> {
        let count = progress as i64;
        if !self.episode_progress.is_on() || count <= 0 {
            return Ok(());
        }
        let episodes = self.api.episodes(subject_id).await?;
        let ids = first_main_episodes(&episodes, count);
        if ids.is_empty() {
            return Ok(());
        }
        self.api.mark_episodes_done(subject_id, &ids).await
    }

    pub fn production(
        client: Option<OAuthClient>,
        redirect_uri: &str,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, BangumiError> {
        Self::new(Endpoints::production(), client, redirect_uri, secrets)
    }

    pub fn api(&self) -> &BangumiApi {
        &self.api
    }

    /// The URL the browser opens for the login.
    pub fn auth_url(&self) -> Result<String, BangumiError> {
        self.api.auth_url()
    }

    /// Completes a login with the code of the redirect (`login(code)`, `Bangumi.kt:190-202`). Nothing is stored
    /// unless the code exchange and the user lookup both succeed, so a failed attempt leaves an existing login alone.
    /// The user name is the login name, and the password is the access token (T-31a-026).
    pub async fn login_with_code(&self, code: &str) -> Result<(), TrackError> {
        let token = self.api.exchange_code(code).await?;
        let user = self.api.me(&token.access_token).await?;
        self.api
            .base()
            .save_oauth(&user.username, &token.access_token, &token)?;
        self.api.tokens().login(token);
        Ok(())
    }

    /// The stored login name, which the collection paths need. Logged out, nothing is sent (EC-B-32).
    fn user(&self) -> Result<String, BangumiError> {
        let name = self.api.base().username();
        if name.is_empty() {
            return Err(BangumiError::NotAuthenticated);
        }
        Ok(name)
    }
}

#[async_trait]
impl Tracker for BangumiTracker {
    fn id(&self) -> TrackerId {
        TrackerId::BANGUMI
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

    /// Bangumi keeps a private flag per collection (`Bangumi.kt:32`). Shikimori has none.
    fn supports_private_tracking(&self) -> bool {
        true
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

    /// Credentials first, then the token (T-31a-034, the same order as Shikimori).
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
}

#[async_trait]
impl MangaTracker for BangumiTracker {
    fn status_list(&self) -> Vec<i64> {
        status_list()
    }

    fn reading_status(&self) -> i64 {
        READING
    }

    fn rereading_status(&self) -> i64 {
        NO_REPEAT_STATUS
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        status::label(Kind::Manga, status)
    }

    /// The score is shown as a whole number (`score.toInt()`).
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
        let write = CollectionWrite {
            status,
            score: t.score,
            progress: t.last_chapter_read,
            private: t.private,
        };
        self.api.update(t.remote_id, &write).await?;
        let mut out = t.clone();
        out.status = status as i64;
        Ok(out)
    }

    /// Found: copy the progress, score, total and status, then PATCH. The private flag is kept (EC-B-22).
    /// Not found: POST as reading or plan to read, with a score of 0 (EC-B-16).
    async fn bind(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), TrackError> {
        let user = self.user()?;
        let (status, found) = match self.api.lookup(&user, t.remote_id).await? {
            Some(lib) => {
                t.last_chapter_read = lib.progress;
                t.score = lib.score;
                t.total_chapters = lib.total;
                let status = bind_found_status(lib.status, has_read);
                t.status = status as i64;
                (status, true)
            }
            None => {
                let status = if has_read {
                    BgmStatus::Reading
                } else {
                    BgmStatus::PlanToRead
                };
                t.status = status as i64;
                t.score = 0.0;
                (status, false)
            }
        };
        let write = CollectionWrite {
            status,
            score: t.score,
            progress: t.last_chapter_read,
            private: t.private,
        };
        if found {
            self.api.update(t.remote_id, &write).await?;
        } else {
            self.api.add(t.remote_id, &write).await?;
        }
        Ok(())
    }

    /// Manga results keep only null or `漫画` platforms (EC-B-23).
    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        let subjects = self.api.search(Kind::Manga, query).await?;
        Ok(subjects
            .iter()
            .filter(|s| search::is_manga(s))
            .map(search::to_manga_search)
            .collect())
    }

    /// Copies the progress, score, status and total. Nothing is written, and `library_id` and `private` are untouched.
    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let user = self.user()?;
        let lib = self
            .api
            .lookup(&user, t.remote_id)
            .await?
            .ok_or(BangumiError::NotFound("manga"))?;
        let mut out = t.clone();
        out.last_chapter_read = lib.progress;
        out.score = lib.score;
        out.status = lib.status as i64;
        out.total_chapters = lib.total;
        Ok(out)
    }
}

#[async_trait]
impl AnimeTracker for BangumiTracker {
    fn status_list(&self) -> Vec<i64> {
        status_list()
    }

    fn watching_status(&self) -> i64 {
        READING
    }

    fn rewatching_status(&self) -> i64 {
        NO_REPEAT_STATUS
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
        let write = CollectionWrite {
            status,
            score: t.score,
            progress: t.last_episode_seen,
            private: t.private,
        };
        self.api.update(t.remote_id, &write).await?;
        self.write_episode_progress(t.remote_id, t.last_episode_seen)
            .await?;
        let mut out = t.clone();
        out.status = status as i64;
        Ok(out)
    }

    async fn bind(&self, t: &mut AnimeTrack, has_seen: bool) -> Result<(), TrackError> {
        let user = self.user()?;
        let (status, found) = match self.api.lookup(&user, t.remote_id).await? {
            Some(lib) => {
                t.last_episode_seen = lib.progress;
                t.score = lib.score;
                t.total_episodes = lib.total;
                let status = bind_found_status(lib.status, has_seen);
                t.status = status as i64;
                (status, true)
            }
            None => {
                let status = if has_seen {
                    BgmStatus::Reading
                } else {
                    BgmStatus::PlanToRead
                };
                t.status = status as i64;
                t.score = 0.0;
                (status, false)
            }
        };
        let write = CollectionWrite {
            status,
            score: t.score,
            progress: t.last_episode_seen,
            private: t.private,
        };
        if found {
            self.api.update(t.remote_id, &write).await?;
        } else {
            self.api.add(t.remote_id, &write).await?;
        }
        self.write_episode_progress(t.remote_id, t.last_episode_seen)
            .await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        let subjects = self.api.search(Kind::Anime, query).await?;
        Ok(subjects.iter().map(search::to_anime_search).collect())
    }

    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        let user = self.user()?;
        let lib = self
            .api
            .lookup(&user, t.remote_id)
            .await?
            .ok_or(BangumiError::NotFound("anime"))?;
        let mut out = t.clone();
        out.last_episode_seen = lib.progress;
        out.score = lib.score;
        out.status = lib.status as i64;
        out.total_episodes = lib.total;
        Ok(out)
    }
}
