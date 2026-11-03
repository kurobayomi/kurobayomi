//! Jellyfin HTTP calls: the item read, the episode walk and the played marks
//! (T-33a-012, T-33a-014, T-33a-016, T-33a-021, T-33a-032).

use chrono::Utc;
use futures::{StreamExt, TryStreamExt};
use reqwest::header::USER_AGENT;

use super::auth::JellyfinAuth;
use super::dto::{JFItem, JFItemList};
use super::progress::{played_date, season_progress, select_marks};
use super::url::{TrackingKind, TrackingUrl};
use super::{COMPLETED, UNSEEN};
use crate::error::redact;
use crate::model::search::AnimeTrackSearch;
use crate::model::{AnimeTrack, TrackerId};
use crate::trackers::enhanced::EnhancedError;
use crate::trackers::http::USER_AGENT as APP_USER_AGENT;

/// Mark-range switch. `true` is the D-11 range rule, `false` is the Kotlin rule.
pub const JELLYFIN_MARK_RANGE: bool = true;
/// Concurrent played POSTs (`buffer_unordered`).
pub const JELLYFIN_MARK_CONCURRENCY: usize = 4;
/// The auth header. The key stays out of URLs and logs (P-8).
const TOKEN_HEADER: &str = "X-Emby-Token";

/// Episodes per request (`Limit`, T-33a-106, E-88).
pub const EPISODE_PAGE: usize = 200;

pub struct JellyfinApi {
    http: reqwest::Client,
    auth: JellyfinAuth,
}

impl JellyfinApi {
    pub fn new(http: reqwest::Client, auth: JellyfinAuth) -> Self {
        Self { http, auth }
    }

    /// The item read (T-33a-012). A movie is one item. A season also runs the episode walk.
    pub async fn get_track_search(
        &self,
        track_url: &str,
    ) -> Result<AnimeTrackSearch, EnhancedError> {
        let tracking = TrackingUrl::parse(track_url)?;
        let key = self.auth.api_key_for(&tracking.user_id)?;
        let item = self.get_item(&tracking, &key, track_url).await?;
        let mut result =
            AnimeTrackSearch::new(TrackerId::JELLYFIN, 0, item.name.clone(), track_url);
        result.total_episodes = 1;
        match &tracking.kind {
            TrackingKind::Movie => {
                let played = item.user_data.played;
                result.last_episode_seen = if played { 1.0 } else { 0.0 };
                result.status = if played { COMPLETED } else { UNSEEN };
            }
            TrackingKind::Season { series_id } => {
                let items = self.episodes(&tracking, series_id, &key).await?;
                let progress = season_progress(&items)?;
                result.total_episodes = progress.total;
                result.last_episode_seen = progress.last_seen;
                result.status = progress.status;
            }
        }
        Ok(result)
    }

    /// Marks the episodes up to `t.last_episode_seen` as played, then returns the track with the server's progress
    /// (status is server-driven, P-1). A movie is marked only at 1.0 or more (D-32). No matching episode is `Ok` with
    /// no POST (D-12).
    pub async fn update_progress(&self, t: &AnimeTrack) -> Result<AnimeTrack, EnhancedError> {
        let tracking = TrackingUrl::parse(&t.remote_url)?;
        let key = self.auth.api_key_for(&tracking.user_id)?;
        let ids: Vec<String> = match &tracking.kind {
            TrackingKind::Movie => {
                if t.last_episode_seen >= 1.0 {
                    vec![tracking.item_id.clone()]
                } else {
                    Vec::new()
                }
            }
            TrackingKind::Season { series_id } => {
                let items = self.episodes(&tracking, series_id, &key).await?;
                let ids = select_marks(&items, t.last_episode_seen, JELLYFIN_MARK_RANGE);
                if ids.is_empty() {
                    tracing::info!("no episode matches progress {}", t.last_episode_seen);
                }
                ids
            }
        };
        futures::stream::iter(ids)
            .map(|id| {
                let (tracking, key) = (&tracking, key.as_str());
                async move { self.mark_played(tracking, key, &id).await }
            })
            .buffer_unordered(JELLYFIN_MARK_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;
        let remote = self.get_track_search(&t.remote_url).await?;
        Ok(apply_remote_progress(t, &remote))
    }

    async fn get_item(
        &self,
        tracking: &TrackingUrl,
        key: &str,
        track_url: &str,
    ) -> Result<JFItem, EnhancedError> {
        let url = tracking.item_url()?;
        let resp = self
            .send(self.authed(self.http.get(url), key))
            .await
            .inspect_err(|_| tracing::warn!("Could not get item: {}", redact(track_url)))?;
        resp.json::<JFItem>().await.map_err(decode_error)
    }

    /// The episodes of a season, sorted by the server (D-14). The season's `seasonId` is the item id. Pages of
    /// [`EPISODE_PAGE`] items with `StartIndex` until a short page (E-88).
    async fn episodes(
        &self,
        tracking: &TrackingUrl,
        series_id: &str,
        key: &str,
    ) -> Result<Vec<JFItem>, EnhancedError> {
        let mut all = Vec::new();
        loop {
            let mut url = tracking.child(&["Shows", series_id, "Episodes"])?;
            url.query_pairs_mut()
                .append_pair("seasonId", &tracking.item_id)
                .append_pair("userId", &tracking.user_id)
                .append_pair("Fields", "Overview,MediaSources")
                .append_pair("SortBy", "IndexNumber")
                .append_pair("SortOrder", "Ascending")
                .append_pair("StartIndex", &all.len().to_string())
                .append_pair("Limit", &EPISODE_PAGE.to_string());
            let resp = self.send(self.authed(self.http.get(url), key)).await?;
            let list: JFItemList = resp.json().await.map_err(decode_error)?;
            let got = list.items.len();
            all.extend(list.items);
            if got < EPISODE_PAGE {
                return Ok(all);
            }
        }
    }

    /// `POST {server}/Users/{u}/PlayedItems/{id}?DatePlayed=…` with the key in the header.
    async fn mark_played(
        &self,
        tracking: &TrackingUrl,
        key: &str,
        item_id: &str,
    ) -> Result<(), EnhancedError> {
        let mut url =
            tracking.child(&["Users", tracking.user_id.as_str(), "PlayedItems", item_id])?;
        url.query_pairs_mut()
            .append_pair("DatePlayed", &played_date(Utc::now()));
        self.send(self.authed(self.http.post(url), key)).await?;
        Ok(())
    }

    fn authed(&self, req: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
        req.header(USER_AGENT, APP_USER_AGENT)
            .header(TOKEN_HEADER, key)
    }

    /// Sends the request. 401 is `Unauthorized`, other non-2xx is `Http`.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response, EnhancedError> {
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(EnhancedError::Unauthorized);
        }
        Ok(resp.error_for_status()?)
    }
}

fn decode_error(e: reqwest::Error) -> EnhancedError {
    EnhancedError::Decode(redact(&e.to_string()))
}

/// The track after a sync: progress and status come from the server, the personal fields stay (P-1, E-92).
pub fn apply_remote_progress(t: &AnimeTrack, remote: &AnimeTrackSearch) -> AnimeTrack {
    let mut out = t.clone();
    out.last_episode_seen = remote.last_episode_seen;
    out.total_episodes = remote.total_episodes;
    out.status = remote.status;
    out
}
