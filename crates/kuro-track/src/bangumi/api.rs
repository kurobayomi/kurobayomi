//! Bangumi REST calls (T-31a-026 to T-31a-030). Every authenticated call goes through [`BangumiApi::send_raw`], which
//! adds the bearer token, the User-Agent and `Cache-Control: no-cache`, and refreshes an expired token first. The code
//! exchange and the refresh are sent without the bearer. Persistence goes through [`BaseTracker`].

use kuro_platform::kind::Kind;
use std::sync::Arc;

use kuro_platform::secrets::SecretStore;
use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE, USER_AGENT};
use reqwest::{Method, Response, StatusCode};
use serde::Serialize;
use url::Url;

use super::auth::{TokenSlot, is_expired};
use super::dto::{
    BgmCollection, BgmEpisode, BgmEpisodeList, BgmOAuth, BgmSearchResult, BgmSubject, BgmUser,
    now_secs,
};
use super::error::BangumiError;
use super::status::BgmStatus;
use crate::base::BaseTracker;
use crate::events::EventBus;
use crate::model::TrackerId;
use crate::shikimori::api::{OAuthClient, http_client};
use crate::shikimori::dto::ensure_finite;

/// The content type of Bangumi bodies (`APP_JSON`, `BangumiApi.kt:277`).
const JSON_MIME: &str = "application/json";
/// Search returns the first 20 results (T-31a-028, `limit=20`).
const SEARCH_LIMIT: &str = "20";
/// The server's largest page of episodes (`limit` max 1000). Keeps a long series in one page.
const EPISODE_PAGE: &str = "1000";

/// Where the calls go. `api` is `https://api.bgm.tv` and `web` is `https://bgm.tv` in production (the code exchange and
/// the authorise page are on `web`). Tests point both at a local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub api: String,
    pub web: String,
}

impl Endpoints {
    pub fn production() -> Self {
        Self {
            api: "https://api.bgm.tv".to_owned(),
            web: "https://bgm.tv".to_owned(),
        }
    }
}

/// One library row for a subject (`BgmLibEntry`). The subject id stays the track's
/// `remote_id`. Bangumi has no separate library id.
#[derive(Debug, Clone, PartialEq)]
pub struct BgmLibEntry {
    pub subject_id: i64,
    pub status: BgmStatus,
    /// The rating, 0 when the user has none.
    pub score: f64,
    /// Episodes watched, or chapters read for a manga.
    pub progress: f64,
    /// Episodes or chapters in the subject, 0 when unknown.
    pub total: i64,
}

/// One collection write, add or update. Both send the same body (`BangumiApi.kt:45-52`).
#[derive(Debug, Clone, PartialEq)]
pub struct CollectionWrite {
    pub status: BgmStatus,
    pub score: f64,
    pub progress: f64,
    pub private: bool,
}

/// Body of a collection write. The key order is the source's.
#[derive(Serialize)]
struct CollectionBody {
    #[serde(rename = "type")]
    kind: i64,
    rate: i64,
    ep_status: i64,
    private: bool,
}

impl From<&CollectionWrite> for CollectionBody {
    fn from(w: &CollectionWrite) -> Self {
        Self {
            kind: w.status.collection_type(),
            // `toInt().coerceIn(0, 10)`. A -1 becomes 0, which the server reads as "no rating" (EC-B-19).
            rate: (w.score as i64).clamp(0, 10),
            ep_status: w.progress as i64,
            private: w.private,
        }
    }
}

#[derive(Serialize)]
struct SearchFilter {
    #[serde(rename = "type")]
    kinds: [i64; 1],
}

/// Body of an episode write (`type` 2 is done, `EpisodeCollectionType`). The key order is the source's.
#[derive(Serialize)]
struct EpisodeBody<'a> {
    episode_id: &'a [i64],
    #[serde(rename = "type")]
    kind: i64,
}

/// Body of a subject search. The key order is the source's.
#[derive(Serialize)]
struct SearchBody<'a> {
    keyword: &'a str,
    sort: &'static str,
    filter: SearchFilter,
}

/// The subject type a search filters on: 1 for books (manga), 2 for anime (`BangumiApi.kt:136`, `165`).
fn subject_type(kind: Kind) -> i64 {
    match kind {
        Kind::Manga => 1,
        Kind::Anime => 2,
    }
}

/// Non-2xx becomes `Http(code)`. A 2xx body is returned as text.
async fn read_success(response: Response) -> Result<String, BangumiError> {
    let status = response.status();
    if !status.is_success() {
        return Err(BangumiError::Http(status.as_u16()));
    }
    Ok(response.text().await?)
}

pub struct BangumiApi {
    http: reqwest::Client,
    endpoints: Endpoints,
    client: Option<OAuthClient>,
    redirect_uri: String,
    base: BaseTracker,
    tokens: TokenSlot,
}

impl BangumiApi {
    /// Builds the client and restores the stored token. A token that does not decode is no token (`restoreToken`).
    pub fn new(
        endpoints: Endpoints,
        client: Option<OAuthClient>,
        redirect_uri: &str,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, BangumiError> {
        let base = BaseTracker::new(TrackerId::BANGUMI, "Bangumi", secrets, EventBus::default());
        let stored = base.load_token::<BgmOAuth>().ok().flatten();
        Ok(Self {
            http: http_client()?,
            endpoints,
            client,
            redirect_uri: redirect_uri.to_owned(),
            base,
            tokens: TokenSlot::with_token(stored),
        })
    }

    pub fn base(&self) -> &BaseTracker {
        &self.base
    }

    pub fn tokens(&self) -> &TokenSlot {
        &self.tokens
    }

    /// True when the user has configured the OAuth client. Without it no login can start (D-OPEN-1).
    pub fn has_client(&self) -> bool {
        self.client.is_some()
    }

    fn client_config(&self) -> Result<&OAuthClient, BangumiError> {
        self.client.as_ref().ok_or(BangumiError::NotConfigured)
    }

    /// `base` with the path segments appended. Each segment is percent-encoded, so a login name with special
    /// characters stays one segment (EC-B-31).
    fn url(base: &str, segments: &[&str]) -> Result<Url, BangumiError> {
        let mut url = Url::parse(base).map_err(|e| BangumiError::Transport(e.to_string()))?;
        url.path_segments_mut()
            .map_err(|()| BangumiError::Transport("base URL cannot carry a path".into()))?
            .extend(segments);
        Ok(url)
    }

    fn token_url(&self) -> String {
        format!("{}/oauth/access_token", self.endpoints.web)
    }

    /// The authorise URL (`BangumiApi.authUrl`): `client_id`, `response_type`, `redirect_uri`, in that order. No scope.
    pub fn auth_url(&self) -> Result<String, BangumiError> {
        let client = self.client_config()?;
        let mut url = Url::parse(&format!("{}/oauth/authorize", self.endpoints.web))
            .map_err(|e| BangumiError::Transport(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("client_id", &client.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &self.redirect_uri);
        Ok(url.to_string())
    }

    /// The code exchange (B1). The form order is the source's. Sent without the bearer.
    pub async fn exchange_code(&self, code: &str) -> Result<BgmOAuth, BangumiError> {
        let client = self.client_config()?;
        let response = self
            .http
            .post(self.token_url())
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client.client_id.as_str()),
                ("client_secret", client.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
            ])
            .send()
            .await?;
        Ok(serde_json::from_str(&read_success(response).await?)?)
    }

    /// The refresh (B2). The form order is the source's, and `redirect_uri` is part of it. Sent without the bearer,
    /// on the same client (D16).
    async fn refresh_request(&self, refresh: &str) -> Result<BgmOAuth, BangumiError> {
        let client = self.client_config()?;
        let response = self
            .http
            .post(self.token_url())
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client.client_id.as_str()),
                ("client_secret", client.client_secret.as_str()),
                ("refresh_token", refresh),
                ("redirect_uri", self.redirect_uri.as_str()),
            ])
            .send()
            .await?;
        Ok(serde_json::from_str(&read_success(response).await?)?)
    }

    /// `GET v0/me` with the given token, before the token is stored (B3). The login name is the user name.
    pub async fn me(&self, access_token: &str) -> Result<BgmUser, BangumiError> {
        let url = Self::url(&self.endpoints.api, &["v0", "me"])?;
        let response = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .header(USER_AGENT, crate::user_agent())
            .header(CACHE_CONTROL, "no-cache")
            .send()
            .await?;
        Ok(serde_json::from_str(&read_success(response).await?)?)
    }

    /// The bearer token to send. An expired token is refreshed first, one refresh at a time (T-31a-027). A refresh the
    /// server refuses (400 or 401) flags the login as expired, and a missing refresh token on an expired token does the
    /// same without a request. Any other refresh failure keeps the token for the next call.
    pub async fn bearer(&self) -> Result<String, BangumiError> {
        let (token, _) = self
            .tokens
            .snapshot()
            .ok_or(BangumiError::NotAuthenticated)?;
        if !is_expired(&token, now_secs()) {
            return Ok(token.access_token);
        }
        let _single_flight = self.tokens.refresh_lock.lock().await;
        let (current, generation) = self
            .tokens
            .snapshot()
            .ok_or(BangumiError::NotAuthenticated)?;
        if !is_expired(&current, now_secs()) {
            return Ok(current.access_token);
        }
        let Some(refresh) = current.refresh_token.clone() else {
            return Err(self.expire());
        };
        match self.refresh_request(&refresh).await {
            Ok(fresh) => {
                let access = fresh.access_token.clone();
                // A logout during the refresh discards the result, so a logged-out user is not logged back in.
                if !self.tokens.replace_if_current(fresh.clone(), generation) {
                    return Err(BangumiError::NotAuthenticated);
                }
                if let Err(e) = self.base.save_token(&fresh) {
                    tracing::warn!("Bangumi token could not be saved after a refresh: {e}");
                }
                Ok(access)
            }
            Err(BangumiError::Http(400 | 401)) => Err(self.expire()),
            Err(other) => Err(other),
        }
    }

    /// Sets the auth-expired flag and returns the error to report (D1).
    fn expire(&self) -> BangumiError {
        if let Err(e) = self.base.set_auth_expired(true) {
            tracing::warn!("Bangumi auth-expired flag could not be saved: {e}");
        }
        BangumiError::AuthExpired
    }

    /// One authenticated call, up to the response. Nothing is sent without a token. A 401 on a token that was not
    /// expired means the server revoked it (EC-X-09).
    async fn send_raw(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
    ) -> Result<Response, BangumiError> {
        let bearer = self.bearer().await?;
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(bearer)
            .header(USER_AGENT, crate::user_agent())
            .header(CACHE_CONTROL, "no-cache");
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, JSON_MIME).body(body);
        }
        let response = request.send().await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(self.expire());
        }
        Ok(response)
    }

    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
    ) -> Result<String, BangumiError> {
        read_success(self.send_raw(method, url, body).await?).await
    }

    /// One call with no bearer, for the public endpoints (T-31a-045, D9). Sends the User-Agent and no-cache, as the
    /// authenticated calls do.
    async fn send_anonymous(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
    ) -> Result<String, BangumiError> {
        let mut request = self
            .http
            .request(method, url)
            .header(USER_AGENT, crate::user_agent())
            .header(CACHE_CONTROL, "no-cache");
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, JSON_MIME).body(body);
        }
        read_success(request.send().await?).await
    }

    /// `POST v0/search/subjects?limit=20` (B4). With a login the bearer is sent, so account filters keep working.
    /// Logged out, the request has no `Authorization` header (T-31a-045). Manga and anime differ only in the subject
    /// type filter. A non-finite rating is a decode error.
    pub async fn search(&self, kind: Kind, keyword: &str) -> Result<Vec<BgmSubject>, BangumiError> {
        let mut url = Self::url(&self.endpoints.api, &["v0", "search", "subjects"])?;
        url.query_pairs_mut().append_pair("limit", SEARCH_LIMIT);
        let body = serde_json::to_string(&SearchBody {
            keyword,
            sort: "match",
            filter: SearchFilter {
                kinds: [subject_type(kind)],
            },
        })?;
        let text = if self.tokens.snapshot().is_some() {
            self.send(Method::POST, url, Some(body)).await?
        } else {
            self.send_anonymous(Method::POST, url, Some(body)).await?
        };
        let subjects = serde_json::from_str::<BgmSearchResult>(&text)?.data;
        for s in &subjects {
            let rating = s.rating.as_ref().and_then(|r| r.score);
            ensure_finite(&[rating.unwrap_or(0.0)]).map_err(BangumiError::Decode)?;
        }
        Ok(subjects)
    }

    /// `GET v0/episodes?subject_id=<id>` (T-31a-044). Public, like the search. The episodes come in one page of up to
    /// 1000.
    pub async fn episodes(&self, subject_id: i64) -> Result<Vec<BgmEpisode>, BangumiError> {
        let mut url = Self::url(&self.endpoints.api, &["v0", "episodes"])?;
        url.query_pairs_mut()
            .append_pair("subject_id", &subject_id.to_string())
            .append_pair("limit", EPISODE_PAGE);
        let text = self.send_anonymous(Method::GET, url, None).await?;
        Ok(serde_json::from_str::<BgmEpisodeList>(&text)?.data)
    }

    /// `PATCH v0/users/-/collections/{id}/episodes` with `type` 2, done, for the given episode ids (T-31a-044). The
    /// server answers 204 with no body.
    pub async fn mark_episodes_done(
        &self,
        subject_id: i64,
        episode_ids: &[i64],
    ) -> Result<(), BangumiError> {
        let body = serde_json::to_string(&EpisodeBody {
            episode_id: episode_ids,
            kind: 2,
        })?;
        let id = subject_id.to_string();
        let url = Self::url(
            &self.endpoints.api,
            &["v0", "users", "-", "collections", id.as_str(), "episodes"],
        )?;
        self.send(Method::PATCH, url, Some(body)).await?;
        Ok(())
    }

    /// `GET v0/users/{username}/collections/{subject_id}` (B5). A 404 is `Ok(None)`: the subject is not collected.
    /// Any other status is an error. An empty user name is refused before any request (EC-B-32).
    pub async fn collection(
        &self,
        username: &str,
        subject_id: i64,
    ) -> Result<Option<BgmCollection>, BangumiError> {
        if username.is_empty() {
            return Err(BangumiError::NotAuthenticated);
        }
        let id = subject_id.to_string();
        let url = Self::url(
            &self.endpoints.api,
            &["v0", "users", username, "collections", id.as_str()],
        )?;
        let response = self.send_raw(Method::GET, url, None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&read_success(response).await?)?))
    }

    /// The collection of `subject_id` as a library row (T-31a-029). `Ok(None)` when the user has not collected it.
    /// The status is checked here, so a bad type is an error, not a stored value.
    pub async fn lookup(
        &self,
        username: &str,
        subject_id: i64,
    ) -> Result<Option<BgmLibEntry>, BangumiError> {
        let Some(c) = self.collection(username, subject_id).await? else {
            return Ok(None);
        };
        let total = c.subject.as_ref().and_then(|s| s.eps).unwrap_or(0);
        Ok(Some(BgmLibEntry {
            subject_id,
            status: BgmStatus::from_collection_type(c.kind)?,
            score: c.rate.unwrap_or(0) as f64,
            progress: c.ep_status.unwrap_or(0) as f64,
            total,
        }))
    }

    fn write_url(&self, subject_id: i64) -> Result<Url, BangumiError> {
        let id = subject_id.to_string();
        Self::url(
            &self.endpoints.api,
            &["v0", "users", "-", "collections", id.as_str()],
        )
    }

    /// `POST v0/users/-/collections/{id}` (B6). The server answers 202 with no body, which is not parsed. The
    /// library row id is not returned (D19).
    pub async fn add(&self, subject_id: i64, write: &CollectionWrite) -> Result<(), BangumiError> {
        let body = serde_json::to_string(&CollectionBody::from(write))?;
        self.send(Method::POST, self.write_url(subject_id)?, Some(body))
            .await?;
        Ok(())
    }

    /// `PATCH v0/users/-/collections/{id}` (B7). The server answers 204 with no body.
    pub async fn update(
        &self,
        subject_id: i64,
        write: &CollectionWrite,
    ) -> Result<(), BangumiError> {
        let body = serde_json::to_string(&CollectionBody::from(write))?;
        self.send(Method::PATCH, self.write_url(subject_id)?, Some(body))
            .await?;
        Ok(())
    }
}
