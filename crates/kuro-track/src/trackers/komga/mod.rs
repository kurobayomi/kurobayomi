//! Komga tracker (manga, id 6; `Komga.kt`; T-33a-060 to T-33a-065).
//!
//! Komga has no URL or key in its source, so the user logs in with the Komga username and password.
//! The password is kept in the secret store through [`BaseTracker`].
//! The enhanced flag of the other trackers does not apply here: a logged-in tracker is one with both credentials.

pub mod api;
pub mod auth;
pub mod dto;

#[cfg(test)]
mod wire_tests;

use std::sync::Arc;

use async_trait::async_trait;
use kuro_platform::secrets::SecretStore;
use tokio::sync::watch;

use self::api::{KomgaApi, apply_remote_progress};
use self::auth::KomgaAuth;
use crate::base::BaseTracker;
use crate::error::TrackError;
use crate::events::EventBus;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::tracker::{EnhancedMangaTracker, LogoRef, MangaTracker, Tracker};
use crate::trackers::enhanced::{EnhancedError, EntryRef, is_track_from, migrate_remote_url};
use crate::trackers::http::tracker_client;

/// Status constants (`Komga.kt:20-22`). Rereading is -1.
pub const UNREAD: i64 = 1;
pub const READING: i64 = 2;
pub const COMPLETED: i64 = 3;
pub const REREADING: i64 = -1;

/// Accepted source identity: `komga/all`.
pub const ACCEPTED_SOURCES: &[&str] = &["komga/all"];

/// Logo colour `rgb(51, 37, 50)` as `0xAARRGGBB` (`Komga.kt`, T-33a-065).
pub const LOGO_COLOR: u32 = 0xFF33_2532;

/// Logo name in the GResource bundle (T-33a-100).
pub const LOGO_NAME: &str = "ic_tracker_komga";

pub struct KomgaTracker {
    base: BaseTracker,
    api: KomgaApi,
}

impl KomgaTracker {
    pub fn new(secrets: Arc<dyn SecretStore>, events: EventBus) -> Result<Self, reqwest::Error> {
        Ok(Self {
            base: BaseTracker::new(TrackerId::KOMGA, "Komga", secrets, events),
            api: KomgaApi::new(tracker_client()?),
        })
    }

    /// The credentials as they are stored now. They are read on every call (P-7).
    fn auth(&self) -> KomgaAuth {
        KomgaAuth::new(self.base.username(), self.base.password())
    }
}

#[async_trait]
impl EnhancedMangaTracker for KomgaTracker {
    fn accepted_sources(&self) -> &[&'static str] {
        ACCEPTED_SOURCES
    }

    /// `remote_url == entry.url` and an accepted source (E-96, T-33a-064).
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

    /// The item read for the entry URL. A failure is `Err`, never a silent `None` (E-93 style, D-29).
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<MangaTrackSearch>, TrackError> {
        Ok(Some(
            self.api.get_track_search(&self.auth(), &entry.url).await?,
        ))
    }
}

#[async_trait]
impl Tracker for KomgaTracker {
    fn id(&self) -> TrackerId {
        TrackerId::KOMGA
    }

    fn name(&self) -> &str {
        self.base.name()
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

    /// Stores the Komga username and password. There is no server call here, because the host is only known from a
    /// tracking URL.
    async fn login(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.base.save_credentials(username, password)
    }

    fn logout(&self) -> Result<(), TrackError> {
        self.base.logout()
    }

    fn is_logged_in(&self) -> bool {
        self.base.is_logged_in()
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.base.login_state()
    }

    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.base.save_credentials(username, password)
    }

    fn username(&self) -> String {
        self.base.username()
    }

    fn password(&self) -> String {
        self.base.password()
    }

    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }

    fn as_enhanced_manga(&self) -> Option<&dyn EnhancedMangaTracker> {
        Some(self)
    }
}

#[async_trait]
impl MangaTracker for KomgaTracker {
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

    /// Writes the progress and keeps the server's status (D-02). `did_read_chapter` is ignored.
    async fn update(
        &self,
        t: &MangaTrack,
        _did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        Ok(self.api.update_progress(&self.auth(), t).await?)
    }

    /// Returns the track unchanged (T-33a-064).
    async fn bind(&self, _t: &mut MangaTrack, _has_read: bool) -> Result<(), TrackError> {
        Ok(())
    }

    async fn search(&self, _query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        Err(EnhancedError::SearchUnsupported.into())
    }

    /// Copies only the server-owned progress fields (D-03, E-91).
    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let remote = self
            .api
            .get_track_search(&self.auth(), &t.remote_url)
            .await?;
        Ok(apply_remote_progress(t, &remote))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::MemSecrets;
    use crate::messages::MsgKey;

    fn tracker() -> KomgaTracker {
        let secrets: Arc<dyn SecretStore> = MemSecrets::new();
        KomgaTracker::new(secrets, EventBus::default()).unwrap()
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
    fn t_33a_060_identity_and_status_constants() {
        let t = tracker();
        assert_eq!(t.id(), TrackerId::KOMGA);
        assert_eq!(t.name(), "Komga");
        assert_eq!((UNREAD, READING, COMPLETED, REREADING), (1, 2, 3, -1));
        assert_eq!(t.completion_status(), COMPLETED);
        let manga = t.as_manga().unwrap();
        assert_eq!(manga.status_list(), vec![1, 2, 3]);
        assert_eq!(manga.reading_status(), READING);
        assert_eq!(manga.rereading_status(), -1);
        assert_eq!(manga.status_label(1), Some("unread"));
        assert_eq!(manga.status_label(2), Some("reading"));
        assert_eq!(manga.status_label(3), Some("completed"));
        assert!(t.score_list().is_empty());
    }

    #[test]
    fn t_33a_065_logo_colour_and_name() {
        let t = tracker();
        assert_eq!(t.logo_color(), 0xFF33_2532);
        assert!(matches!(t.logo(), LogoRef::IconName(LOGO_NAME)));
    }

    #[tokio::test]
    async fn t_33a_064_login_stores_the_credentials_and_logout_clears_them() {
        let t = tracker();
        assert!(!t.is_logged_in());
        t.login("reader", "pw").await.unwrap();
        assert!(t.is_logged_in());
        assert!(*t.login_state().borrow());
        assert_eq!(t.username(), "reader");
        assert_eq!(t.password(), "pw");
        t.logout().unwrap();
        assert!(!t.is_logged_in());
        assert_eq!(t.username(), "");
    }

    #[tokio::test]
    async fn t_33a_064_bind_is_unchanged_and_search_is_unsupported() {
        let t = tracker();
        let manga = t.as_manga().unwrap();
        let mut tr = MangaTrack {
            id: None,
            manga_id: 1,
            tracker_id: TrackerId::KOMGA,
            remote_id: 0,
            library_id: None,
            title: "T".into(),
            last_chapter_read: 1.0,
            total_chapters: 0,
            status: READING,
            score: 0.0,
            remote_url: "https://h/api/v1/series/9".into(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        let before = tr.clone();
        manga.bind(&mut tr, true).await.unwrap();
        assert_eq!(tr, before);
        let err = manga.search("x").await.unwrap_err();
        assert_eq!(
            err.toast("Komga"),
            MsgKey::enhanced("track.enhanced.search_unsupported", "")
        );
    }

    #[test]
    fn t_33a_064_the_accepted_identity_is_komga_all() {
        let t = tracker();
        let e = t.as_enhanced_manga().unwrap();
        assert_eq!(e.accepted_sources(), &["komga/all"]);
        let tr = MangaTrack {
            id: None,
            manga_id: 1,
            tracker_id: TrackerId::KOMGA,
            remote_id: 0,
            library_id: None,
            title: "T".into(),
            last_chapter_read: 0.0,
            total_chapters: 0,
            status: 0,
            score: 0.0,
            remote_url: "https://h/api/v1/series/9".into(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        let same = entry("https://h/api/v1/series/9");
        assert!(e.is_track_from(&tr, &same, Some("komga/all")));
        assert!(!e.is_track_from(&tr, &same, Some("kavita/all")));
        assert!(
            e.migrate_track(&tr, "komga/all", &entry("https://h/api/v1/series/10"))
                .is_some()
        );
        assert!(e.migrate_track(&tr, "kavita/all", &same).is_none());
    }
}
