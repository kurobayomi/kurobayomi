//! MangaUpdates tests on the fixtures of `tests/fixtures/mangaupdates/` (T-32a-061). Every
//! value is invented and holds no real token or user id; a recorded live response replaces each one after the
//! ignored live run (T-32a-069).

use std::sync::Arc;

use kuro_platform::secrets::SecretStore;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::dto::{ListItem, LoginResponse, Rating, SearchResponse};
use super::error::MuError;
use super::*;
use crate::fakes::MemSecrets;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../tests/fixtures/mangaupdates/", $name))
    };
}

fn tracker(server: &MockServer) -> MangaUpdatesTracker {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t =
        MangaUpdatesTracker::with_base_url(&server.uri(), secrets, EventBus::default()).unwrap();
    t.save_credentials("42", "T").unwrap();
    t
}

async fn serve(server: &MockServer, verb: &str, at: &str, status: u16, body: &str) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(ResponseTemplate::new(status).set_body_string(body.to_owned()))
        .mount(server)
        .await;
}

#[test]
fn t_32a_061_the_list_item_fixtures_decode_and_a_null_status_is_absent() {
    let reading: ListItem = serde_json::from_str(fixture!("list_item_reading.json")).unwrap();
    assert_eq!(reading.list_id, Some(0));
    assert_eq!(reading.series.unwrap().id, Some(101));
    let status = reading.status.unwrap();
    assert_eq!((status.volume, status.chapter), (Some(1), Some(12)));
    let null: ListItem = serde_json::from_str(fixture!("list_item_null_status.json")).unwrap();
    assert!(null.status.is_none());
    assert_eq!(null.list_id, Some(1));
}

#[test]
fn t_32a_061_the_rating_fixtures_decode_to_a_value_and_to_none() {
    let some: Rating = serde_json::from_str(fixture!("rating_7_5.json")).unwrap();
    assert_eq!(some.rating, Some(7.5));
    let none: Rating = serde_json::from_str(fixture!("rating_null.json")).unwrap();
    assert_eq!(none.rating, None);
}

#[test]
fn t_32a_061_the_login_fixtures_give_the_context_or_a_decode_error() {
    let ok: LoginResponse = serde_json::from_str(fixture!("login_ok.json")).unwrap();
    assert_eq!((ok.context.uid, ok.context.session_token.as_str()), (42, "FAKE-SESSION-TOKEN"));
    assert!(serde_json::from_str::<LoginResponse>(fixture!("login_missing_context.json")).is_err());
}

#[test]
fn t_32a_061_the_search_fixtures_decode_two_results_one_with_null_type_and_year() {
    let ok: SearchResponse = serde_json::from_str(fixture!("search_ok.json")).unwrap();
    assert_eq!(ok.results.len(), 2);
    let second = &ok.results[1].record;
    assert!(second.kind.is_none() && second.year.is_none());
    let empty: SearchResponse = serde_json::from_str(fixture!("search_empty.json")).unwrap();
    assert!(empty.results.is_empty());
}

#[tokio::test]
async fn t_32a_061_a_fixture_login_stores_the_session_and_a_bodyless_context_fails() {
    let server = MockServer::start().await;
    serve(&server, "PUT", "/v1/account/login", 200, fixture!("login_ok.json")).await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = MangaUpdatesTracker::with_base_url(&server.uri(), secrets, EventBus::default()).unwrap();
    t.login("ann", "pw").await.unwrap();
    assert_eq!((t.username().as_str(), t.password().as_str()), ("42", "FAKE-SESSION-TOKEN"));

    let bad = MockServer::start().await;
    serve(&bad, "PUT", "/v1/account/login", 200, fixture!("login_missing_context.json")).await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = MangaUpdatesTracker::with_base_url(&bad.uri(), secrets, EventBus::default()).unwrap();
    assert!(t.login("ann", "pw").await.is_err());
    assert!(!t.is_logged_in());
}

#[tokio::test]
async fn t_32a_061_fixture_list_and_rating_replies_fill_a_refresh() {
    let server = MockServer::start().await;
    serve(&server, "GET", "/v1/lists/series/101", 200, fixture!("list_item_reading.json")).await;
    serve(&server, "GET", "/v1/series/101/rating", 200, fixture!("rating_7_5.json")).await;
    let t = tracker(&server);
    let mut row = MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TRACKER_ID,
        remote_id: 101,
        library_id: None,
        title: "Example".into(),
        last_chapter_read: 0.0,
        total_chapters: 0,
        status: 0,
        score: 0.0,
        remote_url: String::new(),
        start_date: 0,
        finish_date: 0,
        private: false,
    };
    row = t.refresh(&row).await.unwrap();
    assert_eq!(row.last_chapter_read, 12.0);
    assert_eq!(row.score, 7.5);
}

#[tokio::test]
async fn t_32a_061_a_fixture_search_maps_titles_and_leaves_null_fields_empty() {
    let server = MockServer::start().await;
    serve(&server, "POST", "/v1/series/search", 200, fixture!("search_ok.json")).await;
    let t = tracker(&server);
    let rows = t.search("example").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "Example & Co");
    assert_eq!(rows[1].publishing_type, "");
    assert_eq!(rows[1].start_date, "");
}

#[tokio::test]
async fn t_32a_061_the_error_fixtures_give_the_status_code_and_never_the_body() {
    for (status, body) in [
        (401, fixture!("err_401.json")),
        (404, fixture!("err_404.json")),
        (500, fixture!("err_500.json")),
    ] {
        let server = MockServer::start().await;
        serve(&server, "GET", "/v1/lists/series/101", status, body).await;
        let t = tracker(&server);
        let err = t.api.list_item(101).await.unwrap_err();
        assert_eq!(err.to_string(), format!("HTTP {status}"));
        assert!(matches!(err, MuError::Http { status: s, .. } if s == status));
    }
}
