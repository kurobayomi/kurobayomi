//! MangaUpdates behind the tracker traits (T-32a-001 to T-32a-026 except the
//! logo asset, T-32a-025). The registry and the GUI hold it as `Arc<dyn Tracker>`.
//!
//! Credentials follow the shared layout of [`BaseTracker`]: the username is the account uid as a decimal string and
//! the password is the session token. The typed password is never stored (B-2.5).

pub mod api;
pub mod auth;
pub mod dto;
pub mod error;
pub mod login;
pub mod scores;
pub mod search;
pub mod status;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use kuro_platform::secrets::SecretStore;
use tokio::sync::watch;

use crate::base::BaseTracker;
use crate::error::TrackError;
use crate::events::EventBus;
use crate::keyed_lock::KeyedLocks;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::tracker::{DeletableMangaTracker, LogoRef, MangaTracker, Tracker};

use api::{MuApi, RatingFetch};
use auth::MuAuth;
use dto::ListItem;
use error::MuError;
use status::{COMPLETE_LIST, READING_LIST, REREADING, STATUS_ORDER, WISH_LIST};

pub const TRACKER_ID: TrackerId = TrackerId::MANGAUPDATES;
pub const NAME: &str = "MangaUpdates";

/// The last rating seen on the server, per series: `Some(None)` is "known absent", a missing key is "unknown".
type RatingMemo = Mutex<HashMap<i64, Option<f64>>>;

fn remember_in(memo: &RatingMemo, remote_id: i64, rating: Option<f64>) {
    memo.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(remote_id, rating);
}

fn remembered_in(memo: &RatingMemo, remote_id: i64) -> Option<Option<f64>> {
    memo.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&remote_id)
        .copied()
}

pub struct MangaUpdatesTracker {
    base: BaseTracker,
    auth: Arc<MuAuth>,
    /// Shared with the write tasks that run on their own (T-32a-066).
    api: Arc<MuApi>,
    /// Built once and lent out by [`Tracker::score_list`] (R-12 pitfall 1).
    scores: Vec<String>,
    /// The last rating seen on the server. It decides whether a 0.0 score needs a DELETE (fix D-MU-3). It lives in
    /// memory only.
    remote_ratings: Arc<RatingMemo>,
    /// One lock per series, so two writes of one series never interleave (TST-60).
    locks: Arc<KeyedLocks>,
}

impl MangaUpdatesTracker {
    pub fn new(secrets: Arc<dyn SecretStore>, events: EventBus) -> Result<Self, MuError> {
        Self::with_base_url(api::BASE_URL, secrets, events)
    }

    /// Same as [`MangaUpdatesTracker::new`] with the API at `base_url` (tests point this at a local server).
    pub fn with_base_url(
        base_url: &str,
        secrets: Arc<dyn SecretStore>,
        events: EventBus,
    ) -> Result<Self, MuError> {
        let base = BaseTracker::new(TRACKER_ID, NAME, secrets, events);
        // The session token is restored only with both credentials (B-2.6, read from the password slot).
        let restored = base.is_logged_in().then(|| base.password());
        let auth = Arc::new(MuAuth::new(restored));
        let api = MuApi::new(base_url, Arc::clone(&auth))?;
        Ok(Self {
            base,
            auth,
            api: Arc::new(api),
            scores: scores::labels(),
            remote_ratings: Arc::new(Mutex::new(HashMap::new())),
            locks: Arc::new(KeyedLocks::default()),
        })
    }

    /// Marks the login expired when the server refuses an authenticated call (401 or 403, T-32a-060). The error is
    /// returned unchanged.
    fn expired<T>(&self, result: Result<T, MuError>) -> Result<T, TrackError> {
        if let Err(MuError::Http {
            status: 401 | 403, ..
        }) = &result
        {
            let _ = self.base.mark_expired();
        }
        result.map_err(TrackError::from)
    }

    fn remember(&self, remote_id: i64, rating: Option<f64>) {
        remember_in(&self.remote_ratings, remote_id, rating);
    }

    /// A known rating replaces `score`. An unknown one (the fetch failed) keeps it (fix D-MU-3).
    fn apply_rating(&self, remote_id: i64, fetch: RatingFetch, score: &mut f64) {
        if let RatingFetch::Known(rating) = fetch {
            self.remember(remote_id, rating);
            *score = rating.unwrap_or(0.0);
        }
    }

    /// List update then rating (two requests, in this order, B-2.10). The pair runs on its own task under the series
    /// lock, so a dropped caller cannot stop it between the two requests (T-32a-066, TST-61).
    async fn push(&self, t: &MangaTrack) -> Result<(), MuError> {
        let api = Arc::clone(&self.api);
        let memo = Arc::clone(&self.remote_ratings);
        let t = t.clone();
        self.locks
            .run(t.remote_id, async move {
                api.update_list_item(t.remote_id, t.status, t.last_chapter_read)
                    .await?;
                sync_rating(&api, &memo, t.remote_id, t.score).await
            })
            .await
            .map_err(|e| MuError::Transport(e.to_string()))?
    }

    /// The add path of bind (B-2.9). The status and the chapter are written only on a 200 reply (fix D-MU-4 applies to
    /// the chapter: 1.0 only when the user has read chapters).
    async fn add(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), MuError> {
        let list = if has_read { READING_LIST } else { WISH_LIST };
        if self.api.add_to_list(t.remote_id, list).await? == 200 {
            t.status = list;
            if has_read {
                t.last_chapter_read = 1.0;
            }
        }
        Ok(())
    }
}

/// The rating step after a list update (B-2.10): nothing below 0, PUT for a score, and for 0.0 a DELETE only when the
/// server is known to hold a rating. An unknown server state sends no DELETE.
async fn sync_rating(
    api: &MuApi,
    memo: &RatingMemo,
    remote_id: i64,
    score: f64,
) -> Result<(), MuError> {
    if score < 0.0 {
        return Ok(());
    }
    if score != 0.0 {
        api.set_rating(remote_id, score).await?;
        remember_in(memo, remote_id, Some(score));
        return Ok(());
    }
    let known = match remembered_in(memo, remote_id) {
        Some(known) => known,
        None => match api.rating(remote_id).await {
            RatingFetch::Known(known) => known,
            RatingFetch::Unknown => return Ok(()),
        },
    };
    if known.is_some() {
        api.delete_rating(remote_id).await?;
        remember_in(memo, remote_id, None);
    }
    Ok(())
}

/// Copies status and chapter from the list item (`MUListItem.copyTo`).
fn apply_list_item(t: &mut MangaTrack, item: &ListItem) {
    t.status = item.list_id.unwrap_or(READING_LIST);
    t.last_chapter_read = item
        .status
        .as_ref()
        .and_then(|s| s.chapter)
        .map_or(0.0, |c| c as f64);
}

#[async_trait]
impl Tracker for MangaUpdatesTracker {
    fn id(&self) -> TrackerId {
        TRACKER_ID
    }

    fn name(&self) -> &str {
        NAME
    }

    fn logo(&self) -> LogoRef {
        LogoRef::IconName("mangaupdates")
    }

    fn logo_color(&self) -> u32 {
        status::LOGO_COLOR
    }

    fn completion_status(&self) -> i64 {
        COMPLETE_LIST
    }

    fn score_list(&self) -> &[String] {
        &self.scores
    }

    /// Logs in and stores the uid and the session token (B-2.5). A failure logs the tracker out, as the Kotlin
    /// dialog does, and returns the error text (`HTTP <code>` for a refused login).
    async fn login(&self, username: &str, password: &str) -> Result<(), TrackError> {
        match self.api.authenticate(username, password).await {
            Ok(ctx) => {
                self.save_credentials(&ctx.uid.to_string(), &ctx.session_token)?;
                Ok(())
            }
            Err(e) => {
                let _ = self.logout();
                Err(e.into())
            }
        }
    }

    /// Clears the in-memory token first, so the next authenticated call fails without a request (fix D-MU-1).
    fn logout(&self) -> Result<(), TrackError> {
        self.auth.set(None);
        self.remote_ratings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
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

    /// Stores the credentials and makes the password the token in memory too.
    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.auth.set(Some(password.to_owned()));
        self.base.save_credentials(username, password)
    }

    fn username(&self) -> String {
        self.base.username()
    }

    fn password(&self) -> String {
        self.base.password()
    }

    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }

    fn as_deletable_manga(&self) -> Option<&dyn DeletableMangaTracker> {
        Some(self)
    }
}

#[async_trait]
impl MangaTracker for MangaUpdatesTracker {
    fn status_list(&self) -> Vec<i64> {
        STATUS_ORDER.to_vec()
    }

    fn reading_status(&self) -> i64 {
        READING_LIST
    }

    fn rereading_status(&self) -> i64 {
        REREADING
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        status::label_key(status)
    }

    /// The trait returns no error, so an index past the table gives 0.0 here. The helper reports it as
    /// `ScoreOutOfRange`, and the picker never sends such an index.
    fn index_to_score(&self, index: usize) -> f64 {
        scores::index_to_score(&self.scores, index).unwrap_or(0.0)
    }

    fn display_score(&self, t: &MangaTrack) -> String {
        scores::display_score(t.score)
    }

    /// Completed series keep their status. Any other status becomes Reading when a chapter was read (B-2.3).
    async fn update(
        &self,
        t: &MangaTrack,
        did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        let mut t = t.clone();
        if t.status != COMPLETE_LIST && did_read_chapter {
            t.status = READING_LIST;
        }
        self.expired(self.push(&t).await)?;
        Ok(t)
    }

    async fn bind(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), TrackError> {
        let item = match self.api.list_item(t.remote_id).await {
            Ok(item) => item,
            // Only "not on the list" adds the series. Any other failure stops the bind (fix D-MU-2).
            Err(MuError::Http { status: 404, .. }) => {
                t.score = 0.0;
                return self.expired(self.add(t, has_read).await);
            }
            Err(e) => return self.expired(Err(e)),
        };
        apply_list_item(t, &item);
        let fetch = self.api.rating(t.remote_id).await;
        self.apply_rating(t.remote_id, fetch, &mut t.score);
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        let records = self.api.search(query).await?;
        Ok(records.iter().map(search::to_track_search).collect())
    }

    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let item = self.expired(self.api.list_item(t.remote_id).await)?;
        let mut t = t.clone();
        apply_list_item(&mut t, &item);
        let fetch = self.api.rating(t.remote_id).await;
        self.apply_rating(t.remote_id, fetch, &mut t.score);
        Ok(t)
    }
}

#[async_trait]
impl DeletableMangaTracker for MangaUpdatesTracker {
    /// Removes the series from the server's list. The local row is the caller's to remove.
    async fn delete(&self, t: &MangaTrack) -> Result<(), TrackError> {
        self.expired(self.api.delete_from_list(t.remote_id).await)
    }
}
