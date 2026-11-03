//! Shikimori integration tests TS-S-01 to TS-S-19.
//! Wiremock only; the bodies are the fixtures under `tests/fixtures/shikimori/`.

use kuro_platform::kind::Kind;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kuro_platform::secrets::{SecretError, SecretStore};
use kuro_track::model::{MangaTrack, TrackerId};
use kuro_track::shikimori::api::{Endpoints, OAuthClient, Rating};
use kuro_track::shikimori::auth::is_expired;
use kuro_track::shikimori::dto::SmOAuth;
use kuro_track::shikimori::error::ShikimoriError;
use kuro_track::shikimori::status::ShikiStatus;
use kuro_track::shikimori::{ShikimoriTracker, next_status_after_read};
use kuro_track::tracker::Tracker;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REDIRECT: &str = "kurobayomi://shikimori-auth";
const TOKEN: &str = include_str!("fixtures/shikimori/oauth_token.json");
const TOKEN_NO_REFRESH: &str = include_str!("fixtures/shikimori/oauth_token_no_refresh.json");
const WHOAMI: &str = include_str!("fixtures/shikimori/whoami.json");
const MANGAS_SEARCH: &str = include_str!("fixtures/shikimori/mangas_search.json");
const MANGAS_1: &str = include_str!("fixtures/shikimori/mangas_1.json");
const RATES_ONE: &str = include_str!("fixtures/shikimori/user_rates_one.json");
const RATES_TWO: &str = include_str!("fixtures/shikimori/user_rates_two.json");
const RATES_EMPTY: &str = include_str!("fixtures/shikimori/user_rates_empty.json");
const RATE_CREATE: &str = include_str!("fixtures/shikimori/user_rate_create.json");

#[derive(Default)]
struct MemSecrets(Mutex<BTreeMap<(String, String), String>>);

impl MemSecrets {
    fn accounts(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().map(|k| k.1.clone()).collect()
    }
}

impl SecretStore for MemSecrets {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        let key = (service.to_owned(), account.to_owned());
        Ok(self.0.lock().unwrap().get(&key).cloned())
    }
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError> {
        let key = (service.to_owned(), account.to_owned());
        self.0.lock().unwrap().insert(key, secret.to_owned());
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        let key = (service.to_owned(), account.to_owned());
        self.0.lock().unwrap().remove(&key);
        Ok(())
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The fixture token with `created_at` moved to `created`.
fn token_at(fixture: &str, created: i64) -> SmOAuth {
    let mut token: SmOAuth = serde_json::from_str(fixture).unwrap();
    token.created_at = created;
    token
}

fn tracker_for(server: &MockServer, secrets: Arc<MemSecrets>) -> ShikimoriTracker {
    ShikimoriTracker::new(
        Endpoints { base: server.uri() },
        Some(OAuthClient {
            client_id: "cid".into(),
            client_secret: "sec".into(),
        }),
        REDIRECT,
        secrets,
    )
    .unwrap()
}

/// Logged in as user 1234 with a fresh token.
fn logged_in(server: &MockServer) -> (ShikimoriTracker, Arc<MemSecrets>) {
    let secrets = Arc::new(MemSecrets::default());
    let t = tracker_for(server, secrets.clone());
    let fresh = token_at(TOKEN, now());
    t.api()
        .base()
        .save_oauth("1234", &fresh.access_token, &fresh)
        .unwrap();
    t.api().tokens().login(fresh);
    (t, secrets)
}

fn manga(status: i64, read: f64, total: i64) -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TrackerId::SHIKIMORI,
        remote_id: 5,
        library_id: None,
        title: "T".into(),
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

fn json_body(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::from_str::<Value>(text).unwrap())
}

async fn mount_get(server: &MockServer, route: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(json_body(body))
        .mount(server)
        .await;
}

async fn post_bodies(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| String::from_utf8(r.body.clone()).unwrap())
        .collect()
}

#[test]
fn ts_s_01_status_round_trip() {
    let strings = [
        "watching",
        "completed",
        "on_hold",
        "dropped",
        "planned",
        "rewatching",
    ];
    for (value, wire) in (1..=6).zip(strings) {
        let status = ShikiStatus::from_internal(value).unwrap();
        assert_eq!(status.api_str(), wire);
        assert_eq!(ShikiStatus::from_api(wire), Ok(status));
    }
    assert_eq!(ShikiStatus::from_internal(0), None);
}

#[test]
fn ts_s_02_expiry_boundary() {
    let token = token_at(TOKEN, 1_000);
    let edge = 1_000 + 86_400 - 3_600;
    assert!(
        !is_expired(&token, edge),
        "EC-S-05: exactly at the edge is not expired"
    );
    assert!(is_expired(&token, edge + 1), "EC-S-06: one second later is");
}

#[tokio::test]
async fn ts_s_03_search_logged_out_sends_no_bearer() {
    // T-31a-045 (D9): the public catalogue is searched anonymously, so the request carries no `Authorization`.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/mangas"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .expect(1)
        .mount(&server)
        .await;
    let t = tracker_for(&server, Arc::new(MemSecrets::default()));

    let result = t.as_manga().unwrap().search("berserk").await.unwrap();

    assert!(result.is_empty());
    let sent = server.received_requests().await.unwrap();
    assert!(sent[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn ts_s_04_single_flight_refresh() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "fresh", "token_type": "Bearer", "created_at": now(), "expires_in": 86400, "refresh_token": "r2"}))
                .set_delay(Duration::from_millis(100)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    t.api().tokens().login(token_at(TOKEN, 0));

    let results = futures::future::join_all((0..10).map(|_| t.api().bearer())).await;

    assert!(results.iter().all(|r| r.as_deref() == Ok("fresh")));
}

#[tokio::test]
async fn ts_s_05_refresh_rejected_400() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    t.api().tokens().login(token_at(TOKEN, 0));

    let result = t.as_manga().unwrap().search("x").await;

    assert!(result.is_err());
    assert!(t.auth_expired());
}

#[tokio::test]
async fn ts_s_06_refresh_transport_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    t.api().tokens().login(token_at(TOKEN, 0));

    assert_eq!(t.api().bearer().await, Err(ShikimoriError::Http(500)));
    assert!(t.api().tokens().has_token(), "the token is kept");
    assert_eq!(
        t.api().bearer().await,
        Err(ShikimoriError::Http(500)),
        "the next call retries"
    );
    assert!(!t.auth_expired());
}

#[tokio::test]
async fn ts_s_07_refresh_token_absent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    t.api().tokens().login(token_at(TOKEN_NO_REFRESH, now()));
    assert_eq!(t.api().bearer().await.as_deref(), Ok("fx-access-1"));

    t.api().tokens().login(token_at(TOKEN_NO_REFRESH, 0));
    assert_eq!(t.api().bearer().await, Err(ShikimoriError::AuthExpired));
    assert!(t.auth_expired());
}

#[tokio::test]
async fn ts_s_08_login_code_flow() {
    let server = MockServer::start().await;
    let fresh = serde_json::to_string(&token_at(TOKEN, now())).unwrap();
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(json_body(&fresh))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/users/whoami"))
        .and(header("Authorization", "Bearer fx-access-1"))
        .respond_with(json_body(WHOAMI))
        .mount(&server)
        .await;
    let secrets = Arc::new(MemSecrets::default());
    let t = tracker_for(&server, secrets.clone());

    t.login("", "abc").await.unwrap();

    assert_eq!(
        (t.username().as_str(), t.password().as_str()),
        ("1234", "fx-access-1")
    );
    assert!(
        secrets.accounts().iter().any(|a| a == "tracker/4/oauth"),
        "{:?}",
        secrets.accounts()
    );
}

#[tokio::test]
async fn ts_s_08_a_refused_code_or_a_failed_whoami_stores_nothing() {
    let refused = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&refused)
        .await;
    let secrets = Arc::new(MemSecrets::default());
    let t = tracker_for(&refused, secrets.clone());
    assert!(t.login("", "abc").await.is_err());
    assert!(secrets.accounts().is_empty(), "EC-S-09");

    let broken = MockServer::start().await;
    let fresh = serde_json::to_string(&token_at(TOKEN, now())).unwrap();
    Mock::given(method("POST"))
        .respond_with(json_body(&fresh))
        .mount(&broken)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/users/whoami"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&broken)
        .await;
    let secrets = Arc::new(MemSecrets::default());
    let t = tracker_for(&broken, secrets.clone());
    assert!(t.login("", "abc").await.is_err());
    assert!(secrets.accounts().is_empty(), "EC-S-10: no token stored");
}

#[tokio::test]
async fn ts_s_08_a_failed_second_login_keeps_the_first() {
    // RC-06: the double redirect must not clear a valid login.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    assert!(t.login("", "abc").await.is_err());

    assert!(t.is_logged_in());
    assert_eq!(t.username(), "1234");
}

#[tokio::test]
async fn ts_s_09_find_lib_errors() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server);

    // The catalogue 404 aborts before the user rates are asked.
    Mock::given(method("GET"))
        .and(path("/api/mangas/5"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    assert_eq!(
        t.api().find_library(Kind::Manga, 5, "1234").await,
        Err(ShikimoriError::Http(404))
    );
    server.reset().await;

    mount_get(&server, "/api/mangas/5", MANGAS_1).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/user_rates"))
        .respond_with(json_body(RATES_TWO))
        .mount(&server)
        .await;
    assert_eq!(
        t.api().find_library(Kind::Manga, 5, "1234").await,
        Err(ShikimoriError::TooManyEntries)
    );
    server.reset().await;

    mount_get(&server, "/api/mangas/5", MANGAS_1).await;
    mount_get(&server, "/api/v2/user_rates", RATES_EMPTY).await;
    assert_eq!(t.api().find_library(Kind::Manga, 5, "1234").await, Ok(None));
}

#[tokio::test]
async fn ts_s_10_bind_not_found_adds_the_entry() {
    let server = MockServer::start().await;
    mount_get(&server, "/api/mangas/5", MANGAS_1).await;
    mount_get(&server, "/api/v2/user_rates", RATES_EMPTY).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/user_rates"))
        .respond_with(json_body(RATE_CREATE))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    let mut read = manga(1, 2.0, 12);
    read.score = 5.0;
    t.as_manga().unwrap().bind(&mut read, true).await.unwrap();
    assert_eq!(
        (read.status, read.score, read.library_id),
        (1, 0.0, Some(77))
    );

    let mut unread = manga(1, 0.0, 12);
    t.as_manga()
        .unwrap()
        .bind(&mut unread, false)
        .await
        .unwrap();
    assert_eq!(
        (unread.status, unread.score, unread.library_id),
        (5, 0.0, Some(77))
    );
}

#[tokio::test]
async fn ts_s_10_logged_out_bind_sends_no_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let t = tracker_for(&server, Arc::new(MemSecrets::default()));

    assert!(
        t.as_manga()
            .unwrap()
            .bind(&mut manga(1, 0.0, 12), true)
            .await
            .is_err()
    );
}

#[test]
fn ts_s_11_update_transitions() {
    use ShikiStatus::*;
    // (status, did_read, last, total, expected)
    let table = [
        (Reading, true, 12.0, 12, Completed),
        (Rereading, true, 3.0, 12, Rereading),
        (Dropped, true, 3.0, 12, Reading),
        (Completed, true, 3.0, 12, Completed),
        (Reading, false, 12.0, 12, Reading),
        (Reading, true, 12.0, 0, Reading),
    ];
    for (status, did_read, last, total, expected) in table {
        assert_eq!(
            next_status_after_read(status, did_read, last, total),
            expected,
            "{status:?}"
        );
    }
}

#[tokio::test]
async fn ts_s_12_search_mapping() {
    let server = MockServer::start().await;
    mount_get(&server, "/api/mangas", MANGAS_SEARCH).await;
    let (t, _) = logged_in(&server);

    let found = t.as_manga().unwrap().search("x").await.unwrap();

    let titles: Vec<_> = found.iter().map(|f| f.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Berserk \u{1f5e1}", "Undated Manhwa"],
        "EC-S-12: the record without chapters is skipped, EC-S-32"
    );
    assert_eq!(found[0].start_date, "1989-08-25");
    assert_eq!(found[1].start_date, "", "EC-S-13");
    assert_eq!(found[1].publishing_type, "manhwa", "EC-S-28");
}

#[tokio::test]
async fn ts_s_13_score_clamp_on_write() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_body(RATE_CREATE))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    for score in [-1.0, 10.7] {
        let mut m = manga(1, 1.0, 12);
        m.score = score;
        t.as_manga().unwrap().update(&m, false).await.unwrap();
    }

    let bodies = post_bodies(&server).await;

    assert!(bodies[0].contains(r#""score":0,"#), "{}", bodies[0]);
    assert!(bodies[1].contains(r#""score":10,"#), "{}", bodies[1]);
}

#[tokio::test]
async fn ts_s_14_bind_keeps_dates() {
    let server = MockServer::start().await;
    mount_get(&server, "/api/mangas/5", MANGAS_1).await;
    mount_get(&server, "/api/v2/user_rates", RATES_ONE).await;
    Mock::given(method("POST"))
        .respond_with(json_body(RATE_CREATE))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut m = manga(1, 0.0, 12);
    m.start_date = 1_700_000_000_000;
    m.finish_date = 1_700_000_100_000;

    t.as_manga().unwrap().bind(&mut m, true).await.unwrap();

    assert_eq!(
        (m.start_date, m.finish_date),
        (1_700_000_000_000, 1_700_000_100_000)
    );
}

#[tokio::test]
async fn ts_s_15_logout_then_stale_refresh() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "late", "token_type": "Bearer", "created_at": now(), "expires_in": 86400, "refresh_token": "r2"}))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;
    let (t, secrets) = logged_in(&server);
    t.api().tokens().login(token_at(TOKEN, 0));

    let refresh = t.api().bearer();
    let logout = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        t.logout().unwrap();
    };
    let (result, ()) = tokio::join!(refresh, logout);

    assert_eq!(result, Err(ShikimoriError::NotAuthenticated));
    assert!(
        secrets.accounts().iter().all(|a| a != "tracker/4/oauth"),
        "{:?}",
        secrets.accounts()
    );
    assert_eq!((t.username().as_str(), t.password().as_str()), ("", ""));
}

#[tokio::test]
async fn ts_s_16_delete_remote() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/user_rates/99"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/user_rates/98"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let deletable = t.as_deletable_manga().unwrap();

    let mut m = manga(1, 0.0, 0);
    m.library_id = Some(99);
    deletable.delete(&m).await.unwrap();

    assert!(
        deletable.delete(&manga(1, 0.0, 0)).await.is_err(),
        "EC-S-25: no library id, no request"
    );

    m.library_id = Some(98);
    deletable.delete(&m).await.unwrap();
}

#[tokio::test]
async fn ts_s_17_unknown_status_no_panic() {
    let server = MockServer::start().await;
    mount_get(&server, "/api/mangas/5", MANGAS_1).await;
    let rows = RATES_ONE.replace("watching", "rewatching_again");
    mount_get(&server, "/api/v2/user_rates", &rows).await;
    let (t, _) = logged_in(&server);

    assert_eq!(
        t.api().find_library(Kind::Manga, 5, "1234").await,
        Err(ShikimoriError::UnknownStatus("rewatching_again".into()))
    );
}

#[tokio::test]
async fn ts_s_18_parallel_upserts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/user_rates"))
        .respond_with(json_body(RATE_CREATE))
        .expect(2)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let m = manga(1, 3.0, 12);

    let tracker = t.as_manga().unwrap();
    let (a, b) = tokio::join!(tracker.update(&m, false), tracker.update(&m, false));

    assert_eq!(
        (a.unwrap().library_id, b.unwrap().library_id),
        (Some(77), Some(77))
    );
}

#[tokio::test]
async fn ts_s_19_body_key_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_body(RATE_CREATE))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    for kind in [Kind::Manga, Kind::Anime] {
        let rating = Rating {
            kind,
            user_id: "1234".into(),
            target_id: 5,
            progress: 3.0,
            score: 7.0,
            status: ShikiStatus::Reading,
        };
        t.api().upsert(&rating).await.unwrap();
    }

    assert_eq!(
        post_bodies(&server).await,
        [
            r#"{"user_rate":{"user_id":"1234","target_id":5,"target_type":"Manga","chapters":3,"score":7,"status":"watching"}}"#,
            r#"{"user_rate":{"user_id":"1234","target_id":5,"target_type":"Anime","episodes":3,"score":7,"status":"watching"}}"#,
        ],
        "EC-S-31: user_id is a string"
    );
}
