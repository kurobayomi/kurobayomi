//! Bangumi integration tests TS-B-01 to TS-B-14.
//! Wiremock only; the bodies are the fixtures under `tests/fixtures/bangumi/`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use kuro_platform::secrets::{SecretError, SecretStore};
use kuro_track::bangumi::BangumiTracker;
use kuro_track::bangumi::api::{CollectionWrite, Endpoints};
use kuro_track::bangumi::dto::{BgmOAuth, now_secs};
use kuro_track::bangumi::error::BangumiError;
use kuro_track::bangumi::status::BgmStatus;
use kuro_track::model::{MangaTrack, TrackerId};
use kuro_track::shikimori::api::OAuthClient;
use kuro_track::tracker::Tracker;
use serde_json::Value;
use wiremock::matchers::{body_string, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REDIRECT: &str = "kurobayomi://bangumi-auth";
const REFRESH_BODY: &str = "grant_type=refresh_token&client_id=cid&client_secret=sec&refresh_token=fx-b-refresh&redirect_uri=kurobayomi%3A%2F%2Fbangumi-auth";
const TOKEN: &str = include_str!("fixtures/bangumi/oauth_token.json");
const REFRESH_DOCUMENTED: &str = include_str!("fixtures/bangumi/oauth_refresh_documented.json");
const SEARCH_MANGA: &str = include_str!("fixtures/bangumi/search_manga.json");
const SEARCH_ANIME: &str = include_str!("fixtures/bangumi/search_anime.json");
const DOING: &str = include_str!("fixtures/bangumi/collection_doing.json");
const UNKNOWN_TYPE: &str = include_str!("fixtures/bangumi/collection_unknown_type.json");
const NO_RATE: &str = include_str!("fixtures/bangumi/collection_no_rate.json");

#[derive(Default)]
struct MemSecrets(Mutex<BTreeMap<(String, String), String>>);

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

fn token_at(created: i64, refresh: bool) -> BgmOAuth {
    let mut token: BgmOAuth = serde_json::from_str(TOKEN).unwrap();
    token.created_at = created;
    if !refresh {
        token.refresh_token = None;
    }
    token
}

fn tracker_for(server: &MockServer) -> BangumiTracker {
    BangumiTracker::new(
        Endpoints {
            api: server.uri(),
            web: server.uri(),
        },
        Some(OAuthClient {
            client_id: "cid".into(),
            client_secret: "sec".into(),
        }),
        REDIRECT,
        Arc::new(MemSecrets::default()),
    )
    .unwrap()
}

/// Logged in as `fixture_user` with a fresh token.
fn logged_in(server: &MockServer) -> BangumiTracker {
    let t = tracker_for(server);
    let fresh = token_at(now_secs(), true);
    t.api()
        .base()
        .save_oauth("fixture_user", &fresh.access_token, &fresh)
        .unwrap();
    t.api().tokens().login(fresh);
    t
}

fn manga(status: i64, read: f64) -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TrackerId::BANGUMI,
        remote_id: 5,
        library_id: None,
        title: "T".into(),
        last_chapter_read: read,
        total_chapters: 0,
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

async fn mount_collection(server: &MockServer, body: &str) {
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/5"))
        .respond_with(json_body(body))
        .mount(server)
        .await;
}

async fn bodies(server: &MockServer, verb: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb)
        .map(|r| String::from_utf8(r.body).unwrap())
        .collect()
}

#[test]
fn ts_b_01_status_round_trip() {
    for value in 1..=5 {
        let status = BgmStatus::from_internal(value).unwrap();
        assert_eq!(status.collection_type(), value);
        assert_eq!(BgmStatus::from_collection_type(Some(value)), Ok(status));
    }
    assert_eq!(
        BgmStatus::from_collection_type(Some(6)),
        Err(BangumiError::UnknownStatus("6".into()))
    );
}

#[tokio::test]
async fn ts_b_02_search_mapping() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string(
            r#"{"keyword":"x","sort":"match","filter":{"type":[1]}}"#,
        ))
        .respond_with(json_body(SEARCH_MANGA))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string(
            r#"{"keyword":"x","sort":"match","filter":{"type":[2]}}"#,
        ))
        .respond_with(json_body(SEARCH_ANIME))
        .mount(&server)
        .await;
    let t = logged_in(&server);

    let manga_hits = t.as_manga().unwrap().search("x").await.unwrap();
    let anime_hits = t.as_anime().unwrap().search("x").await.unwrap();

    // EC-B-27: a Chinese name puts the original name first, then the trimmed summary.
    assert_eq!(manga_hits[0].title, "死神");
    assert_eq!(manga_hits[0].summary, "作品原名：Bleach\nhi");
    // EC-B-25: a blank Chinese name falls back to the original name, and a null summary is empty.
    assert_eq!(manga_hits[1].title, "Bleach");
    assert_eq!(manga_hits[1].summary, "");
    assert_eq!(manga_hits[1].total_chapters, 0, "EC-B-30");
    assert_eq!(manga_hits[0].start_date, "2001-08-07");
    // EC-B-26, EC-B-28, EC-B-29: null summary, rating and date.
    assert_eq!(anime_hits[0].summary, "作品原名：Bleach");
    assert_eq!(anime_hits[0].score, None, "no rating has no score");
    assert_eq!(anime_hits[0].start_date, "");
    assert_eq!(anime_hits[0].total_episodes, 366);
}

#[tokio::test]
async fn ts_b_03_search_filters() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(query_param("limit", "20"))
        .respond_with(json_body(SEARCH_MANGA))
        .mount(&server)
        .await;
    let t = logged_in(&server);

    let manga_hits = t.as_manga().unwrap().search("x").await.unwrap();
    let anime_hits = t.as_anime().unwrap().search("x").await.unwrap();

    assert_eq!(
        manga_hits.iter().map(|m| m.remote_id).collect::<Vec<_>>(),
        [101, 102],
        "EC-B-23: the novel is dropped"
    );
    assert_eq!(
        anime_hits.iter().map(|a| a.remote_id).collect::<Vec<_>>(),
        [101, 102, 103],
        "EC-B-24: no platform filter"
    );
    let sent = bodies(&server, "POST").await;
    assert!(
        sent[0].ends_with(r#""filter":{"type":[1]}}"#),
        "{}",
        sent[0]
    );
    assert!(
        sent[1].ends_with(r#""filter":{"type":[2]}}"#),
        "{}",
        sent[1]
    );
}

#[test]
fn ts_b_04_token_decode_fields() {
    let with: BgmOAuth = serde_json::from_str(TOKEN).unwrap();
    assert_eq!(with.user_id, Some(1), "EC-B-05: user_id is kept");
    assert!(
        serde_json::to_string(&with)
            .unwrap()
            .contains("\"user_id\":1")
    );

    let before = now_secs();
    let mut raw: Value = serde_json::from_str(TOKEN).unwrap();
    raw.as_object_mut().unwrap().remove("created_at");
    let without: BgmOAuth = serde_json::from_value(raw).unwrap();
    assert!(
        without.created_at >= before && without.created_at <= now_secs(),
        "EC-B-04"
    );
}

#[tokio::test]
async fn ts_b_05_refresh_documented_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(json_body(REFRESH_DOCUMENTED))
        .expect(1)
        .mount(&server)
        .await;
    let t = logged_in(&server);
    t.api().tokens().login(token_at(0, true));

    assert_eq!(
        t.api().bearer().await.as_deref(),
        Ok("fx-b-access-2"),
        "EC-B-12: decodes without user_id"
    );
    let stored = t.api().base().load_token::<BgmOAuth>().unwrap().unwrap();
    assert_eq!(stored.access_token, "fx-b-access-2");

    // EC-B-02: a refusal means the login expired.
    let refused = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&refused)
        .await;
    let t = logged_in(&refused);
    t.api().tokens().login(token_at(0, true));
    assert_eq!(t.api().bearer().await, Err(BangumiError::AuthExpired));
    assert!(t.auth_expired());
}

#[tokio::test]
async fn ts_b_06_refresh_request_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .and(body_string(REFRESH_BODY))
        .respond_with(json_body(REFRESH_DOCUMENTED))
        .expect(1)
        .mount(&server)
        .await;
    let t = logged_in(&server);
    t.api().tokens().login(token_at(0, true));

    t.api().bearer().await.unwrap();
}

#[tokio::test]
async fn ts_b_07_collection_lookup() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/1"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/2"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/3"))
        .respond_with(json_body(NO_RATE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/users/a%20b%2Fc/collections/4"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let t = logged_in(&server);

    assert_eq!(t.api().lookup("fixture_user", 1).await, Ok(None), "EC-B-06");
    assert_eq!(
        t.api().lookup("fixture_user", 2).await.map(|_| ()),
        Err(BangumiError::Http(403)),
        "EC-B-07"
    );
    let lib = t.api().lookup("fixture_user", 3).await.unwrap().unwrap();
    assert_eq!(
        (lib.score, lib.progress, lib.total),
        (0.0, 0.0, 0),
        "EC-B-10, EC-B-11, EC-B-33"
    );
    assert_eq!(
        t.api().lookup("a b/c", 4).await,
        Ok(None),
        "EC-B-31: the name is one encoded segment"
    );
}

#[tokio::test]
async fn ts_b_08_unknown_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/1"))
        .respond_with(json_body(UNKNOWN_TYPE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/2"))
        .respond_with(json_body(r#"{"rate":3}"#))
        .mount(&server)
        .await;
    let t = logged_in(&server);

    assert_eq!(
        t.api().lookup("fixture_user", 1).await.map(|_| ()),
        Err(BangumiError::UnknownStatus("6".into()))
    );
    assert_eq!(
        t.api().lookup("fixture_user", 2).await.map(|_| ()),
        Err(BangumiError::UnknownStatus("null".into()))
    );
}

#[tokio::test]
async fn ts_b_09_bind_and_update() {
    // EC-B-14: on hold with local progress becomes reading, and the PATCH carries type 3.
    let server = MockServer::start().await;
    mount_collection(&server, &DOING.replace(r#""type":3"#, r#""type":4"#)).await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let t = logged_in(&server);
    let mut track = manga(0, 0.0);
    t.as_manga().unwrap().bind(&mut track, true).await.unwrap();
    assert_eq!(track.status, BgmStatus::Reading as i64);
    assert_eq!(
        bodies(&server, "PATCH").await,
        [r#"{"type":3,"rate":8,"ep_status":5,"private":false}"#]
    );

    // EC-B-15: completed stays completed.
    let server = MockServer::start().await;
    mount_collection(&server, &DOING.replace(r#""type":3"#, r#""type":2"#)).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let t = logged_in(&server);
    let mut track = manga(0, 0.0);
    t.as_manga().unwrap().bind(&mut track, true).await.unwrap();
    assert_eq!(track.status, BgmStatus::Completed as i64);

    // EC-B-13: plan to read without progress stays.
    let server = MockServer::start().await;
    mount_collection(&server, &DOING.replace(r#""type":3"#, r#""type":1"#)).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let t = logged_in(&server);
    let mut track = manga(0, 0.0);
    t.as_manga().unwrap().bind(&mut track, false).await.unwrap();
    assert_eq!(track.status, BgmStatus::PlanToRead as i64);
    assert!(bodies(&server, "PATCH").await[0].starts_with(r#"{"type":1,"#));

    // EC-B-34: the anime ep_status is sent. RC-07: the result of the PATCH is used, there is no re-read.
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let t = logged_in(&server);
    let mut a = kuro_track::model::AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TrackerId::BANGUMI,
        remote_id: 5,
        library_id: None,
        title: "T".into(),
        last_episode_seen: 7.0,
        total_episodes: 12,
        status: BgmStatus::Reading as i64,
        score: 0.0,
        remote_url: String::new(),
        start_date: 0,
        finish_date: 0,
        private: false,
    };
    a.score = 8.0;
    t.as_anime().unwrap().update(&a, true).await.unwrap();
    assert_eq!(
        bodies(&server, "PATCH").await,
        [r#"{"type":3,"rate":8,"ep_status":7,"private":false}"#]
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() != "GET")
    );
}

#[tokio::test]
async fn ts_b_10_bind_not_found_post() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/fixture_user/collections/5"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    let t = logged_in(&server);
    let mut track = manga(0, 0.0);
    track.score = 9.0;

    t.as_manga().unwrap().bind(&mut track, false).await.unwrap();

    assert_eq!(
        bodies(&server, "POST").await,
        [r#"{"type":1,"rate":0,"ep_status":0,"private":false}"#]
    );
    assert_eq!(
        (track.status, track.score, track.library_id),
        (BgmStatus::PlanToRead as i64, 0.0, None),
        "EC-B-16, EC-B-18"
    );
}

#[tokio::test]
async fn ts_b_11_rate_coercion() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let t = logged_in(&server);
    for score in [-1.0, 10.9] {
        let write = CollectionWrite {
            status: BgmStatus::Reading,
            score,
            progress: 1.0,
            private: false,
        };
        t.api().update(5, &write).await.unwrap();
    }

    assert_eq!(
        bodies(&server, "PATCH").await,
        [
            r#"{"type":3,"rate":0,"ep_status":1,"private":false}"#,
            r#"{"type":3,"rate":10,"ep_status":1,"private":false}"#,
        ]
    );
}

#[tokio::test]
async fn ts_b_12_private_write_only() {
    let server = MockServer::start().await;
    mount_collection(&server, DOING).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let t = logged_in(&server);
    let mut track = manga(0, 0.0);
    track.private = true;

    t.as_manga().unwrap().bind(&mut track, true).await.unwrap();

    assert!(bodies(&server, "PATCH").await[0].ends_with(r#""private":true}"#));
    assert!(
        track.private,
        "EC-B-21: the remote false does not change the local true"
    );
}

#[tokio::test]
async fn ts_b_13_refresh_keeps_private() {
    let server = MockServer::start().await;
    mount_collection(&server, DOING).await;
    let t = logged_in(&server);
    let mut local = manga(3, 1.0);
    local.private = true;

    let out = t.as_manga().unwrap().refresh(&local).await.unwrap();

    assert!(out.private, "EC-B-22");
    assert_eq!((out.last_chapter_read, out.score), (5.0, 8.0));
}

#[tokio::test]
async fn ts_b_14_logged_out_guard() {
    let server = MockServer::start().await;
    let t = tracker_for(&server);

    let result = t.api().collection("fixture_user", 5).await.map(|_| ());

    assert_eq!(result, Err(BangumiError::NotAuthenticated));
    assert_eq!(
        BangumiError::NotAuthenticated.to_string(),
        "Not authenticated with Bangumi"
    );
    assert!(
        t.as_manga()
            .unwrap()
            .bind(&mut manga(0, 0.0), true)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
