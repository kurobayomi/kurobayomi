//! `FakeTracker`, the Rust analogue of Aniyomi's `DummyTracker` (T-01a-039).
//! Every field is public, so a test overrides what it needs.

use async_trait::async_trait;
use kuro_track::error::TrackError;
use kuro_track::model::search::{AnimeTrackSearch, MangaTrackSearch};
use kuro_track::model::{AnimeTrack, MangaTrack, TrackerId};
use kuro_track::tracker::{AnimeTracker, LogoRef, MangaTracker, Tracker};
use tokio::sync::watch;

/// A tracker that does nothing: login, logout and saving credentials succeed without effect, search finds nothing and
/// updates echo the track they get.
pub struct FakeTracker {
    pub id: TrackerId,
    pub name: String,
    pub supports_reading_dates: bool,
    pub supports_private_tracking: bool,
    pub logged_in: bool,
    /// `0xAARRGGBB`, default `rgb(18, 25, 35)`.
    pub logo_color: u32,
    pub statuses: Vec<i64>,
    pub completion_status: i64,
    /// Default `"0"` to `"10"`.
    pub score_list: Vec<String>,
    /// What `score_10` returns for any track.
    pub ten_point_score: f64,
    pub manga_search_results: Vec<MangaTrackSearch>,
    pub anime_search_results: Vec<AnimeTrackSearch>,
    login_state: watch::Sender<bool>,
}

impl FakeTracker {
    pub fn new(id: TrackerId, name: &str) -> Self {
        Self {
            id,
            name: name.to_owned(),
            supports_reading_dates: false,
            supports_private_tracking: false,
            logged_in: false,
            logo_color: 0xFF12_1923,
            statuses: (1..=6).collect(),
            completion_status: 2,
            score_list: (0..=10).map(|n| n.to_string()).collect(),
            ten_point_score: 5.4,
            manga_search_results: Vec::new(),
            anime_search_results: Vec::new(),
            login_state: watch::channel(false).0,
        }
    }

    /// The HTTP client of a real tracker. A fake has none: this is an error value, never a panic.
    pub fn client(&self) -> Result<(), TrackError> {
        Err(TrackError::Message("FakeTracker has no client".to_owned()))
    }
}

#[async_trait]
impl Tracker for FakeTracker {
    fn id(&self) -> TrackerId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn logo(&self) -> LogoRef {
        LogoRef::IconName("ic_tracker_anilist")
    }

    fn logo_color(&self) -> u32 {
        self.logo_color
    }

    fn supports_reading_dates(&self) -> bool {
        self.supports_reading_dates
    }

    fn supports_private_tracking(&self) -> bool {
        self.supports_private_tracking
    }

    fn completion_status(&self) -> i64 {
        self.completion_status
    }

    fn score_list(&self) -> &[String] {
        &self.score_list
    }

    async fn login(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        Ok(())
    }

    fn logout(&self) -> Result<(), TrackError> {
        Ok(())
    }

    fn is_logged_in(&self) -> bool {
        self.logged_in
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.login_state.send_replace(self.logged_in);
        self.login_state.subscribe()
    }

    fn save_credentials(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        Ok(())
    }

    fn username(&self) -> String {
        "username".to_owned()
    }

    fn password(&self) -> String {
        "passw0rd".to_owned()
    }

    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        Some(self)
    }

    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }
}

#[async_trait]
impl AnimeTracker for FakeTracker {
    fn status_list(&self) -> Vec<i64> {
        self.statuses.clone()
    }

    fn watching_status(&self) -> i64 {
        1
    }

    fn rewatching_status(&self) -> i64 {
        3
    }

    fn status_label(&self, _status: i64) -> Option<&'static str> {
        None
    }

    fn score_10(&self, _t: &AnimeTrack) -> f64 {
        self.ten_point_score
    }

    fn display_score(&self, t: &AnimeTrack) -> String {
        t.score.to_string()
    }

    async fn update(&self, t: &AnimeTrack, _did_watch_episode: bool) -> Result<AnimeTrack, TrackError> {
        Ok(t.clone())
    }

    async fn bind(&self, _t: &mut AnimeTrack, _has_seen: bool) -> Result<(), TrackError> {
        Ok(())
    }

    async fn search(&self, _query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        Ok(self.anime_search_results.clone())
    }

    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        Ok(t.clone())
    }
}

#[async_trait]
impl MangaTracker for FakeTracker {
    fn status_list(&self) -> Vec<i64> {
        self.statuses.clone()
    }

    fn reading_status(&self) -> i64 {
        1
    }

    fn rereading_status(&self) -> i64 {
        3
    }

    fn status_label(&self, _status: i64) -> Option<&'static str> {
        None
    }

    fn score_10(&self, _t: &MangaTrack) -> f64 {
        self.ten_point_score
    }

    fn display_score(&self, t: &MangaTrack) -> String {
        t.score.to_string()
    }

    async fn update(&self, t: &MangaTrack, _did_read_chapter: bool) -> Result<MangaTrack, TrackError> {
        Ok(t.clone())
    }

    async fn bind(&self, _t: &mut MangaTrack, _has_read: bool) -> Result<(), TrackError> {
        Ok(())
    }

    async fn search(&self, _query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        Ok(self.manga_search_results.clone())
    }

    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        Ok(t.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_dummy_tracker_ones() {
        let t = FakeTracker::new(TrackerId::ANILIST, "Fake");
        assert_eq!(t.logo_color(), 0xFF12_1923);
        assert_eq!(AnimeTracker::status_list(&t), [1, 2, 3, 4, 5, 6]);
        assert_eq!(MangaTracker::status_list(&t), [1, 2, 3, 4, 5, 6]);
        assert_eq!(t.completion_status(), 2);
        assert_eq!(t.score_list(), ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        assert_eq!(t.username(), "username");
        assert_eq!(t.password(), "passw0rd");
        assert!(!t.is_logged_in() && !*t.login_state().borrow());
        assert!(!t.supports_reading_dates() && !t.supports_private_tracking());
    }

    #[test]
    fn the_client_is_an_error_value_and_the_searches_are_empty() {
        let t = FakeTracker::new(TrackerId::ANILIST, "Fake");
        assert!(t.client().is_err());
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            assert!(AnimeTracker::search(&t, "x").await.unwrap().is_empty());
            assert!(MangaTracker::search(&t, "x").await.unwrap().is_empty());
        });
    }
}
