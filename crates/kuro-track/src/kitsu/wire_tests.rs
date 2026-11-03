//! Kitsu tracker tests (T-30a-033): wiremock fixtures for the endpoints of the tracker operations, plus the mapping
//! and concurrency checks of T-30a-017 to T-30a-027. The client-level tests are in `api.rs`.

use kuro_platform::kind::Kind;
use std::sync::Arc;
use std::time::Duration;

use kuro_platform::secrets::SecretStore;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::api::{Endpoints, KitsuApi};
use super::auth::{self, OAuth};
use super::error::KitsuError;
use super::search::FILTER_MANGA;
use super::tracker::KitsuTracker;
use super::{COMPLETED, PLAN_TO_READ, READING, WATCHING};
use crate::base::{Field, SECRET_SERVICE, account};
use crate::fakes::MemSecrets;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::tracker::Tracker;

const UA_PREFIX: &str = "Kurobayomi v";

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

fn token(created: i64, refresh: Option<&str>) -> OAuth {
    OAuth {
        access_token: "tok".into(),
        token_type: "bearer".into(),
        created_at: created,
        expires_in: 7200,
        refresh_token: refresh.map(str::to_owned),
    }
}

/// A tracker with a stored fresh token and user id 42.
fn logged_in(server: &MockServer) -> (KitsuTracker, Arc<dyn SecretStore>) {
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    auth::save(secrets.as_ref(), &token(now_secs(), Some("r"))).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    let t = KitsuTracker::new(endpoints(server), secrets.clone()).unwrap();
    (t, secrets)
}

fn manga(status: i64, read: f64, total: i64) -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id: 1,
        tracker_id: TrackerId::KITSU,
        remote_id: 5,
        library_id: None,
        title: "T".into(),
        last_chapter_read: read,
        total_chapters: total,
        status,
        score: 0.0,
        remote_url: "https://kitsu.app/manga/5".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

fn anime(status: i64, seen: f64, total: i64) -> AnimeTrack {
    AnimeTrack {
        id: None,
        anime_id: 1,
        tracker_id: TrackerId::KITSU,
        remote_id: 5,
        library_id: None,
        title: "T".into(),
        last_episode_seen: seen,
        total_episodes: total,
        status,
        score: 0.0,
        remote_url: "https://kitsu.app/anime/5".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

/// `GET library-entries` answer with one entry (library id 77) and its media (chapter or episode count `count`).
fn one_entry(kind: &str, status: &str, progress: i64, private: bool, count: Value) -> Value {
    let count_key = if kind == "manga" {
        "chapterCount"
    } else {
        "episodeCount"
    };
    json!({
        "data": [{"id": "77", "attributes": {"status": status, "progress": progress,
            "ratingTwenty": 16, "startedAt": "2024-05-01T00:00:00.000Z", "finishedAt": null,
            "private": private}}],
        "included": [{"id": 5, "attributes": {"canonicalTitle": "Title", "status": "finished",
            count_key: count}}]
    })
}

async fn mount_json(server: &MockServer, verb: &'static str, at: &str, body: Value) {
    Mock::given(method(verb))
        .and(path(at.to_owned()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

// ---- T-30a-017, T-30a-018, T-30a-028: search ----

#[tokio::test]
async fn t_30a_017_search_key_then_algolia_with_the_exact_headers_and_body() {
    let server = MockServer::start().await;
    mount_json(
        &server,
        "GET",
        "/api/edge/algolia-keys/media/",
        json!({"media": {"key": "k1"}}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/1/indexes/production_media/query/"))
        .and(header("X-Algolia-Application-Id", "AWQO5J657S"))
        .and(header("X-Algolia-API-Key", "k1"))
        .and(header("Content-Type", "application/json; charset=utf-8"))
        .and(body_json(
            json!({"params": format!("query=one+piece{FILTER_MANGA}")}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"hits": [
            {"id": "1", "canonicalTitle": "One Piece", "chapterCount": null, "subtype": "manga",
             "averageRating": 90.0, "startDate": 1_000_000_000, "endDate": null}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    let found = t.as_manga().unwrap().search("one piece").await.unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].tracking_url, "https://kitsu.app/manga/1");
    let algolia = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path().starts_with("/1/"))
        .unwrap();
    assert!(
        algolia.headers.get("authorization").is_none(),
        "KT-92: no bearer to Algolia"
    );
}

#[tokio::test]
async fn t_30a_017_logged_out_search_sends_nothing() {
    let server = MockServer::start().await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    let t = KitsuTracker::new(endpoints(&server), secrets).unwrap();

    let err = t.as_manga().unwrap().search("x").await.unwrap_err();

    assert_eq!(err.to_string(), "Not authenticated with Kitsu");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn t_30a_028_the_key_is_kept_and_a_403_fetches_it_once_more_then_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/edge/algolia-keys/media/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"media": {"key": "k1"}})))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/1/indexes/production_media/query/"))
        .respond_with(ResponseTemplate::new(403))
        .expect(2)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    // First 403 clears the key and fetches it again; the second 403 is returned as it is.
    assert_eq!(
        t.api().search_media(Kind::Manga, "a").await.unwrap_err(),
        KitsuError::Http(403)
    );
}

#[tokio::test]
async fn t_30a_018_novel_hits_are_dropped_and_a_bad_hit_is_skipped() {
    let server = MockServer::start().await;
    mount_json(
        &server,
        "GET",
        "/api/edge/algolia-keys/media/",
        json!({"media": {"key": "k"}}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/1/indexes/production_media/query/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"hits": [
            {"id": 1, "canonicalTitle": "Novel", "subtype": "novel"},
            {"id": 2, "subtype": null},
            {"id": 3, "canonicalTitle": "Good", "subtype": null, "episodeCount": 12,
             "endDate": 1_600_000_000, "averageRating": null}
        ]})))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    let found = t.as_anime().unwrap().search("q").await.unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].remote_id, 3);
    assert_eq!(found[0].total_episodes, 12);
    assert_eq!(found[0].tracking_url, "https://kitsu.app/anime/3");
    assert_eq!(found[0].publishing_status, "Finished");
    assert_eq!(found[0].score, None);
}

// ---- T-30a-019: bind ----

#[tokio::test]
async fn t_30a_019_bind_without_an_entry_posts_planned_and_takes_the_new_id() {
    let server = MockServer::start().await;
    mount_json(
        &server,
        "GET",
        "/api/edge/library-entries",
        json!({"data": []}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/edge/library-entries"))
        .and(body_json(json!({"data": {
            "type": "libraryEntries",
            "attributes": {"status": "planned", "progress": 0, "private": false},
            "relationships": {
                "user": {"data": {"id": "42", "type": "users"}},
                "media": {"data": {"id": 5, "type": "manga"}}
            }
        }})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"id": 555}})))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut m = manga(PLAN_TO_READ, 0.0, 0);

    t.as_manga().unwrap().bind(&mut m, false).await.unwrap();

    assert_eq!(m.remote_id, 555);
    assert_eq!(m.score, 0.0);
    assert_eq!(
        m.remote_url, "https://kitsu.app/manga/5",
        "KT-66: the tracking URL is unchanged"
    );
}

#[tokio::test]
async fn t_30a_019_bind_with_an_entry_copies_progress_and_keeps_the_local_private_flag() {
    let server = MockServer::start().await;
    mount_json(
        &server,
        "GET",
        "/api/edge/library-entries",
        one_entry("manga", "dropped", 7, true, json!(20)),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(path("/api/edge/library-entries/77"))
        .and(body_json(json!({"data": {
            "type": "libraryEntries", "id": 77,
            "attributes": {
                "status": "current", "progress": 7, "ratingTwenty": "16",
                "startedAt": "2024-05-01T00:00:00.000Z", "finishedAt": null, "private": false
            }
        }})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut m = manga(PLAN_TO_READ, 0.0, 0);

    t.as_manga().unwrap().bind(&mut m, true).await.unwrap();

    // KT-44: dropped with progress becomes reading. KT-42: personal fields copied, private stays local.
    assert_eq!(
        (m.status, m.last_chapter_read, m.score),
        (READING, 7.0, 8.0)
    );
    assert_eq!(m.remote_id, 77);
    assert!(!m.private);
    assert_eq!(m.start_date, 1_714_521_600_000);
}

// ---- T-30a-020: update on read or watch event ----

#[tokio::test]
async fn t_30a_020_a_read_event_at_the_total_completes_with_a_finish_date() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/edge/library-entries/77"))
        .and(body_partial_json(json!({"data": {
            "attributes": {"status": "completed", "progress": 12}
        }})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut m = manga(READING, 12.5, 12);
    m.library_id = Some(77);
    m.remote_id = 77;

    let out = t.as_manga().unwrap().update(&m, true).await.unwrap();

    assert_eq!(out.status, COMPLETED);
    assert!(out.finish_date > 0, "KT-52: finished date is set");
}

#[tokio::test]
async fn t_30a_020_no_event_sends_only_the_patch_with_truncated_progress() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/edge/library-entries/77"))
        .and(body_json(json!({"data": {
            "type": "libraryEntries", "id": 77,
            "attributes": {"status": "dropped", "progress": 12, "ratingTwenty": null,
                "startedAt": null, "finishedAt": null, "private": false}
        }})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut m = manga(4, 12.7, 12);
    m.remote_id = 77;

    let out = t.as_manga().unwrap().update(&m, false).await.unwrap();

    assert_eq!(out.status, 4, "KT-56: the status is untouched");
}

#[tokio::test]
async fn t_30a_023_turning_private_on_changes_only_the_private_flag() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/edge/library-entries/77"))
        .and(body_json(json!({"data": {
            "type": "libraryEntries", "id": 77,
            "attributes": {"status": "current", "progress": 3, "ratingTwenty": null,
                "startedAt": null, "finishedAt": null, "private": true}
        }})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let mut a = anime(WATCHING, 3.0, 12);
    a.remote_id = 77;
    a.private = true;

    let out = t.as_anime().unwrap().update(&a, false).await.unwrap();

    assert_eq!((out.status, out.last_episode_seen), (WATCHING, 3.0));
}

// ---- T-30a-021: refresh ----

#[tokio::test]
async fn t_30a_021_refresh_copies_personal_fields_and_the_remote_total() {
    let server = MockServer::start().await;
    mount_json(
        &server,
        "GET",
        "/api/edge/library-entries",
        one_entry("manga", "current", 9, true, json!(null)),
    )
    .await;
    let (t, _) = logged_in(&server);
    let mut m = manga(PLAN_TO_READ, 0.0, 30);
    m.remote_id = 77;
    m.title = "Kept".into();

    let out = t.as_manga().unwrap().refresh(&m).await.unwrap();

    assert_eq!(
        (out.last_chapter_read, out.score, out.status),
        (9.0, 8.0, READING)
    );
    assert!(out.private, "KT-59: refresh copies private");
    assert_eq!(out.total_chapters, 0, "a null count is 0");
    assert_eq!((out.title.as_str(), out.remote_id), ("Kept", 77));
}

#[tokio::test]
async fn t_30a_021_refresh_without_the_entry_is_not_found_with_the_kind_name() {
    let server = MockServer::start().await;
    mount_json(
        &server,
        "GET",
        "/api/edge/library-entries",
        json!({"data": []}),
    )
    .await;
    let (t, _) = logged_in(&server);
    let mut a = anime(WATCHING, 0.0, 0);
    a.remote_id = 77;
    let mut m = manga(READING, 0.0, 0);
    m.remote_id = 77;

    let err_anime = t.as_anime().unwrap().refresh(&a).await.unwrap_err();
    let err_manga = t.as_manga().unwrap().refresh(&m).await.unwrap_err();

    assert_eq!(err_anime.to_string(), "Could not find anime");
    assert_eq!(err_manga.to_string(), "Could not find manga");
}

// ---- T-30a-022: remote delete ----

#[tokio::test]
async fn t_30a_022_remote_delete_sends_delete_and_a_refusal_is_returned() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/edge/library-entries/77"))
        .and(header("Content-Type", "application/vnd.api+json"))
        .respond_with(ResponseTemplate::new(204))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/edge/library-entries/78"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let deletable = t.as_deletable_manga().unwrap();
    let mut ok = manga(READING, 0.0, 0);
    ok.remote_id = 77;
    let mut refused = manga(READING, 0.0, 0);
    refused.remote_id = 78;

    deletable.delete(&ok).await.unwrap();
    let err = deletable.delete(&refused).await.unwrap_err();

    assert_eq!(err.to_string(), "HTTP error 500");
}

#[tokio::test]
async fn t_30a_022_ok_removes_the_local_row_first_and_a_failed_delete_does_not_restore_it() {
    use crate::fakes::MemMangaRepo;
    use crate::repo::MangaTrackRepo;

    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/edge/library-entries/78"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);
    let repo = MemMangaRepo::default();
    let mut track = manga(READING, 0.0, 0);
    track.remote_id = 78;
    repo.upsert(&track).await.unwrap();

    let err = crate::remove::remove_manga(&track, &t, true, &repo)
        .await
        .unwrap_err();

    assert_eq!(crate::messages::english(&err), "HTTP error 500");
    assert!(
        repo.list_for_entry(1).await.unwrap().is_empty(),
        "the local row is gone although the remote delete failed"
    );
}

#[tokio::test]
async fn t_30a_022_an_unbound_track_is_refused_before_any_request() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server);
    let mut m = manga(READING, 0.0, 0);
    m.remote_id = 0;

    let err = t
        .as_deletable_manga()
        .unwrap()
        .delete(&m)
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "Could not find manga");
    assert!(server.received_requests().await.unwrap().is_empty());
}

// ---- T-30a-024, T-30a-025, T-30a-026, T-30a-027: messages, HTTP, concurrency ----

#[tokio::test]
async fn t_30a_025_authed_calls_send_the_kurobayomi_user_agent_and_no_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/edge/library-entries"))
        .and(header("Cache-Control", "no-cache"))
        .and(header("Accept", "application/vnd.api+json"))
        .and(header("Authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .expect(1)
        .mount(&server)
        .await;
    let (t, _) = logged_in(&server);

    t.api().find_entry(Kind::Manga, 5).await.unwrap();

    let request = server.received_requests().await.unwrap().remove(0);
    let agent = request
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(agent.starts_with(UA_PREFIX));
    assert!(agent.ends_with("(io.github.kurobayomi.Kurobayomi)"));
}

#[tokio::test]
async fn t_30a_027_twenty_concurrent_calls_with_an_expired_token_refresh_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "fresh", "token_type": "bearer", "created_at": now_secs(), "expires_in": 7200
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("Authorization", "Bearer fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .expect(20)
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    auth::save(secrets.as_ref(), &token(0, Some("r"))).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    let api = KitsuApi::new(endpoints(&server), secrets).unwrap();

    let calls = (0..20).map(|_| api.find_entry(Kind::Manga, 5));
    let results = futures::future::join_all(calls).await;

    assert!(results.iter().all(|r| matches!(r, Ok(None))));
}

#[tokio::test]
async fn t_30a_027_a_logout_during_a_refresh_discards_the_refreshed_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(300))
                .set_body_json(json!({
                    "access_token": "fresh", "token_type": "bearer", "created_at": now_secs(), "expires_in": 7200
                })),
        )
        .mount(&server)
        .await;
    let secrets: Arc<dyn SecretStore> = MemSecrets::new();
    auth::save(secrets.as_ref(), &token(0, Some("r"))).unwrap();
    secrets
        .set(SECRET_SERVICE, &auth::user_id_account(), "42")
        .unwrap();
    let t = KitsuTracker::new(endpoints(&server), secrets.clone()).unwrap();

    let call = t.api().find_entry(Kind::Manga, 5);
    let logout = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        t.api().logout().unwrap();
    };
    let (result, ()) = tokio::join!(call, logout);

    assert_eq!(result, Err(KitsuError::NotAuthenticated));
    assert_eq!(
        auth::load(&secrets),
        None,
        "KT-97: no token is stored after logout"
    );
    assert_eq!(
        secrets
            .get(SECRET_SERVICE, &account(TrackerId::KITSU, Field::Username))
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn t_30a_026_operations_on_one_entry_wait_for_each_other() {
    let server = MockServer::start().await;
    let (t, _) = logged_in(&server);
    let wait = Duration::from_millis(50);

    let first = t.lock_entry(Kind::Manga, 77).await;
    assert!(
        tokio::time::timeout(wait, t.lock_entry(Kind::Manga, 77))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(wait, t.lock_entry(Kind::Anime, 77))
            .await
            .is_ok()
    );
    drop(first);
    assert!(
        tokio::time::timeout(wait, t.lock_entry(Kind::Manga, 77))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn t_30a_024_the_kitsu_messages_reach_the_toast_unchanged() {
    let err: crate::error::TrackError = KitsuError::MissingMediaData("Anime").into();
    assert_eq!(err.to_string(), "Missing Anime data from Kitsu");
}
