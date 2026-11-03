//! Tracker traits. Object safe through `async-trait`, so trackers are
//! held as `Arc<dyn Tracker>`.

pub mod anilist;

use async_trait::async_trait;
use tokio::sync::watch;

use crate::error::TrackError;
use crate::messages::MsgKey;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::trackers::enhanced::EntryRef;

/// Where a tracker's logo comes from. Replaces the Android drawable id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoRef {
    Embedded(&'static [u8]),
    IconName(&'static str),
}

/// Every tracker, anime or manga. The `as_*` methods expose the kind-specific traits.
#[async_trait]
pub trait Tracker: Send + Sync {
    fn id(&self) -> TrackerId;
    fn name(&self) -> &str;
    fn logo(&self) -> LogoRef;
    /// `0xAARRGGBB`.
    fn logo_color(&self) -> u32;
    /// The message the Settings row shows instead of its login state while the tracker cannot log in yet, for
    /// example MyAnimeList without a client id (T-29a-040). `None` when the tracker is ready.
    fn setup_message(&self) -> Option<MsgKey> {
        None
    }
    /// The authorise URL of a browser login the tracker builds from its own configured client and redirect URI
    /// (Shikimori, Bangumi, T-31a-038). `None` for trackers whose URL comes from elsewhere. `PendingLogins` adds
    /// `state` and opens the attempt.
    fn browser_auth_url(&self) -> Option<String> {
        None
    }
    fn supports_reading_dates(&self) -> bool {
        false
    }
    fn supports_private_tracking(&self) -> bool {
        false
    }
    /// True when the remove dialog deletes the local row first and then the remote one, so a failed remote delete is
    /// shown but does not bring the local row back (Kitsu, T-30a-022). The default runs the remote delete first.
    fn removes_local_first(&self) -> bool {
        false
    }
    fn completion_status(&self) -> i64;
    /// Score labels in index order. The stored score is `index_to_score(index)`.
    fn score_list(&self) -> &[String];

    async fn login(&self, username: &str, password: &str) -> Result<(), TrackError>;
    /// Clears the stored credentials, token and auth-expired flag.
    fn logout(&self) -> Result<(), TrackError>;
    fn is_logged_in(&self) -> bool;
    /// True when the stored login expired (the token was refused or is past its expiry). The login row then shows
    /// "Session expired, log in again".
    fn auth_expired(&self) -> bool {
        false
    }
    /// Emits the login state. Only changes are sent.
    fn login_state(&self) -> watch::Receiver<bool>;
    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError>;
    fn username(&self) -> String;
    fn password(&self) -> String;

    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        None
    }
    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        None
    }
    fn as_deletable_anime(&self) -> Option<&dyn DeletableAnimeTracker> {
        None
    }
    fn as_deletable_manga(&self) -> Option<&dyn DeletableMangaTracker> {
        None
    }
    fn as_enhanced_anime(&self) -> Option<&dyn EnhancedAnimeTracker> {
        None
    }
    fn as_enhanced_manga(&self) -> Option<&dyn EnhancedMangaTracker> {
        None
    }
}

/// Anime trackers (`AnimeTracker.kt`).
#[async_trait]
pub trait AnimeTracker: Tracker {
    fn status_list(&self) -> Vec<i64>;
    fn watching_status(&self) -> i64;
    fn rewatching_status(&self) -> i64;
    /// Message key of the status label, resolved by the GUI.
    fn status_label(&self, status: i64) -> Option<&'static str>;
    fn index_to_score(&self, index: usize) -> f64 {
        index as f64
    }
    fn score_10(&self, t: &AnimeTrack) -> f64 {
        t.score
    }
    fn display_score(&self, t: &AnimeTrack) -> String;

    /// Pushes `t` to the service. `did_watch_episode` is true when the push follows an episode watched.
    async fn update(
        &self,
        t: &AnimeTrack,
        did_watch_episode: bool,
    ) -> Result<AnimeTrack, TrackError>;
    async fn bind(&self, t: &mut AnimeTrack, has_seen: bool) -> Result<(), TrackError>;
    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError>;
    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError>;
}

/// Manga trackers (`MangaTracker.kt`).
#[async_trait]
pub trait MangaTracker: Tracker {
    fn status_list(&self) -> Vec<i64>;
    fn reading_status(&self) -> i64;
    fn rereading_status(&self) -> i64;
    /// Message key of the status label, resolved by the GUI.
    fn status_label(&self, status: i64) -> Option<&'static str>;
    fn index_to_score(&self, index: usize) -> f64 {
        index as f64
    }
    fn score_10(&self, t: &MangaTrack) -> f64 {
        t.score
    }
    fn display_score(&self, t: &MangaTrack) -> String;

    async fn update(
        &self,
        t: &MangaTrack,
        did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError>;
    async fn bind(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), TrackError>;
    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError>;
    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError>;
}

/// Anime trackers that match entries from installed sources (`EnhancedAnimeTracker.kt`).
#[async_trait]
pub trait EnhancedAnimeTracker: AnimeTracker {
    /// Qualified source ids this tracker serves.
    fn accepted_sources(&self) -> &[&'static str];
    fn accepts(&self, source_id: &str) -> bool {
        self.accepted_sources().contains(&source_id)
    }
    /// `Ok(None)` when nothing matches. Never a partly filled result.
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<AnimeTrackSearch>, TrackError>;
    /// True when `t` points at `entry` (same URL) and the entry's source is accepted.
    fn is_track_from(&self, t: &AnimeTrack, entry: &EntryRef, source_id: Option<&str>) -> bool;
    /// `t` moved to `new_entry` when its source is accepted, else `None`.
    fn migrate_track(
        &self,
        t: &AnimeTrack,
        new_source_id: &str,
        new_entry: &EntryRef,
    ) -> Option<AnimeTrack>;
}

/// Manga trackers that match entries from installed sources (`EnhancedMangaTracker.kt`).
#[async_trait]
pub trait EnhancedMangaTracker: MangaTracker {
    /// Qualified source ids this tracker serves.
    fn accepted_sources(&self) -> &[&'static str];
    fn accepts(&self, source_id: &str) -> bool {
        self.accepted_sources().contains(&source_id)
    }
    /// `Ok(None)` when nothing matches. Never a partly filled result.
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<MangaTrackSearch>, TrackError>;
    /// True when `t` points at `entry` (same URL) and the entry's source is accepted.
    fn is_track_from(&self, t: &MangaTrack, entry: &EntryRef, source_id: Option<&str>) -> bool;
    /// `t` moved to `new_entry` when its source is accepted, else `None`.
    fn migrate_track(
        &self,
        t: &MangaTrack,
        new_source_id: &str,
        new_entry: &EntryRef,
    ) -> Option<MangaTrack>;
}

/// Anime trackers whose remote record can be deleted (`DeletableAnimeTracker.kt`).
#[async_trait]
pub trait DeletableAnimeTracker: AnimeTracker {
    /// A failure is returned so the caller keeps the local binding.
    async fn delete(&self, t: &AnimeTrack) -> Result<(), TrackError>;
}

/// Manga trackers whose remote record can be deleted (`DeletableMangaTracker.kt`).
#[async_trait]
pub trait DeletableMangaTracker: MangaTracker {
    /// A failure is returned so the caller keeps the local binding.
    async fn delete(&self, t: &MangaTrack) -> Result<(), TrackError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fakes::{FakeAnime, FakeManga, MemSecrets, new_log};

    fn anime(enhanced: bool, deletable: bool) -> FakeAnime {
        let mut t = FakeAnime::new(
            TrackerId::JELLYFIN,
            "Jellyfin",
            MemSecrets::new(),
            new_log(),
        );
        t.enhanced = enhanced;
        t.deletable = deletable;
        t.accepted = &["src.one"];
        t
    }

    #[test]
    fn tr_27_001_as_methods_never_panic_and_a_manga_only_tracker_has_no_anime_view() {
        let m = FakeManga::new(
            TrackerId::MANGAUPDATES,
            "MangaUpdates",
            MemSecrets::new(),
            new_log(),
        );
        assert!(m.as_anime().is_none());
        assert!(m.as_manga().is_some());
        assert!(m.as_enhanced_anime().is_none());
        assert!(m.as_deletable_anime().is_none());
    }

    #[test]
    fn tr_27_001_trait_objects_compile_and_dispatch() {
        let t: Arc<dyn Tracker> = Arc::new(anime(false, false));
        assert_eq!(t.id(), TrackerId::JELLYFIN);
        assert!(!t.supports_reading_dates());
        assert!(!t.supports_private_tracking());
    }

    #[test]
    fn tr_27_005_default_score_mapping_is_the_index() {
        let t = anime(false, false);
        assert_eq!(t.index_to_score(2), 2.0);
        let track = AnimeTrack {
            id: None,
            anime_id: 1,
            tracker_id: TrackerId::JELLYFIN,
            remote_id: 1,
            library_id: None,
            title: String::new(),
            last_episode_seen: 0.0,
            total_episodes: 0,
            status: 0,
            score: 4.5,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        assert_eq!(t.score_10(&track), 4.5);
    }

    #[test]
    fn tr_27_006_manga_default_score_mapping_is_the_index() {
        let m = FakeManga::new(TrackerId::KAVITA, "Kavita", MemSecrets::new(), new_log());
        assert_eq!(m.index_to_score(1), 1.0);
    }

    #[test]
    fn tr_27_012_accepts_is_a_string_match_on_listed_sources() {
        let t = anime(true, false);
        assert!(t.accepts("src.one"));
        assert!(!t.accepts("src.two"));
        assert!(t.as_enhanced_anime().is_some());
    }

    #[tokio::test]
    async fn tr_27_013_a_remote_delete_failure_is_returned_to_the_caller() {
        let t = anime(false, true);
        *t.fail.lock().unwrap() = Some("delete refused".into());
        let track = AnimeTrack {
            id: Some(1),
            anime_id: 1,
            tracker_id: TrackerId::JELLYFIN,
            remote_id: 1,
            library_id: None,
            title: String::new(),
            last_episode_seen: 0.0,
            total_episodes: 0,
            status: 0,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        let deletable = t.as_deletable_anime().expect("deletable");
        let err = deletable.delete(&track).await.unwrap_err();
        assert_eq!(err.to_string(), "delete refused");
    }

    #[test]
    fn tr_27_013_only_deletable_trackers_expose_delete() {
        assert!(anime(false, false).as_deletable_anime().is_none());
        assert!(anime(false, true).as_deletable_anime().is_some());
    }
}
