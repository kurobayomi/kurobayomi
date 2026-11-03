//! Tests against a local wiremock server (integration table).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{
    body_string, header, method, path, path_regex, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::error::MalError;
use super::testkit::{MemorySecrets, expired_token, fresh_token, tracker_for};
use super::tracker::MalTracker;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};

const TOKEN_PATH: &str = "/v1/oauth2/token";

fn manga_track(remote_id: i64, status: i64, read: f64, total: i64) -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TrackerId::MYANIMELIST,
        remote_id,
        library_id: None,
        title: "t".into(),
        last_chapter_read: read,
        total_chapters: total,
        status,
        score: 0.0,
        remote_url: String::new(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

fn anime_track(remote_id: i64, status: i64, seen: f64, total: i64) -> AnimeTrack {
    AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TrackerId::MYANIMELIST,
        remote_id,
        library_id: None,
        title: "t".into(),
        last_episode_seen: seen,
        total_episodes: total,
        status,
        score: 0.0,
        remote_url: String::new(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

fn token_body(access: &str) -> serde_json::Value {
    json!({"token_type":"Bearer","expires_in":2592000,"access_token":access,"refresh_token":format!("{access}-refresh")})
}

async fn mount_login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .and(body_string(
            "client_id=test-client&code=CODE&code_verifier=VER&grant_type=authorization_code",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("new-access")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"fixture_user"})))
        .mount(server)
        .await;
}

async fn logged_in(server: &MockServer, tracker: &MalTracker) {
    mount_login(server).await;
    tracker
        .login_with_code("CODE", "VER")
        .await
        .expect("login succeeds");
}

async fn query_count(server: &MockServer, key: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.query_pairs().filter(|(k, _)| k == key).count())
        .sum()
}

#[tokio::test]
async fn login_happy_path() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let tracker = tracker_for(&server, dir.path(), secrets.clone());

    logged_in(&server, &tracker).await;

    assert!(tracker.is_logged_in());
    assert_eq!(tracker.username(), "fixture_user");
    assert!(!tracker.auth().expired());
    let stored = secrets.token_text().expect("token stored");
    assert!(stored.contains("new-access"));
    assert!(!stored.contains("null"));
}

#[tokio::test]
async fn login_after_expiry_succeeds() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(expired_token("a")).unwrap();
    tracker.auth().set_expired(true).unwrap();
    mount_login(&server).await;

    tracker
        .login_with_code("CODE", "VER")
        .await
        .expect("re-login clears the expired flag first");
    assert!(tracker.is_logged_in());
    assert!(!tracker.auth().expired());
}

#[tokio::test]
async fn login_exchange_400_keeps_existing_login() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let tracker = tracker_for(&server, dir.path(), secrets.clone());
    logged_in(&server, &tracker).await;
    let before = secrets.token_text();

    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .and(body_string(
            "client_id=test-client&code=BAD&code_verifier=VER&grant_type=authorization_code",
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant"})))
        .mount(&server)
        .await;
    let err = tracker.login_with_code("BAD", "VER").await.unwrap_err();

    assert_eq!(err.to_string(), "HTTP error 400");
    assert!(tracker.is_logged_in(), "existing login is kept");
    assert_eq!(secrets.token_text(), before);
}

#[tokio::test]
async fn login_exchange_failure_without_login_stays_logged_out() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    assert!(tracker.login_with_code("BAD", "VER").await.is_err());
    assert!(!tracker.is_logged_in());
}

#[tokio::test]
async fn login_empty_username_is_rejected() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("x")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":""})))
        .mount(&server)
        .await;

    let err = tracker.login_with_code("CODE", "VER").await.unwrap_err();
    assert_eq!(err.to_string(), "MAL: User is not authenticated");
    assert!(!tracker.is_logged_in());
}

#[tokio::test]
async fn current_user_missing_name_is_decode_error() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::Decode(_))
    ));
}

#[tokio::test]
async fn bearer_only_on_api_calls_not_on_exchange() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    logged_in(&server, &tracker).await;

    let requests = server.received_requests().await.unwrap();
    let exchange = requests
        .iter()
        .find(|r| r.url.path() == TOKEN_PATH)
        .unwrap();
    assert!(exchange.headers.get("authorization").is_none());
    let user = requests
        .iter()
        .find(|r| r.url.path() == "/v2/users/@me")
        .unwrap();
    assert_eq!(
        user.headers.get("authorization").unwrap(),
        "Bearer new-access"
    );
    assert!(user.headers.get("cookie").is_none());
    assert!(user.headers.get("cache-control").is_none());
}

#[tokio::test]
async fn http_error_maps_to_display_text() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/-7"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = tracker.api().manga_details(-7).await.unwrap_err();
    assert!(matches!(err, MalError::Http(404)));
    assert_eq!(err.to_string(), "HTTP error 404");
}

#[tokio::test]
async fn refresh_success_uses_new_bearer() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let tracker = tracker_for(&server, dir.path(), secrets.clone());
    tracker
        .auth()
        .set_token(expired_token("old-access"))
        .unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .and(header("authorization", "Bearer old-access"))
        .and(body_string(
            "client_id=test-client&refresh_token=old-access-refresh&grant_type=refresh_token",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("new-access")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .and(header("authorization", "Bearer new-access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"u"})))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(tracker.api().current_user().await.unwrap(), "u");
    assert!(secrets.token_text().unwrap().contains("new-access"));
}

#[tokio::test]
async fn refresh_401_sets_flag_and_stops_network() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(expired_token("a")).unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"u"})))
        .expect(0)
        .mount(&server)
        .await;

    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::LoginExpired)
    ));
    assert!(tracker.auth().expired());
    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::LoginExpired)
    ));
}

#[tokio::test]
async fn api_401_sets_the_expired_flag_and_returns_login_expired() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    // The 401 triggers one refresh (T-29a-053); MAL refuses that refresh too, so the login has expired.
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;

    assert!(!tracker.auth().expired());
    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::LoginExpired)
    ));
    assert!(tracker.auth().expired(), "T-27a-074");
}

#[tokio::test]
async fn t_29a_053_api_401_refreshes_once_and_retries_once() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("old-access")).unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("new-access")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .and(header("authorization", "Bearer old-access"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .and(header("authorization", "Bearer new-access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"u"})))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(tracker.api().current_user().await.unwrap(), "u");
    assert!(!tracker.auth().expired());
}

#[tokio::test]
async fn t_29a_053_second_401_after_the_retry_is_http_401_without_a_loop() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("old-access")).unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("new-access")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2)
        .mount(&server)
        .await;

    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::Http(401))
    ));
}

#[tokio::test]
async fn t_29a_054_a_failed_refresh_is_not_retried_for_thirty_seconds() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(expired_token("old")).unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&server)
        .await;
    let http = reqwest::Client::new();
    let t0 = 1_700_000_000;
    assert!(matches!(
        tracker.auth().bearer_at(&http, t0).await,
        Err(MalError::RefreshFailed)
    ));
    // Within 30 s: no request, RefreshFailed at once.
    assert!(matches!(
        tracker.auth().bearer_at(&http, t0 + 10).await,
        Err(MalError::RefreshFailed)
    ));
    // After 30 s the refresh runs again (the second POST).
    assert!(matches!(
        tracker.auth().bearer_at(&http, t0 + 31).await,
        Err(MalError::RefreshFailed)
    ));
}

#[tokio::test]
async fn refresh_400_keeps_flag_clear_and_backs_off() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(expired_token("a")).unwrap();
    // One refusal; the second call within 30 s sends no refresh (T-29a-054).
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;

    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::RefreshFailed)
    ));
    assert!(!tracker.auth().expired());
    assert!(matches!(
        tracker.api().current_user().await,
        Err(MalError::RefreshFailed)
    ));
}

#[tokio::test]
async fn refresh_single_flight_for_concurrent_calls() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(expired_token("a")).unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(token_body("fresh"))
                .set_delay(Duration::from_millis(150)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"u"})))
        .mount(&server)
        .await;

    let calls = futures::future::join_all((0..10).map(|_| tracker.api().current_user())).await;
    assert!(calls.iter().all(|r| r.is_ok()));
}

#[tokio::test]
async fn logout_during_refresh_leaves_no_token() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let tracker = tracker_for(&server, dir.path(), secrets.clone());
    tracker.auth().set_token(expired_token("a")).unwrap();
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(token_body("late"))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;

    let call = tracker.api().current_user();
    let logout = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        tracker.logout().unwrap();
    };
    let (result, ()) = tokio::join!(call, logout);

    assert!(matches!(result, Err(MalError::NotAuthenticated)));
    assert!(!tracker.auth().has_token());
    assert!(
        secrets.is_empty(),
        "no token is stored after logout, and no null text either"
    );
}

#[tokio::test]
async fn manga_text_search_filters_novels_and_keeps_order() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga"))
        .and(query_param("q", "abc"))
        .and(query_param("nsfw", "true"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"node":{"id":1}},{"node":{"id":2}},{"node":{"id":3}}]}),
            ),
        )
        .mount(&server)
        .await;
    let details = |id: i64, media: &str, delay: u64| {
        Mock::given(method("GET"))
            .and(path(format!("/v2/manga/{id}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":id,"title":format!("T{id}"),"num_chapters":3,"status":"finished","media_type":media}))
                    .set_delay(Duration::from_millis(delay)),
            )
            .mount(&server)
    };
    details(1, "light_novel", 100).await;
    details(2, "manga", 0).await;
    details(3, "manga", 10).await;

    let hits = tracker.api().search_manga("abc").await.unwrap();
    let ids: Vec<i64> = hits.iter().map(|h| h.remote_id).collect();
    assert_eq!(ids, vec![2, 3]);
}

#[tokio::test]
async fn anime_search_sends_one_q_truncated() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    let query = "x".repeat(80);
    let expected: String = "x".repeat(64);
    Mock::given(method("GET"))
        .and(path("/v2/anime"))
        .and(query_param("q", expected.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
        .expect(1)
        .mount(&server)
        .await;

    assert!(tracker.api().search_anime(&query).await.unwrap().is_empty());
    assert_eq!(query_count(&server, "q").await, 1);
}

#[tokio::test]
async fn id_prefix_is_direct_lookup() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":42,"title":"X","num_chapters":1,"status":"finished","media_type":"manga"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/manga"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
        .expect(0)
        .mount(&server)
        .await;

    let hits = tracker.api().search_manga("id:42").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].remote_id, 42);
}

#[tokio::test]
async fn my_manga_search_pages_offsets_zero_then_250() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    let page1: Vec<_> = (1..=250)
        .map(|i| json!({"node":{"id":i,"title":format!("Title {i}")}}))
        .collect();
    Mock::given(method("GET"))
        .and(path("/v2/users/@me/mangalist"))
        .and(query_param("limit", "250"))
        .and(query_param_is_missing("offset"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":page1,"paging":{"next":"https://x?offset=250"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me/mangalist"))
        .and(query_param("offset", "250"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"node":{"id":251,"title":"Last"}}],"paging":{}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v2/manga/\d+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":1,"title":"d","num_chapters":1,"status":"finished","media_type":"manga"}),
        ))
        .mount(&server)
        .await;

    let hits = tracker.api().search_manga("my:").await.unwrap();
    assert_eq!(hits.len(), 251);
    assert_eq!(hits[0].remote_id, 1);
    assert_eq!(hits[250].remote_id, 251);
    assert!(
        hits.iter().all(|h| h.from_my_list),
        "my: rows offer Refresh"
    );
}

#[tokio::test]
async fn my_anime_search_reads_animelist() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/users/@me/animelist"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"node":{"id":7,"title":"Show"}}],"paging":{}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/anime/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":7,"title":"Show","num_episodes":12,"status":"finished_airing","media_type":"tv"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me/mangalist"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let hits = tracker.api().search_anime("my:show").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].total_episodes, 12);
    assert!(hits[0].from_my_list);
}

#[tokio::test]
async fn list_entry_absent_sets_total_only() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10})))
        .mount(&server)
        .await;

    let mut track = manga_track(5, 6, 0.0, 0);
    assert!(!tracker.find_manga_list_item(&mut track).await.unwrap());
    assert_eq!(track.total_chapters, 10);
    assert_eq!(track.status, 6);
}

#[tokio::test]
async fn list_entry_present_copies_status_and_progress() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10,"my_list_status":{"is_rereading":false,"status":"reading","num_chapters_read":3.0,"score":7,"start_date":"2021-01-05","finish_date":null}})))
        .mount(&server)
        .await;

    let mut track = manga_track(5, 6, 0.0, 0);
    assert!(tracker.find_manga_list_item(&mut track).await.unwrap());
    assert_eq!(track.status, 1);
    assert_eq!(track.last_chapter_read, 3.0);
    assert_eq!(track.score, 7.0);
    assert!(track.start_date > 0);
    assert_eq!(track.finish_date, 0, "absent date keeps the local value");
}

#[tokio::test]
async fn put_manga_form_in_order_over_the_wire() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string("status=reading&is_rereading=false&score=8.0&num_chapters_read=12&start_date=&finish_date="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"is_rereading":false,"status":"reading","num_chapters_read":12.0,"score":8})))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 1, 12.7, 20);
    track.score = 8.0;
    tracker.put_manga(&mut track).await.unwrap();
    assert_eq!(track.last_chapter_read, 12.0);
}

#[tokio::test]
async fn put_anime_form_and_response_key() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("PUT"))
        .and(path("/v2/anime/9/my_list_status"))
        .and(body_string("status=watching&is_rewatching=false&score=0.0&num_watched_episodes=4&start_date=&finish_date="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"is_rewatching":false,"status":"watching","num_episodes_watched":4.0,"score":0})))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = anime_track(9, 1, 4.0, 12);
    tracker.put_anime(&mut track).await.unwrap();
    assert_eq!(track.last_episode_seen, 4.0);
}

#[tokio::test]
async fn delete_404_is_http_error() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("DELETE"))
        .and(path("/v2/manga/5/my_list_status"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = tracker.delete_manga(5).await.unwrap_err();
    assert_eq!(err.to_string(), "HTTP error 404");
}

#[tokio::test]
async fn set_score_rejects_unknown_label_without_request() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 1, 0.0, 0);
    track.score = 3.0;
    assert!(tracker.set_manga_score(&mut track, "foo").await.is_err());
    assert_eq!(track.score, 3.0);
}

#[tokio::test]
async fn set_score_stores_seven_and_puts_it() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string_starts(
            "status=reading&is_rereading=false&score=7.0",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":false,"status":"reading","num_chapters_read":0.0,"score":7}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 1, 0.0, 0);
    tracker.set_manga_score(&mut track, "7").await.unwrap();
    assert_eq!(track.score, 7.0);
}

#[tokio::test]
async fn set_completed_status_sets_progress_to_total() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string_starts(
            "status=completed&is_rereading=false&score=0.0&num_chapters_read=10",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":false,"status":"completed","num_chapters_read":10.0,"score":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 1, 2.0, 10);
    tracker.set_manga_status(&mut track, 2).await.unwrap();
    assert_eq!(track.last_chapter_read, 10.0);
}

#[tokio::test]
async fn set_last_chapter_completes_on_total() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string_starts("status=completed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":false,"status":"completed","num_chapters_read":10.0,"score":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 6, 0.0, 10);
    tracker
        .set_manga_last_chapter_read(&mut track, 10)
        .await
        .unwrap();
    assert_eq!(track.status, 2);
    assert!(track.finish_date > 0);
}

fn body_string_starts(prefix: &'static str) -> impl wiremock::Match {
    struct Starts(&'static str);
    impl wiremock::Match for Starts {
        fn matches(&self, request: &wiremock::Request) -> bool {
            std::str::from_utf8(&request.body).is_ok_and(|b| b.starts_with(self.0))
        }
    }
    Starts(prefix)
}

#[tokio::test]
async fn bind_manga_found_moves_plan_to_reading() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10,"my_list_status":{"is_rereading":false,"status":"plan_to_read","num_chapters_read":0.0,"score":0}})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string_starts("status=reading&is_rereading=false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":false,"status":"reading","num_chapters_read":0.0,"score":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 6, 0.0, 0);
    tracker.bind_manga(&mut track, true).await.unwrap();
    assert_eq!(track.status, 1);
}

#[tokio::test]
async fn bind_manga_not_found_creates_plan_entry() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string("status=plan_to_read&is_rereading=false&score=0.0&num_chapters_read=0&start_date=&finish_date="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"is_rereading":false,"status":"plan_to_read","num_chapters_read":0.0,"score":0})))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 0, 0.0, 0);
    tracker.bind_manga(&mut track, false).await.unwrap();
    assert_eq!(track.status, 6);
    assert_eq!(track.score, 0.0);
}

#[tokio::test]
async fn bind_manga_rereading_is_kept() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10,"my_list_status":{"is_rereading":true,"status":"reading","num_chapters_read":2.0,"score":0}})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .and(body_string_starts("status=reading&is_rereading=true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":true,"status":"reading","num_chapters_read":2.0,"score":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 7, 0.0, 0);
    tracker.bind_manga(&mut track, true).await.unwrap();
    assert_eq!(track.status, 7);
}

#[tokio::test]
async fn bind_anime_found_watching() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/anime/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_episodes":12,"my_list_status":{"is_rewatching":false,"status":"watching","num_episodes_watched":0.0,"score":0}})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v2/anime/9/my_list_status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rewatching":false,"status":"watching","num_episodes_watched":0.0,"score":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = anime_track(9, 16, 0.0, 0);
    tracker.bind_anime(&mut track, true).await.unwrap();
    assert_eq!(track.status, 11);
}

#[cfg(unix)]
#[tokio::test]
async fn config_file_is_private_and_holds_no_token() {
    use std::os::unix::fs::PermissionsExt;

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker: MalTracker = tracker_for(&server, dir.path(), Arc::new(MemorySecrets::default()));
    logged_in(&server, &tracker).await;

    let file = dir.path().join("tracking").join("myanimelist.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("access_token") && !text.contains("new-access"));
    assert!(text.contains("fixture_user"));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}
