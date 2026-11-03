//! Simkl tests on the fixtures of `tests/fixtures/simkl/` (T-32a-061). Every value is invented
//! and holds no real token or user id. The `all-items` and `search` keys stay unverified until a live run records them
//! (O-8, T-32a-069).

use std::sync::Arc;

use kuro_platform::secrets::SecretStore;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::api::SearchKind;
use super::dto::{SimklOAuth, SimklSearchResult, SimklSyncResult, SimklSyncWatched, SimklUser};
use super::error::SimklError;
use super::*;
use crate::fakes::MemSecrets;
use crate::model::AnimeTrack;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../tests/fixtures/simkl/", $name))
    };
}

fn tracker(server: &MockServer) -> SimklTracker {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    SimklTracker::with_base_url(&server.uri(), secrets, EventBus::default()).unwrap()
}

async fn serve(server: &MockServer, verb: &str, at: &str, status: u16, body: &str) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(ResponseTemplate::new(status).set_body_string(body.to_owned()))
        .mount(server)
        .await;
}

fn row(remote_id: i64, url: &str) -> AnimeTrack {
    AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TRACKER_ID,
        remote_id,
        library_id: None,
        title: "T".into(),
        last_episode_seen: 0.0,
        total_episodes: 0,
        status: 0,
        score: 0.0,
        remote_url: url.into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

#[test]
fn t_32a_061_the_oauth_and_user_fixtures_decode_or_fail_as_the_plan_says() {
    let ok: SimklOAuth = serde_json::from_str(fixture!("oauth_ok.json")).unwrap();
    assert_eq!(ok.access_token, "FAKE-ACCESS-TOKEN");
    assert!(serde_json::from_str::<SimklOAuth>(fixture!("oauth_400.json")).is_err());
    let user: SimklUser = serde_json::from_str(fixture!("user_settings.json")).unwrap();
    assert_eq!(user.account.id, 7);
    assert!(serde_json::from_str::<SimklUser>(fixture!("user_settings_missing.json")).is_err());
}

#[test]
fn t_32a_061_the_watched_fixtures_decode_to_listed_not_listed_and_result_false() {
    let ok: Vec<SimklSyncWatched> = serde_json::from_str(fixture!("watched_ok.json")).unwrap();
    assert_eq!(ok[0].result, Some(true));
    assert_eq!(ok[0].list.as_deref(), Some("watching"));
    let empty: Vec<SimklSyncWatched> = serde_json::from_str(fixture!("watched_empty.json")).unwrap();
    assert!(empty.is_empty());
    let no: Vec<SimklSyncWatched> = serde_json::from_str(fixture!("watched_result_false.json")).unwrap();
    assert_eq!(no[0].result, Some(false));
}

#[test]
fn t_32a_061_the_all_items_fixtures_map_through_to_track() {
    let tv: SimklSyncResult = serde_json::from_str(fixture!("all_items_tv.json")).unwrap();
    let t = sync::to_track(&tv.tv.unwrap()[0], "show", "tv", "watching").unwrap();
    assert_eq!((t.remote_id, t.total_episodes, t.last_episode_seen, t.score), (5, 12, 3.0, 8.0));
    assert_eq!(t.remote_url, "/tv/5");

    let movies: SimklSyncResult = serde_json::from_str(fixture!("all_items_movies.json")).unwrap();
    let m = sync::to_track(&movies.movies.unwrap()[0], "movie", "movies", "completed").unwrap();
    assert_eq!((m.total_episodes, m.last_episode_seen, m.score), (1, 1.0, 0.0));

    let anime: SimklSyncResult = serde_json::from_str(fixture!("all_items_anime.json")).unwrap();
    let a = sync::to_track(&anime.anime.unwrap()[0], "show", "anime", "completed").unwrap();
    assert_eq!((a.remote_url.as_str(), a.last_episode_seen), ("/anime/42", 24.0));

    let broken: SimklSyncResult = serde_json::from_str(fixture!("all_items_show_null_counts.json")).unwrap();
    assert!(matches!(
        sync::to_track(&broken.tv.unwrap()[0], "show", "tv", "watching"),
        Err(SimklError::MissingField("total_episodes_count"))
    ));
}

#[test]
fn t_32a_061_the_search_fixtures_map_to_rows_with_the_stored_and_the_source_url() {
    let anime: Vec<SimklSearchResult> = serde_json::from_str(fixture!("search_anime.json")).unwrap();
    let first = search::to_track_search(&anime[0], SearchKind::Anime);
    assert_eq!(first.title, "Romaji Title");
    assert_eq!(first.tracking_url, "/anime/42");
    assert_eq!(first.source_url, "https://simkl.com/anime/42/romaji-title");
    assert_eq!(first.cover_url, "https://simkl.in/posters/ab/abcd1234_m.webp");
    // The record with no titles, an empty title list and no poster.
    let bare = search::to_track_search(&anime[1], SearchKind::Anime);
    assert_eq!((bare.title.as_str(), bare.cover_url.as_str()), ("", ""));
    assert_eq!(bare.summary, "All titles:\n");
    assert_eq!(bare.publishing_status, "ended");

    let tv: Vec<SimklSearchResult> = serde_json::from_str(fixture!("search_tv.json")).unwrap();
    assert_eq!(search::to_track_search(&tv[0], SearchKind::Tv).tracking_url, "/tv/5");

    let movie: Vec<SimklSearchResult> = serde_json::from_str(fixture!("search_movie.json")).unwrap();
    let m = search::to_track_search(&movie[0], SearchKind::Movie);
    assert_eq!((m.tracking_url.as_str(), m.cover_url.as_str()), ("/movies/9", ""));
}

#[tokio::test]
async fn t_32a_061_a_fixture_login_reaches_the_user_id_and_a_rejected_code_fails() {
    let server = MockServer::start().await;
    serve(&server, "POST", "/oauth/token", 200, fixture!("oauth_ok.json")).await;
    serve(&server, "GET", "/users/settings", 200, fixture!("user_settings.json")).await;
    let t = tracker(&server);
    t.login_code("C1").await.unwrap();
    assert_eq!((t.username().as_str(), t.password().as_str()), ("7", "FAKE-ACCESS-TOKEN"));

    let bad = MockServer::start().await;
    serve(&bad, "POST", "/oauth/token", 400, fixture!("oauth_400.json")).await;
    let t = tracker(&bad);
    assert_eq!(t.login_code("C1").await.unwrap_err().to_string(), "HTTP 400");
    assert!(!t.is_logged_in());
}

#[tokio::test]
async fn t_32a_061_the_watched_and_all_items_fixtures_bind_a_listed_show() {
    let server = MockServer::start().await;
    serve(&server, "POST", "/sync/watched", 200, fixture!("watched_ok.json")).await;
    serve(&server, "GET", "/sync/all-items/shows/watching", 200, fixture!("all_items_tv.json")).await;
    let t = tracker(&server);
    t.save_credentials("7", "AT").unwrap();
    let got = t.refresh(&row(5, "/tv/5")).await.unwrap();
    assert_eq!((got.total_episodes, got.last_episode_seen, got.score), (12, 3.0, 8.0));

    for body in [fixture!("watched_empty.json"), fixture!("watched_result_false.json")] {
        let server = MockServer::start().await;
        serve(&server, "POST", "/sync/watched", 200, body).await;
        let t = tracker(&server);
        t.save_credentials("7", "AT").unwrap();
        let same = t.refresh(&row(5, "/tv/5")).await.unwrap();
        assert_eq!(same.total_episodes, 0, "not listed: unchanged");
    }
}

#[tokio::test]
async fn t_32a_061_the_three_search_fixtures_concatenate_and_a_failing_kind_fails_all() {
    let server = MockServer::start().await;
    serve(&server, "GET", "/search/anime", 200, fixture!("search_anime.json")).await;
    serve(&server, "GET", "/search/tv", 200, fixture!("search_tv.json")).await;
    serve(&server, "GET", "/search/movie", 200, fixture!("search_movie.json")).await;
    let t = tracker(&server);
    let rows = t.search("x").await.unwrap();
    let urls: Vec<_> = rows.iter().map(|r| r.tracking_url.as_str()).collect();
    assert_eq!(urls, ["/anime/42", "/anime/43", "/tv/5", "/movies/9"]);

    let partial = MockServer::start().await;
    serve(&partial, "GET", "/search/anime", 200, fixture!("search_anime.json")).await;
    serve(&partial, "GET", "/search/tv", 500, fixture!("search_tv_500.json")).await;
    serve(&partial, "GET", "/search/movie", 200, fixture!("search_movie.json")).await;
    assert!(tracker(&partial).search("x").await.is_err());
}
