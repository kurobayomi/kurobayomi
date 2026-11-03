//! Kavita tracker (manga, id 8; `Kavita.kt`; T-33a-040 to T-33a-046).
//!
//! The server URL and API key come from the Kavita sources (slots 1 to 3), so the tracker has no credentials of its
//! own. Login is the enhanced flag (`loginNoop`, T-33a-046).

pub mod api;
pub mod auth;
pub mod dto;

#[cfg(test)]
mod wire_tests;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::watch;

use self::api::{KavitaApi, apply_remote_progress};
use self::auth::KavitaAuth;
use crate::error::TrackError;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::tracker::{EnhancedMangaTracker, LogoRef, MangaTracker, Tracker};
use crate::trackers::enhanced::{
    EnhancedError, EntryRef, FlagStore, LoginFlag, is_track_from, migrate_remote_url,
};
use crate::trackers::http::tracker_client;
use crate::trackers::sources::SourceDirectory;

/// Status constants (`Kavita.kt:20-22`). Rereading is -1 (no reread status).
pub const UNREAD: i64 = 1;
pub const READING: i64 = 2;
pub const COMPLETED: i64 = 3;
pub const REREADING: i64 = -1;

/// Accepted source identity: `kavita/all`.
pub const ACCEPTED_SOURCES: &[&str] = &["kavita/all"];

/// Logo colour `rgb(74, 198, 148)` as `0xAARRGGBB` (`Kavita.kt`, T-33a-041).
pub const LOGO_COLOR: u32 = 0xFF4A_C694;

/// Logo name in the GResource bundle (T-33a-100).
pub const LOGO_NAME: &str = "ic_tracker_kavita";

pub struct KavitaTracker {
    login: LoginFlag,
    api: KavitaApi,
}

impl KavitaTracker {
    pub fn new(
        dir: Arc<dyn SourceDirectory>,
        flags: Arc<dyn FlagStore>,
    ) -> Result<Self, reqwest::Error> {
        let http = tracker_client()?;
        let auth = KavitaAuth::new(dir, http.clone());
        Ok(Self {
            login: LoginFlag::new(TrackerId::KAVITA, flags),
            api: KavitaApi::new(http, auth),
        })
    }
}

#[async_trait]
impl EnhancedMangaTracker for KavitaTracker {
    fn accepted_sources(&self) -> &[&'static str] {
        ACCEPTED_SOURCES
    }

    /// `remote_url == entry.url` and an accepted source (E-96, T-33a-045).
    fn is_track_from(&self, t: &MangaTrack, entry: &EntryRef, source_id: Option<&str>) -> bool {
        is_track_from(&t.remote_url, &entry.url, source_id, ACCEPTED_SOURCES)
    }

    /// Copies the new entry URL into the track for an accepted source (E-97, E-98).
    fn migrate_track(
        &self,
        t: &MangaTrack,
        new_source_id: &str,
        new_entry: &EntryRef,
    ) -> Option<MangaTrack> {
        migrate_remote_url(new_source_id, ACCEPTED_SOURCES, &new_entry.url).map(|url| {
            let mut out = t.clone();
            out.remote_url = url;
            out
        })
    }

    /// The series read for the entry URL. A failure is `Err`, never a silent `None` (E-93).
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<MangaTrackSearch>, TrackError> {
        Ok(Some(self.api.get_track_search(&entry.url).await?))
    }
}

#[async_trait]
impl Tracker for KavitaTracker {
    fn id(&self) -> TrackerId {
        TrackerId::KAVITA
    }

    fn name(&self) -> &str {
        "Kavita"
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

    /// The enhanced login takes no credentials (`loginNoop`, E-102).
    async fn login(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        self.login.login_noop()
    }

    /// Clears the flag (E-101).
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

    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }

    fn as_enhanced_manga(&self) -> Option<&dyn EnhancedMangaTracker> {
        Some(self)
    }
}

#[async_trait]
impl MangaTracker for KavitaTracker {
    fn status_list(&self) -> Vec<i64> {
        vec![UNREAD, READING, COMPLETED]
    }

    fn reading_status(&self) -> i64 {
        READING
    }

    fn rereading_status(&self) -> i64 {
        REREADING
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        match status {
            UNREAD => Some("unread"),
            READING => Some("reading"),
            COMPLETED => Some("completed"),
            _ => None,
        }
    }

    fn display_score(&self, _t: &MangaTrack) -> String {
        String::new()
    }

    /// Writes the progress and returns the server's status, chapter count and last read (D-02, E-92). The caller keeps
    /// that status. `did_read_chapter` is ignored: the status is never computed locally.
    async fn update(
        &self,
        t: &MangaTrack,
        _did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        Ok(self.api.update_progress(t).await?)
    }

    /// Returns the track unchanged (T-33a-044).
    async fn bind(&self, _t: &mut MangaTrack, _has_read: bool) -> Result<(), TrackError> {
        Ok(())
    }

    async fn search(&self, _query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        Err(EnhancedError::SearchUnsupported.into())
    }

    /// Copies only the server-owned progress fields (D-03, E-91).
    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let remote = self.api.get_track_search(&t.remote_url).await?;
        Ok(apply_remote_progress(t, &remote))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::Harness;
    use crate::messages::MsgKey;
    use crate::trackers::enhanced::MemFlags;
    use crate::trackers::sources::memory::MemDirectory;

    fn tracker() -> KavitaTracker {
        KavitaTracker::new(
            Arc::new(MemDirectory::default()),
            Arc::new(MemFlags::default()),
        )
        .unwrap()
    }

    fn track(url: &str) -> MangaTrack {
        MangaTrack {
            id: Some(1),
            manga_id: 1,
            tracker_id: TrackerId::KAVITA,
            remote_id: 0,
            library_id: None,
            title: "T".into(),
            last_chapter_read: 3.0,
            total_chapters: 10,
            status: READING,
            score: 8.0,
            remote_url: url.into(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
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
    fn t_33a_040_identity_and_status_constants() {
        let t = tracker();
        assert_eq!(t.id(), TrackerId::KAVITA);
        assert_eq!(t.name(), "Kavita");
        assert_eq!((UNREAD, READING, COMPLETED, REREADING), (1, 2, 3, -1));
        assert_eq!(t.completion_status(), COMPLETED);
        let manga = t.as_manga().expect("manga view");
        assert_eq!(manga.status_list(), vec![1, 2, 3]);
        assert_eq!(manga.reading_status(), READING);
        assert_eq!(manga.rereading_status(), -1);
    }

    #[test]
    fn t_33a_040_status_labels_empty_score_list_and_display_score() {
        let t = tracker();
        let manga = t.as_manga().unwrap();
        assert_eq!(manga.status_label(1), Some("unread"));
        assert_eq!(manga.status_label(2), Some("reading"));
        assert_eq!(manga.status_label(3), Some("completed"));
        assert_eq!(manga.status_label(9), None);
        assert!(t.score_list().is_empty());
        assert_eq!(manga.display_score(&track("https://h/api/series/1")), "");
    }

    #[test]
    fn t_33a_041_logo_colour_and_name() {
        let t = tracker();
        assert_eq!(t.logo_color(), 0xFF4A_C694);
        assert!(matches!(t.logo(), LogoRef::IconName(LOGO_NAME)));
    }

    #[tokio::test]
    async fn t_33a_044_bind_returns_unchanged_and_search_is_unsupported() {
        let t = tracker();
        let manga = t.as_manga().unwrap();
        let mut tr = track("https://h/api/series/1");
        let before = tr.clone();
        manga.bind(&mut tr, true).await.unwrap();
        assert_eq!(tr, before);
        let err = manga.search("x").await.unwrap_err();
        assert_eq!(
            err.toast("Kavita"),
            MsgKey::enhanced("track.enhanced.search_unsupported", "")
        );
    }

    #[test]
    fn t_33a_045_the_accepted_identity_is_kavita_all() {
        assert_eq!(
            tracker().as_enhanced_manga().unwrap().accepted_sources(),
            &["kavita/all"]
        );
        assert!(tracker().as_enhanced_manga().unwrap().accepts("kavita/all"));
    }

    #[test]
    fn t_33a_045_e96_a_different_identity_is_not_accepted() {
        let t = tracker();
        let e = t.as_enhanced_manga().unwrap();
        let tr = track("https://h/api/series/1");
        assert!(!e.is_track_from(&tr, &entry("https://h/api/series/1"), Some("komga/all")));
        assert!(e.is_track_from(&tr, &entry("https://h/api/series/1"), Some("kavita/all")));
    }

    #[test]
    fn t_33a_045_e97_e98_migrate_copies_the_url_only_for_an_accepted_source() {
        let t = tracker();
        let e = t.as_enhanced_manga().unwrap();
        let tr = track("https://h/api/series/1");
        let new = entry("https://h/api/series/2");
        assert_eq!(
            e.migrate_track(&tr, "kavita/all", &new).unwrap().remote_url,
            "https://h/api/series/2"
        );
        assert!(e.migrate_track(&tr, "komga/all", &new).is_none());
    }

    #[tokio::test]
    async fn t_33a_046_login_sets_the_flag_and_logout_clears_it() {
        let t = tracker();
        assert!(!t.is_logged_in());
        t.login("", "").await.unwrap();
        assert!(t.is_logged_in());
        assert!(*t.login_state().borrow());
        t.logout().unwrap();
        assert!(!t.is_logged_in());
    }

    #[tokio::test]
    async fn t_33a_046_a_logged_out_kavita_is_skipped_by_the_bind_path() {
        let mut dir = MemDirectory::default();
        dir.install(
            "kavita_1",
            "all",
            &[("APIURL", "http://127.0.0.1:1/api"), ("APIKEY", "k")],
        );
        let t = Arc::new(KavitaTracker::new(Arc::new(dir), Arc::new(MemFlags::default())).unwrap());
        let h = Harness::with_tracker(Some(t));
        let failures = crate::bind::bind_enhanced_manga_entry(
            &h.ctx,
            &entry("http://127.0.0.1:1/api/series/1"),
            "kavita/all",
        )
        .await;
        assert!(failures.is_empty());
        assert!(
            h.ctx
                .manga_tracks
                .list_for_entry(1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn t_33a_046_a_match_without_a_source_fails_with_the_unauthorised_toast() {
        let t = tracker();
        let err = t
            .as_enhanced_manga()
            .unwrap()
            .match_entry(&entry("http://127.0.0.1:1/api/series/1"))
            .await
            .unwrap_err();
        assert_eq!(
            err.toast("Kavita"),
            MsgKey::enhanced("track.enhanced.unauthorized", "")
        );
    }
}
