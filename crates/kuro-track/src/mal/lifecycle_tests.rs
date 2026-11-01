//! Login, logout, refresh, delete and logging against a local wiremock server (T-29a-026, T-29a-027, T-29a-035,
//! T-29a-043, T-29a-048). No test contacts myanimelist.net.

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{body_string, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::error::MalError;
use super::redirect::CodeOutcome;
use super::store::MalSettings;
use super::testkit::{MemorySecrets, expired_token, fresh_token, tracker_for};
use super::tracker::MalTracker;
use super::tracker_impl::login_toast;
use crate::messages::MsgKey;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::tracker::Tracker;

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

fn token_body(access: &str) -> serde_json::Value {
    json!({"token_type":"Bearer","expires_in":2592000,"access_token":access,"refresh_token":format!("{access}-refresh")})
}

/// Mounts the token exchange for `verifier` and the user lookup.
async fn mount_exchange(server: &MockServer, verifier: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .and(body_string(format!(
            "client_id=test-client&code=CODE&code_verifier={verifier}&grant_type=authorization_code"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("new-access")))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"fixture_user"})))
        .mount(server)
        .await;
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap_or_default().len()
}

fn header(request: &Request, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[tokio::test]
async fn logout_sends_no_request_and_later_calls_are_not_authenticated() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    mount_exchange(&server, "VER").await;
    tracker.login_with_code("CODE", "VER").await.unwrap();
    let before = request_count(&server).await;

    tracker.logout().unwrap();
    assert_eq!(request_count(&server).await, before, "logout sends nothing");
    assert!(!tracker.is_logged_in());
    assert!(!tracker.auth().expired(), "logout clears the expired flag");

    let err = tracker.api().current_user().await.unwrap_err();
    assert!(matches!(err, MalError::NotAuthenticated));
    assert_eq!(err.to_string(), "MAL: User is not authenticated");
    assert_eq!(
        request_count(&server).await,
        before,
        "the refused call sends nothing"
    );
}

#[tokio::test]
async fn logout_leaves_no_token_and_never_writes_the_word_null() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let tracker = tracker_for(&server, dir.path(), secrets.clone());
    mount_exchange(&server, "VER").await;
    tracker.login_with_code("CODE", "VER").await.unwrap();
    assert!(secrets.token_text().is_some(), "login stores the token");

    tracker.logout().unwrap();
    assert!(secrets.token_text().is_none(), "logout removes the item");
    assert!(secrets.is_empty(), "no other secret is left behind");
}

#[tokio::test]
async fn begin_login_builds_the_url_and_the_code_is_exchanged_with_its_verifier() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    let url = tracker.begin_login().expect("client id is set");
    assert!(url.as_str().starts_with(
        "https://myanimelist.net/v1/oauth2/authorize?client_id=test-client&code_challenge="
    ));
    let verifier = url
        .query_pairs()
        .find(|(k, _)| k == "code_challenge")
        .map(|(_, v)| v.into_owned())
        .expect("challenge");
    assert!(tracker.login_in_progress());
    mount_exchange(&server, &verifier).await;

    tracker
        .finish_login(CodeOutcome::Code("CODE".into()))
        .await
        .expect("login completes with the redirect code");
    assert!(tracker.is_logged_in());
    assert!(!tracker.login_in_progress(), "the verifier is consumed");
}

#[tokio::test]
async fn a_second_login_replaces_the_first_verifier() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    let first = tracker.begin_login().unwrap();
    let second = tracker.begin_login().unwrap();
    let challenge = |u: &url::Url| {
        u.query_pairs()
            .find(|(k, _)| k == "code_challenge")
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    assert_ne!(challenge(&first), challenge(&second));
    mount_exchange(&server, &challenge(&second)).await;
    tracker
        .finish_login(CodeOutcome::Code("CODE".into()))
        .await
        .unwrap();
    assert!(tracker.is_logged_in(), "the latest attempt completes");
}

#[tokio::test]
async fn denied_or_timed_out_login_keeps_the_existing_login_and_sends_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    mount_exchange(&server, "VER").await;
    tracker.login_with_code("CODE", "VER").await.unwrap();
    let before = request_count(&server).await;

    tracker.begin_login().unwrap();
    let denied = tracker.finish_login(CodeOutcome::Denied).await.unwrap_err();
    assert!(matches!(denied, MalError::LoginNotCompleted));
    assert!(tracker.is_logged_in(), "D-04: the existing login stays");
    assert_eq!(
        login_toast(&Err(denied)),
        Some(MsgKey::MalLoginNotCompleted)
    );

    tracker.begin_login().unwrap();
    let timed_out = tracker
        .finish_login(CodeOutcome::TimedOut)
        .await
        .unwrap_err();
    assert!(matches!(timed_out, MalError::LoginTimedOut));
    assert!(tracker.is_logged_in());
    assert!(!tracker.login_in_progress());
    assert_eq!(
        request_count(&server).await,
        before,
        "no request for a refusal"
    );
}

#[tokio::test]
async fn a_failed_exchange_without_a_prior_login_stays_logged_out() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"invalid_grant"})))
        .mount(&server)
        .await;
    let err = tracker
        .login_with_code("BAD", "VER")
        .await
        .expect_err("a 400 fails the login");
    assert!(matches!(err, MalError::Http(400)));
    assert_eq!(
        login_toast(&Err(err)),
        Some(MsgKey::Raw("HTTP error 400".into()))
    );
    assert!(!tracker.is_logged_in());
}

#[tokio::test]
async fn refresh_readds_a_missing_title_and_raises_one_notice() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":false,"status":"reading","num_chapters_read":3.0,"score":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut track = manga_track(5, 1, 3.0, 0);
    tracker.refresh_manga(&mut track).await.expect("re-added");
    assert_eq!(tracker.take_notices(), vec![MsgKey::MalReAdded]);
    assert!(tracker.take_notices().is_empty(), "notices are drained");
}

#[tokio::test]
async fn refresh_of_a_listed_title_only_reads() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10,"my_list_status":{"is_rereading":false,"status":"completed","num_chapters_read":10.0,"score":9}})))
        .mount(&server)
        .await;
    let mut track = manga_track(5, 1, 0.0, 0);
    tracker.refresh_manga(&mut track).await.unwrap();
    assert_eq!(track.status, 2);
    assert!(tracker.take_notices().is_empty());
}

#[tokio::test]
async fn api_calls_send_the_bearer_and_no_cookie_or_cache_headers() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v2/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"fixture_user"})))
        .mount(&server)
        .await;

    tracker.api().current_user().await.unwrap();
    tracker.api().current_user().await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(
            header(request, "authorization").as_deref(),
            Some("Bearer a")
        );
        assert!(header(request, "cookie").is_none(), "no cookie store");
        assert!(
            header(request, "cache-control").is_none(),
            "no HTTP cache header"
        );
    }
}

#[tokio::test]
async fn the_exchange_request_carries_no_bearer() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    mount_exchange(&server, "VER").await;
    tracker.login_with_code("CODE", "VER").await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let exchange = requests
        .iter()
        .find(|r| r.url.path() == "/v1/oauth2/token")
        .unwrap();
    assert!(header(exchange, "authorization").is_none());
}

#[tokio::test]
async fn the_trait_object_reaches_every_operation() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let concrete = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    mount_exchange(&server, "VER").await;
    concrete.login_with_code("CODE", "VER").await.unwrap();
    let tracker: Arc<dyn Tracker> = Arc::new(concrete);
    let mut login_state = tracker.login_state();
    assert!(tracker.is_logged_in());

    Mock::given(method("GET"))
        .and(path("/v2/manga"))
        .and(query_param("q", "naruto"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"node":{"id":1}}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/manga/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":1,"title":"Naruto","synopsis":"s","num_chapters":700,"mean":8.2,"status":"finished","media_type":"manga"})))
        .mount(&server)
        .await;
    let manga = tracker.as_manga().expect("manga view");
    let hits = manga.search("naruto").await.expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].score, Some(8.2));

    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"is_rereading":false,"status":"reading","num_chapters_read":4.0,"score":0}),
        ))
        .mount(&server)
        .await;
    let mut track = manga_track(5, 1, 3.0, 0);
    let updated = manga.update(&track, true).await.expect("update");
    assert_eq!(updated.last_chapter_read, 4.0);

    Mock::given(method("GET"))
        .and(path("/v2/manga/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"num_chapters":10,"my_list_status":{"is_rereading":false,"status":"reading","num_chapters_read":4.0,"score":0}})))
        .mount(&server)
        .await;
    manga.refresh(&track).await.expect("refresh reads the list");
    manga.bind(&mut track, true).await.expect("bind");

    Mock::given(method("DELETE"))
        .and(path("/v2/manga/5/my_list_status"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    tracker
        .as_deletable_manga()
        .expect("deletable")
        .delete(&track)
        .await
        .expect("delete");

    let anime = tracker.as_anime().expect("anime view");
    assert_eq!(anime.status_list().len(), 6);
    let anime_track = AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TrackerId::MYANIMELIST,
        remote_id: 9,
        library_id: None,
        title: "t".into(),
        last_episode_seen: 0.0,
        total_episodes: 0,
        status: 11,
        score: 0.0,
        remote_url: String::new(),
        start_date: 0,
        finish_date: 0,
        private: false,
    };
    assert_eq!(anime.display_score(&anime_track), "0");

    assert!(
        *login_state.borrow_and_update(),
        "the watcher saw the login"
    );
    tracker.logout().expect("logout");
    assert!(
        !*login_state.borrow_and_update(),
        "the watcher sees the logout"
    );
}

#[tokio::test]
async fn a_remote_delete_failure_is_logged_and_the_local_removal_still_happens() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    tracker.auth().set_token(fresh_token("a")).unwrap();
    Mock::given(method("DELETE"))
        .and(path("/v2/manga/5/my_list_status"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let track = manga_track(5, 1, 0.0, 0);
    let deletable = tracker.as_deletable_manga().expect("deletable");
    assert!(
        deletable.delete(&track).await.is_ok(),
        "no toast, the local row goes"
    );
}

#[tokio::test]
async fn a_failed_remote_delete_is_logged_at_error_and_a_404_at_warn() {
    let (captured, _guard) = crate::fakes::capture_logs(tracing::Level::TRACE);

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    tracker.auth().set_token(fresh_token("a")).unwrap();
    for (id, status) in [(5, 500), (6, 404)] {
        Mock::given(method("DELETE"))
            .and(path(format!("/v2/manga/{id}/my_list_status")))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
    }
    let deletable = tracker.as_deletable_manga().expect("deletable");
    deletable.delete(&manga_track(5, 1, 0.0, 0)).await.unwrap();
    let logs = captured.text();
    assert!(
        logs.lines()
            .any(|l| l.contains("ERROR") && l.contains("Failed to remove the MAL entry")),
        "{logs}"
    );
    deletable.delete(&manga_track(6, 1, 0.0, 0)).await.unwrap();
    let logs = captured.text();
    assert!(
        logs.lines()
            .any(|l| l.contains("WARN") && l.contains("not on the list")),
        "{logs}"
    );
}

#[tokio::test]
async fn a_remote_update_failure_is_logged_with_the_title_id_and_no_token() {
    let (captured, _guard) = crate::fakes::capture_logs(tracing::Level::TRACE);

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tracker = tracker_for(&server, dir.path(), MemorySecrets::default().into());
    tracker
        .auth()
        .set_token(expired_token("fixture-access"))
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body("new-access")))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v2/manga/5/my_list_status"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let concrete: Arc<dyn Tracker> = Arc::new(tracker);
    let result = concrete
        .as_manga()
        .expect("manga view")
        .update(&manga_track(5, 1, 3.0, 0), true)
        .await;
    assert!(result.is_err());

    let logs = captured.text();
    assert!(
        logs.contains("Failed to update remote track data id=5"),
        "{logs}"
    );
    for secret in [
        "fixture-access",
        "new-access",
        "fixture-refresh",
        "new-access-refresh",
    ] {
        assert!(!logs.contains(secret), "token value in logs: {logs}");
    }
}


#[allow(dead_code)]
fn _tracker_type_is_send_sync() {
    fn assert_bounds<T: Send + Sync>() {}
    assert_bounds::<MalTracker>();
}

#[tokio::test]
async fn t_29a_038_a_restart_between_the_two_steps_still_completes_the_login() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets: Arc<MemorySecrets> = MemorySecrets::default().into();
    let challenge = {
        let tracker = tracker_for(&server, dir.path(), secrets.clone());
        let url = tracker.begin_login().expect("client id is set");
        url.query_pairs()
            .find(|(k, _)| k == "code_challenge")
            .map(|(_, v)| v.into_owned())
            .expect("challenge")
    };
    // The verifier is written to the config file while the login waits.
    let stored = MalSettings::open(dir.path()).unwrap().pending_verifier();
    assert!(stored.is_some(), "pending_verifier is persisted");

    // A fresh tracker (the restart) reads it back and completes the login.
    let tracker = tracker_for(&server, dir.path(), secrets.clone());
    assert!(tracker.login_in_progress());
    mount_exchange(&server, &challenge).await;
    tracker
        .finish_login(CodeOutcome::Code("CODE".into()))
        .await
        .expect("the login completes after the restart");
    assert!(tracker.is_logged_in());
    assert_eq!(
        MalSettings::open(dir.path()).unwrap().pending_verifier(),
        None,
        "removed once the code is consumed"
    );
}
