//! Kitsu REST calls (T-30a-006 to T-30a-013). Every authenticated call goes through [`KitsuApi::authed`], which adds
//! the bearer token, the JSON:API headers and the app's User-Agent, and refreshes an expired token first. The password
//! is never stored: login exchanges it for a token and keeps only the token, the user id and the username.
//!
//! Sessions use a std mutex that is never held across an `await`. The refresh has its own async lock (single flight,
//! T-30a-027). Logout bumps a generation counter, so a refresh that finishes after logout is discarded.
//!
//! Keyring I/O on the async paths (login, token refresh, the expired flag) runs on the blocking pool through
//! [`KitsuApi::on_blocking_pool`] (T-30a-026). `logout` is synchronous by the tracker trait; its caller runs it on
//! the blocking pool.

use kuro_platform::kind::Kind;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kuro_platform::secrets::SecretStore;
use reqwest::Method;
use reqwest::Response;
use reqwest::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, USER_AGENT};
use serde_json::{Value, json};

use super::auth::{self, OAuth};
use super::dates;
use super::dto::{AddResponse, Included, LibraryEntry, ListResponse, UserResponse};
use super::error::KitsuError;
use super::score;
use super::status;
use crate::base::{Field, SECRET_SERVICE, account};
use crate::model::TrackerId;

/// The public client credentials Aniyomi sends. They are not user secrets.
pub const CLIENT_ID: &str = "dd031b32d2f56c990b1425efe6c42ad847e7fe3ab46bf1299f05ecd856bdb7dd";
pub const CLIENT_SECRET: &str = "54d7307928f63414defd96399fc31ba847961ceaecef3a5fd93144e960c0e151";
pub const VND_API_JSON: &str = "application/vnd.api+json";
const USER_AGENT_VALUE: &str = concat!(
    "Kurobayomi v",
    env!("CARGO_PKG_VERSION"),
    " (io.github.kurobayomi.Kurobayomi)"
);
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the calls go. Tests point these at a local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub token_url: String,
    pub api_url: String,
    pub algolia_url: String,
}

impl Endpoints {
    pub fn production() -> Self {
        Self {
            token_url: "https://kitsu.app/api/oauth/token".to_owned(),
            api_url: "https://kitsu.app/api/edge/".to_owned(),
            algolia_url: "https://AWQO5J657S-dsn.algolia.net/1/indexes/production_media/query/"
                .to_owned(),
        }
    }
}

/// The fields of a library entry that a push sends. `score` is the stored score, `progress` the stored episode or
/// chapter count. Dates are epoch milliseconds, 0 for unset.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryFields {
    pub status: i64,
    pub progress: f64,
    pub score: f64,
    pub start_date_ms: i64,
    pub finish_date_ms: i64,
    pub private: bool,
}

/// A library entry with the media it points to (`included`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntryMatch {
    pub entry: LibraryEntry,
    pub media: Included,
}

#[derive(Default)]
pub(super) struct Session {
    pub(super) token: Option<OAuth>,
    pub(super) user_id: Option<String>,
    /// Bumped by logout and login. A refresh started under an older value is discarded.
    pub(super) generation: u64,
}

pub struct KitsuApi {
    pub(super) http: reqwest::Client,
    pub(super) endpoints: Endpoints,
    pub(super) secrets: Arc<dyn SecretStore>,
    session: Mutex<Session>,
    /// Single flight for the token refresh (T-30a-027). Held across the refresh request only.
    refresh_lock: tokio::sync::Mutex<()>,
    /// The Algolia search key, kept for the session (T-30a-028 keeps it until a 403).
    pub(super) algolia_key: Mutex<Option<String>>,
}

/// The client with the connect, read and whole-call timeouts (E-75). Tests pass short ones.
pub(super) fn http_client(
    connect: Duration,
    read: Duration,
    call: Duration,
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .read_timeout(read)
        .timeout(call)
        .build()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The label of the media kind in the "Missing ... data" message (`Manga` or `Anime`).
fn media_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "Manga",
        Kind::Anime => "Anime",
    }
}

/// Non-2xx becomes `Http(code)`. A 2xx body is returned as text. The body of a non-2xx answer is dropped unread.
pub(super) async fn read_success(response: Response) -> Result<String, KitsuError> {
    let status = response.status();
    if !status.is_success() {
        return Err(KitsuError::Http(status.as_u16()));
    }
    Ok(response.text().await?)
}

/// The first entry of a list and its media. `Ok(None)` when there is no entry. An entry without its media is an error.
fn first_match(kind: Kind, list: ListResponse) -> Result<Option<EntryMatch>, KitsuError> {
    let Some(entry) = list.data.into_iter().next() else {
        return Ok(None);
    };
    let media = list
        .included
        .into_iter()
        .next()
        .ok_or(KitsuError::MissingMediaData(media_label(kind)))?;
    Ok(Some(EntryMatch { entry, media }))
}

impl KitsuApi {
    /// Builds the client and restores the stored token and user id. A missing or malformed token is no token.
    pub fn new(endpoints: Endpoints, secrets: Arc<dyn SecretStore>) -> Result<Self, KitsuError> {
        let token = auth::load(&secrets);
        let user_id = secrets
            .get(SECRET_SERVICE, &auth::user_id_account())
            .ok()
            .flatten()
            .filter(|id| !id.is_empty());
        let http = http_client(CONNECT_TIMEOUT, READ_TIMEOUT, CALL_TIMEOUT)?;
        Ok(Self {
            http,
            endpoints,
            secrets,
            session: Mutex::new(Session {
                token,
                user_id,
                generation: 0,
            }),
            refresh_lock: tokio::sync::Mutex::new(()),
            algolia_key: Mutex::new(None),
        })
    }

    /// Runs keyring work on the blocking pool, so it never stalls the async runtime (T-30a-026).
    async fn on_blocking_pool<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn SecretStore) -> T + Send + 'static,
    ) -> T {
        let secrets = Arc::clone(&self.secrets);
        match tokio::task::spawn_blocking(move || work(secrets.as_ref())).await {
            Ok(value) => value,
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    }

    pub(super) fn lock_session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The user id of the logged-in account, `None` when logged out.
    pub async fn user_id(&self) -> Option<String> {
        self.lock_session().user_id.clone()
    }

    /// True when a token and a user id are both held. No network call.
    pub fn has_session(&self) -> bool {
        let session = self.lock_session();
        session.token.is_some() && session.user_id.is_some()
    }

    pub(super) fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.endpoints.api_url.trim_end_matches('/'), path)
    }

    /// The token and the generation it was read under.
    fn snapshot(&self) -> Result<(OAuth, u64), KitsuError> {
        let session = self.lock_session();
        session
            .token
            .clone()
            .map(|t| (t, session.generation))
            .ok_or(KitsuError::NotAuthenticated)
    }

    /// The access token to send. An expired token is refreshed first.
    /// Only one refresh runs at a time: a caller that waited for the lock reuses the token the first one stored
    /// (T-30a-027). A refresh that finishes after logout or login is discarded.
    async fn bearer(&self) -> Result<String, KitsuError> {
        let (token, _) = self.snapshot()?;
        if !token.is_expired(now_secs()) {
            return Ok(token.access_token);
        }
        let _single_flight = self.refresh_lock.lock().await;
        let (current, generation) = self.snapshot()?;
        if !current.is_expired(now_secs()) {
            return Ok(current.access_token);
        }
        let refresh = current
            .refresh_token
            .clone()
            .ok_or(KitsuError::ReauthRequired)?;
        match self.refresh(&refresh).await {
            Ok(mut fresh) => {
                if fresh.refresh_token.is_none() {
                    fresh.refresh_token = current.refresh_token.clone();
                }
                {
                    let mut session = self.lock_session();
                    if session.generation != generation {
                        return Err(KitsuError::NotAuthenticated);
                    }
                    session.token = Some(fresh.clone());
                }
                let saved = fresh.clone();
                if self
                    .on_blocking_pool(move |s| auth::save(s, &saved))
                    .await
                    .is_err()
                {
                    tracing::warn!("Kitsu token could not be saved after a refresh");
                }
                // A logout or login that ran while the token was being written decides what stays stored.
                // ponytail: re-persisting the live session narrows the race, a storage lock would close it.
                let live = {
                    let session = self.lock_session();
                    (session.generation != generation).then(|| session.token.clone())
                };
                if let Some(live) = live {
                    self.on_blocking_pool(move |s| match live {
                        Some(t) => auth::save(s, &t),
                        None => auth::clear(s),
                    })
                    .await
                    .ok();
                    return Err(KitsuError::NotAuthenticated);
                }
                Ok(fresh.access_token)
            }
            Err(KitsuError::Http(code)) if (400..500).contains(&code) => {
                if code == 401 {
                    self.mark_expired().await;
                }
                Err(KitsuError::ReauthRequired)
            }
            Err(e) => {
                tracing::warn!("Kitsu token refresh failed, keeping the stale token: {e}");
                let session = self.lock_session();
                if session.generation != generation {
                    return Err(KitsuError::NotAuthenticated);
                }
                Ok(current.access_token)
            }
        }
    }

    /// `grant_type=refresh_token` exchange. The form order is the source's.
    async fn refresh(&self, refresh_token: &str) -> Result<OAuth, KitsuError> {
        let response = self
            .http
            .post(&self.endpoints.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
            ])
            .send()
            .await?;
        Ok(serde_json::from_str(&read_success(response).await?)?)
    }

    /// One authenticated call. Without a token nothing is sent.
    pub(super) async fn authed(
        &self,
        method: Method,
        url: String,
        body: Option<Value>,
    ) -> Result<String, KitsuError> {
        let bearer = self.bearer().await?;
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(bearer)
            .header(ACCEPT, VND_API_JSON)
            .header(CONTENT_TYPE, VND_API_JSON)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .header(CACHE_CONTROL, "no-cache");
        if let Some(body) = body {
            request = request.body(body.to_string());
        }
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.mark_expired().await;
            return Err(KitsuError::ReauthRequired);
        }
        read_success(response).await
    }

    /// Sets the auth-expired flag (`tracker/3/expired`) after Kitsu answered 401 (T-27a-074). The stored token stays
    /// until the user logs in again, which clears the flag.
    async fn mark_expired(&self) {
        let account = account(TrackerId::KITSU, Field::Expired);
        let saved = self
            .on_blocking_pool(move |s| s.set(SECRET_SERVICE, &account, "true"))
            .await;
        if let Err(e) = saved {
            tracing::warn!("Kitsu auth-expired flag could not be saved: {e}");
        }
    }

    async fn list(&self, url: String) -> Result<ListResponse, KitsuError> {
        let body = self.authed(Method::GET, url, None).await?;
        Ok(serde_json::from_str(&body)?)
    }

    async fn require_user_id(&self) -> Result<String, KitsuError> {
        self.user_id().await.ok_or(KitsuError::NotAuthenticated)
    }

    /// `GET users?filter[self]=true`, reading `data[0].id`.
    async fn fetch_user_id(&self) -> Result<String, KitsuError> {
        let url = self.endpoint("users?filter[self]=true");
        let body = self.authed(Method::GET, url, None).await?;
        let parsed: UserResponse = serde_json::from_str(&body)?;
        parsed
            .data
            .into_iter()
            .next()
            .map(|user| user.id)
            .ok_or(KitsuError::NoUser)
    }

    /// Password-grant login (T-30a-007). The token is kept in memory first so the user lookup can use it. If the lookup
    /// fails the previous token comes back and nothing is written. The password is never stored.
    pub async fn login(&self, username: &str, password: &str) -> Result<(), KitsuError> {
        let form = [
            ("username", username),
            ("password", password),
            ("grant_type", "password"),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ];
        let response = self
            .http
            .post(&self.endpoints.token_url)
            .form(&form)
            .send()
            .await?;
        let token: OAuth = serde_json::from_str(&read_success(response).await?)?;

        let previous = {
            let mut session = self.lock_session();
            session.generation += 1;
            std::mem::replace(&mut session.token, Some(token.clone()))
        };
        let user_id = match self.fetch_user_id().await {
            Ok(id) => id,
            Err(e) => {
                self.lock_session().token = previous;
                return Err(e);
            }
        };
        let (stored, name) = (token.clone(), username.to_owned());
        let id = user_id.clone();
        let stored = self
            .on_blocking_pool(move |s| {
                let result = Self::store_login(s, &stored, &id, &name);
                if result.is_err() {
                    let _ = auth::clear(s);
                }
                result
            })
            .await;
        if let Err(e) = stored {
            self.lock_session().token = previous;
            return Err(e);
        }
        self.lock_session().user_id = Some(user_id);
        Ok(())
    }

    fn store_login(
        secrets: &dyn SecretStore,
        token: &OAuth,
        user_id: &str,
        username: &str,
    ) -> Result<(), KitsuError> {
        auth::save(secrets, token)?;
        secrets.set(SECRET_SERVICE, &auth::user_id_account(), user_id)?;
        secrets.set(
            SECRET_SERVICE,
            &account(TrackerId::KITSU, Field::Username),
            username,
        )?;
        // A new login clears the expired flag (T-27a-074).
        secrets.delete(SECRET_SERVICE, &account(TrackerId::KITSU, Field::Expired))?;
        Ok(())
    }

    /// Clears the token, user id, username and expired flag, and drops the Algolia key. No network call, and calling
    /// it again is fine (KT-32). A refresh still in flight is discarded (KT-97).
    pub fn logout(&self) -> Result<(), KitsuError> {
        {
            let mut session = self.lock_session();
            session.token = None;
            session.user_id = None;
            session.generation += 1;
        }
        *self
            .algolia_key
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        auth::clear(self.secrets.as_ref())?;
        self.secrets
            .delete(SECRET_SERVICE, &account(TrackerId::KITSU, Field::Expired))?;
        Ok(())
    }

    /// `POST library-entries` (T-30a-009). Dates and rating are not sent. Returns the new library-entry id.
    pub async fn add_entry(
        &self,
        kind: Kind,
        media_id: i64,
        fields: &EntryFields,
    ) -> Result<i64, KitsuError> {
        let user_id = self.require_user_id().await?;
        let body = json!({"data": {
            "type": "libraryEntries",
            "attributes": {
                "status": status::to_wire(kind, fields.status)?,
                "progress": fields.progress.trunc() as i64,
                "private": fields.private,
            },
            "relationships": {
                "user": {"data": {"id": user_id, "type": "users"}},
                "media": {"data": {"id": media_id, "type": kind.as_str()}},
            },
        }});
        let text = self
            .authed(Method::POST, self.endpoint("library-entries"), Some(body))
            .await?;
        let parsed: AddResponse = serde_json::from_str(&text)?;
        Ok(parsed.data.id)
    }

    /// `PATCH library-entries/<id>` with the null rules of T-30a-010. A library id of 0 is refused before any request.
    pub async fn update_entry(
        &self,
        kind: Kind,
        remote_id: i64,
        fields: &EntryFields,
    ) -> Result<(), KitsuError> {
        if remote_id == 0 {
            return Err(KitsuError::NotFound(kind.as_str()));
        }
        let body = json!({"data": {
            "type": "libraryEntries",
            "id": remote_id,
            "attributes": {
                "status": status::to_wire(kind, fields.status)?,
                "progress": fields.progress.trunc() as i64,
                "ratingTwenty": score::to_wire(fields.score),
                "startedAt": dates::to_wire(fields.start_date_ms),
                "finishedAt": dates::to_wire(fields.finish_date_ms),
                "private": fields.private,
            },
        }});
        let url = self.endpoint(&format!("library-entries/{remote_id}"));
        self.authed(Method::PATCH, url, Some(body)).await?;
        Ok(())
    }

    /// `DELETE library-entries/<id>` (T-30a-011). An unbound track (id 0) is refused before any request.
    pub async fn remove_entry(&self, kind: Kind, remote_id: i64) -> Result<(), KitsuError> {
        if remote_id == 0 {
            return Err(KitsuError::NotFound(kind.as_str()));
        }
        let url = self.endpoint(&format!("library-entries/{remote_id}"));
        self.authed(Method::DELETE, url, None).await?;
        Ok(())
    }

    /// `GET library-entries?filter[<kind>_id]=<media>&filter[user_id]=<user>&include=<kind>` (T-30a-012). The
    /// brackets stay literal. `Ok(None)` when the user has no entry for the media.
    pub async fn find_entry(
        &self,
        kind: Kind,
        media_id: i64,
    ) -> Result<Option<EntryMatch>, KitsuError> {
        let user_id = self.require_user_id().await?;
        let kind_name = kind.as_str();
        let url = self.endpoint(&format!(
            "library-entries?filter[{kind_name}_id]={media_id}&filter[user_id]={user_id}&include={kind_name}"
        ));
        first_match(kind, self.list(url).await?)
    }

    /// `GET library-entries?filter[id]=<library id>&include=<kind>` (T-30a-013). Not found is `NotFound(kind)`.
    pub async fn get_entry(&self, kind: Kind, library_id: i64) -> Result<EntryMatch, KitsuError> {
        let kind_name = kind.as_str();
        let url = self.endpoint(&format!(
            "library-entries?filter[id]={library_id}&include={kind_name}"
        ));
        first_match(kind, self.list(url).await?)?.ok_or(KitsuError::NotFound(kind_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::MemSecrets;
    use serde_json::json;
    use wiremock::matchers::{body_json, body_string, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn token(created_at: i64, refresh: Option<&str>) -> OAuth {
        OAuth {
            access_token: "tok-1".into(),
            token_type: "bearer".into(),
            created_at,
            expires_in: 7200,
            refresh_token: refresh.map(str::to_owned),
        }
    }

    async fn endpoints(server: &MockServer) -> Endpoints {
        Endpoints {
            token_url: format!("{}/api/oauth/token", server.uri()),
            api_url: format!("{}/api/edge/", server.uri()),
            algolia_url: format!("{}/1/indexes/production_media/query/", server.uri()),
        }
    }

    /// A client with a stored, fresh token and user id 42.
    async fn logged_in(server: &MockServer) -> (KitsuApi, Arc<dyn SecretStore>) {
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        auth::save(secrets.as_ref(), &token(now_secs(), Some("r"))).unwrap();
        secrets
            .set(SECRET_SERVICE, &auth::user_id_account(), "42")
            .unwrap();
        let api = KitsuApi::new(endpoints(server).await, secrets.clone()).unwrap();
        (api, secrets)
    }

    fn fields() -> EntryFields {
        EntryFields {
            status: crate::kitsu::READING,
            progress: 12.7,
            score: 0.0,
            start_date_ms: 0,
            finish_date_ms: 0,
            private: false,
        }
    }

    const ENTRY_LIST: &str = r#"{"data":[{"id":"77","attributes":{"status":"current","progress":4,
        "ratingTwenty":null,"startedAt":null,"finishedAt":null,"private":false}}],
        "included":[{"id":9,"attributes":{"canonicalTitle":"T","status":"finished"}}]}"#;

    #[tokio::test]
    async fn t_30a_007_login_stores_token_user_and_username_but_not_the_password() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .and(body_string_contains_in_order())
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-login", "token_type": "bearer",
                "created_at": now_secs(), "expires_in": 7200, "refresh_token": "ref", "scope": "public"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/edge/users"))
            .and(query_param("filter[self]", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "42"}]})))
            .mount(&server)
            .await;
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        let api = KitsuApi::new(endpoints(&server).await, secrets.clone()).unwrap();

        api.login("ann@example.test", "hunter2").await.unwrap();

        assert_eq!(api.user_id().await.as_deref(), Some("42"));
        assert_eq!(auth::load(&secrets).unwrap().access_token, "tok-login");
        assert_eq!(
            secrets
                .get(SECRET_SERVICE, &auth::user_id_account())
                .unwrap()
                .as_deref(),
            Some("42")
        );
        assert_eq!(
            secrets
                .get(SECRET_SERVICE, &account(TrackerId::KITSU, Field::Username))
                .unwrap()
                .as_deref(),
            Some("ann@example.test")
        );
        assert!(!format!("{:?}", auth::load(&secrets)).contains("hunter2"));
    }

    /// The token request body carries the fields in the source order.
    fn body_string_contains_in_order() -> impl wiremock::Match {
        struct InOrder;
        impl wiremock::Match for InOrder {
            fn matches(&self, request: &wiremock::Request) -> bool {
                let body = String::from_utf8_lossy(&request.body).to_string();
                let keys = [
                    "username=",
                    "password=",
                    "grant_type=password",
                    "client_id=",
                    "client_secret=",
                ];
                let mut at = 0;
                keys.iter().all(|k| match body[at..].find(k) {
                    Some(p) => {
                        at += p + k.len();
                        true
                    }
                    None => false,
                })
            }
        }
        InOrder
    }

    #[tokio::test]
    async fn t_30a_007_a_failed_user_lookup_rolls_back_and_writes_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-new", "token_type": "bearer",
                "created_at": now_secs(), "expires_in": 7200
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/edge/users"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        let api = KitsuApi::new(endpoints(&server).await, secrets.clone()).unwrap();

        let err = api.login("ann", "pw").await.unwrap_err();

        assert_eq!(err, KitsuError::Http(500));
        assert_eq!(api.user_id().await, None);
        assert_eq!(auth::load(&secrets), None);
        assert_eq!(
            secrets
                .get(SECRET_SERVICE, &account(TrackerId::KITSU, Field::Username))
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn t_30a_007_an_empty_user_lookup_is_no_user_and_rolls_back() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-new", "token_type": "bearer", "created_at": now_secs(), "expires_in": 7200
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/edge/users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;
        let api = KitsuApi::new(endpoints(&server).await, MemSecrets::new()).unwrap();
        assert_eq!(api.login("ann", "pw").await, Err(KitsuError::NoUser));
    }

    #[tokio::test]
    async fn t_30a_006_authed_calls_send_the_bearer_and_json_api_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/edge/library-entries"))
            .and(header("Authorization", "Bearer tok-1"))
            .and(header("Accept", VND_API_JSON))
            .and(header("Content-Type", VND_API_JSON))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        assert_eq!(api.find_entry(Kind::Manga, 5).await.unwrap(), None);
    }

    #[tokio::test]
    async fn t_30a_006_no_token_sends_no_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let api = KitsuApi::new(endpoints(&server).await, MemSecrets::new()).unwrap();
        assert_eq!(
            api.get_entry(Kind::Anime, 3).await,
            Err(KitsuError::NotAuthenticated)
        );
    }

    #[tokio::test]
    async fn t_30a_006_an_expired_token_is_refreshed_before_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .and(body_string_contains_grant("refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-fresh", "token_type": "bearer", "created_at": now_secs(), "expires_in": 7200
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/edge/library-entries"))
            .and(header("Authorization", "Bearer tok-fresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .expect(1)
            .mount(&server)
            .await;
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        auth::save(secrets.as_ref(), &token(0, Some("old-refresh"))).unwrap();
        secrets
            .set(SECRET_SERVICE, &auth::user_id_account(), "42")
            .unwrap();
        let api = KitsuApi::new(endpoints(&server).await, secrets.clone()).unwrap();

        api.find_entry(Kind::Manga, 5).await.unwrap();

        // The refresh response had no refresh_token, so the old one is kept.
        let stored = auth::load(&secrets).unwrap();
        assert_eq!(stored.access_token, "tok-fresh");
        assert_eq!(stored.refresh_token.as_deref(), Some("old-refresh"));
    }

    #[tokio::test]
    async fn t_30a_006_a_client_error_on_refresh_is_reauth_and_sends_nothing_else() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/oauth/token"))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        auth::save(secrets.as_ref(), &token(0, Some("old"))).unwrap();
        secrets
            .set(SECRET_SERVICE, &auth::user_id_account(), "42")
            .unwrap();
        let api = KitsuApi::new(endpoints(&server).await, secrets).unwrap();
        assert_eq!(
            api.get_entry(Kind::Manga, 1).await,
            Err(KitsuError::ReauthRequired)
        );
    }

    #[tokio::test]
    async fn t_30a_006_an_expired_token_without_refresh_is_reauth_required() {
        let server = MockServer::start().await;
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        auth::save(secrets.as_ref(), &token(0, None)).unwrap();
        let api = KitsuApi::new(endpoints(&server).await, secrets).unwrap();
        assert_eq!(
            api.get_entry(Kind::Manga, 1).await,
            Err(KitsuError::ReauthRequired)
        );
    }

    fn body_string_contains_grant(grant: &'static str) -> impl wiremock::Match {
        struct Grant(&'static str);
        impl wiremock::Match for Grant {
            fn matches(&self, request: &wiremock::Request) -> bool {
                String::from_utf8_lossy(&request.body).contains(&format!("grant_type={}", self.0))
            }
        }
        Grant(grant)
    }

    #[tokio::test]
    async fn t_30a_009_add_sends_the_source_body_without_dates_or_rating() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/edge/library-entries"))
            .and(body_json(json!({"data": {
                "type": "libraryEntries",
                "attributes": {"status": "current", "progress": 12, "private": false},
                "relationships": {
                    "user": {"data": {"id": "42", "type": "users"}},
                    "media": {"data": {"id": 5, "type": "manga"}}
                }
            }})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "555"}})))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        let id = api.add_entry(Kind::Manga, 5, &fields()).await.unwrap();
        assert_eq!(id, 555);
    }

    #[tokio::test]
    async fn t_30a_010_update_sends_null_for_zero_rating_and_zero_dates() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/edge/library-entries/77"))
            .and(body_json(json!({"data": {
                "type": "libraryEntries", "id": 77,
                "attributes": {
                    "status": "current", "progress": 12, "ratingTwenty": null,
                    "startedAt": null, "finishedAt": null, "private": false
                }
            }})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        api.update_entry(Kind::Manga, 77, &fields()).await.unwrap();
    }

    #[tokio::test]
    async fn t_30a_009_add_manga_body_is_the_fixture_byte_for_byte() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/edge/library-entries"))
            .and(body_string(include_str!(
                "../../tests/fixtures/kitsu/add_manga_expected.json"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "1"}})))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        api.add_entry(Kind::Manga, 5, &fields()).await.unwrap();
    }

    #[tokio::test]
    async fn t_30a_009_add_anime_body_is_the_fixture_byte_for_byte() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/edge/library-entries"))
            .and(body_string(include_str!(
                "../../tests/fixtures/kitsu/add_anime_expected.json"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "1"}})))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        let anime = EntryFields {
            status: crate::kitsu::WATCHING,
            ..fields()
        };
        api.add_entry(Kind::Anime, 5, &anime).await.unwrap();
    }

    #[tokio::test]
    async fn t_30a_010_update_manga_body_is_the_fixture_byte_for_byte() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/edge/library-entries/77"))
            .and(body_string(include_str!(
                "../../tests/fixtures/kitsu/update_manga_expected.json"
            )))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        api.update_entry(Kind::Manga, 77, &fields()).await.unwrap();
    }

    #[tokio::test]
    async fn t_27a_074_a_401_on_an_api_call_sets_the_expired_flag() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let (api, secrets) = logged_in(&server).await;
        let flag = account(TrackerId::KITSU, Field::Expired);
        assert_eq!(secrets.get(SECRET_SERVICE, &flag).unwrap(), None);
        assert_eq!(
            api.remove_entry(Kind::Manga, 77).await,
            Err(KitsuError::ReauthRequired)
        );
        assert_eq!(
            secrets.get(SECRET_SERVICE, &flag).unwrap().as_deref(),
            Some("true")
        );
    }

    #[tokio::test]
    async fn t_30a_010_update_sends_rating_and_dates_when_set() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/edge/library-entries/77"))
            .and(body_json(json!({"data": {
                "type": "libraryEntries", "id": 77,
                "attributes": {
                    "status": "completed", "progress": 3, "ratingTwenty": "15",
                    "startedAt": "2026-10-10T08:30:00.000Z", "finishedAt": null, "private": true
                }
            }})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        let fields = EntryFields {
            status: crate::kitsu::COMPLETED,
            progress: 3.0,
            score: 7.5,
            start_date_ms: 1_791_621_000_000,
            finish_date_ms: 0,
            private: true,
        };
        api.update_entry(Kind::Manga, 77, &fields).await.unwrap();
    }

    #[tokio::test]
    async fn t_30a_010_update_with_id_zero_is_refused_before_any_request() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        assert_eq!(
            api.update_entry(Kind::Manga, 0, &fields()).await,
            Err(KitsuError::NotFound("manga"))
        );
    }

    #[tokio::test]
    async fn t_30a_010_a_404_is_an_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        assert_eq!(
            api.update_entry(Kind::Manga, 9, &fields()).await,
            Err(KitsuError::Http(404))
        );
    }

    #[tokio::test]
    async fn t_30a_011_remove_sends_delete_and_refuses_an_unbound_track() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/edge/library-entries/77"))
            .and(header("Content-Type", VND_API_JSON))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        api.remove_entry(Kind::Anime, 77).await.unwrap();
        assert_eq!(
            api.remove_entry(Kind::Anime, 0).await,
            Err(KitsuError::NotFound("anime"))
        );
    }

    #[tokio::test]
    async fn t_30a_012_find_keeps_the_brackets_literal_and_reads_the_entry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/edge/library-entries"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ENTRY_LIST))
            .expect(1)
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        let found = api.find_entry(Kind::Manga, 5).await.unwrap().unwrap();
        assert_eq!(found.entry.id, 77);
        assert_eq!(found.media.id, 9);

        let requests = server.received_requests().await.unwrap();
        let url = requests[0].url.as_str().to_owned();
        assert!(
            url.contains("filter[manga_id]=5&filter[user_id]=42&include=manga"),
            "{url}"
        );
    }

    #[tokio::test]
    async fn t_30a_012_data_without_included_is_an_error_and_empty_data_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/edge/library-entries"))
            .and(query_param("filter[anime_id]", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/edge/library-entries"))
            .and(query_param("filter[manga_id]", "6"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"id": "1", "attributes": {"status": "current", "progress": 0, "private": false}}]
            })))
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        assert_eq!(api.find_entry(Kind::Anime, 5).await.unwrap(), None);
        assert_eq!(
            api.find_entry(Kind::Manga, 6).await,
            Err(KitsuError::MissingMediaData("Manga"))
        );
    }

    #[tokio::test]
    async fn t_30a_013_get_by_entry_id_reads_the_entry_and_names_the_kind_when_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/edge/library-entries"))
            .and(query_param("filter[id]", "77"))
            .and(query_param("include", "anime"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        assert_eq!(
            api.get_entry(Kind::Anime, 77).await,
            Err(KitsuError::NotFound("anime"))
        );
    }

    #[tokio::test]
    async fn t_30a_007_logout_clears_everything_without_a_network_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let (api, secrets) = logged_in(&server).await;
        api.logout().unwrap();
        api.logout().unwrap();
        assert_eq!(api.user_id().await, None);
        assert_eq!(auth::load(&secrets), None);
    }

    #[tokio::test]
    async fn t_30a_014_a_malformed_body_is_a_decode_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let (api, _) = logged_in(&server).await;
        assert!(matches!(
            api.get_entry(Kind::Manga, 1).await,
            Err(KitsuError::Decode(_))
        ));
    }
}
