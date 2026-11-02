//! Tracker-level tests of Simkl against a local HTTP server (`wiremock`). Each test names the task it covers.

use std::sync::Arc;

use wiremock::matchers::{body_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::status::{COMPLETED, PLAN_TO_WATCH, WATCHING};
use super::*;
use crate::base::{SECRET_SERVICE, account};
use crate::fakes::MemSecrets;
use crate::model::{AnimeTrack, TrackerId, copy_personal_from_anime};

const TOKEN_BODY: &str = r#"{"code":"C1","client_id":"aa62a7da32518aae5d5049a658b87fa4837c3b739e06ed250b315aab6af82b0e","client_secret":"2bec9c1d0c00a1e9b0e9e096a71f88d555a6f52da7923df07906df3b21351783","redirect_uri":"kurobayomi://simkl-auth","grant_type":"authorization_code"}"#;

fn tracker(server: &MockServer) -> (SimklTracker, Arc<MemSecrets>) {
    let secrets = MemSecrets::new();
    let dyn_secrets: Arc<dyn SecretStore> = secrets.clone();
    let t = SimklTracker::with_base_url(&server.uri(), dyn_secrets, EventBus::default()).unwrap();
    (t, secrets)
}

fn offline() -> SimklTracker {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    SimklTracker::with_base_url("http://127.0.0.1:9", secrets, EventBus::default()).unwrap()
}

async fn reply(server: &MockServer, verb: &str, at: &str, status: u16, body: &str) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(ResponseTemplate::new(status).set_body_string(body.to_owned()))
        .mount(server)
        .await;
}

async fn sent(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap_or_default()
}

/// Mounts a healthy exchange: token `AT`, user 7.
async fn healthy_login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string(TOKEN_BODY))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"access_token":"AT"}"#))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/settings"))
        .and(header("authorization", "Bearer AT"))
        .and(header("simkl-api-key", api::CLIENT_ID))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"account":{"id":7}}"#))
        .mount(server)
        .await;
}

// ---- T-32a-001, T-32a-030: identity and status table ----

#[test]
fn t_32a_001_the_tracker_id_and_identity_are_the_source_values() {
    let t = offline();
    assert_eq!(TRACKER_ID, TrackerId::SIMKL);
    assert_eq!(TRACKER_ID.0, 101);
    assert_eq!(t.id(), TrackerId::SIMKL);
    assert_eq!(t.name(), "Simkl");
    assert_eq!(t.logo_color(), 0xFF000000);
    assert_eq!(t.completion_status(), 2);
    assert!(t.as_anime().is_some());
    assert!(t.as_manga().is_none());
}

#[test]
fn t_32a_030_the_score_list_is_zero_to_ten() {
    let t = offline();
    assert_eq!(t.score_list().len(), 11);
    assert_eq!(t.score_list()[0], "0");
    assert_eq!(t.score_list()[10], "10");
}

#[test]
fn t_32a_047_display_score_is_the_truncated_integer_and_the_status_picker_order_is_fixed() {
    let t = offline();
    let anime = t.as_anime().unwrap();
    assert_eq!(anime.display_score(&show(1, 1, 0.0, 0, 7.9)), "7");
    assert_eq!(anime.display_score(&show(1, 1, 0.0, 0, 0.0)), "0");
    assert_eq!(
        anime.status_list(),
        vec![1, 2, 3, 4, 5],
        "watching, completed, on hold, not interesting, plan to watch"
    );
    assert_eq!(anime.status_label(3), Some("on_hold"));
    assert_eq!(anime.status_label(4), Some("not_interesting"));
}

// ---- T-32a-031: authorise URL and redirect ----

#[tokio::test]
async fn t_32a_031_a_redirect_without_a_code_logs_out_and_makes_no_request() {
    let server = MockServer::start().await;
    let (t, _) = tracker(&server);
    t.save_credentials("7", "AT").unwrap();
    assert!(t.is_logged_in());
    t.handle_redirect("aniyomi://simkl-auth").await.unwrap();
    assert!(!t.is_logged_in());
    assert!(sent(&server).await.is_empty());
}

// ---- T-32a-032, T-32a-033, T-32a-034: token exchange, login completion and token storage ----

#[tokio::test]
async fn t_32a_032_the_code_exchange_sends_the_keys_in_source_order() {
    let server = MockServer::start().await;
    healthy_login(&server).await;
    let (t, _) = tracker(&server);
    t.login("", "C1").await.unwrap();
    let requests = sent(&server).await;
    let exchange = requests
        .iter()
        .find(|r| r.url.path() == "/oauth/token")
        .unwrap();
    assert_eq!(
        String::from_utf8(exchange.body.clone()).unwrap(),
        TOKEN_BODY
    );
    assert_eq!(
        exchange.headers.get("content-type").unwrap(),
        "application/json; charset=utf-8"
    );
}

#[tokio::test]
async fn t_32a_033_login_stores_the_token_then_the_user_id_and_the_token() {
    let server = MockServer::start().await;
    healthy_login(&server).await;
    let (t, secrets) = tracker(&server);
    t.login("", "C1").await.unwrap();
    assert_eq!(t.username(), "7");
    assert_eq!(t.password(), "AT");
    assert!(t.is_logged_in());
    assert!(*t.login_state().borrow());
    assert_eq!(
        secrets
            .raw(&account(TrackerId::SIMKL, crate::base::Field::Token))
            .as_deref(),
        Some(r#"{"access_token":"AT"}"#)
    );
    assert_eq!(
        secrets
            .get(SECRET_SERVICE, "tracker/101/oauth")
            .unwrap()
            .as_deref(),
        Some(r#"{"access_token":"AT"}"#)
    );
}

#[tokio::test]
async fn t_32a_033_a_rejected_code_clears_everything_and_returns_the_error() {
    let server = MockServer::start().await;
    reply(&server, "POST", "/oauth/token", 400, "bad code").await;
    let (t, secrets) = tracker(&server);
    let err = t.login("", "C1").await.unwrap_err();
    assert_eq!(err.to_string(), "HTTP 400");
    assert!(!t.is_logged_in());
    assert!(secrets.all_values().iter().all(|v| v != "C1"));
    assert!(secrets.raw("tracker/101/oauth").is_none());
}

#[tokio::test]
async fn t_32a_033_a_user_reply_without_account_clears_the_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"access_token":"AT"}"#))
        .mount(&server)
        .await;
    reply(&server, "GET", "/users/settings", 200, "{}").await;
    let (t, _) = tracker(&server);
    assert!(t.login_code("C1").await.is_err());
    assert!(!t.is_logged_in());
    assert_eq!(t.username(), "");
}

#[tokio::test]
async fn t_32a_031_a_redirect_with_a_code_completes_the_login() {
    let server = MockServer::start().await;
    healthy_login(&server).await;
    let (t, _) = tracker(&server);
    t.handle_redirect("aniyomi://simkl-auth?code=C1")
        .await
        .unwrap();
    assert!(t.is_logged_in());
}

#[tokio::test]
async fn t_32a_031_a_launch_uri_for_simkl_logs_in_and_other_links_are_left_alone() {
    let server = MockServer::start().await;
    healthy_login(&server).await;
    let (t, _) = tracker(&server);
    assert!(
        t.handle_launch_uri("aniyomi://add-repo?url=x")
            .await
            .is_none()
    );
    assert!(!t.is_logged_in());
    let done = t.handle_launch_uri("aniyomi://simkl-auth?code=C1").await;
    assert!(matches!(done, Some(Ok(()))));
    assert!(t.is_logged_in());
}

#[tokio::test]
async fn t_32a_034_a_restored_token_is_the_bearer_and_a_null_or_empty_token_is_absent() {
    let server = MockServer::start().await;
    let (first, secrets) = tracker(&server);
    first.save_credentials("7", "AT").unwrap();
    first
        .base
        .save_token(&SimklOAuth {
            access_token: "AT".into(),
        })
        .unwrap();
    let dyn_secrets: Arc<dyn SecretStore> = secrets.clone();
    let restored =
        SimklTracker::with_base_url(&server.uri(), dyn_secrets, EventBus::default()).unwrap();
    assert_eq!(restored.auth.bearer().unwrap(), "AT");

    for stored in ["null", ""] {
        let secrets = MemSecrets::new();
        secrets
            .set(SECRET_SERVICE, "tracker/101/oauth", stored)
            .unwrap();
        let dyn_secrets: Arc<dyn SecretStore> = secrets;
        let t =
            SimklTracker::with_base_url(&server.uri(), dyn_secrets, EventBus::default()).unwrap();
        assert_eq!(
            t.auth.bearer(),
            Err(SimklError::NotAuthenticated),
            "stored {stored:?}"
        );
    }
}

#[tokio::test]
async fn t_32a_034_logout_removes_the_token_once_and_writes_no_null_text() {
    let server = MockServer::start().await;
    healthy_login(&server).await;
    let (t, secrets) = tracker(&server);
    t.login("", "C1").await.unwrap();
    t.logout().unwrap();
    assert!(secrets.raw("tracker/101/oauth").is_none());
    assert!(secrets.all_values().iter().all(|v| v != "null"));
    assert_eq!(t.auth.bearer(), Err(SimklError::NotAuthenticated));
    assert!(!t.is_logged_in());
    assert!(!*t.login_state().borrow());
}

// ---- T-32a-004: track model mapping for Simkl ----

#[test]
fn t_32a_004_simkl_personal_fields_copy_and_the_remote_fields_stay() {
    let base = AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TRACKER_ID,
        remote_id: 9,
        library_id: None,
        title: "Mine".into(),
        last_episode_seen: 0.0,
        total_episodes: 12,
        status: 0,
        score: 0.0,
        remote_url: "/tv/9".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    };
    let mut dst = base.clone();
    let src = AnimeTrack {
        last_episode_seen: 4.0,
        score: 8.0,
        status: 1,
        title: "Remote".into(),
        total_episodes: 99,
        remote_url: "/anime/1".into(),
        ..base.clone()
    };
    copy_personal_from_anime(&mut dst, &src, false);
    assert_eq!(
        (dst.last_episode_seen, dst.score, dst.status),
        (4.0, 8.0, 1)
    );
    assert_eq!(dst.title, "Mine");
    assert_eq!(dst.total_episodes, 12);
    assert_eq!(dst.remote_url, "/tv/9");
}

#[test]
fn t_32a_004_a_simkl_track_has_an_empty_remote_url_by_default_and_round_trips_the_row() {
    let mut t = AnimeTrack {
        id: Some(3),
        anime_id: 1,
        tracker_id: TRACKER_ID,
        remote_id: 9,
        library_id: Some(77),
        title: String::new(),
        last_episode_seen: 2.0,
        total_episodes: 12,
        status: 5,
        score: 6.0,
        remote_url: String::new(),
        start_date: 10,
        finish_date: 20,
        private: true,
    };
    assert_eq!(t.remote_url, "", "never null");
    let row = t.to_db(true).unwrap();
    assert_eq!(AnimeTrack::from_db(&row), t);
    t.id = None;
    assert_eq!(t.to_db(false).unwrap().id, 0);
}

// ---- T-32a-035 to T-32a-045, T-32a-060, T-32a-066: anime operations ----

/// A logged-in tracker: the healthy login is mounted and the code exchanged.
async fn signed_in(server: &MockServer) -> SimklTracker {
    healthy_login(server).await;
    let (t, _) = tracker(server);
    t.login("", "C1").await.unwrap();
    t
}

fn show(remote_id: i64, status: i64, last: f64, total: i64, score: f64) -> AnimeTrack {
    AnimeTrack {
        id: Some(1),
        anime_id: 1,
        tracker_id: TrackerId::SIMKL,
        remote_id,
        library_id: None,
        title: "Show".into(),
        last_episode_seen: last,
        total_episodes: total,
        status,
        score,
        remote_url: format!("/tv/{remote_id}"),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

fn movie(remote_id: i64, status: i64) -> AnimeTrack {
    AnimeTrack {
        remote_url: format!("/movies/{remote_id}"),
        ..show(remote_id, status, 0.0, 1, 0.0)
    }
}

/// Mounts 200 for every write an update can send.
async fn accept_writes(server: &MockServer) {
    for at in [
        "/sync/history/remove",
        "/sync/history",
        "/sync/add-to-list",
        "/sync/ratings",
        "/sync/ratings/remove",
    ] {
        reply(server, "POST", at, 200, "{}").await;
    }
}

/// The paths of the requests the tracker sent, login requests left out.
async fn sent_paths(server: &MockServer) -> Vec<String> {
    sent(server)
        .await
        .iter()
        .map(|r| r.url.path().to_owned())
        .filter(|p| p != "/oauth/token" && p != "/users/settings")
        .collect()
}

/// The JSON bodies of the requests sent to `at`, in order.
async fn bodies(server: &MockServer, at: &str) -> Vec<serde_json::Value> {
    sent(server)
        .await
        .iter()
        .filter(|r| r.url.path() == at)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn t_32a_035_an_authenticated_call_sends_the_bearer_key_and_app_agent() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    Mock::given(method("POST"))
        .and(path("/sync/watched"))
        .and(header("authorization", "Bearer AT"))
        .and(header("simkl-api-key", api::CLIENT_ID))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;
    t.refresh(&show(5, WATCHING, 0.0, 12, 0.0)).await.unwrap();
    let got = sent(&server).await;
    let watched = got
        .iter()
        .find(|r| r.url.path() == "/sync/watched")
        .unwrap();
    let agent = watched.headers.get("user-agent").unwrap().to_str().unwrap();
    assert!(agent.starts_with("Kurobayomi v"), "{agent}");
}

#[tokio::test]
async fn t_32a_035_a_missing_token_fails_before_any_request() {
    let server = MockServer::start().await;
    let (t, _) = tracker(&server);
    let err = t
        .refresh(&show(5, WATCHING, 0.0, 12, 0.0))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Not authenticated with Simkl");
    assert!(sent(&server).await.is_empty());
}

#[tokio::test]
async fn t_32a_036_search_sends_no_bearer_and_carries_the_client_id_in_the_query() {
    let server = MockServer::start().await;
    let (t, _) = tracker(&server);
    Mock::given(method("GET"))
        .and(path("/search/anime"))
        .and(query_param("q", "one piece"))
        .and(query_param("extended", "full"))
        .and(query_param("client_id", api::CLIENT_ID))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;
    reply(&server, "GET", "/search/tv", 200, "[]").await;
    reply(&server, "GET", "/search/movie", 200, "[]").await;
    t.search("one piece").await.unwrap();
    let got = sent(&server).await;
    assert_eq!(got.len(), 3);
    assert!(got.iter().all(|r| r.headers.get("authorization").is_none()));
}

#[tokio::test]
async fn t_32a_036_three_searches_run_in_order_and_results_concatenate() {
    let server = MockServer::start().await;
    let (t, _) = tracker(&server);
    let one = |id: i64, url: &str| {
        format!(r#"[{{"ids":{{"simkl_id":{id}}},"title":"T{id}","url":"{url}"}}]"#)
    };
    reply(
        &server,
        "GET",
        "/search/anime",
        200,
        &one(1, "https://simkl.com/anime/1/x"),
    )
    .await;
    reply(
        &server,
        "GET",
        "/search/tv",
        200,
        &one(2, "https://simkl.com/tv/2/x"),
    )
    .await;
    reply(
        &server,
        "GET",
        "/search/movie",
        200,
        &one(3, "https://simkl.com/movie/3/x"),
    )
    .await;
    let got = t.search("q").await.unwrap();
    let ids: Vec<i64> = got.iter().map(|r| r.remote_id).collect();
    assert_eq!(ids, [1, 2, 3]);
    let order: Vec<String> = sent(&server)
        .await
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(order, ["/search/anime", "/search/tv", "/search/movie"]);
    assert_eq!(got[2].tracking_url, "/movies/3");
}

#[tokio::test]
async fn t_32a_036_one_failing_kind_fails_the_whole_search() {
    let server = MockServer::start().await;
    let (t, _) = tracker(&server);
    reply(&server, "GET", "/search/anime", 200, "[]").await;
    reply(&server, "GET", "/search/tv", 500, "oops").await;
    reply(&server, "GET", "/search/movie", 200, "[]").await;
    let err = t.search("q").await.unwrap_err();
    assert_eq!(err.to_string(), "HTTP 500");
}

#[tokio::test]
async fn t_32a_039_a_listed_tv_show_is_found_through_the_tv_key() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(
        &server,
        "POST",
        "/sync/watched",
        200,
        r#"[{"result":true,"last_watched":"2024-01-02 03:04:05+00:00","list":"watching"}]"#,
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/sync/all-items/shows/watching"))
        .and(query_param("date_from", "2024-01-02 03:04:05+00:00"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"tv":[{"show":{"title":"S","ids":{"simkl":5}},"total_episodes_count":12,"watched_episodes_count":3.0,"user_rating":8}]}"#,
        ))
        .mount(&server)
        .await;
    let got = t.refresh(&show(5, WATCHING, 0.0, 0, 0.0)).await.unwrap();
    assert_eq!(got.total_episodes, 12);
    assert_eq!(got.last_episode_seen, 3.0);
    assert_eq!(got.score, 8.0);
}

#[tokio::test]
async fn t_32a_039_an_unknown_tracking_type_fails_before_the_all_items_request() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(
        &server,
        "POST",
        "/sync/watched",
        200,
        r#"[{"result":true,"last_watched":"x","list":"watching"}]"#,
    )
    .await;
    let mut absolute = show(5, WATCHING, 0.0, 0, 0.0);
    absolute.remote_url = "https://simkl.com/tv/5/x".into();
    let err = t.refresh(&absolute).await.unwrap_err();
    assert_eq!(err.to_string(), "Unknown type: ");
    let paths = sent_paths(&server).await;
    assert_eq!(paths, ["/sync/watched"]);
}

#[tokio::test]
async fn t_32a_039_a_watched_reply_that_is_not_true_means_not_listed() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(
        &server,
        "POST",
        "/sync/watched",
        200,
        r#"[{"result":false}]"#,
    )
    .await;
    let before = show(5, WATCHING, 4.0, 12, 6.0);
    assert_eq!(t.refresh(&before).await.unwrap(), before);
    assert_eq!(sent_paths(&server).await, ["/sync/watched"]);
}

#[tokio::test]
async fn t_32a_040_an_unlisted_title_is_added_with_plan_to_watch_and_no_progress() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(&server, "POST", "/sync/watched", 200, "[]").await;
    reply(&server, "POST", "/sync/add-to-list", 200, "{}").await;
    let mut track = show(5, 0, 0.0, 12, 4.0);
    t.bind(&mut track, false).await.unwrap();
    assert_eq!(track.status, PLAN_TO_WATCH);
    assert_eq!(track.score, 0.0);
    assert_eq!(
        sent_paths(&server).await,
        ["/sync/watched", "/sync/add-to-list"]
    );
    let body = bodies(&server, "/sync/add-to-list").await;
    assert_eq!(
        body[0],
        serde_json::json!({"shows":[{"ids":{"simkl":5},"to":"plantowatch"}]})
    );
}

#[tokio::test]
async fn t_32a_040_an_unlisted_title_with_seen_episodes_is_added_as_watching() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(&server, "POST", "/sync/watched", 200, "[]").await;
    reply(&server, "POST", "/sync/add-to-list", 200, "{}").await;
    let mut track = show(5, 0, 2.0, 12, 0.0);
    t.bind(&mut track, true).await.unwrap();
    assert_eq!(track.status, WATCHING);
    let body = bodies(&server, "/sync/add-to-list").await;
    assert_eq!(body[0]["shows"][0]["to"], "watching");
}

#[tokio::test]
async fn t_32a_040_an_unknown_status_is_refused_before_any_request() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    let err = t
        .update(&show(5, 0, 2.0, 12, 0.0), false)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Unknown status: 0");
    assert!(sent_paths(&server).await.is_empty());
}

#[tokio::test]
async fn t_32a_041_the_last_episode_of_a_known_total_completes_the_title() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    let out = t
        .update(&show(5, WATCHING, 12.0, 12, 0.0), true)
        .await
        .unwrap();
    assert_eq!(out.status, COMPLETED);
    let body = bodies(&server, "/sync/add-to-list").await;
    assert_eq!(body[0]["shows"][0]["to"], "completed");
}

#[tokio::test]
async fn t_32a_041_a_zero_total_never_completes_and_a_partial_count_watches() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    let unknown_total = t
        .update(&show(5, WATCHING, 12.0, 0, 0.0), true)
        .await
        .unwrap();
    assert_eq!(unknown_total.status, WATCHING);
    let partial = t
        .update(&show(5, WATCHING, 3.0, 12, 0.0), true)
        .await
        .unwrap();
    assert_eq!(partial.status, WATCHING);
}

#[tokio::test]
async fn t_32a_041_a_completed_status_is_never_changed_by_update() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    let out = t
        .update(&show(5, COMPLETED, 3.0, 12, 0.0), true)
        .await
        .unwrap();
    assert_eq!(out.status, COMPLETED);
    let body = bodies(&server, "/sync/add-to-list").await;
    assert_eq!(body[0]["shows"][0]["to"], "completed");
}

#[tokio::test]
async fn t_32a_042_progress_is_a_remove_then_an_add_of_truncated_episodes_then_list_then_rating() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 12.9, 24, 0.0), false)
        .await
        .unwrap();
    assert_eq!(
        sent_paths(&server).await,
        [
            "/sync/history/remove",
            "/sync/history",
            "/sync/add-to-list",
            "/sync/ratings/remove"
        ]
    );
    let add = bodies(&server, "/sync/history").await;
    let episodes = add[0]["shows"][0]["seasons"][0]["episodes"]
        .as_array()
        .unwrap();
    assert_eq!(episodes.len(), 12);
    assert_eq!(episodes[11], serde_json::json!({"number": 12}));
    let remove = bodies(&server, "/sync/history/remove").await;
    assert_eq!(
        remove[0]["shows"][0]["seasons"][0],
        serde_json::json!({"number": 1})
    );
}

#[tokio::test]
async fn t_32a_042_zero_episodes_send_an_empty_list_and_a_movie_sends_no_progress() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 0.0, 12, 0.0), false)
        .await
        .unwrap();
    let add = bodies(&server, "/sync/history").await;
    assert_eq!(
        add[0]["shows"][0]["seasons"][0]["episodes"],
        serde_json::json!([])
    );

    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&movie(9, WATCHING), false).await.unwrap();
    assert_eq!(
        sent_paths(&server).await,
        ["/sync/add-to-list", "/sync/ratings/remove"]
    );
    let body = bodies(&server, "/sync/add-to-list").await;
    assert_eq!(body[0]["movies"][0]["ids"]["simkl"], 9);
}

#[tokio::test]
async fn t_32a_042_a_failed_add_after_the_remove_returns_an_error_and_keeps_the_track() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(&server, "POST", "/sync/history/remove", 200, "{}").await;
    reply(&server, "POST", "/sync/history", 500, "boom").await;
    let track = show(5, WATCHING, 3.0, 12, 0.0);
    let err = t.update(&track, false).await.unwrap_err();
    assert_eq!(err.to_string(), "HTTP 500");
    assert_eq!(
        sent_paths(&server).await,
        ["/sync/history/remove", "/sync/history"]
    );
}

#[tokio::test]
async fn t_32a_043_a_zero_score_removes_the_rating_with_rating_zero() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 1.0, 12, 0.0), false)
        .await
        .unwrap();
    let remove = bodies(&server, "/sync/ratings/remove").await;
    assert_eq!(
        remove[0],
        serde_json::json!({"shows":[{"ids":{"simkl":5},"rating":0}]})
    );
    assert!(bodies(&server, "/sync/ratings").await.is_empty());
}

#[tokio::test]
async fn t_32a_043_a_fractional_score_is_sent_as_its_integer_part() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 1.0, 12, 7.9), false)
        .await
        .unwrap();
    let rate = bodies(&server, "/sync/ratings").await;
    assert_eq!(rate[0]["shows"][0]["rating"], 7);
}

#[tokio::test]
async fn t_32a_043_a_negative_score_is_not_sent_as_a_rating() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 1.0, 12, -1.0), false)
        .await
        .unwrap();
    let paths = sent_paths(&server).await;
    assert!(
        !paths.iter().any(|p| p.starts_with("/sync/ratings")),
        "{paths:?}"
    );
}

#[tokio::test]
async fn t_32a_044_a_listed_title_keeps_the_local_library_id_and_takes_the_remote_progress() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    reply(
        &server,
        "POST",
        "/sync/watched",
        200,
        r#"[{"result":true,"last_watched":"x","list":"watching"}]"#,
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/sync/all-items/shows/watching"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"tv":[{"show":{"title":"S","ids":{"simkl":5}},"total_episodes_count":12,"watched_episodes_count":3.0,"user_rating":null}]}"#,
        ))
        .mount(&server)
        .await;
    let mut track = show(5, 0, 0.0, 0, 0.0);
    track.library_id = Some(77);
    t.bind(&mut track, true).await.unwrap();
    assert_eq!(track.library_id, Some(77), "fix D-SK-3");
    assert_eq!(track.last_episode_seen, 3.0);
    assert_eq!(track.status, WATCHING);
    assert_eq!(track.total_episodes, 0, "bind does not copy the total");
}

#[tokio::test]
async fn t_32a_045_refresh_copies_the_total_and_lookup_errors_propagate() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(&server, "POST", "/sync/watched", 500, "down").await;
    let err = t
        .refresh(&show(5, WATCHING, 1.0, 12, 0.0))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "HTTP 500");
}

#[tokio::test]
async fn t_32a_060_a_refused_call_marks_the_login_expired() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    reply(&server, "POST", "/sync/watched", 401, "no").await;
    assert!(t.refresh(&show(5, WATCHING, 1.0, 12, 0.0)).await.is_err());
    assert!(t.auth_expired());
    assert!(!t.is_logged_in());
}

#[tokio::test]
async fn t_32a_060_an_error_snippet_is_redacted_and_the_text_is_the_code_only() {
    let server = MockServer::start().await;
    reply(
        &server,
        "GET",
        "/search/anime",
        400,
        r#"{"error":"bad","access_token":"SECRET-VALUE"}"#,
    )
    .await;
    let auth = Arc::new(auth::SimklAuth::new(None));
    let api = api::SimklApi::new(&server.uri(), auth).unwrap();
    match api.search(api::SearchKind::Anime, "x").await {
        Err(SimklError::Http {
            status,
            body_snippet,
        }) => {
            assert_eq!(status, 400);
            assert!(!body_snippet.contains("SECRET-VALUE"), "{body_snippet}");
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

#[tokio::test]
async fn t_32a_066_a_dropped_caller_does_not_stop_the_second_write() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    Mock::given(method("POST"))
        .and(path("/sync/history/remove"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(40)))
        .mount(&server)
        .await;
    accept_writes(&server).await;
    let caller = tokio::time::timeout(
        std::time::Duration::from_millis(10),
        t.update(&show(5, WATCHING, 3.0, 12, 0.0), false),
    )
    .await;
    assert!(caller.is_err(), "the caller should have been dropped");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        sent_paths(&server)
            .await
            .contains(&"/sync/history".to_owned())
    );
}

// ---- T-32a-049, TST-58: login logs carry no code, token or secret ----

#[tokio::test]
async fn t_32a_049_a_login_writes_no_code_token_or_secret_to_the_log() {
    let server = MockServer::start().await;
    healthy_login(&server).await;
    let (t, _) = tracker(&server);
    let (captured, _guard) = crate::fakes::capture_logs(tracing::Level::TRACE);
    t.login("", "C1").await.unwrap();
    t.refresh(&show(5, WATCHING, 1.0, 12, 0.0)).await.ok();
    let logged = captured.text();
    for secret in ["C1", "AT", api::CLIENT_SECRET] {
        assert!(!logged.contains(secret), "log leaked {secret}: {logged}");
    }
}

// ---- T-32a-061: TST-59 (no blocking in the user lookup) and TST-68 (history body size) ----

/// TST-59 (D-SK-9): the user lookup is async, so a task that shares the one thread finishes while the request is
/// still pending. A blocking lookup would hold the thread and the tick would come after the answer.
#[tokio::test(flavor = "current_thread")]
async fn t_32a_061_the_user_lookup_does_not_block_a_single_thread_runtime() {
    use std::sync::Mutex;
    use std::time::Duration;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/settings"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(300))
                .set_body_string(r#"{"account":{"id":7}}"#),
        )
        .mount(&server)
        .await;
    let (t, _) = tracker(&server);
    t.save_credentials("", "AT").unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let tick = {
        let events = Arc::clone(&events);
        async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            events.lock().unwrap().push("tick");
        }
    };
    let lookup = {
        let events = Arc::clone(&events);
        let api = Arc::clone(&t.api);
        async move {
            let id = api.current_user_id().await.unwrap();
            events.lock().unwrap().push("user");
            id
        }
    };
    let ((), id) = tokio::join!(tick, lookup);
    assert_eq!(id, 7);
    assert_eq!(*events.lock().unwrap(), ["tick", "user"]);
}

/// TST-68 (E-X-9): a show with 2000 watched episodes sends one history body of 2000 objects, no batching, under 64 KiB.
#[tokio::test]
async fn t_32a_061_a_2000_episode_history_is_one_body_under_64_kib() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 2000.0, 2500, 0.0), false)
        .await
        .unwrap();
    let adds: Vec<_> = sent(&server)
        .await
        .into_iter()
        .filter(|r| r.url.path() == "/sync/history")
        .collect();
    assert_eq!(adds.len(), 1, "one request, no batching");
    assert!(adds[0].body.len() < 64 * 1024, "{} bytes", adds[0].body.len());
    let body: serde_json::Value = serde_json::from_slice(&adds[0].body).unwrap();
    let episodes = body["shows"][0]["seasons"][0]["episodes"].as_array().unwrap();
    assert_eq!(episodes.len(), 2000);
    assert_eq!(episodes[1999], serde_json::json!({"number": 2000}));
}

// ---- T-32a-069: live checks (O-4, O-5, O-8), run by hand with a throwaway account ----

fn live_tracker(token: &str) -> SimklTracker {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = SimklTracker::new(secrets, EventBus::default()).unwrap();
    t.save_credentials("", token).unwrap();
    t
}

/// TST-78: search results carry an absolute page URL whose first segment is a known kind, so the stored
/// tracking URL is the relative form. Needs no account.
#[tokio::test]
#[ignore = "live: calls api.simkl.com without an account"]
async fn live_t_32a_078_search_urls_have_a_known_first_segment() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = SimklTracker::new(secrets, EventBus::default()).unwrap();
    let rows = t.search("naruto").await.unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r.tracking_url.starts_with("/anime/")
        || r.tracking_url.starts_with("/tv/")
        || r.tracking_url.starts_with("/movies/")));
    assert!(rows.iter().all(|r| r.source_url.starts_with("https://simkl.com/")));
}

/// TST-76 (O-4): a movie added to the movies list is found in `sync/watched` with that status.
/// Needs `KUROBAYOMI_SIMKL_TOKEN` and `KUROBAYOMI_SIMKL_MOVIE_ID`.
#[tokio::test]
#[ignore = "live: needs KUROBAYOMI_SIMKL_TOKEN and KUROBAYOMI_SIMKL_MOVIE_ID"]
async fn live_t_32a_076_a_movie_added_to_movies_is_found_in_the_movies_list() {
    let token = std::env::var("KUROBAYOMI_SIMKL_TOKEN").expect("set KUROBAYOMI_SIMKL_TOKEN");
    let id: i64 = std::env::var("KUROBAYOMI_SIMKL_MOVIE_ID")
        .expect("set KUROBAYOMI_SIMKL_MOVIE_ID")
        .parse()
        .unwrap();
    let t = live_tracker(&token);
    let api = Arc::clone(&t.api);
    api.add_to_list(api::MediaKind::Movies, id, "plantowatch")
        .await
        .unwrap();
    let watched = api
        .find_watched(id)
        .await
        .unwrap()
        .expect("a watched reply");
    assert_eq!(watched.list.as_deref(), Some("plantowatch"));
}

/// TST-77 (O-5): removing and re-adding the history leaves exactly the episodes sent.
/// Needs `KUROBAYOMI_SIMKL_TOKEN` and `KUROBAYOMI_SIMKL_SHOW_ID` (a show the account can change).
#[tokio::test]
#[ignore = "live: needs KUROBAYOMI_SIMKL_TOKEN and KUROBAYOMI_SIMKL_SHOW_ID"]
async fn live_t_32a_077_history_remove_then_add_leaves_the_sent_count() {
    let token = std::env::var("KUROBAYOMI_SIMKL_TOKEN").expect("set KUROBAYOMI_SIMKL_TOKEN");
    let id: i64 = std::env::var("KUROBAYOMI_SIMKL_SHOW_ID")
        .expect("set KUROBAYOMI_SIMKL_SHOW_ID")
        .parse()
        .unwrap();
    let t = live_tracker(&token);
    let api = Arc::clone(&t.api);
    api.add_to_list(api::MediaKind::Shows, id, "watching")
        .await
        .unwrap();
    api.history_remove(id).await.unwrap();
    api.history_add(id, 2).await.unwrap();
    let watched = api
        .find_watched(id)
        .await
        .unwrap()
        .expect("a watched reply");
    let since = watched
        .last_watched
        .unwrap_or_else(|| "2000-01-01T00:00:00Z".into());
    let all = api.all_items("shows", "watching", &since).await.unwrap();
    let seen = all
        .tv
        .unwrap_or_default()
        .iter()
        .find(|item| item.result("show").is_some_and(|r| r.ids.simkl == id))
        .and_then(|item| item.watched_episodes_count);
    assert_eq!(seen, Some(2.0));
}

// ---- TST-63, TST-69 (cross-cutting): a network failure changes no state; no write body carries a clock value ----

#[tokio::test]
async fn t_32a_061_a_network_failure_changes_no_login_state() {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t =
        SimklTracker::with_base_url("http://127.0.0.1:9", secrets, EventBus::default()).unwrap();
    t.save_credentials("7", "AT").unwrap();
    let err = t
        .refresh(&show(5, WATCHING, 1.0, 12, 0.0))
        .await
        .unwrap_err();
    assert!(err.to_string().starts_with("Network error"), "{err}");
    assert!(t.is_logged_in());
    assert!(!t.auth_expired());
}

#[tokio::test]
async fn t_32a_061_no_write_body_carries_a_date_or_time() {
    let server = MockServer::start().await;
    let t = signed_in(&server).await;
    accept_writes(&server).await;
    t.update(&show(5, WATCHING, 3.0, 12, 7.0), false)
        .await
        .unwrap();
    let writes: Vec<String> = sent(&server)
        .await
        .iter()
        .filter(|r| r.method == wiremock::http::Method::POST && r.url.path() != "/oauth/token")
        .map(|r| String::from_utf8_lossy(&r.body).to_ascii_lowercase())
        .collect();
    assert_eq!(writes.len(), 4);
    for body in writes {
        assert!(!body.contains("date") && !body.contains("time"), "{body}");
    }
}

#[test]
fn t_32a_051_the_tracker_logo_is_the_embedded_black_svg() {
    use crate::tracker::{LogoRef, Tracker};
    let t = offline();
    let LogoRef::Embedded(bytes) = t.logo() else {
        panic!("Simkl returns its embedded logo");
    };
    assert!(bytes.starts_with(b"<svg"));
    assert_eq!(t.logo_color(), 0xFF00_0000);
}
