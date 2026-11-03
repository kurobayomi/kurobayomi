//! Jellyfin tracker (anime, id 102; `Jellyfin.kt`; T-33a-030 to T-33a-035).
//!
//! Matching, `is_track_from` and `migrate_track` go through the `EnhancedAnimeTracker` contract (T-33a-006,
//! T-33a-034), so the bind path reaches them through `as_enhanced_anime`.

pub mod api;
pub mod auth;
pub mod dto;
pub mod progress;
pub mod url;

#[cfg(test)]
mod wire_tests;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::watch;

use self::api::{JellyfinApi, apply_remote_progress};
use self::auth::JellyfinAuth;
use crate::error::TrackError;
use crate::model::search::AnimeTrackSearch;
use crate::model::{AnimeTrack, TrackerId};
use crate::tracker::{AnimeTracker, EnhancedAnimeTracker, LogoRef, Tracker};
use crate::trackers::enhanced::{
    EnhancedError, EntryRef, FlagStore, LoginFlag, is_track_from, migrate_remote_url,
};
use crate::trackers::http::tracker_client;
use crate::trackers::sources::SourceDirectory;

/// Status constants (`Jellyfin.kt:23-25`). Rewatching is -1 (no rewatch status).
pub const UNSEEN: i64 = 1;
pub const WATCHING: i64 = 2;
pub const COMPLETED: i64 = 3;
pub const REWATCHING: i64 = -1;

/// Accepted source identity: `jellyfin/all`.
pub const ACCEPTED_SOURCES: &[&str] = &["jellyfin/all"];

/// Logo colour `rgb(0, 11, 37)` as `0xAARRGGBB` (`Jellyfin.kt:39`).
pub const LOGO_COLOR: u32 = 0xFF00_0B25;

/// Logo name in the GResource bundle (T-33a-100).
pub const LOGO_NAME: &str = "ic_tracker_jellyfin";

pub struct JellyfinTracker {
    login: LoginFlag,
    api: JellyfinApi,
}

impl JellyfinTracker {
    pub fn new(
        dir: Arc<dyn SourceDirectory>,
        flags: Arc<dyn FlagStore>,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            login: LoginFlag::new(TrackerId::JELLYFIN, flags),
            api: JellyfinApi::new(tracker_client()?, JellyfinAuth::new(dir)),
        })
    }
}

#[async_trait]
impl EnhancedAnimeTracker for JellyfinTracker {
    fn accepted_sources(&self) -> &[&'static str] {
        ACCEPTED_SOURCES
    }

    /// `remote_url == entry.url` and an accepted source (E-96, T-33a-034).
    fn is_track_from(&self, t: &AnimeTrack, entry: &EntryRef, source_id: Option<&str>) -> bool {
        is_track_from(&t.remote_url, &entry.url, source_id, ACCEPTED_SOURCES)
    }

    /// Copies the new entry URL into the track for an accepted source (E-97, E-98).
    fn migrate_track(
        &self,
        t: &AnimeTrack,
        new_source_id: &str,
        new_entry: &EntryRef,
    ) -> Option<AnimeTrack> {
        migrate_remote_url(new_source_id, ACCEPTED_SOURCES, &new_entry.url).map(|url| {
            let mut out = t.clone();
            out.remote_url = url;
            out
        })
    }

    /// Fetches the entry's tracking URL. A failure is `Err`, never a silent `None` (D-29).
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<AnimeTrackSearch>, TrackError> {
        Ok(Some(self.api.get_track_search(&entry.url).await?))
    }
}

#[async_trait]
impl Tracker for JellyfinTracker {
    fn id(&self) -> TrackerId {
        TrackerId::JELLYFIN
    }

    fn name(&self) -> &str {
        "Jellyfin"
    }

    fn logo(&self) -> LogoRef {
        LogoRef::IconName(LOGO_NAME)
    }

    fn logo_color(&self) -> u32 {
        LOGO_COLOR
    }

    fn completion_status(&self) -> i64 {
        COMPLETED
    }

    fn score_list(&self) -> &[String] {
        &[]
    }

    /// The enhanced login takes no credentials (`loginNoop`, T-33a-035).
    async fn login(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        self.login.login_noop()
    }

    fn logout(&self) -> Result<(), TrackError> {
        self.login.logout()
    }

    fn is_logged_in(&self) -> bool {
        self.login.is_logged_in()
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.login.login_state()
    }

    /// Nothing is stored: enhanced trackers keep no secret.
    fn save_credentials(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        Ok(())
    }

    fn username(&self) -> String {
        String::new()
    }

    fn password(&self) -> String {
        String::new()
    }

    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        Some(self)
    }

    fn as_enhanced_anime(&self) -> Option<&dyn EnhancedAnimeTracker> {
        Some(self)
    }
}

#[async_trait]
impl AnimeTracker for JellyfinTracker {
    fn status_list(&self) -> Vec<i64> {
        vec![UNSEEN, WATCHING, COMPLETED]
    }

    fn watching_status(&self) -> i64 {
        WATCHING
    }

    fn rewatching_status(&self) -> i64 {
        REWATCHING
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        match status {
            UNSEEN => Some("unseen"),
            WATCHING => Some("watching"),
            COMPLETED => Some("completed"),
            _ => None,
        }
    }

    fn display_score(&self, _t: &AnimeTrack) -> String {
        String::new()
    }

    /// `did_watch_episode` is ignored (D-31, parity).
    async fn update(
        &self,
        t: &AnimeTrack,
        _did_watch_episode: bool,
    ) -> Result<AnimeTrack, TrackError> {
        Ok(self.api.update_progress(t).await?)
    }

    /// Returns the track unchanged (T-33a-033).
    async fn bind(&self, _t: &mut AnimeTrack, _has_seen: bool) -> Result<(), TrackError> {
        Ok(())
    }

    async fn search(&self, _query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        Err(EnhancedError::SearchUnsupported.into())
    }

    /// Copies only the remote-owned progress fields (D-03, E-91).
    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        let remote = self.api.get_track_search(&t.remote_url).await?;
        Ok(apply_remote_progress(t, &remote))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::MsgKey;
    use crate::trackers::enhanced::MemFlags;

    fn tracker() -> JellyfinTracker {
        let dir = crate::trackers::sources::memory::MemDirectory::default();
        JellyfinTracker::new(Arc::new(dir), Arc::new(MemFlags::default())).unwrap()
    }

    fn track(url: &str) -> AnimeTrack {
        AnimeTrack {
            id: Some(1),
            anime_id: 1,
            tracker_id: TrackerId::JELLYFIN,
            remote_id: 0,
            library_id: None,
            title: "T".into(),
            last_episode_seen: 0.0,
            total_episodes: 0,
            status: 0,
            score: 8.0,
            remote_url: url.into(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    #[test]
    fn t_33a_030_identity_and_status_constants() {
        let t = tracker();
        assert_eq!(t.id(), TrackerId::JELLYFIN);
        assert_eq!(t.name(), "Jellyfin");
        assert_eq!((UNSEEN, WATCHING, COMPLETED, REWATCHING), (1, 2, 3, -1));
        assert_eq!(t.completion_status(), COMPLETED);
        let anime = t.as_anime().expect("anime view");
        assert_eq!(anime.status_list(), vec![1, 2, 3]);
        assert_eq!(anime.watching_status(), WATCHING);
        assert_eq!(anime.rewatching_status(), -1);
    }

    #[test]
    fn t_33a_030_status_labels_and_empty_score_list() {
        let t = tracker();
        let anime = t.as_anime().unwrap();
        assert_eq!(anime.status_label(1), Some("unseen"));
        assert_eq!(anime.status_label(2), Some("watching"));
        assert_eq!(anime.status_label(3), Some("completed"));
        assert_eq!(anime.status_label(9), None);
        assert!(t.score_list().is_empty());
        assert_eq!(
            anime.display_score(&track("https://h/Users/u/Items/m#movie")),
            ""
        );
    }

    #[test]
    fn t_33a_031_logo_colour_is_the_jellyfin_blue() {
        assert_eq!(tracker().logo_color(), 0xFF00_0B25);
        assert!(matches!(tracker().logo(), LogoRef::IconName(LOGO_NAME)));
    }

    #[tokio::test]
    async fn t_33a_033_bind_returns_the_track_and_search_is_unsupported() {
        let t = tracker();
        let anime = t.as_anime().unwrap();
        let mut tr = track("https://h/Users/u/Items/m#movie");
        anime.bind(&mut tr, true).await.unwrap();
        assert_eq!(tr.remote_url, "https://h/Users/u/Items/m#movie");
        let err = anime.search("x").await.unwrap_err();
        assert_eq!(
            err.toast("Jellyfin"),
            MsgKey::enhanced("track.enhanced.search_unsupported", "")
        );
    }

    fn entry(url: &str) -> EntryRef {
        EntryRef {
            id: 1,
            source_id: 5,
            url: url.into(),
            title: "T".into(),
        }
    }

    #[test]
    fn t_33a_034_is_track_from_compares_the_entry_url_and_the_identity() {
        let t = tracker();
        let e = t.as_enhanced_anime().expect("enhanced view");
        let tr = track("https://h/Users/u/Items/m#movie");
        let same = entry("https://h/Users/u/Items/m#movie");
        assert!(e.is_track_from(&tr, &same, Some("jellyfin/all")));
        assert!(!e.is_track_from(&tr, &entry("https://h/other#movie"), Some("jellyfin/all")));
        assert!(!e.is_track_from(&tr, &same, Some("kavita/all")));
    }

    #[test]
    fn t_33a_034_migrate_copies_the_url_only_for_an_accepted_source() {
        let t = tracker();
        let e = t.as_enhanced_anime().unwrap();
        let tr = track("https://h/Users/u/Items/m#movie");
        let new = entry("https://h/Users/u/Items/n#movie");
        let moved = e.migrate_track(&tr, "jellyfin/all", &new).unwrap();
        assert_eq!(moved.remote_url, "https://h/Users/u/Items/n#movie");
        assert!(e.migrate_track(&tr, "komga/all", &new).is_none());
    }

    #[tokio::test]
    async fn t_33a_035_login_sets_the_flag_and_logout_clears_it() {
        let t = tracker();
        assert!(!t.is_logged_in());
        t.login("", "").await.unwrap();
        assert!(t.is_logged_in());
        assert!(*t.login_state().borrow());
        t.logout().unwrap();
        assert!(!t.is_logged_in());
        assert_eq!(t.username(), "");
    }

    #[test]
    fn t_33a_035_the_accepted_identity_is_jellyfin_all() {
        let t = tracker();
        assert_eq!(
            t.as_enhanced_anime().unwrap().accepted_sources(),
            &["jellyfin/all"]
        );
    }

    #[tokio::test]
    async fn t_33a_022_a_missing_login_reaches_the_toast_as_the_login_key() {
        // No source preferences are installed, so the key lookup fails with SourceNotConfigured.
        let t = tracker();
        let err = t
            .as_enhanced_anime()
            .unwrap()
            .match_entry(&entry("https://h/Users/u/Items/m#movie"))
            .await
            .unwrap_err();
        assert_eq!(
            err.toast("Jellyfin"),
            MsgKey::enhanced("track.enhanced.login_required", "")
        );
    }

    /// A real Jellyfin tracker inside the fake registry, with the source's key installed.
    async fn bind_harness(logged_in: bool) -> crate::fakes::Harness {
        let mut dir = crate::trackers::sources::memory::MemDirectory::default();
        dir.install("jellyfin", "all", &[("user_id", "U1"), ("api_key", "K1")]);
        let t =
            Arc::new(JellyfinTracker::new(Arc::new(dir), Arc::new(MemFlags::default())).unwrap());
        if logged_in {
            t.login("", "").await.unwrap();
        }
        crate::fakes::Harness::with_tracker(Some(t))
    }

    async fn movie_server() -> wiremock::MockServer {
        use wiremock::matchers::{method, path};
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(method("GET"))
            .and(path("/Users/U1/Items/M1"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string(r#"{"Name":"Film","Id":"M1","UserData":{"Played":false}}"#),
            )
            .mount(&server)
            .await;
        server
    }

    fn movie_entry(server: &wiremock::MockServer) -> EntryRef {
        entry(&format!("{}/Users/U1/Items/M1#movie", server.uri()))
    }

    #[tokio::test]
    async fn t_33a_034_e94_a_logged_out_jellyfin_is_skipped_by_the_bind_path() {
        let h = bind_harness(false).await;
        let server = movie_server().await;
        let failures =
            crate::bind::bind_enhanced_anime_entry(&h.ctx, &movie_entry(&server), "jellyfin/all")
                .await;
        assert!(failures.is_empty());
        assert!(
            h.ctx
                .anime_tracks
                .list_for_entry(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn t_33a_034_e95_a_source_jellyfin_does_not_accept_is_skipped_by_the_bind_path() {
        let h = bind_harness(true).await;
        let server = movie_server().await;
        let failures =
            crate::bind::bind_enhanced_anime_entry(&h.ctx, &movie_entry(&server), "komga/all")
                .await;
        assert!(failures.is_empty());
        assert!(
            h.ctx
                .anime_tracks
                .list_for_entry(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn t_33a_034_the_bind_path_matches_by_entry_url_and_stores_the_binding() {
        let h = bind_harness(true).await;
        let server = movie_server().await;
        let e = movie_entry(&server);
        let failures = crate::bind::bind_enhanced_anime_entry(&h.ctx, &e, "jellyfin/all").await;
        assert!(failures.is_empty(), "{failures:?}");
        let rows = h.ctx.anime_tracks.list_for_entry(1).await.unwrap();
        assert_eq!(
            (rows.len(), rows[0].remote_url.as_str()),
            (1, e.url.as_str())
        );
    }

    #[tokio::test]
    async fn t_33a_034_a_failed_match_is_returned_and_nothing_is_stored() {
        let h = bind_harness(true).await;
        let failures = crate::bind::bind_enhanced_anime_entry(
            &h.ctx,
            &entry("not a tracking url"),
            "jellyfin/all",
        )
        .await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, TrackerId::JELLYFIN);
        assert!(
            h.ctx
                .anime_tracks
                .list_for_entry(1)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
