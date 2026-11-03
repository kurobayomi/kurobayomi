//! GraphQL transport (5 and 7). One rate limiter
//! and one auth state serve every AniList call. The HTTP call sits behind [`Transport`] so tests can answer
//! without a network.

use std::collections::VecDeque;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::auth::{AccessToken, AuthState, is_expired};
use super::date::now_ms;
use super::dto::{AniListIdData, GraphQlEnvelope, MalMediaData, NextAiringData};
use super::error::AniListError;
use super::{endpoint, queries};
use crate::base::BaseTracker;
use crate::model::TrackerId;

/// Requests allowed in any rolling window (`API:42`).
pub const RATE_PERMITS: usize = 85;
/// Length of the rolling window (`API:42`).
pub const RATE_PERIOD: Duration = Duration::from_secs(60);
/// Sent in the `User-Agent` header with the crate version.
pub const APP_ID: &str = "io.github.anas1412.Kurobayomi";

/// Sliding-window log: at most `permits` requests in any `period`. One instance is shared by all calls.
pub struct RateLimiter {
    permits: usize,
    period: Duration,
    log: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new(permits: usize, period: Duration) -> Self {
        Self {
            permits,
            period,
            log: Mutex::new(VecDeque::new()),
        }
    }

    /// Returns once a request may be sent, sleeping until the oldest entry leaves the window.
    pub async fn acquire(&self) {
        loop {
            let wait_until = {
                let mut log = self.log.lock().await;
                let now = Instant::now();
                while let Some(&front) = log.front() {
                    if now.duration_since(front) >= self.period {
                        log.pop_front();
                    } else {
                        break;
                    }
                }
                if log.len() < self.permits {
                    log.push_back(now);
                    return;
                }
                log.front().map(|&front| front + self.period)
            };
            if let Some(at) = wait_until {
                tokio::time::sleep_until(at).await;
            }
        }
    }
}

/// What the transport returns for one POST.
#[derive(Debug)]
pub struct RawResponse {
    pub status: u16,
    pub body: Bytes,
    /// From a `Retry-After` header given in seconds, when present.
    pub retry_after: Option<Duration>,
}

/// One POST of a GraphQL body to the AniList endpoint.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn post(&self, body: &Value, bearer: Option<&str>) -> Result<RawResponse, AniListError>;
}

/// The production transport over `reqwest`.
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Result<Self, AniListError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(user_agent())
            .build()
            .map_err(AniListError::from_transport)?;
        Ok(Self { client })
    }
}

/// `Kurobayomi v<version> (<app id>)`, the shape of the Kotlin `User-Agent`.
pub fn user_agent() -> String {
    format!("Kurobayomi v{} ({APP_ID})", env!("CARGO_PKG_VERSION"))
}

#[async_trait]
impl Transport for HttpTransport {
    async fn post(&self, body: &Value, bearer: Option<&str>) -> Result<RawResponse, AniListError> {
        let mut request = self
            .client
            .post(endpoint())
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )
            .body(body.to_string());
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(AniListError::from_transport)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = response
            .bytes()
            .await
            .map_err(AniListError::from_transport)?;
        Ok(RawResponse {
            status,
            body,
            retry_after,
        })
    }
}

/// Auth state, the limiter and the transport. Every tracker call goes through [`Api::execute`].
pub struct Api {
    transport: Arc<dyn Transport>,
    base: Arc<BaseTracker>,
    limiter: RateLimiter,
    state: RwLock<AuthState>,
}

impl Api {
    pub fn new(transport: Arc<dyn Transport>, base: Arc<BaseTracker>, state: AuthState) -> Self {
        Self {
            transport,
            base,
            limiter: RateLimiter::new(RATE_PERMITS, RATE_PERIOD),
            state: RwLock::new(state),
        }
    }

    pub fn state(&self) -> AuthState {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_state(&self, next: AuthState) {
        *self.state.write().unwrap_or_else(PoisonError::into_inner) = next;
    }

    /// The token for an authenticated call, checked at `now_s`. A token past its expiry is cleared here, so
    /// it is not used for any request (no HTTP call is made).
    pub fn bearer(&self, now_s: i64) -> Result<AccessToken, AniListError> {
        let current = self.state();
        match current {
            AuthState::LoggedOut => Err(AniListError::NotAuthenticated),
            AuthState::Expired { .. } => Err(AniListError::AuthExpired),
            AuthState::LoggedIn {
                token,
                viewer_id,
                expires_at,
                ..
            } => {
                if is_expired(expires_at, now_s) {
                    self.expire(viewer_id);
                    Err(AniListError::AuthExpired)
                } else {
                    Ok(token)
                }
            }
        }
    }

    /// Marks the login expired: removes the stored token, sets the auth-expired flag and tells the subscribers. The
    /// viewer id is kept for the re-login prompt.
    pub fn expire(&self, viewer_id: i64) {
        self.set_state(AuthState::Expired { viewer_id });
        if let Err(e) = self.base.mark_expired() {
            tracing::warn!(target: "kurobayomi::tracker::anilist", error = %e, "could not clear the expired token");
        }
    }

    /// Sends one GraphQL operation and reads `data` as `T`. The auth gate runs before the rate limiter, so a
    /// logged-out or expired call returns at once without waiting or sending.
    pub async fn execute<V: Serialize, T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Option<V>,
    ) -> Result<T, AniListError> {
        self.execute_inner(query, variables, true).await
    }

    /// Like [`Api::execute`], but a 401 only returns [`AniListError::AuthExpired`]. It does not clear the stored
    /// login or set the flag. A login attempt checks its new token this way, so a refused token cannot wipe the
    /// session that was stored before the attempt.
    pub async fn execute_probe<V: Serialize, T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Option<V>,
    ) -> Result<T, AniListError> {
        self.execute_inner(query, variables, false).await
    }

    async fn execute_inner<V: Serialize, T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Option<V>,
        expire_on_401: bool,
    ) -> Result<T, AniListError> {
        let token = self.bearer(now_s())?;
        self.limiter.acquire().await;

        let mut body = json!({ "query": query });
        if let Some(vars) = variables {
            body["variables"] = serde_json::to_value(vars).map_err(AniListError::Decode)?;
        }

        let raw = self.transport.post(&body, Some(token.expose())).await?;
        match raw.status {
            401 if !expire_on_401 => Err(AniListError::AuthExpired),
            401 => {
                let viewer_id = match self.state() {
                    AuthState::LoggedIn { viewer_id, .. } | AuthState::Expired { viewer_id } => {
                        viewer_id
                    }
                    AuthState::LoggedOut => 0,
                };
                self.expire(viewer_id);
                Err(AniListError::AuthExpired)
            }
            429 => Err(AniListError::RateLimited {
                retry_after: raw.retry_after,
            }),
            500..=599 => Err(AniListError::Transient),
            s if !(200..300).contains(&s) => Err(AniListError::Http(s)),
            _ => {
                let envelope: GraphQlEnvelope =
                    serde_json::from_slice(&raw.body).map_err(AniListError::Decode)?;
                envelope.into_data()
            }
        }
    }
}

/// The MyAnimeList id of an AniList media, for the AniSkip lookup (E-43, E-44). Sent without a token. A failed
/// call, a null `idMal` and a zero id all give `None`, so the caller skips AniSkip instead of querying it with 0.
pub async fn anilist_to_mal_id(transport: &dyn Transport, media_id: i64) -> Option<i64> {
    let body = json!({ "query": queries::MAL_ID, "variables": { "id": media_id } });
    let raw = transport.post(&body, None).await.ok()?;
    if !(200..300).contains(&raw.status) {
        return None;
    }
    let envelope: GraphQlEnvelope = serde_json::from_slice(&raw.body).ok()?;
    let data: MalMediaData = envelope.into_data().ok()?;
    data.media?.id_mal.filter(|id| *id != 0)
}

/// The next episode of an AniList media: its number and its airing time in epoch seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextAiring {
    pub episode: i64,
    pub airing_at: i64,
}

/// What the airing row shows when no next episode is known: episode 1 at time 0 (E-45, the Kotlin fallback).
pub const NEXT_AIRING_FALLBACK: NextAiring = NextAiring {
    episode: 1,
    airing_at: 0,
};

/// The next airing episode of an AniList media (AniChart lookup). Sent without a token. `None` when AniList has no
/// scheduled episode, or the call fails. Callers use [`NEXT_AIRING_FALLBACK`] then. The reply is read with serde, not
/// by substring search.
pub async fn next_airing(transport: &dyn Transport, media_id: i64) -> Option<NextAiring> {
    let body = json!({ "query": queries::NEXT_AIRING, "variables": { "id": media_id } });
    let raw = transport.post(&body, None).await.ok()?;
    if !(200..300).contains(&raw.status) {
        return None;
    }
    let envelope: GraphQlEnvelope = serde_json::from_slice(&raw.body).ok()?;
    let data: NextAiringData = envelope.into_data().ok()?;
    let next = data.media?.next_airing_episode?;
    Some(NextAiring {
        episode: next.episode,
        airing_at: next.airing_at,
    })
}

/// The AniList id of a MyAnimeList id, for the airing lookup of a MAL track (T-29a-032). Sent without a token. A failed
/// call, a null media and a body that is not the expected JSON all give `0`, as the Kotlin does (`AniChartApi.kt:72-74`).
pub async fn mal_to_anilist_id(transport: &dyn Transport, mal_id: i64) -> i64 {
    let body = json!({ "query": queries::anilist_id_from_mal(mal_id) });
    let Ok(raw) = transport.post(&body, None).await else {
        return 0;
    };
    if !(200..300).contains(&raw.status) {
        return 0;
    }
    let Ok(envelope) = serde_json::from_slice::<GraphQlEnvelope>(&raw.body) else {
        return 0;
    };
    envelope
        .into_data::<AniListIdData>()
        .ok()
        .and_then(|d| d.media)
        .map_or(0, |m| m.id)
}

/// The next airing episode of a tracked title. A MAL track is first mapped to its AniList id. The remote id of an
/// AniList track is the AniList media id. `None` for any other tracker, an unknown id, or no scheduled episode.
pub async fn airing_for_track(
    transport: &dyn Transport,
    tracker: TrackerId,
    remote_id: i64,
) -> Option<NextAiring> {
    let media_id = if tracker == TrackerId::MYANIMELIST {
        match mal_to_anilist_id(transport, remote_id).await {
            0 => return None,
            id => id,
        }
    } else if tracker == TrackerId::ANILIST {
        remote_id
    } else {
        return None;
    };
    next_airing(transport, media_id).await
}

/// Seconds since the epoch, the clock of every expiry check.
pub fn now_s() -> i64 {
    now_ms() / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::anilist::testkit::{FakeTransport, ok, status};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn test_base() -> Arc<BaseTracker> {
        Arc::new(BaseTracker::new(
            crate::model::TrackerId::ANILIST,
            "AniList",
            crate::fakes::MemSecrets::new(),
            crate::events::EventBus::default(),
        ))
    }

    #[tokio::test(start_paused = true)]
    async fn t_28a_014_eighty_five_calls_pass_at_once_and_the_86th_waits_a_minute() {
        let limiter = RateLimiter::new(85, Duration::from_secs(60));
        let start = Instant::now();
        for _ in 0..85 {
            limiter.acquire().await;
        }
        assert_eq!(
            Instant::now() - start,
            Duration::ZERO,
            "85 calls need no wait"
        );
        limiter.acquire().await;
        assert_eq!(
            Instant::now() - start,
            Duration::from_secs(60),
            "the 86th waits for the window"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn t_28a_014_the_window_slides_and_restores_capacity() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        limiter.acquire().await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        limiter.acquire().await;
        let before = Instant::now();
        limiter.acquire().await;
        assert_eq!(
            Instant::now() - before,
            Duration::from_secs(30),
            "waits for the first entry only"
        );
        tokio::time::sleep(Duration::from_secs(120)).await;
        let later = Instant::now();
        limiter.acquire().await;
        limiter.acquire().await;
        assert_eq!(
            Instant::now() - later,
            Duration::ZERO,
            "capacity is back after the window"
        );
    }

    #[tokio::test]
    async fn t_28a_010_logged_out_call_returns_before_any_http_request() {
        let fake = FakeTransport::new(vec![]);
        let api = Api::new(fake.clone(), test_base(), AuthState::LoggedOut);
        let r: Result<Value, _> = api.execute("query", None::<()>).await;
        assert!(matches!(r, Err(AniListError::NotAuthenticated)));
        assert_eq!(fake.calls(), 0);
    }

    #[tokio::test]
    async fn t_28a_009_expired_token_is_never_sent() {
        let fake = FakeTransport::new(vec![]);
        let state = AuthState::LoggedIn {
            token: AccessToken::new("old"),
            viewer_id: 7,
            expires_at: 1,
            score_format: crate::tracker::anilist::score::ScoreFormat::Point10,
        };
        let api = Api::new(fake.clone(), test_base(), state);
        let r: Result<Value, _> = api.execute("query", None::<()>).await;
        assert!(matches!(r, Err(AniListError::AuthExpired)));
        assert_eq!(fake.calls(), 0, "no HTTP call with an expired token");
        assert_eq!(api.state(), AuthState::Expired { viewer_id: 7 });
    }

    #[tokio::test]
    async fn t_28a_010_bearer_and_user_agent_are_sent_with_a_fresh_token() -> TestResult {
        let fake = FakeTransport::new(vec![ok(r#"{"data":{"x":1}}"#)]);
        let state = AuthState::LoggedIn {
            token: AccessToken::new("tok-123"),
            viewer_id: 1,
            expires_at: i64::MAX,
            score_format: crate::tracker::anilist::score::ScoreFormat::Point10,
        };
        let api = Api::new(fake.clone(), test_base(), state);
        let _: Value = api.execute("query Q { x }", Some(json!({"a": 1}))).await?;
        assert_eq!(fake.bearers(), vec![Some("tok-123".to_owned())]);
        assert_eq!(fake.bodies()[0]["variables"]["a"], json!(1));
        assert!(user_agent().starts_with("Kurobayomi v"));
        assert!(user_agent().ends_with("(io.github.anas1412.Kurobayomi)"));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_014_variables_key_is_omitted_for_the_viewer_query() -> TestResult {
        let fake = FakeTransport::new(vec![ok(r#"{"data":{}}"#)]);
        let state = AuthState::LoggedIn {
            token: AccessToken::new("t"),
            viewer_id: 1,
            expires_at: i64::MAX,
            score_format: crate::tracker::anilist::score::ScoreFormat::Point10,
        };
        let api = Api::new(fake.clone(), test_base(), state);
        let _: Value = api
            .execute("query User { Viewer { id } }", None::<()>)
            .await?;
        assert!(fake.bodies()[0].get("variables").is_none());
        assert_eq!(
            fake.bodies()[0]["query"],
            json!("query User { Viewer { id } }")
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_015_status_codes_map_to_typed_errors() -> TestResult {
        let cases: Vec<(RawResponse, &str)> = vec![
            (
                status(
                    401,
                    include_str!("../../../tests/fixtures/anilist/http_401.txt"),
                ),
                "auth",
            ),
            (
                status(
                    429,
                    include_str!("../../../tests/fixtures/anilist/http_429.txt"),
                ),
                "rate",
            ),
            (
                status(
                    500,
                    include_str!("../../../tests/fixtures/anilist/http_500.txt"),
                ),
                "transient",
            ),
            (status(404, "nope"), "http404"),
        ];
        for (raw, kind) in cases {
            let fake = FakeTransport::new(vec![raw]);
            let state = AuthState::LoggedIn {
                token: AccessToken::new("t"),
                viewer_id: 9,
                expires_at: i64::MAX,
                score_format: crate::tracker::anilist::score::ScoreFormat::Point10,
            };
            let api = Api::new(fake.clone(), test_base(), state);
            let r: Result<Value, _> = api.execute("q", None::<()>).await;
            match kind {
                "auth" => {
                    assert!(matches!(r, Err(AniListError::AuthExpired)));
                    assert_eq!(api.state(), AuthState::Expired { viewer_id: 9 });
                }
                "rate" => assert!(matches!(r, Err(AniListError::RateLimited { .. }))),
                "transient" => assert!(matches!(r, Err(AniListError::Transient))),
                _ => assert!(matches!(r, Err(AniListError::Http(404)))),
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn t_27b_028_a_401_sets_the_expired_flag_and_sends_login_changed() {
        let secrets = crate::fakes::MemSecrets::new();
        let bus = crate::events::EventBus::default();
        let mut events = bus.subscribe();
        let base = Arc::new(BaseTracker::new(
            crate::model::TrackerId::ANILIST,
            "AniList",
            secrets.clone(),
            bus,
        ));
        base.save_oauth("9", "t", &json!({"access_token": "t"}))
            .unwrap();
        events.recv().await.unwrap();
        let state = AuthState::LoggedIn {
            token: AccessToken::new("t"),
            viewer_id: 9,
            expires_at: i64::MAX,
            score_format: crate::tracker::anilist::score::ScoreFormat::Point10,
        };
        let api = Api::new(
            FakeTransport::new(vec![status(401, "{}")]),
            base.clone(),
            state,
        );
        let r: Result<Value, _> = api.execute("q", None::<()>).await;
        assert!(matches!(r, Err(AniListError::AuthExpired)));
        assert!(base.auth_expired());
        assert!(!base.is_logged_in());
        assert!(secrets.raw("tracker/2/oauth").is_none());
        assert!(events.recv().await.is_ok(), "LoginChanged after the expiry");
    }

    #[tokio::test]
    async fn t_28a_015_graphql_errors_on_http_200_are_an_error() -> TestResult {
        let fake = FakeTransport::new(vec![ok(include_str!(
            "../../../tests/fixtures/anilist/errors_http200.json"
        ))]);
        let state = AuthState::LoggedIn {
            token: AccessToken::new("t"),
            viewer_id: 1,
            expires_at: i64::MAX,
            score_format: crate::tracker::anilist::score::ScoreFormat::Point10,
        };
        let api = Api::new(fake, test_base(), state);
        let r: Result<Value, _> = api.execute("q", None::<()>).await;
        assert!(matches!(r, Err(AniListError::GraphQl(s)) if s == "Invalid id"));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_034_mal_id_is_read_without_a_token() -> TestResult {
        let fake = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/media_mal_16498.json"
            )),
        ]);
        assert_eq!(anilist_to_mal_id(fake.as_ref(), 9253).await, Some(16498));
        assert_eq!(fake.bearers(), vec![None]);
        assert_eq!(fake.bodies()[0]["variables"]["id"], 9253);
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_034_null_mal_id_and_failures_give_none() {
        let null = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/media_mal_null.json"
            )),
        ]);
        assert_eq!(anilist_to_mal_id(null.as_ref(), 1).await, None);
        let failing = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::status(500, "boom"),
        ]);
        assert_eq!(anilist_to_mal_id(failing.as_ref(), 1).await, None);
        let empty = crate::tracker::anilist::testkit::FakeTransport::new(vec![]);
        assert_eq!(anilist_to_mal_id(empty.as_ref(), 1).await, None);
    }

    #[tokio::test]
    async fn t_28a_035_next_airing_reads_the_scheduled_episode() {
        let fake = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/next_airing_populated.json"
            )),
        ]);
        assert_eq!(
            next_airing(fake.as_ref(), 9253).await,
            Some(NextAiring {
                episode: 7,
                airing_at: 1_767_225_600
            })
        );
        assert_eq!(fake.bearers(), vec![None]);
        assert_eq!(fake.bodies()[0]["variables"]["id"], 9253);
        assert!(
            fake.bodies()[0]["query"]
                .as_str()
                .is_some_and(|q| q.contains("nextAiringEpisode"))
        );
    }

    #[tokio::test]
    async fn t_28a_035_null_next_airing_and_failures_give_none_and_the_fallback_is_episode_one() {
        let null = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/next_airing_null.json"
            )),
        ]);
        assert_eq!(next_airing(null.as_ref(), 1).await, None);
        let failing = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::status(500, "boom"),
        ]);
        assert_eq!(next_airing(failing.as_ref(), 1).await, None);
        assert_eq!(
            NEXT_AIRING_FALLBACK,
            NextAiring {
                episode: 1,
                airing_at: 0
            }
        );
    }

    #[tokio::test]
    async fn t_29a_032_mal_track_maps_the_mal_id_then_reads_the_airing_time() {
        let fake = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/media_anilist_id_9253.json"
            )),
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/next_airing_populated.json"
            )),
        ]);
        let got = airing_for_track(fake.as_ref(), TrackerId::MYANIMELIST, 16498).await;
        assert_eq!(got.map(|n| n.episode), Some(7));
        let bodies = fake.bodies();
        assert!(
            bodies[0]["query"]
                .as_str()
                .is_some_and(|q| q.contains("Media(idMal:16498,type: ANIME)"))
        );
        assert_eq!(bodies[1]["variables"]["id"], 9253);
    }

    #[tokio::test]
    async fn t_29a_032_non_numeric_or_missing_media_gives_zero() {
        let missing = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok(include_str!(
                "../../../tests/fixtures/anilist/media_anilist_id_none.json"
            )),
        ]);
        assert_eq!(mal_to_anilist_id(missing.as_ref(), 1).await, 0);
        let text = crate::tracker::anilist::testkit::FakeTransport::new(vec![
            crate::tracker::anilist::testkit::ok("not json at all"),
        ]);
        assert_eq!(mal_to_anilist_id(text.as_ref(), 1).await, 0);
    }

    #[tokio::test]
    async fn t_29a_032_other_trackers_have_no_airing_lookup() {
        let fake = crate::tracker::anilist::testkit::FakeTransport::new(vec![]);
        assert_eq!(airing_for_track(fake.as_ref(), TrackerId::KITSU, 5).await, None);
        assert_eq!(fake.calls(), 0);
    }
}
