//! Suwayomi tracker (manga, id 9; `Suwayomi.kt`; T-33a-080 to T-33a-096).
//!
//! The server address and the Basic credential come from the Tachidesk extension's preferences, read on every call
//! (see [`api::SuwayomiApi::settings`]). The enhanced login takes no credentials and only sets a flag.

pub mod api;
pub mod dto;

#[cfg(test)]
mod wire_tests;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::watch;

use self::api::{SuwayomiApi, apply_remote_progress};
use crate::error::TrackError;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::tracker::{EnhancedMangaTracker, LogoRef, MangaTracker, Tracker};
use crate::trackers::enhanced::{
    EnhancedError, EntryRef, FlagStore, LoginFlag, is_track_from, migrate_remote_url,
};
use crate::trackers::http::tracker_client;
use crate::trackers::sources::SourceDirectory;

/// Status constants (`Suwayomi.kt:27-29`). Rereading is -1.
pub const UNREAD: i64 = 1;
pub const READING: i64 = 2;
pub const COMPLETED: i64 = 3;
pub const REREADING: i64 = -1;

/// Accepted source identity: `tachidesk/en` (`Suwayomi.kt:89`, the Tachidesk extension).
pub const ACCEPTED_SOURCES: &[&str] = &["tachidesk/en"];

/// Logo colour `#FFD600`: the dominant brand yellow of `ic_tracker_suwayomi`, sampled from the logo (T-33a-084, D-27).
/// Replaces the Kotlin `rgb(255, 35, 35)` that Kotlin marked as a placeholder (`Suwayomi.kt:24`).
pub const LOGO_COLOR: u32 = 0xFFFF_D600;

/// Logo name in the bundled assets (T-33a-100).
pub const LOGO_NAME: &str = "ic_tracker_suwayomi";

pub struct SuwayomiTracker {
    login: LoginFlag,
    api: SuwayomiApi,
}

impl SuwayomiTracker {
    pub fn new(
        dir: Arc<dyn SourceDirectory>,
        flags: Arc<dyn FlagStore>,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            login: LoginFlag::new(TrackerId::SUWAYOMI, flags),
            api: SuwayomiApi::new(tracker_client()?, dir),
        })
    }
}

#[async_trait]
impl EnhancedMangaTracker for SuwayomiTracker {
    fn accepted_sources(&self) -> &[&'static str] {
        ACCEPTED_SOURCES
    }

    /// `remote_url == entry.url` and an accepted source (D-09 fix, E-64).
    fn is_track_from(&self, t: &MangaTrack, entry: &EntryRef, source_id: Option<&str>) -> bool {
        is_track_from(&t.remote_url, &entry.url, source_id, ACCEPTED_SOURCES)
    }

    /// Copies the new entry URL into the track for an accepted source (T-33a-007).
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

    /// The read of the entry URL. A failure is `Err`, never a silent `None` (D-29).
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<MangaTrackSearch>, TrackError> {
        Ok(Some(self.api.get_track_search(&entry.url).await?))
    }
}

#[async_trait]
impl Tracker for SuwayomiTracker {
    fn id(&self) -> TrackerId {
        TrackerId::SUWAYOMI
    }

    fn name(&self) -> &str {
        "Suwayomi"
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

    /// The enhanced login takes no credentials and sets the flag (T-33a-083).
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

    /// Nothing is stored: the credentials stay in the extension's preferences.
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
impl MangaTracker for SuwayomiTracker {
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

    /// Writes the progress and returns the server's track. The status is server-driven, so the local value is not
    /// kept (D-02). `did_read_chapter` is ignored.
    async fn update(&self, t: &MangaTrack, _did_read_chapter: bool) -> Result<MangaTrack, TrackError> {
        Ok(self.api.update_progress(t).await?)
    }

    /// Returns the track unchanged (T-33a-082).
    async fn bind(&self, _t: &mut MangaTrack, _has_read: bool) -> Result<(), TrackError> {
        Ok(())
    }

    /// Search is not offered: the tracker only matches by the entry URL (D-21).
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
    use crate::messages::MsgKey;
    use crate::trackers::enhanced::MemFlags;
    use crate::trackers::sources::memory::MemDirectory;

    pub(super) fn tracker_with(dir: MemDirectory) -> SuwayomiTracker {
        SuwayomiTracker::new(Arc::new(dir), Arc::new(MemFlags::default())).unwrap()
    }

    fn tracker() -> SuwayomiTracker {
        tracker_with(MemDirectory::default())
    }

    fn track(url: &str) -> MangaTrack {
        MangaTrack {
            id: Some(1),
            manga_id: 1,
            tracker_id: TrackerId::SUWAYOMI,
            remote_id: 0,
            library_id: None,
            title: "T".into(),
            last_chapter_read: 5.0,
            total_chapters: 0,
            status: READING,
            score: 0.0,
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
    fn t_33a_080_identity_constants_and_labels() {
        let t = tracker();
        assert_eq!(t.id(), TrackerId::SUWAYOMI);
        assert_eq!(t.name(), "Suwayomi");
        assert_eq!((UNREAD, READING, COMPLETED, REREADING), (1, 2, 3, -1));
        assert_eq!(t.completion_status(), COMPLETED);
        assert!(t.score_list().is_empty());
        let manga = t.as_manga().unwrap();
        assert_eq!(manga.status_list(), vec![1, 2, 3]);
        assert_eq!(manga.reading_status(), READING);
        assert_eq!(manga.rereading_status(), -1);
        assert_eq!(manga.status_label(1), Some("unread"));
        assert_eq!(manga.status_label(2), Some("reading"));
        assert_eq!(manga.status_label(3), Some("completed"));
        assert_eq!(manga.status_label(9), None);
        assert_eq!(manga.display_score(&track("x")), "");
        assert_eq!(t.logo_color(), 0xFFFF_D600);
        assert!(matches!(t.logo(), LogoRef::IconName(LOGO_NAME)));
    }

    #[tokio::test]
    async fn t_33a_082_search_is_unsupported_and_bind_is_unchanged() {
        let t = tracker();
        let manga = t.as_manga().unwrap();
        let mut tr = track("https://h/api/v1/manga/9");
        let before = tr.clone();
        manga.bind(&mut tr, true).await.unwrap();
        assert_eq!(tr, before);
        let err = manga.search("x").await.unwrap_err();
        assert_eq!(
            err.toast("Suwayomi"),
            MsgKey::enhanced("track.enhanced.search_unsupported", "")
        );
    }

    #[test]
    fn t_33a_082_is_track_from_compares_the_url_and_the_identity() {
        let t = tracker();
        let e = t.as_enhanced_manga().unwrap();
        let tr = track("https://h/api/v1/manga/9");
        let same = entry("https://h/api/v1/manga/9");
        assert!(e.is_track_from(&tr, &same, Some("tachidesk/en")));
        assert!(!e.is_track_from(&tr, &entry("https://h/other"), Some("tachidesk/en")));
        assert!(!e.is_track_from(&tr, &same, Some("komga/all")));
        assert!(!e.is_track_from(&tr, &same, None));
    }

    #[test]
    fn t_33a_082_migrate_copies_the_url_only_for_the_accepted_identity() {
        let t = tracker();
        let e = t.as_enhanced_manga().unwrap();
        let tr = track("https://h/api/v1/manga/9");
        let moved = e
            .migrate_track(&tr, "tachidesk/en", &entry("https://h/api/v1/manga/10"))
            .unwrap();
        assert_eq!(moved.remote_url, "https://h/api/v1/manga/10");
        assert!(e.migrate_track(&tr, "komga/all", &entry("https://x")).is_none());
        assert_eq!(e.accepted_sources(), &["tachidesk/en"]);
    }

    #[tokio::test]
    async fn t_33a_083_login_sets_the_flag_and_logout_clears_it() {
        let t = tracker();
        assert!(!t.is_logged_in());
        t.login("", "").await.unwrap();
        assert!(t.is_logged_in());
        assert!(*t.login_state().borrow());
        t.logout().unwrap();
        assert!(!t.is_logged_in());
        assert_eq!(t.username(), "");
        assert_eq!(t.password(), "");
    }
}
