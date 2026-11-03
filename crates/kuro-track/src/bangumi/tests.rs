//! Bangumi tests: identity and rules (pure), and wiremock tests for the endpoints.
//! Fixtures are inline JSON written from the DTO fields and the server schema, not from a live account.

use std::sync::Arc;

use kuro_platform::secrets::SecretStore;
use serde_json::{Value, json};
use wiremock::matchers::{body_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::api::Endpoints;
use super::dto::{BgmEpisode, BgmOAuth, BgmSubject, now_secs};
use super::*;
use crate::base::SECRET_SERVICE;
use crate::fakes::MemSecrets;
use crate::model::{AnimeTrack, MangaTrack};
use crate::shikimori::api::OAuthClient;

const REDIRECT: &str = "kurobayomi://bangumi-auth";
const TOKEN_BODY: &str = "grant_type=authorization_code&client_id=cid&client_secret=sec&code=abc&redirect_uri=kurobayomi%3A%2F%2Fbangumi-auth";
const REFRESH_BODY: &str = "grant_type=refresh_token&client_id=cid&client_secret=sec&refresh_token=r&redirect_uri=kurobayomi%3A%2F%2Fbangumi-auth";
const WEEK: i64 = 604_800;

fn token(access: &str, created: i64, refresh: Option<&str>) -> BgmOAuth {
    BgmOAuth {
        access_token: access.into(),
        token_type: "Bearer".into(),
        created_at: created,
        expires_in: WEEK,
        refresh_token: refresh.map(str::to_owned),
        user_id: None,
    }
}

fn client() -> Option<OAuthClient> {
    Some(OAuthClient {
        client_id: "cid".into(),
        client_secret: "sec".into(),
    })
}

async fn tracker_for(server: &MockServer, secrets: Arc<dyn SecretStore>) -> BangumiTracker {
    BangumiTracker::new(
        Endpoints {
            api: server.uri(),
            web: server.uri(),
        },
        client(),
        REDIRECT,
        secrets,
    )
    .unwrap()
}

/// A tracker logged in as `Ann` with a fresh token `tok`.
async fn logged_in(server: &MockServer) -> (BangumiTracker, Arc<dyn SecretStore>) {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = tracker_for(server, secrets.clone()).await;
    let fresh = token("tok", now_secs(), Some("r"));
    t.api().base().save_oauth("Ann", "tok", &fresh).unwrap();
    t.api().tokens().login(fresh);
    (t, secrets)
}

fn manga(status: i64, read: f64, total: i64) -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TrackerId::BANGUMI,
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

fn anime(status: i64, seen: f64, total: i64) -> AnimeTrack {
    AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TrackerId::BANGUMI,
        remote_id: 5,
        library_id: None,
        title: "T".into(),
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

fn subject(v: Value) -> BgmSubject {
    serde_json::from_value(v).unwrap()
}

/// The bodies of the requests the server received with `verb`, in order.
async fn sent(server: &MockServer, verb: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.to_string() == verb)
        .map(|r| String::from_utf8(r.body).unwrap())
        .collect()
}

// ---- T-31a-020, T-31a-021, T-31a-022, T-31a-031, T-31a-035: identity, tables, rules (pure) ----

#[test]
fn t_31a_020_identity_and_status_tables() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = BangumiTracker::new(Endpoints::production(), client(), REDIRECT, secrets).unwrap();
    assert_eq!(t.id(), TrackerId::BANGUMI);
    assert_eq!(t.id().0, 5);
    assert_eq!(t.name(), "Bangumi");
    assert!(t.supports_private_tracking());
    assert!(!t.supports_reading_dates());
    assert_eq!(t.completion_status(), 2);
    assert_eq!(t.logo_color(), 0xFFF0_9199);
    let m = t.as_manga().unwrap();
    assert_eq!((m.reading_status(), m.rereading_status()), (3, -1));
    assert_eq!(m.status_list(), vec![3, 2, 4, 5, 1]);
    let a = t.as_anime().unwrap();
    assert_eq!((a.watching_status(), a.rewatching_status()), (3, -1));
    assert_eq!(a.status_list(), vec![3, 2, 4, 5, 1]);
    assert!(t.as_deletable_manga().is_none());
    assert!(t.as_deletable_anime().is_none());
}

#[test]
fn t_31a_022_scores_are_zero_to_ten_and_display_truncates() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = BangumiTracker::new(Endpoints::production(), client(), REDIRECT, secrets).unwrap();
    assert_eq!(t.score_list().len(), 11);
    assert_eq!(t.score_list()[10], "10");
    assert_eq!(t.as_manga().unwrap().index_to_score(4), 4.0);
    let mut m = manga(3, 0.0, 0);
    m.score = 7.9;
    assert_eq!(t.as_manga().unwrap().display_score(&m), "7");
}

#[test]
fn t_31a_021_labels_per_kind_from_the_tracker() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = BangumiTracker::new(Endpoints::production(), client(), REDIRECT, secrets).unwrap();
    assert_eq!(t.as_anime().unwrap().status_label(3), Some("Watching"));
    assert_eq!(t.as_manga().unwrap().status_label(3), Some("Reading"));
    assert_eq!(t.as_manga().unwrap().status_label(-1), None);
}

#[test]
fn t_31a_031_read_transitions_for_bangumi() {
    use BgmStatus::*;
    let cases = [
        (PlanToRead, true, 1.0, 5, Reading),
        (OnHold, true, 1.0, 5, Reading),
        (Dropped, true, 1.0, 5, Reading),
        (Completed, true, 5.0, 5, Completed),
        (Reading, true, 12.0, 12, Completed),
        (Reading, true, 0.0, 0, Reading),
        (Reading, false, 12.0, 12, Reading),
        (PlanToRead, false, 1.0, 5, PlanToRead),
    ];
    for (current, did_read, progress, total, expected) in cases {
        assert_eq!(
            next_status_after_read(current, did_read, progress, total),
            expected,
            "{current:?} did_read={did_read} {progress}/{total}"
        );
    }
}

#[test]
fn t_31a_024_title_and_summary_composition() {
    let blank_cn = subject(json!({"id": 1, "name_cn": "", "name": "Bleach", "summary": "  hi  "}));
    assert_eq!(search::title(&blank_cn), "Bleach");
    assert_eq!(search::summary(&blank_cn), "hi");

    let no_summary =
        subject(json!({"id": 2, "name_cn": "死神", "name": "Bleach", "summary": null}));
    assert_eq!(search::title(&no_summary), "死神");
    assert_eq!(search::summary(&no_summary), "作品原名：Bleach");

    let with_summary =
        subject(json!({"id": 3, "name_cn": "死神", "name": "Bleach", "summary": "  hi  "}));
    assert_eq!(search::summary(&with_summary), "作品原名：Bleach\nhi");

    let blank_spaces = subject(json!({"id": 4, "name_cn": "   ", "name": "Bleach"}));
    assert_eq!(search::title(&blank_spaces), "Bleach");
}

#[test]
fn t_31a_024_search_mapping_fields() {
    let s = subject(json!({
        "id": 9, "name_cn": "死神", "name": "Bleach", "summary": null,
        "date": "2004-10-05", "eps": 0, "rating": null, "images": {"common": "https://img/x.jpg"}
    }));
    let m = search::to_manga_search(&s);
    assert_eq!(m.remote_id, 9);
    assert_eq!(m.total_chapters, 0);
    assert_eq!(m.score, None, "no rating is no score, not -1");
    assert_eq!(m.start_date, "2004-10-05");
    assert_eq!(m.cover_url, "https://img/x.jpg");
    assert_eq!(m.tracking_url, "https://bangumi.tv/subject/9");

    let rated = subject(
        json!({"id": 10, "name_cn": "", "name": "A", "eps": 12, "rating": {"score": 7.5}, "date": null}),
    );
    let a = search::to_anime_search(&rated);
    assert_eq!(a.total_episodes, 12);
    assert_eq!(a.score, Some(7.5));
    assert_eq!(a.start_date, "");
}

#[test]
fn t_31a_024_manga_search_keeps_only_null_or_manga_platforms() {
    assert!(search::is_manga(&subject(
        json!({"id": 1, "name_cn": "", "name": "a", "platform": null})
    )));
    assert!(search::is_manga(&subject(
        json!({"id": 1, "name_cn": "", "name": "a", "platform": "漫画"})
    )));
    assert!(!search::is_manga(&subject(
        json!({"id": 1, "name_cn": "", "name": "a", "platform": "小说"})
    )));
}

#[test]
fn t_31a_035_a_bangumi_track_has_private_and_no_dates_or_deletes() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = BangumiTracker::new(Endpoints::production(), client(), REDIRECT, secrets).unwrap();
    assert!(t.supports_private_tracking());
    assert!(!t.supports_reading_dates());
    assert!(t.as_deletable_manga().is_none());
}

#[test]
fn t_31a_026_the_setup_message_and_auth_url_need_the_client() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let off =
        BangumiTracker::new(Endpoints::production(), None, REDIRECT, secrets.clone()).unwrap();
    assert!(matches!(
        off.setup_message(),
        Some(crate::messages::MsgKey::TrackerNotConfigured(name)) if name == "Bangumi"
    ));
    assert_eq!(off.auth_url(), Err(BangumiError::NotConfigured));

    let on = BangumiTracker::new(Endpoints::production(), client(), REDIRECT, secrets).unwrap();
    assert!(on.setup_message().is_none());
    assert_eq!(
        on.auth_url().unwrap(),
        "https://bgm.tv/oauth/authorize?client_id=cid&response_type=code&redirect_uri=kurobayomi%3A%2F%2Fbangumi-auth"
    );
}

#[test]
fn t_31a_040_the_user_agent_is_descriptive_and_names_the_project() {
    let ua = crate::user_agent();
    assert!(ua.starts_with("Kurobayomi/"), "{ua}");
    assert!(ua.contains(std::env::consts::OS), "{ua}");
    assert_ne!(ua, "Bangumi/1.0");
}

// ---- T-31a-025, T-31a-026, T-31a-034: token and login ----

#[tokio::test]
async fn t_31a_026_login_stores_the_login_name_and_the_token_after_v0_me() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .and(body_string(TOKEN_BODY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "new", "token_type": "Bearer", "expires_in": WEEK,
            "refresh_token": "rr", "user_id": 12345
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/me"))
        .and(header("Authorization", "Bearer new"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"username": "ann", "nickname": "Ann"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = tracker_for(&server, secrets.clone()).await;

    t.login("", "abc").await.unwrap();

    assert!(t.is_logged_in());
    assert_eq!(t.username(), "ann");
    assert_eq!(t.password(), "new");
    let stored = t.api().base().load_token::<BgmOAuth>().unwrap().unwrap();
    assert_eq!(stored.user_id, Some(12345));
    assert!(stored.created_at > 0);
}

#[tokio::test]
async fn t_31a_026_a_refused_code_stores_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = tracker_for(&server, secrets).await;

    assert!(t.login("", "abc").await.is_err());
    assert!(!t.is_logged_in());
    assert!(t.api().base().load_token::<BgmOAuth>().unwrap().is_none());
}

#[tokio::test]
async fn t_31a_026_a_failed_me_keeps_an_existing_login() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "second", "token_type": "Bearer", "expires_in": WEEK
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/me"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    assert!(t.login("", "abc").await.is_err());

    assert!(t.is_logged_in());
    assert_eq!(t.username(), "Ann");
    assert_eq!(t.password(), "tok");
}

#[tokio::test]
async fn t_31a_034_logout_clears_credentials_and_the_token_and_a_restart_restores_nothing() {
    let server = MockServer::start().await;
    let (t, secrets) = logged_in(&server).await;

    t.logout().unwrap();

    assert!(!t.is_logged_in());
    assert!(!t.api().tokens().has_token());
    assert!(t.api().base().load_token::<BgmOAuth>().unwrap().is_none());
    let again = tracker_for(&server, secrets).await;
    assert!(!again.api().tokens().has_token());
}

#[tokio::test]
async fn t_31a_034_a_stored_token_is_restored_in_the_constructor() {
    let server = MockServer::start().await;
    let (_, secrets) = logged_in(&server).await;

    let again = tracker_for(&server, secrets).await;

    assert!(again.api().tokens().has_token());
}

#[tokio::test]
async fn t_31a_034_a_stored_token_that_does_not_decode_means_logged_out() {
    let server = MockServer::start().await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    secrets
        .set(SECRET_SERVICE, "tracker/5/oauth", "{not json")
        .unwrap();

    let t = tracker_for(&server, secrets).await;

    assert!(!t.api().tokens().has_token());
}

// ---- T-31a-027: request authentication and refresh ----

#[tokio::test]
async fn t_31a_027_an_authed_call_sends_bearer_user_agent_and_no_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .and(header("Authorization", "Bearer tok"))
        .and(header("User-Agent", crate::user_agent()))
        .and(header("Cache-Control", "no-cache"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type": 3})))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    assert!(t.api().collection("Ann", 5).await.unwrap().is_some());
}

#[tokio::test]
async fn t_31a_045_logged_out_search_is_anonymous_and_collections_send_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/search/subjects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total": 0, "limit": 20, "offset": 0, "data": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = tracker_for(&server, secrets).await;

    assert!(t.as_manga().unwrap().search("x").await.unwrap().is_empty());
    assert_eq!(
        t.api().collection("Ann", 5).await.map(|_| ()),
        Err(BangumiError::NotAuthenticated)
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "only the public search is sent");
    assert!(requests[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn t_31a_027_an_expired_token_is_refreshed_once_with_the_documented_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .and(body_string(REFRESH_BODY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "fresh", "token_type": "Bearer", "expires_in": WEEK, "refresh_token": "r2"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    t.api().tokens().login(token("old", 0, Some("r")));

    let results = futures::future::join_all((0..5).map(|_| t.api().bearer())).await;

    assert!(results.iter().all(|r| r.as_deref() == Ok("fresh")));
    let stored = t.api().base().load_token::<BgmOAuth>().unwrap().unwrap();
    assert_eq!(stored.refresh_token.as_deref(), Some("r2"));
    assert!(
        stored.created_at > 0,
        "a refresh without created_at gets the decode time"
    );
}

#[tokio::test]
async fn t_31a_027_a_refresh_refused_with_400_flags_the_login_and_sends_no_old_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    t.api().tokens().login(token("old", 0, Some("r")));

    assert_eq!(t.api().bearer().await, Err(BangumiError::AuthExpired));
    assert!(t.auth_expired());
}

#[tokio::test]
async fn t_31a_027_a_refresh_failing_with_500_keeps_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    t.api().tokens().login(token("old", 0, Some("r")));

    assert_eq!(t.api().bearer().await, Err(BangumiError::Http(500)));
    assert!(t.api().tokens().has_token());
    assert!(!t.auth_expired());
}

#[tokio::test]
async fn t_31a_027_an_expired_token_without_a_refresh_token_is_auth_expired_with_no_request() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server).await;
    t.api().tokens().login(token("old", 0, None));

    assert_eq!(t.api().bearer().await, Err(BangumiError::AuthExpired));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn t_31a_027_a_401_on_a_fresh_token_means_the_login_expired() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    assert_eq!(
        t.api().collection("Ann", 5).await.map(|_| ()),
        Err(BangumiError::AuthExpired)
    );
    assert!(t.auth_expired());
}

// ---- T-31a-028, T-31a-029: search and collection lookup ----

#[tokio::test]
async fn t_31a_028_search_sends_the_type_filter_and_manga_drops_novels() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/search/subjects"))
        .and(query_param("limit", "20"))
        .and(header("Content-Type", "application/json"))
        .and(body_string(
            r#"{"keyword":"x","sort":"match","filter":{"type":[1]}}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total": 3, "limit": 20, "offset": 0, "data": [
                {"id": 1, "name_cn": "甲", "name": "A", "platform": "漫画"},
                {"id": 2, "name_cn": "", "name": "B", "platform": null},
                {"id": 3, "name_cn": "", "name": "C", "platform": "小说"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/search/subjects"))
        .and(body_string(
            r#"{"keyword":"x","sort":"match","filter":{"type":[2]}}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total": 1, "limit": 20, "offset": 0, "data": [
                {"id": 4, "name_cn": "", "name": "D", "platform": "TV", "eps": 12}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    let manga_hits = t.as_manga().unwrap().search("x").await.unwrap();
    let anime_hits = t.as_anime().unwrap().search("x").await.unwrap();

    assert_eq!(
        manga_hits.iter().map(|m| m.remote_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        anime_hits.iter().map(|a| a.remote_id).collect::<Vec<_>>(),
        vec![4]
    );
    assert_eq!(anime_hits[0].total_episodes, 12);
}

#[tokio::test]
async fn t_31a_029_collection_lookup_404_is_none_and_403_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/6"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    assert_eq!(t.api().lookup("Ann", 5).await, Ok(None));
    assert_eq!(
        t.api().lookup("Ann", 6).await.map(|_| ()),
        Err(BangumiError::Http(403))
    );
}

#[tokio::test]
async fn t_31a_029_missing_rate_progress_and_subject_default_to_zero() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type": 3})))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    let lib = t.api().lookup("Ann", 5).await.unwrap().unwrap();

    assert_eq!(lib.status, BgmStatus::Reading);
    assert_eq!((lib.score, lib.progress, lib.total), (0.0, 0.0, 0));
}

#[tokio::test]
async fn t_31a_029_a_collection_type_outside_the_table_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type": 6})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/6"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"rate": 3})))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    assert_eq!(
        t.api().lookup("Ann", 5).await.map(|_| ()),
        Err(BangumiError::UnknownStatus("6".into()))
    );
    assert_eq!(
        t.api().lookup("Ann", 6).await.map(|_| ()),
        Err(BangumiError::UnknownStatus("null".into()))
    );
}

#[tokio::test]
async fn t_31a_029_the_username_is_one_percent_encoded_path_segment() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/a%20b%2Fc/collections/5"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    assert_eq!(t.api().lookup("a b/c", 5).await, Ok(None));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.path(), "/v0/users/a%20b%2Fc/collections/5");
}

// ---- T-31a-030, T-31a-032, T-31a-033, TS-B-09 to TS-B-13: writes, bind, refresh ----

#[tokio::test]
async fn t_31a_030_add_is_post_202_and_update_is_patch_204_with_the_body_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/users/-/collections/5"))
        .and(header("Content-Type", "application/json"))
        .and(body_string(
            r#"{"type":3,"rate":7,"ep_status":2,"private":true}"#,
        ))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":3,"rate":7,"ep_status":2,"private":true}"#,
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let write = api::CollectionWrite {
        status: BgmStatus::Reading,
        score: 7.0,
        progress: 2.0,
        private: true,
    };

    t.api().add(5, &write).await.unwrap();
    t.api().update(5, &write).await.unwrap();
}

#[tokio::test]
async fn t_31a_030_the_rating_is_clamped_to_zero_and_ten_and_progress_truncates() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":3,"rate":0,"ep_status":12,"private":false}"#,
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":3,"rate":10,"ep_status":12,"private":false}"#,
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let minus_one = api::CollectionWrite {
        status: BgmStatus::Reading,
        score: -1.0,
        progress: 12.9,
        private: false,
    };
    let ten_point_nine = api::CollectionWrite {
        status: BgmStatus::Reading,
        score: 10.9,
        progress: 12.0,
        private: false,
    };

    t.api().update(5, &minus_one).await.unwrap();
    t.api().update(5, &ten_point_nine).await.unwrap();
}

#[tokio::test]
async fn t_31a_030_anime_progress_is_sent_as_ep_status() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":2,"rate":8,"ep_status":12,"private":false}"#,
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let mut a = anime(3, 12.0, 12);
    a.score = 8.0;

    let out = t.as_anime().unwrap().update(&a, true).await.unwrap();

    assert_eq!(out.status, BgmStatus::Completed as i64);
}

#[tokio::test]
async fn t_31a_031_update_completes_at_the_total_and_sends_the_status_code() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":2,"rate":0,"ep_status":5,"private":false}"#,
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    let out = t
        .as_manga()
        .unwrap()
        .update(&manga(3, 5.0, 5), true)
        .await
        .unwrap();

    assert_eq!(out.status, BgmStatus::Completed as i64);
}

#[tokio::test]
async fn ts_x_02_an_unknown_internal_status_is_refused_before_any_request() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server).await;

    for bad in [0, -1, 99] {
        assert!(matches!(
            t.as_manga()
                .unwrap()
                .update(&manga(bad, 1.0, 5), true)
                .await,
            Err(TrackError::Key(_))
        ));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

async fn bind_found(
    collection: Value,
    has_read: bool,
    local_private: bool,
) -> (MangaTrack, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(collection))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let mut track = manga(0, 0.0, 0);
    track.private = local_private;
    t.as_manga()
        .unwrap()
        .bind(&mut track, has_read)
        .await
        .unwrap();
    let body = sent(&server, "PATCH").await.remove(0);
    (track, body)
}

#[tokio::test]
async fn t_31a_032_found_on_hold_with_local_progress_becomes_reading() {
    let (track, body) = bind_found(
        json!({"type": 4, "rate": 6, "ep_status": 3, "subject": {"eps": 9}}),
        true,
        false,
    )
    .await;
    assert_eq!(track.status, BgmStatus::Reading as i64);
    assert_eq!(
        (track.last_chapter_read, track.score, track.total_chapters),
        (3.0, 6.0, 9)
    );
    assert_eq!(body, r#"{"type":3,"rate":6,"ep_status":3,"private":false}"#);
}

#[tokio::test]
async fn t_31a_032_found_completed_stays_completed_even_with_progress() {
    let (track, body) = bind_found(
        json!({"type": 2, "ep_status": 9, "subject": {"eps": 9}}),
        true,
        false,
    )
    .await;
    assert_eq!(track.status, BgmStatus::Completed as i64);
    assert_eq!(body, r#"{"type":2,"rate":0,"ep_status":9,"private":false}"#);
}

#[tokio::test]
async fn t_31a_032_found_plan_to_read_without_local_progress_keeps_the_remote_status() {
    let (track, body) = bind_found(json!({"type": 1}), false, false).await;
    assert_eq!(track.status, BgmStatus::PlanToRead as i64);
    assert_eq!(body, r#"{"type":1,"rate":0,"ep_status":0,"private":false}"#);
}

#[tokio::test]
async fn t_31a_032_the_local_private_flag_survives_a_bind_and_is_sent() {
    let (track, body) = bind_found(json!({"type": 3, "private": false}), false, true).await;
    assert!(track.private, "EC-B-22: the remote flag is never read back");
    assert!(body.ends_with(r#""private":true}"#), "{body}");
}

#[tokio::test]
async fn t_31a_032_not_found_is_posted_as_reading_or_plan_to_read_with_score_zero() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":3,"rate":0,"ep_status":4,"private":false}"#,
        ))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let mut track = manga(0, 4.0, 0);
    track.score = 9.0;

    t.as_manga().unwrap().bind(&mut track, true).await.unwrap();

    assert_eq!(track.status, BgmStatus::Reading as i64);
    assert_eq!(track.score, 0.0);
    assert_eq!(track.library_id, None, "POST returns no library id (D19)");
}

#[tokio::test]
async fn t_31a_032_a_logged_out_bind_sends_nothing() {
    let server = MockServer::start().await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = tracker_for(&server, secrets).await;
    let mut track = manga(0, 0.0, 0);

    assert!(t.as_manga().unwrap().bind(&mut track, false).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn t_31a_033_refresh_copies_the_remote_state_and_keeps_private_and_library_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": 2, "rate": 9, "ep_status": 12, "private": false, "subject": {"eps": 12}
        })))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let mut local = manga(3, 1.0, 0);
    local.private = true;
    local.library_id = Some(77);

    let out = t.as_manga().unwrap().refresh(&local).await.unwrap();

    assert_eq!(out.status, BgmStatus::Completed as i64);
    assert_eq!(
        (out.last_chapter_read, out.score, out.total_chapters),
        (12.0, 9.0, 12)
    );
    assert!(
        out.private,
        "TS-B-13: a remote private flag never changes the local one"
    );
    assert_eq!(out.library_id, Some(77));
    assert!(sent(&server, "POST").await.is_empty());
    assert!(sent(&server, "PATCH").await.is_empty());
}

#[tokio::test]
async fn t_31a_033_refresh_without_a_collection_is_could_not_find_manga() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    let err = t
        .as_manga()
        .unwrap()
        .refresh(&manga(3, 0.0, 0))
        .await
        .map(|_| ())
        .unwrap_err();

    assert_eq!(err.to_string(), "Bangumi error: Could not find manga");
}

#[tokio::test]
async fn t_31a_033_refresh_for_anime_copies_episodes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": 3, "rate": 5, "ep_status": 4, "subject": {"eps": 24}
        })))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    let out = t
        .as_anime()
        .unwrap()
        .refresh(&anime(3, 1.0, 0))
        .await
        .unwrap();

    assert_eq!(
        (out.last_episode_seen, out.score, out.total_episodes),
        (4.0, 5.0, 24)
    );
}

#[tokio::test]
async fn t_31a_035_the_private_flag_is_never_read_from_the_remote_on_bind_for_anime() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type": 3, "private": true})))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .and(body_string(
            r#"{"type":3,"rate":0,"ep_status":0,"private":false}"#,
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let mut track = anime(0, 0.0, 0);

    t.as_anime().unwrap().bind(&mut track, false).await.unwrap();

    assert!(!track.private);
}

#[tokio::test]
async fn t_31a_034_logout_then_a_stale_refresh_does_not_log_back_in() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "late", "token_type": "Bearer", "expires_in": WEEK, "refresh_token": "r2"}))
                .set_delay(std::time::Duration::from_millis(300)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    t.api().tokens().login(token("old", 0, Some("r")));

    let refresh = t.api().bearer();
    let logout = async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        t.logout().unwrap();
    };
    let (result, ()) = tokio::join!(refresh, logout);

    assert_eq!(result, Err(BangumiError::NotAuthenticated));
    assert!(!t.is_logged_in());
    assert!(t.api().base().load_token::<BgmOAuth>().unwrap().is_none());
    assert!(!t.api().tokens().has_token());
}

#[tokio::test]
async fn t_31a_036_a_401_on_a_fresh_token_is_auth_expired_for_writes_too() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    let err = t
        .as_manga()
        .unwrap()
        .update(&manga(3, 1.0, 5), false)
        .await
        .unwrap_err();

    assert!(matches!(err, TrackError::Key(_)));
    assert!(t.auth_expired());
}

#[tokio::test]
async fn t_31a_036_no_request_carries_a_token_in_the_query_string() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/users/Ann/collections/5"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    t.api().lookup("Ann", 5).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| r.url.query().is_none_or(|q| !q.contains("tok")))
    );
}

// ---- T-31a-042, T-31a-044, T-31a-047, T-31a-048 ----

#[test]
fn t_31a_042_bangumi_offers_no_remote_removal() {
    let t = BangumiTracker::new(
        Endpoints {
            api: "http://127.0.0.1:1".into(),
            web: "http://127.0.0.1:1".into(),
        },
        client(),
        REDIRECT,
        MemSecrets::new(),
    )
    .unwrap();
    assert!(t.as_deletable_manga().is_none());
    assert!(t.as_deletable_anime().is_none());
}

#[test]
fn t_31a_047_the_logo_is_the_embedded_48_px_png() {
    let t = BangumiTracker::new(
        Endpoints {
            api: "http://127.0.0.1:1".into(),
            web: "http://127.0.0.1:1".into(),
        },
        client(),
        REDIRECT,
        MemSecrets::new(),
    )
    .unwrap();
    match t.logo() {
        LogoRef::Embedded(bytes) => {
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(&bytes[16..20], &48u32.to_be_bytes());
            assert_eq!(&bytes[20..24], &48u32.to_be_bytes());
        }
        other => panic!("expected the embedded logo, got {other:?}"),
    }
}

#[test]
fn t_31a_048_bangumi_id_and_status_numbers_are_the_backup_values() {
    assert_eq!(
        TrackerId::BANGUMI,
        TrackerId(5),
        "syncId 5 in Aniyomi backups"
    );
    for n in 1..=5 {
        let s = BgmStatus::from_internal(n).expect("1..5 restore unchanged");
        assert_eq!(s as i64, n);
    }
    assert!(BgmStatus::from_internal(6).is_none());
}

#[test]
fn t_31a_044_first_main_episodes_skip_specials_and_follow_the_sort_order() {
    let ep = |id, kind, sort| BgmEpisode { id, kind, sort };
    let episodes = vec![
        ep(30, 0, 3.0),
        ep(91, 1, 0.5), // a special, never counted
        ep(10, 0, 1.0),
        ep(20, 0, 2.0),
    ];
    assert_eq!(first_main_episodes(&episodes, 2), vec![10, 20]);
    assert_eq!(
        first_main_episodes(&episodes, 9),
        vec![10, 20, 30],
        "capped at the main count"
    );
    assert!(first_main_episodes(&episodes, 0).is_empty());
}

#[tokio::test]
async fn t_31a_044_the_episode_setting_is_off_by_default_and_marks_itself_unverified() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server).await;
    assert!(t.anime_progress_unverified());
    assert!(!t.with_episode_progress(true).anime_progress_unverified());
}

#[tokio::test]
async fn t_31a_044_with_the_setting_on_progress_goes_through_the_episode_endpoints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/episodes"))
        .and(query_param("subject_id", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
            {"id": 11, "type": 0, "sort": 1.0},
            {"id": 12, "type": 0, "sort": 2.0},
            {"id": 13, "type": 0, "sort": 3.0}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5/episodes"))
        .and(body_string(r#"{"episode_id":[11,12],"type":2}"#))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;
    let t = t.with_episode_progress(true);

    let out = t
        .as_anime()
        .unwrap()
        .update(&anime(3, 2.0, 12), true)
        .await
        .unwrap();

    assert_eq!(out.status, 3);
    // `expect(1)` on both mocks is checked when the server is dropped.
}

#[tokio::test]
async fn t_31a_044_with_the_setting_off_no_episode_request_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/episodes"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v0/users/-/collections/5"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server).await;

    t.as_anime()
        .unwrap()
        .update(&anime(3, 1.0, 12), true)
        .await
        .unwrap();
}

#[tokio::test]
async fn t_31a_044_the_tracker_follows_the_user_setting_live() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server).await;
    let prefs = crate::prefs::TrackPrefs::new(kuro_prefs::store::PrefStore::in_memory());
    let t = t.with_episode_progress_setting(prefs.bangumi_episode_progress.clone());
    assert!(t.anime_progress_unverified(), "off by default");
    prefs.bangumi_episode_progress.set(true).unwrap();
    assert!(
        !t.anime_progress_unverified(),
        "the next write sees the change"
    );
}
