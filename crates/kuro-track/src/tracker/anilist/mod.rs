//! AniList tracker (area:28). The data flows are in `service`, the wire format in `api`/`dto`/`queries`,
//! and the typed rules in `status`, `score`, `auth`, `date`, `convert` and `html`.
//!
//! The login lives in the shared credential layout of [`BaseTracker`] (`tracker/2/*` under
//! [`crate::base::SECRET_SERVICE`]): the token JSON (access token and expiry) at `oauth`, the viewer id in the
//! `account` slot, the access token in the `password` slot and the auth-expired flag at `expired`. The score
//! format is in the preference `anilist_score_type`. Every committed login, logout and expiry publishes the login
//! state and sends [`crate::events::TrackEvent::LoginChanged`]. Local track rows are never touched here; callers
//! persist what the flows change.

pub mod api;
pub mod auth;
pub mod convert;
pub mod date;
pub mod discovery;
pub mod dto;
pub mod error;
pub mod html;
pub mod queries;
pub mod score;
pub mod service;
pub mod status;
mod tracker_impl;

#[cfg(test)]
pub(crate) mod testkit;

use std::sync::Arc;

use async_trait::async_trait;
use kuro_platform::secrets::SecretStore;
use kuro_prefs::pref::Pref;
use kuro_prefs::store::PrefStore;
use kuro_prefs::values::PrefValue;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::base::BaseTracker;
use crate::error::TrackError;
use crate::events::EventBus;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::prefs::{AniListScoreType, TrackPrefs};
use crate::tracker::{
    AnimeTracker, DeletableAnimeTracker, DeletableMangaTracker, LogoRef, MangaTracker, Tracker,
};

use api::{Api, HttpTransport, Transport};
use auth::{AccessToken, AuthState, DEFAULT_LIFETIME_S, is_expired, normalize_expiry_s};
use date::{FuzzyDateInput, create_date};
use dto::{DeleteData, FindData, SaveEntryData, SearchData, ViewerData};
use error::AniListError;
use score::ScoreFormat;
use status::{AnimeStatus, MangaStatus};

pub use auth::{AUTHORIZE_URL, RedirectOutcome, parse_redirect};

/// The GraphQL endpoint (`API:571`).
pub fn endpoint() -> &'static str {
    "https://graphql.anilist.co/"
}

/// Result of a successful login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginSuccess {
    pub viewer_id: i64,
    pub score_format: ScoreFormat,
}

/// The token JSON stored at `tracker/2/oauth`. The expiry is in unix seconds.
#[derive(Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    expires_at: i64,
}

const FORMATS: [ScoreFormat; 5] = [
    ScoreFormat::Point100,
    ScoreFormat::Point10,
    ScoreFormat::Point10Decimal,
    ScoreFormat::Point5,
    ScoreFormat::Point3,
];

/// Preference key of the score scale. Before the string scale it held an integer (T-28a-013).
const SCORE_KEY: &str = "anilist_score_type";

/// The AniList tracker: login, search, find, add, save.
pub struct AniList {
    api: Api,
    base: Arc<BaseTracker>,
    score_type: Pref<AniListScoreType>,
    /// Picker labels of every score format, in [`FORMATS`] order, so `score_list` can lend a slice.
    scores: Vec<Vec<String>>,
    /// Set when [`AniList::open`] found a legacy score preference and logged the user out once.
    relogin_required: bool,
}

#[derive(Serialize)]
struct SearchVars<'a> {
    query: &'a str,
}

#[derive(Serialize)]
struct FindMangaVars {
    id: i64,
    manga_id: i64,
}

#[derive(Serialize)]
struct FindAnimeVars {
    id: i64,
    anime_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddVars {
    media_id: i64,
    progress: i32,
    status: &'static str,
    private: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteVars {
    list_id: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateVars {
    list_id: i64,
    progress: i32,
    status: &'static str,
    private: bool,
    score: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<FuzzyDateInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<FuzzyDateInput>,
}

impl AniList {
    /// Tracker id stored in the track rows (`TrackerManager.ANILIST`).
    pub const TRACKER_ID: i64 = 2;
    pub const NAME: &'static str = "AniList";
    pub const LOGO_RGB: (u8, u8, u8) = (18, 25, 35);

    pub fn supports_reading_dates() -> bool {
        true
    }

    pub fn supports_private_tracking() -> bool {
        true
    }

    pub fn is_deletable() -> bool {
        true
    }

    /// Opens the tracker with the login stored by an earlier run. A stored token past its expiry is cleared and the
    /// auth-expired flag is set. `events` receives a `LoginChanged` after every committed login change.
    pub fn open(
        transport: Arc<dyn Transport>,
        secrets: Arc<dyn SecretStore>,
        prefs: &PrefStore,
        events: EventBus,
    ) -> Result<Self, AniListError> {
        let base = Arc::new(BaseTracker::new(
            TrackerId::ANILIST,
            Self::NAME,
            secrets,
            events,
        ));
        let score_type = TrackPrefs::new(prefs.clone()).anilist_score_type;
        if legacy_score_pref(prefs) {
            // The old integer scale is gone, so the login is renewed once: logout, drop the key, tell the user.
            base.logout()?;
            score_type.delete()?;
            let mut api = Self::assemble(transport, base, score_type, AuthState::LoggedOut);
            api.relogin_required = true;
            return Ok(api);
        }
        let state = restore(&base, score_type.get(), now_s())?;
        Ok(Self::assemble(transport, base, score_type, state))
    }

    /// True when [`AniList::open`] logged the user out because of a legacy score preference. The GUI shows
    /// [`crate::messages::MsgKey::AniListReloginRequired`] once.
    pub fn relogin_required(&self) -> bool {
        self.relogin_required
    }

    fn assemble(
        transport: Arc<dyn Transport>,
        base: Arc<BaseTracker>,
        score_type: Pref<AniListScoreType>,
        state: AuthState,
    ) -> Self {
        Self {
            api: Api::new(transport, base.clone(), state),
            base,
            score_type,
            scores: FORMATS.iter().map(|f| f.options()).collect(),
            relogin_required: false,
        }
    }

    /// Opens the tracker in the given state, without reading the stores. Used by tests.
    #[cfg(test)]
    pub(crate) fn from_parts(
        transport: Arc<dyn Transport>,
        secrets: Arc<dyn SecretStore>,
        prefs: &PrefStore,
        state: AuthState,
    ) -> Self {
        let base = Arc::new(BaseTracker::new(
            TrackerId::ANILIST,
            Self::NAME,
            secrets,
            EventBus::default(),
        ));
        let score_type = TrackPrefs::new(prefs.clone()).anilist_score_type;
        Self::assemble(transport, base, score_type, state)
    }

    /// The production client with the stores given.
    pub fn with_http(
        secrets: Arc<dyn SecretStore>,
        prefs: &PrefStore,
        events: EventBus,
    ) -> Result<Self, AniListError> {
        Self::open(Arc::new(HttpTransport::new()?), secrets, prefs, events)
    }

    /// The shared credential state: username, access token, auth-expired flag and the login-state watch.
    pub fn base(&self) -> &BaseTracker {
        &self.base
    }

    /// Reads the Viewer with the new token, then stores the login. A failed Viewer query restores the state held
    /// before the attempt and writes nothing, so an earlier valid session stays (T-27b-024). A failure while storing
    /// returns the tracker to logged out, because the attempt had begun to overwrite the stored slots (E-01).
    pub async fn login(
        &self,
        token: AccessToken,
        expires_in: Option<i64>,
    ) -> Result<LoginSuccess, AniListError> {
        let expires_at = now_s() + expires_in.unwrap_or(DEFAULT_LIFETIME_S);
        let previous = self.api.state();
        self.api.set_state(AuthState::LoggedIn {
            token: token.clone(),
            viewer_id: 0,
            expires_at,
            score_format: ScoreFormat::Point10,
        });
        let data: ViewerData = match self.api.execute_probe(queries::VIEWER, None::<()>).await {
            Ok(data) => data,
            Err(e) => {
                self.api.set_state(previous);
                return Err(e);
            }
        };
        match self.store_viewer(&token, expires_at, data) {
            Ok(success) => Ok(success),
            Err(e) => {
                if let Err(clear) = self.logout() {
                    tracing::warn!(target: "kurobayomi::tracker::anilist", error = %clear, "logout after failed login");
                }
                Err(e)
            }
        }
    }

    fn store_viewer(
        &self,
        token: &AccessToken,
        expires_at: i64,
        data: ViewerData,
    ) -> Result<LoginSuccess, AniListError> {
        let viewer_id = data.viewer.id;
        let format_text = data
            .viewer
            .media_list_options
            .as_ref()
            .map_or("POINT_10", |o| o.score_format.as_str());
        let score_format = ScoreFormat::from_api_or_default(format_text);

        self.api.set_state(AuthState::LoggedIn {
            token: token.clone(),
            viewer_id,
            expires_at,
            score_format,
        });
        self.score_type.set(
            AniListScoreType::from_name(score_format.to_api())
                .unwrap_or(AniListScoreType::POINT_10),
        )?;
        // The username slot holds the viewer id (as in Aniyomi); the event follows this commit.
        let stored = StoredToken {
            access_token: token.expose().to_owned(),
            expires_at,
        };
        self.base
            .save_oauth(&viewer_id.to_string(), token.expose(), &stored)?;
        Ok(LoginSuccess {
            viewer_id,
            score_format,
        })
    }

    /// Clears the token JSON, the username, the access token, the auth-expired flag and the score format, then
    /// publishes the login state and sends `LoginChanged`. Local track rows are kept (E-48).
    pub fn logout(&self) -> Result<(), AniListError> {
        self.api.set_state(AuthState::LoggedOut);
        let score = self.score_type.delete();
        self.base.logout()?;
        score?;
        Ok(())
    }

    /// True while a token is held and not past its expiry.
    pub fn is_logged_in(&self) -> bool {
        self.api.bearer(now_s()).is_ok()
    }

    /// The score format of the account (POINT_10 before a login).
    pub fn score_format(&self) -> ScoreFormat {
        match self.api.state() {
            AuthState::LoggedIn { score_format, .. } => score_format,
            _ => ScoreFormat::Point10,
        }
    }

    fn viewer_id(&self) -> Result<i64, AniListError> {
        match self.api.state() {
            AuthState::LoggedIn { viewer_id, .. } => Ok(viewer_id),
            AuthState::Expired { .. } => Err(AniListError::AuthExpired),
            AuthState::LoggedOut => Err(AniListError::NotAuthenticated),
        }
    }

    pub async fn search_manga(&self, q: &str) -> Result<Vec<MangaTrackSearch>, AniListError> {
        let data: SearchData = self
            .api
            .execute(queries::SEARCH_MANGA, Some(SearchVars { query: q }))
            .await?;
        Ok(data.page.media.iter().map(convert::manga_search).collect())
    }

    pub async fn search_anime(&self, q: &str) -> Result<Vec<AnimeTrackSearch>, AniListError> {
        let data: SearchData = self
            .api
            .execute(queries::SEARCH_ANIME, Some(SearchVars { query: q }))
            .await?;
        Ok(data.page.media.iter().map(convert::anime_search).collect())
    }

    /// The user's list entry for a manga, or `None`. When AniList returns several, the first is used.
    pub async fn find_manga_entry(
        &self,
        media_id: i64,
    ) -> Result<Option<MangaTrack>, AniListError> {
        let vars = FindMangaVars {
            id: self.viewer_id()?,
            manga_id: media_id,
        };
        let data: FindData = self.api.execute(queries::FIND_MANGA, Some(vars)).await?;
        let entries = data.page.media_list;
        if entries.len() > 1 {
            tracing::warn!(target: "kurobayomi::tracker::anilist", count = entries.len(), "several list entries for one media");
        }
        entries.first().map(convert::manga_from_entry).transpose()
    }

    /// The user's list entry for an anime, or `None`. Same rules as [`AniList::find_manga_entry`].
    pub async fn find_anime_entry(
        &self,
        media_id: i64,
    ) -> Result<Option<AnimeTrack>, AniListError> {
        let vars = FindAnimeVars {
            id: self.viewer_id()?,
            anime_id: media_id,
        };
        let data: FindData = self.api.execute(queries::FIND_ANIME, Some(vars)).await?;
        let entries = data.page.media_list;
        if entries.len() > 1 {
            tracing::warn!(target: "kurobayomi::tracker::anilist", count = entries.len(), "several list entries for one media");
        }
        entries.first().map(convert::anime_from_entry).transpose()
    }

    /// Adds the manga to the list and returns the new MediaList id, which the caller stores as `library_id`.
    pub async fn add_manga(&self, t: &MangaTrack) -> Result<i64, AniListError> {
        let status = manga_status(t.status)?.to_api().as_str();
        let vars = AddVars {
            media_id: t.remote_id,
            progress: t.last_chapter_read as i32,
            status,
            private: t.private,
        };
        let data: SaveEntryData = self.api.execute(queries::ADD_MANGA, Some(vars)).await?;
        Ok(data.entry.id)
    }

    /// Adds the anime to the list and returns the MediaList id. Same rules as [`AniList::add_manga`].
    pub async fn add_anime(&self, t: &AnimeTrack) -> Result<i64, AniListError> {
        let status = anime_status(t.status)?.to_api().as_str();
        let vars = AddVars {
            media_id: t.remote_id,
            progress: t.last_episode_seen as i32,
            status,
            private: t.private,
        };
        let data: SaveEntryData = self.api.execute(queries::ADD_ANIME, Some(vars)).await?;
        Ok(data.entry.id)
    }

    /// Deletes the MediaList entry `list_id`. `Ok(false)` when AniList reports nothing was deleted, which means the
    /// entry was already absent (E-21). The id is the MediaList id, never the local row id (E-20).
    pub async fn delete_entry(&self, list_id: i64) -> Result<bool, AniListError> {
        let data: DeleteData = self
            .api
            .execute(queries::DELETE_ENTRY, Some(DeleteVars { list_id }))
            .await?;
        Ok(data.result.deleted)
    }

    /// Writes progress, status, score, dates and privacy of an entry that has a `library_id`.
    pub async fn save_manga(&self, t: &MangaTrack) -> Result<(), AniListError> {
        let list_id = list_id(t.library_id)?;
        let status = manga_status(t.status)?.to_api().as_str();
        let vars = UpdateVars {
            list_id,
            progress: t.last_chapter_read as i32,
            status,
            private: t.private,
            score: ScoreFormat::to_score_raw(t.score),
            started_at: create_date(t.start_date),
            completed_at: create_date(t.finish_date),
        };
        let _: IgnoredAny = self.api.execute(queries::UPDATE_MANGA, Some(vars)).await?;
        Ok(())
    }

    /// Writes an anime entry that has a `library_id`. Same rules as [`AniList::save_manga`].
    pub async fn save_anime(&self, t: &AnimeTrack) -> Result<(), AniListError> {
        let list_id = list_id(t.library_id)?;
        let status = anime_status(t.status)?.to_api().as_str();
        let vars = UpdateVars {
            list_id,
            progress: t.last_episode_seen as i32,
            status,
            private: t.private,
            score: ScoreFormat::to_score_raw(t.score),
            started_at: create_date(t.start_date),
            completed_at: create_date(t.finish_date),
        };
        let _: IgnoredAny = self.api.execute(queries::UPDATE_ANIME, Some(vars)).await?;
        Ok(())
    }
}

/// True when the stored score preference still holds an integer (the scale before the string names).
fn legacy_score_pref(prefs: &PrefStore) -> bool {
    matches!(
        prefs.get_raw(SCORE_KEY),
        Some(PrefValue::Int(_) | PrefValue::Long(_))
    )
}

fn list_id(library_id: Option<i64>) -> Result<i64, AniListError> {
    match library_id {
        Some(id) if id != 0 => Ok(id),
        _ => Err(AniListError::NotInList),
    }
}

fn manga_status(db: i64) -> Result<MangaStatus, AniListError> {
    MangaStatus::from_db(db).ok_or_else(|| AniListError::UnknownStatus(db.to_string()))
}

fn anime_status(db: i64) -> Result<AnimeStatus, AniListError> {
    AnimeStatus::from_db(db).ok_or_else(|| AniListError::UnknownStatus(db.to_string()))
}

/// Builds the login state from the secret store at `now`. A token without its expiry or viewer is not a login
/// (E-04). A token past its expiry is cleared here, so the stores and the state agree. An unreadable token JSON
/// counts as no token.
fn restore(
    base: &BaseTracker,
    score_type: AniListScoreType,
    now: i64,
) -> Result<AuthState, AniListError> {
    let viewer_id = base.username().parse::<i64>().unwrap_or(0);
    let stored = base.load_token::<StoredToken>().ok().flatten();
    let Some(stored) = stored.filter(|t| {
        viewer_id != 0 && t.expires_at != 0 && base.is_logged_in() && !t.access_token.is_empty()
    }) else {
        return Ok(if base.auth_expired() {
            AuthState::Expired { viewer_id }
        } else {
            AuthState::LoggedOut
        });
    };
    let expires_at = normalize_expiry_s(stored.expires_at);
    if is_expired(expires_at, now) {
        base.mark_expired()?;
        return Ok(AuthState::Expired { viewer_id });
    }
    Ok(AuthState::LoggedIn {
        token: AccessToken::new(stored.access_token),
        viewer_id,
        expires_at,
        score_format: ScoreFormat::from_api_or_default(score_type.name()),
    })
}

fn now_s() -> i64 {
    api::now_s()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::{Field, SECRET_SERVICE, account};
    use crate::events::TrackEvent;
    use crate::fakes::MemSecrets;
    use crate::model::TrackerId;
    use crate::tracker::anilist::testkit::{FakeTransport, ok, status};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const VIEWER_10: &str = include_str!("../../../tests/fixtures/anilist/viewer_point10.json");
    const VIEWER_3: &str = include_str!("../../../tests/fixtures/anilist/viewer_point3.json");
    const VIEWER_UNKNOWN: &str =
        include_str!("../../../tests/fixtures/anilist/viewer_unknown_format.json");
    const SEARCH_MANGA: &str = include_str!("../../../tests/fixtures/anilist/search_manga_ok.json");
    const SEARCH_ANIME: &str = include_str!("../../../tests/fixtures/anilist/search_anime_ok.json");
    const SAVE_OK: &str = include_str!("../../../tests/fixtures/anilist/save_entry_ok.json");

    /// A login written by an earlier run, in the `tracker/2/*` layout.
    fn seed(secrets: &MemSecrets, token: &str, viewer: i64, expires_at: i64) -> TestResult {
        let id = TrackerId::ANILIST;
        secrets.set(
            SECRET_SERVICE,
            &account(id, Field::Username),
            &viewer.to_string(),
        )?;
        secrets.set(SECRET_SERVICE, &account(id, Field::Password), token)?;
        let json = serde_json::json!({ "access_token": token, "expires_at": expires_at });
        secrets.set(
            SECRET_SERVICE,
            &account(id, Field::Token),
            &json.to_string(),
        )?;
        Ok(())
    }

    fn token() -> AccessToken {
        AccessToken::new("token-value")
    }

    #[test]
    fn t_28a_002_identity_constants_and_capabilities() {
        assert_eq!(AniList::TRACKER_ID, 2);
        assert_eq!(AniList::NAME, "AniList");
        assert_eq!(AniList::LOGO_RGB, (18, 25, 35));
        assert!(AniList::supports_reading_dates());
        assert!(AniList::supports_private_tracking());
        assert!(AniList::is_deletable());
        assert_eq!(AniList::TRACKER_ID, TrackerId::ANILIST.0);
    }

    #[tokio::test]
    async fn t_27b_038_expiry_is_now_plus_expires_in_or_the_default_lifetime() -> TestResult {
        for (given, lifetime) in [(Some(3600), 3600), (None, DEFAULT_LIFETIME_S)] {
            let fake = FakeTransport::new(vec![ok(VIEWER_10)]);
            let secrets = MemSecrets::new();
            let prefs = PrefStore::in_memory();
            let api = AniList::open(fake.clone(), secrets.clone(), &prefs, EventBus::default())?;
            let before = now_s();
            api.login(token(), given).await?;
            let json: serde_json::Value =
                serde_json::from_str(&secrets.raw("tracker/2/oauth").unwrap_or_default())?;
            let expires_at = json["expires_at"].as_i64().unwrap_or(0);
            assert!(
                (before + lifetime..=now_s() + lifetime).contains(&expires_at),
                "expires_at {expires_at} for expires_in {given:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_011_login_reads_the_viewer_and_persists_the_login() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10)]);
        let secrets = MemSecrets::new();
        let prefs = PrefStore::in_memory();
        let api = AniList::open(fake.clone(), secrets.clone(), &prefs, EventBus::default())?;
        assert!(!api.is_logged_in());
        let success = api.login(token(), Some(3600)).await?;
        assert_eq!(success.viewer_id, 4242);
        assert_eq!(success.score_format, ScoreFormat::Point10);
        assert!(api.is_logged_in());
        assert_eq!(fake.bearers(), vec![Some("token-value".to_owned())]);
        assert_eq!(
            secrets.raw("tracker/2/password"),
            Some("token-value".to_owned())
        );
        assert_eq!(secrets.raw("tracker/2/account"), Some("4242".to_owned()));
        let json: serde_json::Value =
            serde_json::from_str(&secrets.raw("tracker/2/oauth").unwrap_or_default())?;
        assert_eq!(json["access_token"], "token-value");
        assert!(json["expires_at"].as_i64().unwrap_or(0) > now_s());
        assert_eq!(secrets.raw("tracker/2/expired"), None);
        assert_eq!(
            prefs.pref("anilist.viewer_id", 0_i64).get(),
            0,
            "no login data in preferences"
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_011_a_failed_viewer_query_leaves_the_tracker_logged_out() -> TestResult {
        let fake = FakeTransport::new(vec![status(401, "{}")]);
        let secrets = MemSecrets::new();
        let api = AniList::open(
            fake,
            secrets.clone(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        let r = api.login(token(), None).await;
        assert!(r.is_err(), "E-01: the failure is returned");
        assert!(!api.is_logged_in());
        assert_eq!(api.api.state(), AuthState::LoggedOut);
        assert!(
            secrets.all_values().is_empty(),
            "nothing stored after a failed login"
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_27b_024_a_failed_second_login_keeps_the_valid_session() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10), status(401, "{}")]);
        let secrets = MemSecrets::new();
        let prefs = PrefStore::in_memory();
        let api = AniList::open(fake, secrets.clone(), &prefs, EventBus::default())?;
        api.login(token(), None).await?;
        let stored = secrets.all_values();
        let state = api.api.state();

        let r = api.login(AccessToken::new("bad-token"), None).await;
        assert!(r.is_err());
        assert!(api.is_logged_in(), "the first session is still held");
        assert!(api.api.state() == state);
        assert_eq!(
            secrets.raw("tracker/2/password"),
            Some("token-value".into())
        );
        assert_eq!(secrets.raw("tracker/2/account"), Some("4242".into()));
        assert_eq!(secrets.raw("tracker/2/expired"), None, "no expired flag");
        assert_eq!(secrets.all_values().len(), stored.len());
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_011_score_format_is_read_on_every_login() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10), ok(VIEWER_3)]);
        let prefs = PrefStore::in_memory();
        let api = AniList::open(fake, MemSecrets::new(), &prefs, EventBus::default())?;
        api.login(token(), None).await?;
        let second = api.login(token(), None).await?;
        assert_eq!(second.score_format, ScoreFormat::Point3, "E-50");
        assert_eq!(api.score_format(), ScoreFormat::Point3);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_011_decimal_viewer_sets_point10_decimal() -> TestResult {
        let decimal = include_str!("../../../tests/fixtures/anilist/viewer_point_decimal.json");
        let fake = FakeTransport::new(vec![ok(decimal)]);
        let api = AniList::open(
            fake,
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        let success = api.login(token(), None).await?;
        assert_eq!(success.viewer_id, 4242);
        assert_eq!(success.score_format, ScoreFormat::Point10Decimal);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_005_unknown_score_format_falls_back_to_point10() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_UNKNOWN)]);
        let api = AniList::open(
            fake,
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        let success = api.login(token(), None).await?;
        assert_eq!(success.score_format, ScoreFormat::Point10);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_012_logout_removes_the_token_row_and_keeps_no_null_text() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10)]);
        let secrets = MemSecrets::new();
        let prefs = PrefStore::in_memory();
        let api = AniList::open(fake, secrets.clone(), &prefs, EventBus::default())?;
        api.login(token(), None).await?;
        api.logout()?;
        assert!(!api.is_logged_in());
        for slot in ["oauth", "account", "password", "expired"] {
            assert_eq!(
                secrets.raw(&format!("tracker/2/{slot}")),
                None,
                "E-05: {slot} deleted, not the text null"
            );
        }
        assert!(secrets.all_values().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_012_logout_keeps_nothing_for_the_next_run() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10)]);
        let secrets = MemSecrets::new();
        let prefs = PrefStore::in_memory();
        let api = AniList::open(fake.clone(), secrets.clone(), &prefs, EventBus::default())?;
        api.login(token(), None).await?;
        api.logout()?;
        let reopened = AniList::open(fake, secrets, &prefs, EventBus::default())?;
        assert!(
            !reopened.is_logged_in(),
            "E-48 is about tracks; the login itself is gone"
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_010_token_without_its_record_is_not_authenticated_and_sends_nothing()
    -> TestResult {
        let fake = FakeTransport::new(vec![]);
        let secrets = MemSecrets::new();
        secrets.set(
            SECRET_SERVICE,
            &account(TrackerId::ANILIST, Field::Password),
            "orphan-token",
        )?;
        let api = AniList::open(
            fake.clone(),
            secrets,
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        let r = api.search_manga("x").await;
        assert!(matches!(r, Err(AniListError::NotAuthenticated)), "E-04");
        assert_eq!(fake.calls(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_009_restored_token_past_its_expiry_is_expired_and_unused() -> TestResult {
        let fake = FakeTransport::new(vec![]);
        let secrets = MemSecrets::new();
        seed(&secrets, "stale", 77, 1)?;
        let prefs = PrefStore::in_memory();
        let api = AniList::open(fake.clone(), secrets.clone(), &prefs, EventBus::default())?;
        assert!(matches!(
            api.api.state(),
            AuthState::Expired { viewer_id: 77 }
        ));
        assert!(api.base().auth_expired(), "the flag is set at open");
        assert_eq!(
            secrets.raw("tracker/2/oauth"),
            None,
            "the stale token is removed"
        );
        let r = api.search_manga("x").await;
        assert!(matches!(r, Err(AniListError::AuthExpired)));
        assert_eq!(fake.calls(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_009_legacy_millisecond_expiry_is_read_as_seconds() -> TestResult {
        let secrets = MemSecrets::new();
        let far_future_ms: i64 = 4_000_000_000_000;
        seed(&secrets, "tok", 5, far_future_ms)?;
        let prefs = PrefStore::in_memory();
        let api = AniList::open(
            FakeTransport::new(vec![]),
            secrets,
            &prefs,
            EventBus::default(),
        )?;
        assert!(api.is_logged_in(), "4e12 ms is 4e9 s, still in the future");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_016_manga_search_asks_for_the_novel_filter_and_maps_three_results() -> TestResult
    {
        let fake = FakeTransport::new(vec![ok(VIEWER_10), ok(SEARCH_MANGA)]);
        let api = AniList::open(
            fake.clone(),
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        api.login(token(), None).await?;
        let rows = api.search_manga("bocchi").await?;
        assert_eq!(rows.len(), 3);
        let sent = fake.bodies();
        let query = sent[1]["query"].as_str().unwrap_or_default();
        assert!(query.contains("format_not_in: [NOVEL]"));
        assert_eq!(sent[1]["variables"]["query"], serde_json::json!("bocchi"));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_017_anime_search_maps_rows() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10), ok(SEARCH_ANIME)]);
        let api = AniList::open(
            fake.clone(),
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        api.login(token(), None).await?;
        let rows = api.search_anime("show").await?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].authors.len(), 3);
        let sent = fake.bodies();
        assert!(
            !sent[1]["query"]
                .as_str()
                .unwrap_or_default()
                .contains("format_not_in")
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_020_add_manga_through_the_client_returns_the_id() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10), ok(SAVE_OK)]);
        let api = AniList::open(
            fake,
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        api.login(token(), None).await?;
        let t = MangaTrack {
            id: None,
            manga_id: 1,
            tracker_id: TrackerId::ANILIST,
            remote_id: 30001,
            library_id: None,
            title: String::new(),
            last_chapter_read: 0.0,
            total_chapters: 0,
            status: MangaStatus::PlanToRead.to_db(),
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        assert_eq!(api.add_manga(&t).await?, 9001);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_021_unknown_local_status_is_an_error_before_any_request() -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_10)]);
        let api = AniList::open(
            fake.clone(),
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        api.login(token(), None).await?;
        let t = AnimeTrack {
            id: None,
            anime_id: 1,
            tracker_id: TrackerId::ANILIST,
            remote_id: 1,
            library_id: Some(3),
            title: String::new(),
            last_episode_seen: 0.0,
            total_episodes: 0,
            status: 6,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        let r = api.save_anime(&t).await;
        assert!(matches!(r, Err(AniListError::UnknownStatus(_))));
        assert_eq!(fake.calls(), 1, "only the login viewer call");
        Ok(())
    }
    #[tokio::test]
    async fn t_27b_040_login_and_logout_each_send_login_changed_and_publish_the_state() -> TestResult
    {
        let fake = FakeTransport::new(vec![ok(VIEWER_10)]);
        let bus = EventBus::default();
        let mut events = bus.subscribe();
        let api = AniList::open(fake, MemSecrets::new(), &PrefStore::in_memory(), bus)?;
        let mut state = Tracker::login_state(&api);
        assert!(!*state.borrow_and_update());
        api.login(token(), None).await?;
        let changed = TrackEvent::LoginChanged {
            tracker_id: TrackerId::ANILIST,
        };
        assert_eq!(events.recv().await?, changed);
        assert!(
            state.has_changed()? && *state.borrow_and_update(),
            "settings sees the login"
        );
        api.logout()?;
        assert_eq!(events.recv().await?, changed);
        assert!(
            state.has_changed()? && !*state.borrow_and_update(),
            "settings sees the logout"
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_27b_028_logout_clears_the_expired_flag_and_the_memory_state() -> TestResult {
        let secrets = MemSecrets::new();
        seed(&secrets, "stale", 77, 1)?;
        let api = AniList::open(
            FakeTransport::new(vec![]),
            secrets.clone(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?;
        assert!(api.base().auth_expired());
        api.logout()?;
        assert!(!api.base().auth_expired());
        assert_eq!(api.api.state(), AuthState::LoggedOut);
        assert!(secrets.all_values().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn t_27a_044_the_tracker_trait_logs_in_with_the_token_and_reports_it_through_the_manager_views()
    -> TestResult {
        let fake = FakeTransport::new(vec![ok(VIEWER_3), ok(SEARCH_ANIME)]);
        let api: Arc<dyn Tracker> = Arc::new(AniList::open(
            fake,
            MemSecrets::new(),
            &PrefStore::in_memory(),
            EventBus::default(),
        )?);
        assert!(!api.is_logged_in());
        api.login("", "token-value").await?;
        assert!(api.is_logged_in());
        assert_eq!(api.username(), "4242");
        assert_eq!(api.password(), "token-value");
        assert_eq!(api.id(), TrackerId::ANILIST);
        assert_eq!(
            api.score_list(),
            ["-", "😦", "😐", "😊"],
            "the list follows the account's score format"
        );
        let anime = api.as_anime().ok_or("anime view")?;
        assert_eq!(anime.status_list().len(), 6);
        assert_eq!(
            anime.status_label(anime.watching_status()),
            Some("watching")
        );
        assert_eq!(anime.search("show").await?.len(), 2);
        assert!(api.as_manga().is_some());
        api.logout()?;
        assert!(!api.is_logged_in());
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_013_a_legacy_integer_score_logs_out_once_and_drops_the_key() -> TestResult {
        let secrets = MemSecrets::new();
        seed(&secrets, "tok", 5, 4_000_000_000)?;
        let prefs = PrefStore::in_memory();
        prefs.set_raw("anilist_score_type", PrefValue::Int(10));
        let api = AniList::open(
            FakeTransport::new(vec![]),
            secrets.clone(),
            &prefs,
            EventBus::default(),
        )?;
        assert!(api.relogin_required());
        assert!(!api.is_logged_in());
        assert!(
            prefs.get_raw("anilist_score_type").is_none(),
            "the key is removed"
        );
        assert_eq!(secrets.raw("tracker/2/oauth"), None);
        let again = AniList::open(
            FakeTransport::new(vec![]),
            MemSecrets::new(),
            &prefs,
            EventBus::default(),
        )?;
        assert!(!again.relogin_required(), "the logout happens once");
        Ok(())
    }

    #[test]
    fn t_28a_013_a_string_score_does_not_trigger_the_relogin() -> TestResult {
        let prefs = PrefStore::in_memory();
        prefs.set_raw("anilist_score_type", PrefValue::Str("POINT_5".into()));
        let api = AniList::open(
            FakeTransport::new(vec![]),
            MemSecrets::new(),
            &prefs,
            EventBus::default(),
        )?;
        assert!(!api.relogin_required());
        Ok(())
    }

    #[test]
    fn t_28a_022_the_tracker_exposes_the_remote_delete_for_both_kinds() {
        let api = AniList::from_parts(
            FakeTransport::new(vec![]),
            MemSecrets::new(),
            &PrefStore::in_memory(),
            AuthState::LoggedOut,
        );
        assert!(api.as_deletable_anime().is_some());
        assert!(api.as_deletable_manga().is_some());
    }

    #[tokio::test]
    async fn t_28a_022_delete_entry_reports_whether_anything_was_deleted() -> TestResult {
        let fake = FakeTransport::new(vec![ok(
            r#"{"data":{"DeleteMediaListEntry":{"deleted":true}}}"#,
        )]);
        let api = AniList::from_parts(
            fake.clone(),
            MemSecrets::new(),
            &PrefStore::in_memory(),
            logged_in(),
        );
        assert!(api.delete_entry(9001).await?);
        assert_eq!(
            fake.bodies()[0]["variables"]["listId"],
            serde_json::json!(9001)
        );
        Ok(())
    }

    fn logged_in() -> AuthState {
        AuthState::LoggedIn {
            token: token(),
            viewer_id: 4242,
            expires_at: i64::MAX,
            score_format: ScoreFormat::Point10,
        }
    }
}
