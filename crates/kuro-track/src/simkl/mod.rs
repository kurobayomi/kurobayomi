//! Simkl behind the tracker traits (T-32a-030 to T-32a-046). The identity, the
//! status table, the browser login, the token store, logout and the anime operations (search, bind, update, refresh).
//!
//! Credentials follow the shared layout of [`BaseTracker`]: the username is the user id as a decimal string, the
//! password is the access token, and the token JSON is kept in its own secret (B-3.7, B-3.11).

pub mod api;
pub mod auth;
pub mod dto;
pub mod error;
pub mod login;
pub mod oauth;
pub mod search;
pub mod status;
pub mod sync;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use kuro_platform::secrets::SecretStore;
use tokio::sync::watch;

use crate::base::BaseTracker;
use crate::error::TrackError;
use crate::events::EventBus;
use crate::keyed_lock::KeyedLocks;
use crate::model::search::AnimeTrackSearch;
use crate::model::{AnimeTrack, TrackerId, copy_personal_from_anime};
use crate::tracker::{AnimeTracker, LogoRef, Tracker};

use api::{SearchKind, SimklApi};
use auth::SimklAuth;
use dto::SimklOAuth;
use error::SimklError;

pub const TRACKER_ID: TrackerId = TrackerId::SIMKL;
pub const NAME: &str = "Simkl";
/// Brand colour `Color.rgb(0, 0, 0)`, `0xAARRGGBB`.
pub const LOGO_COLOR: u32 = 0xFF00_0000;

/// The score labels `"0"` to `"10"` (`Simkl.kt:29-31`).
fn score_labels() -> Vec<String> {
    (0..=10).map(|n| n.to_string()).collect()
}

pub struct SimklTracker {
    base: BaseTracker,
    auth: Arc<SimklAuth>,
    /// Shared with the write tasks that run on their own (T-32a-066).
    api: Arc<SimklApi>,
    /// One lock per title, so two writes of one title never interleave (TST-60).
    locks: Arc<KeyedLocks>,
    scores: Vec<String>,
}

impl SimklTracker {
    pub fn new(secrets: Arc<dyn SecretStore>, events: EventBus) -> Result<Self, SimklError> {
        Self::with_base_url(api::API_URL, secrets, events)
    }

    /// Same as [`SimklTracker::new`] with the API at `base_url` (tests point this at a local server).
    pub fn with_base_url(
        base_url: &str,
        secrets: Arc<dyn SecretStore>,
        events: EventBus,
    ) -> Result<Self, SimklError> {
        let base = BaseTracker::new(TRACKER_ID, NAME, secrets, events);
        // A missing, empty, `null` or malformed token is no token (B-3.11, T-34).
        let restored = base.load_token::<SimklOAuth>().ok().flatten();
        let auth = Arc::new(SimklAuth::new(restored));
        Ok(Self {
            api: Arc::new(SimklApi::new(base_url, Arc::clone(&auth))?),
            auth,
            base,
            locks: Arc::new(KeyedLocks::default()),
            scores: score_labels(),
        })
    }

    /// Marks the login expired when the server refuses an authenticated call (401 or 403, U-9, T-32a-060). The error is
    /// returned unchanged.
    fn expired<T>(&self, result: Result<T, SimklError>) -> Result<T, TrackError> {
        if let Err(SimklError::Http {
            status: 401 | 403, ..
        }) = &result
        {
            let _ = self.base.mark_expired();
        }
        result.map_err(TrackError::from)
    }

    /// Completes a browser login from its one-time code (B-3.7). Any failure clears every credential and returns the
    /// error; the caller decides whether to show it.
    pub async fn login_code(&self, code: &str) -> Result<(), TrackError> {
        match self.complete_login(code).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = self.logout();
                Err(e)
            }
        }
    }

    /// The token is stored before the user lookup (B-3.7 step b), and the username and password after it.
    async fn complete_login(&self, code: &str) -> Result<(), TrackError> {
        let token = self.api.access_token(code).await?;
        self.auth.set(Some(token.clone()));
        self.base.save_token(&token)?;
        let uid = self.api.current_user_id().await?;
        self.base
            .save_credentials(&uid.to_string(), &token.access_token)?;
        Ok(())
    }

    /// Routes a launch argument (startup or single-instance activation) to [`SimklTracker::handle_redirect`].
    /// `None` when `uri` is not the Simkl redirect, so the caller can offer it to the next handler (R-6.3).
    pub async fn handle_launch_uri(&self, uri: &str) -> Option<Result<(), TrackError>> {
        if !oauth::is_simkl_redirect(uri) {
            return None;
        }
        Some(self.handle_redirect(uri).await)
    }

    /// Handles the redirect `aniyomi://simkl-auth?code=...`. A redirect without a code logs out and reports nothing
    /// else (TrackLoginActivity.kt:79-82, TST-57).
    pub async fn handle_redirect(&self, redirect: &str) -> Result<(), TrackError> {
        match oauth::code_from_redirect(redirect) {
            Some(code) => self.login_code(&code).await,
            None => self.logout(),
        }
    }
}

#[async_trait]
impl Tracker for SimklTracker {
    fn id(&self) -> TrackerId {
        TRACKER_ID
    }

    fn name(&self) -> &str {
        NAME
    }

    fn logo(&self) -> LogoRef {
        crate::logo::logo_asset(TRACKER_ID).unwrap_or(LogoRef::IconName("simkl"))
    }

    fn logo_color(&self) -> u32 {
        LOGO_COLOR
    }

    fn completion_status(&self) -> i64 {
        status::COMPLETED
    }

    fn score_list(&self) -> &[String] {
        &self.scores
    }

    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        Some(self)
    }

    /// The second argument is the OAuth code (`login(password)`, Simkl.kt:120). The settings row never shows a
    /// credential dialog for Simkl, so this is reached by programmatic calls only.
    async fn login(&self, _username: &str, password: &str) -> Result<(), TrackError> {
        self.login_code(password).await
    }

    /// Clears the in-memory token first, then every stored credential and the token secret, once (fix D-SK-4).
    fn logout(&self) -> Result<(), TrackError> {
        self.auth.set(None);
        self.base.logout()
    }

    fn is_logged_in(&self) -> bool {
        self.base.is_logged_in()
    }

    fn auth_expired(&self) -> bool {
        self.base.auth_expired()
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.base.login_state()
    }

    /// The password is the access token, so the in-memory token follows it.
    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError> {
        let token = (!password.is_empty()).then(|| SimklOAuth {
            access_token: password.to_owned(),
        });
        self.auth.set(token);
        self.base.save_credentials(username, password)
    }

    fn username(&self) -> String {
        self.base.username()
    }

    fn password(&self) -> String {
        self.base.password()
    }
}

#[async_trait]
impl AnimeTracker for SimklTracker {
    fn status_list(&self) -> Vec<i64> {
        status::STATUS_ORDER.to_vec()
    }

    fn watching_status(&self) -> i64 {
        status::WATCHING
    }

    fn rewatching_status(&self) -> i64 {
        status::REWATCHING
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        status::label_key(status)
    }

    /// Truncated, as `toInt()` (B-3.2).
    fn display_score(&self, t: &AnimeTrack) -> String {
        (t.score.trunc() as i64).to_string()
    }

    /// Completed titles keep their status. A watched episode moves the rest to Completed when the last episode is
    /// reached, or to Watching otherwise. A total of zero never completes (B-3.3).
    async fn update(&self, t: &AnimeTrack, did_watch: bool) -> Result<AnimeTrack, TrackError> {
        let mut t = t.clone();
        if t.status != status::COMPLETED && did_watch {
            t.status = if t.total_episodes > 0 && t.last_episode_seen as i64 == t.total_episodes {
                status::COMPLETED
            } else {
                status::WATCHING
            };
        }
        let api = Arc::clone(&self.api);
        let pushed = t.clone();
        let outcome = self
            .locks
            .run(t.remote_id, async move { sync::push(&api, &pushed).await })
            .await
            .map_err(|e| SimklError::Transport(e.to_string()))
            .and_then(|r| r);
        self.expired(outcome)?;
        Ok(t)
    }

    /// Links the track to the user's list (B-3.4). A title already listed keeps its remote progress and rating and
    /// is then pushed; a title that is not listed is added.
    async fn bind(&self, t: &mut AnimeTrack, has_seen: bool) -> Result<(), TrackError> {
        let found = self.expired(sync::find_lib_anime(&self.api, t).await)?;
        match found {
            Some(remote) => {
                copy_personal_from_anime(t, &remote, true);
                // A re-bind keeps the local library id when the remote has none (fix D-SK-3).
                if remote.library_id.is_some() {
                    t.library_id = remote.library_id;
                }
                if t.status != status::COMPLETED && has_seen {
                    t.status = status::WATCHING;
                }
                *t = self.update(t, false).await?;
                Ok(())
            }
            None => {
                t.status = if has_seen {
                    status::WATCHING
                } else {
                    status::PLAN_TO_WATCH
                };
                t.score = 0.0;
                self.expired(sync::add(&self.api, t).await)
            }
        }
    }

    /// The three kinds, one after another, concatenated with no sort or de-duplication. One failure fails the whole
    /// search (B-3.14).
    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        let mut out = Vec::new();
        for kind in [SearchKind::Anime, SearchKind::Tv, SearchKind::Movie] {
            for r in self.api.search(kind, query).await? {
                out.push(search::to_track_search(&r, kind));
            }
        }
        Ok(out)
    }

    /// Copies the personal fields and the episode count of the listed title. A title that is not listed comes back
    /// unchanged (B-3.15).
    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        let mut t = t.clone();
        if let Some(remote) = self.expired(sync::find_lib_anime(&self.api, &t).await)? {
            copy_personal_from_anime(&mut t, &remote, true);
            t.total_episodes = remote.total_episodes;
        }
        Ok(t)
    }
}
