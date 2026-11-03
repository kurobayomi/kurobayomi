//! Kitsu tests that run on the recorded fixtures of `tests/fixtures/kitsu/` (T-30a-033), and
//! the failure-mode tests KT-93, KT-96, KT-98 and KT-99. The fixtures are shaped like Kitsu answers and hold no
//! credentials. `tests/T_ID_MAP.md` maps every KT id to its test.

use kuro_platform::kind::Kind;
use std::sync::Arc;
use std::time::Duration;

use kuro_platform::secrets::SecretStore;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::api::{self, Endpoints, KitsuApi};
use super::auth::{self, OAuth};
use super::error::KitsuError;
use super::search::{FILTER_ANIME, FILTER_MANGA};
use super::tracker::KitsuTracker;
use super::{COMPLETED, PLAN_TO_READ, READING};
use crate::base::{Field, SECRET_SERVICE, account};
use crate::fakes::MemSecrets;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::tracker::Tracker;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../tests/fixtures/kitsu/", $name))
    };
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn endpoints(server: &MockServer) -> Endpoints {
    Endpoints {
        token_url: format!("{}/api/oauth/token", server.uri()),
        api_url: format!("{}/api/edge/", server.uri()),
        algolia_url: format!("{}/1/indexes/production_media/query/", server.uri()),
    }
}

/// An address nothing listens on, so every call fails to connect.
fn dead_endpoints() -> Endpoints {
    Endpoints {
        token_url: "http://127.0.0.1:1/api/oauth/token".into(),
        api_url: "http://127.0.0.1:1/api/edge/".into(),
        algolia_url: "http://127.0.0.1:1/1/indexes/production_media/query/".into(),
    }
}

/// A token fixture made current, so it is not refreshed before its first use.
fn fresh(text: &str) -> Value {
    let mut v: Value = serde_json::from_str(text).unwrap();
    v["created_at"] = json!(now_secs());
    v
}

fn fresh_oauth(text: &str) -> OAuth {
    serde_json::from_value(fresh(text)).unwrap()
}

/// A tracker with the stored token fixture made current and user id 42.
fn logged_in(server: &MockServer) -> (KitsuTracker, Arc<dyn SecretStore>) {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    auth::save(secrets.as_ref(), &fresh_oauth(fixture!("token_ok.json"))).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    (
        KitsuTracker::new(endpoints(server), secrets.clone()).unwrap(),
        secrets,
    )
}

fn json_body(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(text.to_owned(), "application/json")
}

async fn mount(server: &MockServer, verb: &'static str, at: &str, text: &str) {
    Mock::given(method(verb))
        .and(path(at.to_owned()))
        .respond_with(json_body(text))
        .mount(server)
        .await;
}

fn manga() -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TrackerId::KITSU,
        remote_id: 77,
        library_id: None,
        title: "Fixture".into(),
        last_chapter_read: 0.0,
        total_chapters: 0,
        status: PLAN_TO_READ,
        score: 0.0,
        remote_url: "https://kitsu.app/manga/77".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

fn anime() -> AnimeTrack {
    AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TrackerId::KITSU,
        remote_id: 9002,
        library_id: None,
        title: "Fixture Anime".into(),
        last_episode_seen: 0.0,
        total_episodes: 0,
        status: 0,
        score: 0.0,
        remote_url: "https://kitsu.app/anime/201".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

// ---- login, session and search on the fixtures (KT-23, KT-91, KT-96) ----

#[tokio::test]
async fn t_30a_033_login_search_after_login_and_no_bearer_on_the_login_post() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fresh(fixture!("token_ok.json"))))
        .mount(&server)
        .await;
    mount(&server, "GET", "/api/edge/users", fixture!("users_ok.json")).await;
    mount(
        &server,
        "GET",
        "/api/edge/algolia-keys/media/",
        fixture!("key_ok.json"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/1/indexes/production_media/query/"))
        .and(header("X-Algolia-API-Key", "k1"))
        .respond_with(json_body(fixture!("algolia_manga_hits.json")))
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = KitsuTracker::new(endpoints(&server), secrets.clone()).unwrap();

    // KT-96: without a login the search is refused, and it works once the login finished.
    let early = t.as_manga().unwrap().search("fixture").await.unwrap_err();
    assert_eq!(early.to_string(), "Not authenticated with Kitsu");
    t.login("ann@example.org", "hunter2").await.unwrap();
    let found = t.as_manga().unwrap().search("fixture").await.unwrap();
    assert_eq!(found.len(), 1, "the novel hit is dropped");
    assert_eq!(found[0].title, "Fixture Manga");
    assert_eq!(found[0].tracking_url, "https://kitsu.app/manga/101");

    // KT-23: logged in, username and user id stored.
    assert!(t.is_logged_in());
    assert_eq!(t.username(), "ann@example.org");
    assert_eq!(
        secrets
            .get(SECRET_SERVICE, &auth::user_id_account())
            .unwrap()
            .as_deref(),
        Some("42")
    );
    // KT-91: the token POST carries no bearer.
    let login_post = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == "/api/oauth/token")
        .unwrap();
    assert!(login_post.headers.get("authorization").is_none());
}

#[tokio::test]
async fn t_30a_033_an_empty_users_answer_is_no_user_and_stores_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fresh(fixture!("token_ok.json"))))
        .mount(&server)
        .await;
    mount(
        &server,
        "GET",
        "/api/edge/users",
        fixture!("users_empty.json"),
    )
    .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = KitsuTracker::new(endpoints(&server), secrets.clone()).unwrap();

    let err = t.login("ann", "pw").await.unwrap_err();

    assert_eq!(err.to_string(), "Missing User data from Kitsu");
    assert_eq!(auth::load(&secrets), None);
    assert!(!t.is_logged_in());
}

// ---- refresh on the token fixtures (KT-27, KT-29) ----

#[tokio::test]
async fn t_30a_033_a_refresh_answer_without_a_refresh_token_keeps_the_old_one() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fresh(fixture!("token_no_refresh.json"))),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/edge/library-entries"))
        .and(header("Authorization", "Bearer a2"))
        .respond_with(json_body(fixture!("list_manga_empty.json")))
        .expect(1)
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let expired: OAuth = serde_json::from_str(fixture!("token_expired.json")).unwrap();
    auth::save(secrets.as_ref(), &expired).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    let api = KitsuApi::new(endpoints(&server), secrets.clone()).unwrap();

    assert_eq!(api.find_entry(Kind::Manga, 77).await.unwrap(), None);

    let stored = auth::load(&secrets).unwrap();
    assert_eq!(stored.access_token, "a2");
    assert_eq!(stored.refresh_token.as_deref(), Some("r1"));
}

#[tokio::test]
async fn t_30a_033_a_refresh_that_cannot_connect_sends_the_request_with_the_stale_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/edge/library-entries"))
        .and(header("Authorization", "Bearer a1"))
        .respond_with(json_body(fixture!("list_manga_empty.json")))
        .expect(1)
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let expired: OAuth = serde_json::from_str(fixture!("token_expired.json")).unwrap();
    auth::save(secrets.as_ref(), &expired).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    let mut ends = endpoints(&server);
    ends.token_url = "http://127.0.0.1:1/api/oauth/token".into();
    let api = KitsuApi::new(ends, secrets).unwrap();

    assert_eq!(api.find_entry(Kind::Manga, 77).await.unwrap(), None);
}

// ---- search on the Algolia fixtures (KT-71, KT-72, KT-77, KT-80) ----

#[test]
fn t_30a_033_the_filter_constants_equal_the_fixture_files() {
    assert_eq!(
        FILTER_MANGA,
        fixture!("algolia_filter_manga.txt").trim_end()
    );
    assert_eq!(
        FILTER_ANIME,
        fixture!("algolia_filter_anime.txt").trim_end()
    );
}

#[tokio::test]
async fn t_30a_033_an_anime_hit_uses_the_episode_count_and_the_anime_url() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/edge/algolia-keys/media/",
        fixture!("key_ok.json"),
    )
    .await;
    mount(
        &server,
        "POST",
        "/1/indexes/production_media/query/",
        fixture!("algolia_anime_hits.json"),
    )
    .await;
    let (t, _) = logged_in(&server);

    let found = t.as_anime().unwrap().search("fixture").await.unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].total_episodes, 12);
    assert_eq!(found[0].tracking_url, "https://kitsu.app/anime/201");
}

#[tokio::test]
async fn t_30a_033_a_rotated_key_is_fetched_again_and_the_search_is_retried_once() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/edge/algolia-keys/media/"))
        .respond_with(json_body(fixture!("key_ok.json")))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/1/indexes/production_media/query/"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_raw(fixture!("key_rotated_403.json"), "application/json"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount(
        &server,
        "POST",
        "/1/indexes/production_media/query/",
        fixture!("algolia_manga_hits.json"),
    )
    .await;
    let (t, _) = logged_in(&server);

    let found = t.as_manga().unwrap().search("fixture").await.unwrap();

    assert_eq!(found.len(), 1);
}

// ---- library entries on the list fixtures (KT-42, KT-60, KT-61, KT-66, KT-87) ----

#[tokio::test]
async fn t_30a_033_bind_to_a_found_manga_entry_copies_it_and_keeps_the_tracking_url() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/edge/library-entries",
        fixture!("list_manga_found.json"),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(path("/api/edge/library-entries/9001"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut m = manga();

    t.as_manga().unwrap().bind(&mut m, true).await.unwrap();

    assert_eq!(
        m.remote_id, 9001,
        "the library entry id replaces the media id"
    );
    assert_eq!(
        (m.status, m.last_chapter_read, m.score),
        (READING, 7.0, 8.0)
    );
    assert_eq!(m.total_chapters, 0, "KT-87: a null chapter count is 0");
    assert_eq!(m.start_date, 1_714_521_600_000);
    assert_eq!(m.finish_date, 0, "KT-87: a null finish date is 0");
    assert_eq!(m.remote_url, "https://kitsu.app/manga/77", "KT-66");
}

#[tokio::test]
async fn t_30a_033_an_empty_list_is_no_entry_for_find_and_not_found_for_get() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/edge/library-entries",
        fixture!("list_manga_empty.json"),
    )
    .await;
    let (t, _) = logged_in(&server);

    assert_eq!(t.api().find_entry(Kind::Manga, 77).await.unwrap(), None);
    assert_eq!(
        t.api().get_entry(Kind::Manga, 9001).await.unwrap_err(),
        KitsuError::NotFound("manga")
    );
}

#[tokio::test]
async fn t_30a_033_refresh_of_an_anime_entry_reads_status_score_and_the_episode_total() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/edge/library-entries",
        fixture!("list_anime_found.json"),
    )
    .await;
    let (t, _) = logged_in(&server);

    let out = t.as_anime().unwrap().refresh(&anime()).await.unwrap();

    assert_eq!(out.status, COMPLETED);
    assert_eq!(
        (out.last_episode_seen, out.score, out.total_episodes),
        (12.0, 10.0, 12)
    );
    assert_eq!(out.finish_date, 1_717_200_000_000);
}

#[tokio::test]
async fn t_30a_033_the_add_response_gives_the_new_library_id() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/api/edge/library-entries",
        fixture!("add_response.json"),
    )
    .await;
    mount(
        &server,
        "GET",
        "/api/edge/library-entries",
        fixture!("list_manga_empty.json"),
    )
    .await;
    let (t, _) = logged_in(&server);
    let mut m = manga();

    t.as_manga().unwrap().bind(&mut m, false).await.unwrap();

    assert_eq!(m.remote_id, 555);
}

// ---- failure modes (KT-93, KT-98, KT-99) ----

#[test]
fn t_30a_033_the_timeouts_are_thirty_thirty_and_one_hundred_twenty_seconds() {
    assert_eq!(api::CONNECT_TIMEOUT, Duration::from_secs(30));
    assert_eq!(api::READ_TIMEOUT, Duration::from_secs(30));
    assert_eq!(api::CALL_TIMEOUT, Duration::from_secs(120));
}

#[tokio::test]
async fn t_30a_033_a_slow_server_fails_with_the_read_timeout_and_the_call_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            json_body(fixture!("list_manga_empty.json")).set_delay(Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    let slow = Duration::from_millis(50);
    let long = Duration::from_secs(30);

    // Same client builder as production, with the limits shortened. Read timeout alone, then call timeout alone.
    for (read, call) in [(slow, long), (long, slow)] {
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        auth::save(secrets.as_ref(), &fresh_oauth(fixture!("token_ok.json"))).unwrap();
        secrets
            .set(SECRET_SERVICE, &auth::user_id_account(), "42")
            .unwrap();
        let mut api = KitsuApi::new(endpoints(&server), secrets).unwrap();
        api.http = api::http_client(long, read, call).unwrap();
        let err = api.find_entry(Kind::Manga, 77).await.unwrap_err();
        assert!(matches!(err, KitsuError::Transport(_)), "{err:?}");
    }
}

#[tokio::test]
async fn t_30a_033_offline_every_call_is_a_transport_error_and_changes_nothing() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    auth::save(secrets.as_ref(), &fresh_oauth(fixture!("token_ok.json"))).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    let t = KitsuTracker::new(dead_endpoints(), secrets.clone()).unwrap();
    let mut m = manga();
    let before = m.clone();

    let bind = t.as_manga().unwrap().bind(&mut m, true).await;
    let login = t.login("ann", "pw").await;

    assert!(bind.is_err() && login.is_err());
    assert_eq!(m, before, "a failed bind leaves the track alone");
    assert!(t.is_logged_in(), "a failed login keeps the earlier session");
    assert_eq!(
        secrets
            .get(SECRET_SERVICE, &account(TrackerId::KITSU, Field::Username))
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn t_30a_033_a_429_is_an_http_error_without_a_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/edge/library-entries"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    let err = t.api().find_entry(Kind::Manga, 77).await.unwrap_err();

    assert_eq!(err, KitsuError::Http(429));
}

// ---- KT-25, KT-74, KT-76 ----

#[tokio::test]
async fn t_30a_033_a_refused_password_is_an_http_401_and_saves_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/token"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = KitsuTracker::new(endpoints(&server), secrets.clone()).unwrap();

    let err = t.login("ann", "wrong").await.unwrap_err();

    assert_eq!(err.to_string(), "HTTP error 401");
    assert_eq!(auth::load(&secrets), None);
    assert!(!t.is_logged_in());
}

#[tokio::test]
async fn t_30a_033_an_empty_query_is_sent_as_it_is_and_the_key_is_fetched_once_for_two_searches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/edge/algolia-keys/media/"))
        .respond_with(json_body(fixture!("key_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/1/indexes/production_media/query/"))
        .and(wiremock::matchers::body_json(
            json!({"params": format!("query={FILTER_MANGA}")}),
        ))
        .respond_with(json_body(fixture!("algolia_manga_hits.json")))
        .expect(2)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    t.as_manga().unwrap().search("").await.unwrap();
    t.as_manga().unwrap().search("").await.unwrap();
}
