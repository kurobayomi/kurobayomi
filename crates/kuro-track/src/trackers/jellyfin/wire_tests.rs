//! Wire tests of the Jellyfin client against a local mock server: request shape (path, query, headers) and the
//! progress each response leads to (T-33a-012, T-33a-014, T-33a-016, T-33a-021, T-33a-032, T-33a-036).

use std::sync::Arc;

use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::COMPLETED;
use super::UNSEEN;
use super::WATCHING;
use super::api::JellyfinApi;
use super::auth::JellyfinAuth;
use crate::model::{AnimeTrack, TrackerId};
use crate::trackers::enhanced::EnhancedError;
use crate::trackers::http::{USER_AGENT, tracker_client};
use crate::trackers::sources::memory::MemDirectory;

const KEY: &str = "K1";

/// Recorded server responses (`tests/fixtures/jellyfin/`).
const ITEM_MOVIE: &str = include_str!("../../../tests/fixtures/jellyfin/item_movie.json");
const EPISODES_MIXED: &str = include_str!("../../../tests/fixtures/jellyfin/episodes_mixed.json");
const EPISODES_UNSORTED: &str =
    include_str!("../../../tests/fixtures/jellyfin/episodes_unsorted.json");

fn api_for(user: &str, key: &str) -> JellyfinApi {
    let mut dir = MemDirectory::default();
    dir.install("jellyfin", "all", &[("user_id", user), ("api_key", key)]);
    JellyfinApi::new(tracker_client().unwrap(), JellyfinAuth::new(Arc::new(dir)))
}

fn item(name: &str, id: &str, played: bool) -> String {
    format!(r#"{{"Name":"{name}","Id":"{id}","UserData":{{"Played":{played}}}}}"#)
}

fn episode(id: &str, index: i64, played: bool) -> String {
    format!(
        r#"{{"Name":"Ep {index}","Id":"{id}","IndexNumber":{index},"UserData":{{"Played":{played}}}}}"#
    )
}

fn episodes(items: &[String]) -> String {
    format!(
        r#"{{"Items":[{}],"TotalRecordCount":{}}}"#,
        items.join(","),
        items.len()
    )
}

fn track(url: &str, last: f64) -> AnimeTrack {
    AnimeTrack {
        id: Some(1),
        anime_id: 1,
        tracker_id: TrackerId::JELLYFIN,
        remote_id: 0,
        library_id: None,
        title: "T".into(),
        last_episode_seen: last,
        total_episodes: 0,
        status: 0,
        score: 8.0,
        remote_url: url.into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

async fn mount_item(server: &MockServer, item_path: &str, body: String) {
    Mock::given(method("GET"))
        .and(path(item_path))
        .and(header("X-Emby-Token", KEY))
        .and(header("user-agent", USER_AGENT))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

async fn mount_episodes(server: &MockServer, body: String) {
    Mock::given(method("GET"))
        .and(path("/Shows/R9/Episodes"))
        .and(query_param("seasonId", "S1"))
        .and(query_param("userId", "U1"))
        .and(query_param("SortBy", "IndexNumber"))
        .and(query_param("SortOrder", "Ascending"))
        .and(header("X-Emby-Token", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn t_33a_012_a_movie_read_gives_one_episode_and_the_played_status() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/M1", ITEM_MOVIE.to_owned()).await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(found.title, "Film");
    assert_eq!(found.total_episodes, 1);
    assert_eq!(found.last_episode_seen, 1.0);
    assert_eq!(found.status, COMPLETED);
    assert_eq!(found.tracking_url, url);
}

#[tokio::test]
async fn t_33a_012_an_unplayed_movie_is_unseen_at_zero() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/M1", item("Film", "M1", false)).await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!((found.last_episode_seen, found.status), (0.0, UNSEEN));
}

#[tokio::test]
async fn t_33a_036_a_season_flag_alone_does_not_make_the_season_completed() {
    let server = MockServer::start().await;
    // The season's own flag is played, but its first episode is not watched.
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", true)).await;
    mount_episodes(
        &server,
        episodes(&[episode("e1", 1, false), episode("e2", 2, false)]),
    )
    .await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(found.status, UNSEEN);
    assert_eq!(found.last_episode_seen, 0.0);
    assert_eq!(found.total_episodes, 2);
}

#[tokio::test]
async fn t_33a_014_a_season_read_queries_the_sorted_episode_list() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    mount_episodes(
        &server,
        episodes(&[
            episode("e1", 1, true),
            episode("e2", 2, true),
            episode("e3", 3, false),
        ]),
    )
    .await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!((found.last_episode_seen, found.status), (2.0, WATCHING));
    assert_eq!(found.total_episodes, 3);
}

#[tokio::test]
async fn t_33a_014_e81_a_proxy_prefix_is_used_for_the_episode_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jf/Users/U1/Items/S1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(item("Season 1", "S1", false)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jf/Shows/R9/Episodes"))
        .and(query_param("seasonId", "S1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(episodes(&[episode("e1", 1, false)])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let url = format!("{}/jf/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(found.status, UNSEEN);
}

#[tokio::test]
async fn t_33a_020_e82_a_user_without_a_key_sends_no_request() {
    let server = MockServer::start().await;
    // No mocks: any request would get a 404, so the error must come before any request.
    let url = format!("{}/Users/U9/Items/M1#movie", server.uri());
    let r = api_for("U1", KEY).get_track_search(&url).await;
    assert!(matches!(r, Err(EnhancedError::SourceNotConfigured)));
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn t_33a_021_the_key_travels_in_a_header_never_in_the_url() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/M1", item("Film", "M1", false)).await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    api_for("U1", KEY).get_track_search(&url).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].url.as_str().contains("api_key"));
    assert!(!requests[0].url.as_str().contains(KEY));
}

#[tokio::test]
async fn t_33a_016_e68_a_movie_at_zero_posts_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Users/U1/PlayedItems/M1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    mount_item(&server, "/Users/U1/Items/M1", item("Film", "M1", false)).await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    let updated = api_for("U1", KEY)
        .update_progress(&track(&url, 0.0))
        .await
        .unwrap();
    assert_eq!(updated.status, UNSEEN);
}

#[tokio::test]
async fn t_33a_016_e69_a_movie_at_one_posts_played_with_a_utc_date() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Users/U1/PlayedItems/M1"))
        .and(header("X-Emby-Token", KEY))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    mount_item(&server, "/Users/U1/Items/M1", ITEM_MOVIE.to_owned()).await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    let updated = api_for("U1", KEY)
        .update_progress(&track(&url, 1.0))
        .await
        .unwrap();
    assert_eq!(
        (updated.last_episode_seen, updated.status),
        (1.0, COMPLETED)
    );
    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|r| r.method.as_str() == "POST")
        .unwrap();
    let date = post
        .url
        .query_pairs()
        .find(|(k, _)| k == "DatePlayed")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    assert!(date.ends_with('Z') && date.len() == 24, "got {date}");
}

#[tokio::test]
async fn t_33a_016_e76_a_season_update_marks_the_unplayed_range() {
    let server = MockServer::start().await;
    let eps: Vec<String> = (1..=5)
        .map(|i| episode(&format!("e{i}"), i, false))
        .collect();
    mount_episodes(&server, episodes(&eps)).await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    for id in ["e1", "e2", "e3"] {
        Mock::given(method("POST"))
            .and(path(format!("/Users/U1/PlayedItems/{id}")))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
    }
    for id in ["e4", "e5"] {
        Mock::given(method("POST"))
            .and(path(format!("/Users/U1/PlayedItems/{id}")))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
    }
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    api_for("U1", KEY)
        .update_progress(&track(&url, 3.0))
        .await
        .unwrap();
}

#[tokio::test]
async fn t_33a_016_e89_a_failed_played_post_is_an_error() {
    let server = MockServer::start().await;
    mount_episodes(&server, episodes(&[episode("e1", 1, false)])).await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    Mock::given(method("POST"))
        .and(path("/Users/U1/PlayedItems/e1"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let r = api_for("U1", KEY).update_progress(&track(&url, 1.0)).await;
    assert!(matches!(r, Err(EnhancedError::Http(_))));
}

#[tokio::test]
async fn t_33a_032_e90_an_empty_season_is_a_decode_error_and_the_track_stays() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    mount_episodes(&server, episodes(&[])).await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let r = api_for("U1", KEY).get_track_search(&url).await;
    assert!(matches!(r, Err(EnhancedError::Decode(_))));
}

#[tokio::test]
async fn t_33a_032_a_refresh_keeps_the_local_score() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/M1", ITEM_MOVIE.to_owned()).await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    let local = track(&url, 0.0);
    let remote = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    let refreshed = super::api::apply_remote_progress(&local, &remote);
    assert_eq!(refreshed.score, 8.0);
    assert_eq!(
        (refreshed.last_episode_seen, refreshed.status),
        (1.0, COMPLETED)
    );
}

#[tokio::test]
async fn t_33a_022_an_unauthorized_response_is_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Users/U1/Items/M1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    let r = api_for("U1", KEY).get_track_search(&url).await;
    assert!(matches!(r, Err(EnhancedError::Unauthorized)));
}

#[tokio::test]
async fn t_33a_034_a_match_failure_is_an_error_not_a_silent_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Users/U1/Items/M1"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let url = format!("{}/Users/U1/Items/M1#movie", server.uri());
    assert!(api_for("U1", KEY).get_track_search(&url).await.is_err());
}

#[tokio::test]
async fn t_33a_013_e70_a_mixed_season_reads_the_continuous_prefix() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    mount_episodes(&server, EPISODES_MIXED.to_owned()).await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(
        (found.last_episode_seen, found.status, found.total_episodes),
        (2.0, WATCHING, 5)
    );
}

#[tokio::test]
async fn t_33a_013_e74_an_unsorted_episode_list_is_sorted_by_index_before_the_rule() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    mount_episodes(&server, EPISODES_UNSORTED.to_owned()).await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(
        (found.last_episode_seen, found.status, found.total_episodes),
        (2.0, WATCHING, 3)
    );
}

#[tokio::test]
async fn t_33a_037_e88_a_season_of_150_episodes_is_not_truncated() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    let items: Vec<String> = (1..=150)
        .map(|i| episode(&format!("e{i}"), i, i <= 100))
        .collect();
    mount_episodes(&server, episodes(&items)).await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(found.total_episodes, 150);
    assert_eq!(found.last_episode_seen, 100.0);
    assert_eq!(found.status, WATCHING);
}

#[tokio::test]
async fn t_33a_106_a_season_of_150_episodes_is_read_in_one_short_page() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    let items: Vec<String> = (1..=150)
        .map(|i| episode(&format!("e{i}"), i, true))
        .collect();
    mount_episodes(&server, episodes(&items)).await;
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(found.total_episodes, 150);
}

#[tokio::test]
async fn t_33a_106_a_full_page_is_followed_by_the_next_start_index() {
    let server = MockServer::start().await;
    mount_item(&server, "/Users/U1/Items/S1", item("Season 1", "S1", false)).await;
    let first: Vec<String> = (1..=200)
        .map(|i| episode(&format!("e{i}"), i, true))
        .collect();
    let second: Vec<String> = (201..=250)
        .map(|i| episode(&format!("e{i}"), i, false))
        .collect();
    for (start, body) in [("0", episodes(&first)), ("200", episodes(&second))] {
        Mock::given(method("GET"))
            .and(path("/Shows/R9/Episodes"))
            .and(query_param("StartIndex", start))
            .and(query_param("Limit", "200"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
    }
    let url = format!("{}/Users/U1/Items/S1#seriesId,R9", server.uri());
    let found = api_for("U1", KEY).get_track_search(&url).await.unwrap();
    assert_eq!(found.total_episodes, 250, "both pages are read");
    assert_eq!(
        found.last_episode_seen, 200.0,
        "the watched run ends at episode 200"
    );
}
