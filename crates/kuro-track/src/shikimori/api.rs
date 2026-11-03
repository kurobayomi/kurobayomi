//! Shikimori REST calls (T-31a-005 to T-31a-011). Every authenticated call goes through [`ShikimoriApi::send`], which
//! adds the bearer token, the User-Agent and `Cache-Control: no-cache`, and refreshes an expired token first. The
//! refresh and the code exchange are sent without the bearer. Persistence goes through [`BaseTracker`].

use kuro_platform::kind::Kind;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use kuro_platform::secrets::SecretStore;
use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE, USER_AGENT};
use reqwest::{Method, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use super::auth::{TokenSlot, is_expired};
use super::dto::{SmAddEntryResponse, SmEntry, SmOAuth, SmUser, SmUserListEntry, ensure_finite};
use super::error::ShikimoriError;
use super::status::ShikiStatus;
use crate::base::BaseTracker;
use crate::events::EventBus;
use crate::model::TrackerId;

const JSON_MIME: &str = "application/json; charset=utf-8";

/// The one HTTP client of the Shikimori and Bangumi calls (T-31a-040). It is built once from area 12's
/// `kuro_net::transport::client_builder` with the Aniyomi transport limits (timeouts, redirect policy, pool, rustls) and
/// an in-memory cookie store, and every tracker call clones it. Neither tracker builds a client of its own.
pub(crate) fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    static SHARED: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = SHARED.get() {
        return Ok(client.clone());
    }
    let client = kuro_net::transport::client_builder(
        &kuro_net::transport::TransportLimits::default(),
        Arc::new(reqwest::cookie::Jar::default()),
    )
    .build()?;
    Ok(SHARED.get_or_init(|| client).clone())
}

/// Where the calls go. Tests point this at a local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// `https://shikimori.io` in production. The old `shikimori.one` redirects there, and a redirect drops the bearer.
    pub base: String,
}

impl Endpoints {
    pub fn production() -> Self {
        Self {
            base: "https://shikimori.io".to_owned(),
        }
    }
}

/// An OAuth client: the user's `[track.clients]` table, or Kurobayomi's built-in Shikimori application (D-OPEN-1).
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

/// The secret never reaches a log through `Debug` (TS-X-12).
impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .finish()
    }
}

/// One library row for a catalogue record (`ShikiLibEntry`, section 3.4 of the design). `library_id` is the user-rate
/// id. The catalogue id stays the track's `remote_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct LibEntry {
    pub library_id: i64,
    pub status: ShikiStatus,
    pub score: f64,
    pub progress: f64,
    /// Catalogue chapters or episodes.
    pub total: i64,
    pub tracking_url: String,
}

/// A user-rate write. The same call adds and updates (the server treats `POST` as an upsert, T-31a-011).
#[derive(Debug, Clone, PartialEq)]
pub struct Rating {
    pub kind: Kind,
    pub user_id: String,
    pub target_id: i64,
    pub progress: f64,
    pub score: f64,
    pub status: ShikiStatus,
}

#[derive(Serialize)]
struct Payload<T> {
    user_rate: T,
}

/// Body of a manga write. Key order is the source's.
#[derive(Serialize)]
struct MangaRate<'a> {
    user_id: &'a str,
    target_id: i64,
    target_type: &'static str,
    chapters: i64,
    score: i64,
    status: &'static str,
}

/// Body of an anime write.
#[derive(Serialize)]
struct AnimeRate<'a> {
    user_id: &'a str,
    target_id: i64,
    target_type: &'static str,
    episodes: i64,
    score: i64,
    status: &'static str,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn media_path(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "mangas",
        Kind::Anime => "animes",
    }
}

fn target_type(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "Manga",
        Kind::Anime => "Anime",
    }
}

/// Non-2xx becomes `Http(code)`. A 2xx body is returned as text.
async fn read_success(response: Response) -> Result<String, ShikimoriError> {
    let status = response.status();
    if !status.is_success() {
        return Err(ShikimoriError::Http(status.as_u16()));
    }
    Ok(response.text().await?)
}

pub struct ShikimoriApi {
    http: reqwest::Client,
    endpoints: Endpoints,
    client: Option<OAuthClient>,
    redirect_uri: String,
    base: BaseTracker,
    tokens: TokenSlot,
}

impl ShikimoriApi {
    /// Builds the client and restores the stored token. A token that does not decode is no token (p1-behavior 2.5).
    pub fn new(
        endpoints: Endpoints,
        client: Option<OAuthClient>,
        redirect_uri: &str,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, ShikimoriError> {
        let base = BaseTracker::new(
            TrackerId::SHIKIMORI,
            "Shikimori",
            secrets,
            EventBus::default(),
        );
        let stored = base.load_token::<SmOAuth>().ok().flatten();
        Ok(Self {
            http: http_client()?,
            endpoints,
            client,
            redirect_uri: redirect_uri.to_owned(),
            base,
            tokens: TokenSlot::with_token(stored),
        })
    }

    /// The site root of the calls, for building absolute URLs of search results.
    pub fn base_url(&self) -> &str {
        &self.endpoints.base
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

    fn client_config(&self) -> Result<&OAuthClient, ShikimoriError> {
        self.client.as_ref().ok_or(ShikimoriError::NotConfigured)
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api/{path}", self.endpoints.base)
    }

    fn token_url(&self) -> String {
        format!("{}/oauth/token", self.endpoints.base)
    }

    /// The authorise URL (`ShikimoriApi.authUrl`): `client_id`, `redirect_uri`, `response_type=code`, in that order.
    pub fn auth_url(&self) -> Result<String, ShikimoriError> {
        let client = self.client_config()?;
        let mut url = Url::parse(&format!("{}/oauth/authorize", self.endpoints.base))
            .map_err(|e| ShikimoriError::Transport(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("response_type", "code");
        Ok(url.to_string())
    }

    /// The code exchange. The form order is the source's. Sent without the bearer.
    pub async fn exchange_code(&self, code: &str) -> Result<SmOAuth, ShikimoriError> {
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

    /// `POST oauth/token` with `refresh_token`. The form order is the source's. Sent without the bearer (D16).
    async fn refresh_request(&self, refresh: &str) -> Result<SmOAuth, ShikimoriError> {
        let client = self.client_config()?;
        let response = self
            .http
            .post(self.token_url())
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client.client_id.as_str()),
                ("client_secret", client.client_secret.as_str()),
                ("refresh_token", refresh),
            ])
            .send()
            .await?;
        Ok(serde_json::from_str(&read_success(response).await?)?)
    }

    /// `GET users/whoami` with the given token, before the token is stored (T-31a-006).
    pub async fn who_am_i(&self, access_token: &str) -> Result<SmUser, ShikimoriError> {
        let response = self
            .http
            .get(self.api_url("users/whoami"))
            .bearer_auth(access_token)
            .header(USER_AGENT, crate::user_agent())
            .header(CACHE_CONTROL, "no-cache")
            .send()
            .await?;
        Ok(serde_json::from_str(&read_success(response).await?)?)
    }

    /// The bearer token to send. An expired token is refreshed first, one refresh at a time (T-31a-005). A refresh the
    /// server refuses (400 or 401) flags the login as expired. A missing refresh token on an expired token does the
    /// same, without a request. Any other refresh failure keeps the token for the next call.
    pub async fn bearer(&self) -> Result<String, ShikimoriError> {
        let (token, _) = self
            .tokens
            .snapshot()
            .ok_or(ShikimoriError::NotAuthenticated)?;
        if !is_expired(&token, now_secs()) {
            return Ok(token.access_token);
        }
        let _single_flight = self.tokens.refresh_lock.lock().await;
        let (current, generation) = self
            .tokens
            .snapshot()
            .ok_or(ShikimoriError::NotAuthenticated)?;
        if !is_expired(&current, now_secs()) {
            return Ok(current.access_token);
        }
        let Some(refresh) = current.refresh_token.clone() else {
            return Err(self.expire());
        };
        match self.refresh_request(&refresh).await {
            Ok(fresh) => {
                let access = fresh.access_token.clone();
                if !self.tokens.replace_if_current(fresh.clone(), generation) {
                    return Err(ShikimoriError::NotAuthenticated);
                }
                // Only the token is written here. Writing the credentials too could log a logged-out user back in.
                if let Err(e) = self.base.save_token(&fresh) {
                    tracing::warn!("Shikimori token could not be saved after a refresh: {e}");
                }
                Ok(access)
            }
            Err(ShikimoriError::Http(400 | 401)) => Err(self.expire()),
            Err(other) => Err(other),
        }
    }

    /// Sets the auth-expired flag and returns the error to report (D1).
    fn expire(&self) -> ShikimoriError {
        if let Err(e) = self.base.set_auth_expired(true) {
            tracing::warn!("Shikimori auth-expired flag could not be saved: {e}");
        }
        ShikimoriError::AuthExpired
    }

    /// One authenticated call. Nothing is sent without a token (T-31a-008).
    async fn send(
        &self,
        method: Method,
        url: String,
        body: Option<String>,
    ) -> Result<String, ShikimoriError> {
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
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(self.expire());
        }
        read_success(response).await
    }

    async fn get<T: DeserializeOwned>(&self, url: String) -> Result<T, ShikimoriError> {
        let text = self.send(Method::GET, url, None).await?;
        Ok(serde_json::from_str(&text)?)
    }

    /// `GET mangas/{id}` or `animes/{id}`. A 404 is `Http(404)`.
    pub async fn catalogue_entry(&self, kind: Kind, id: i64) -> Result<SmEntry, ShikimoriError> {
        self.get(self.api_url(&format!("{}/{id}", media_path(kind))))
            .await
    }

    /// A GET with no bearer, for the public catalogue (T-31a-045, D9). Still sends the User-Agent and no-cache.
    async fn get_anonymous(&self, url: String) -> Result<String, ShikimoriError> {
        let response = self
            .http
            .get(url)
            .header(USER_AGENT, crate::user_agent())
            .header(CACHE_CONTROL, "no-cache")
            .send()
            .await?;
        read_success(response).await
    }

    /// Catalogue search, first 20 by popularity (T-31a-009). With a login the bearer is sent, so account filters keep
    /// working. Logged out, the request has no `Authorization` header (T-31a-045).
    pub async fn search(&self, kind: Kind, query: &str) -> Result<Vec<SmEntry>, ShikimoriError> {
        let mut url = Url::parse(&self.api_url(media_path(kind)))
            .map_err(|e| ShikimoriError::Transport(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("order", "popularity")
            .append_pair("search", query)
            .append_pair("limit", "20");
        let text = if self.tokens.snapshot().is_some() {
            self.send(Method::GET, url.to_string(), None).await?
        } else {
            self.get_anonymous(url.to_string()).await?
        };
        Ok(serde_json::from_str(&text)?)
    }

    /// The two-step lookup (T-31a-010): the catalogue record, then the user's rows for it. `Ok(None)` when the user
    /// has no row. Two rows are an error. A row with a catalogue record that has no count is a decode error.
    pub async fn find_library(
        &self,
        kind: Kind,
        target_id: i64,
        user_id: &str,
    ) -> Result<Option<LibEntry>, ShikimoriError> {
        let catalogue = self.catalogue_entry(kind, target_id).await?;
        let mut url = Url::parse(&self.api_url("v2/user_rates"))
            .map_err(|e| ShikimoriError::Transport(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("user_id", user_id)
            .append_pair("target_id", &target_id.to_string())
            .append_pair("target_type", target_type(kind));
        let rows: Vec<SmUserListEntry> = self.get(url.to_string()).await?;
        if rows.len() > 1 {
            return Err(ShikimoriError::TooManyEntries);
        }
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        ensure_finite(&[row.chapters, row.episodes, catalogue.score])
            .map_err(ShikimoriError::Decode)?;
        let total = match kind {
            Kind::Manga => catalogue.chapters,
            Kind::Anime => catalogue.episodes,
        }
        .ok_or_else(|| ShikimoriError::Decode("catalogue entry has no count".into()))?;
        Ok(Some(LibEntry {
            library_id: row.id,
            status: ShikiStatus::from_api(&row.status)?,
            score: row.score as f64,
            progress: match kind {
                Kind::Manga => row.chapters,
                Kind::Anime => row.episodes,
            },
            total,
            tracking_url: format!("{}{}", self.endpoints.base, catalogue.url),
        }))
    }

    /// `POST v2/user_rates` (T-31a-011). Progress and score are truncated, and the score is clamped to 0..=10 (D8).
    /// Returns the user-rate id.
    pub async fn upsert(&self, rating: &Rating) -> Result<i64, ShikimoriError> {
        let score = rating.score.clamp(0.0, 10.0) as i64;
        let progress = rating.progress as i64;
        let status = rating.status.api_str();
        let body = match rating.kind {
            Kind::Manga => serde_json::to_string(&Payload {
                user_rate: MangaRate {
                    user_id: &rating.user_id,
                    target_id: rating.target_id,
                    target_type: target_type(Kind::Manga),
                    chapters: progress,
                    score,
                    status,
                },
            })?,
            Kind::Anime => serde_json::to_string(&Payload {
                user_rate: AnimeRate {
                    user_id: &rating.user_id,
                    target_id: rating.target_id,
                    target_type: target_type(Kind::Anime),
                    episodes: progress,
                    score,
                    status,
                },
            })?,
        };
        let text = self
            .send(Method::POST, self.api_url("v2/user_rates"), Some(body))
            .await?;
        Ok(serde_json::from_str::<SmAddEntryResponse>(&text)?.id)
    }

    /// `DELETE v2/user_rates/<library id>` (T-31a-015). A 404 means the row is already gone remotely, which is what
    /// delete is for, so it is success (EC-S-26, OQ-2).
    pub async fn delete(&self, library_id: i64) -> Result<(), ShikimoriError> {
        match self
            .send(
                Method::DELETE,
                self.api_url(&format!("v2/user_rates/{library_id}")),
                None,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(ShikimoriError::Http(404)) => {
                tracing::info!("Shikimori entry {library_id} was already removed");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}
