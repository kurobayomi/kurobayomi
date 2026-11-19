//! The AniList tracker behind the [`Tracker`], [`AnimeTracker`] and [`MangaTracker`] traits, so the registry
//! ([`crate::manager::TrackerManager`]), the login rows and the track sheet can hold it as `Arc<dyn Tracker>`.
//! The data flows are in `service`; the login state is the one of [`super::AniList::base`].

use super::date::now_ms;
use super::service;
use super::status::{AnimeStatus, MangaStatus};
use super::*;

/// `0xAARRGGBB` of the logo colour.
fn logo_color() -> u32 {
    let (r, g, b) = AniList::LOGO_RGB;
    0xFF00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

#[async_trait]
impl Tracker for AniList {
    fn id(&self) -> TrackerId {
        TrackerId::ANILIST
    }

    fn name(&self) -> &str {
        AniList::NAME
    }

    fn logo(&self) -> LogoRef {
        LogoRef::IconName("anilist")
    }

    fn logo_color(&self) -> u32 {
        logo_color()
    }

    fn supports_reading_dates(&self) -> bool {
        true
    }

    fn supports_private_tracking(&self) -> bool {
        true
    }

    fn completion_status(&self) -> i64 {
        AnimeStatus::Completed.to_db()
    }

    fn score_list(&self) -> &[String] {
        let format = self.score_format();
        let at = FORMATS.iter().position(|f| *f == format).unwrap_or(1);
        &self.scores[at]
    }

    /// AniList logs in through the browser: `password` carries the access token, `username` is ignored.
    async fn login(&self, _username: &str, password: &str) -> Result<(), TrackError> {
        AniList::login(self, AccessToken::new(password), None)
            .await
            .map(|_| ())
            .map_err(TrackError::from)
    }

    fn logout(&self) -> Result<(), TrackError> {
        AniList::logout(self).map_err(TrackError::from)
    }

    fn is_logged_in(&self) -> bool {
        AniList::is_logged_in(self)
    }

    fn auth_expired(&self) -> bool {
        self.base.auth_expired()
    }

    fn login_state(&self) -> watch::Receiver<bool> {
        self.base.login_state()
    }

    /// A browser login has no typed credentials, so there is nothing to save here.
    fn save_credentials(&self, _username: &str, _password: &str) -> Result<(), TrackError> {
        Err(TrackError::Message(
            "AniList signs in through the browser".into(),
        ))
    }

    fn username(&self) -> String {
        self.base.username()
    }

    fn password(&self) -> String {
        self.base.password()
    }

    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        Some(self)
    }

    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }

    fn as_deletable_anime(&self) -> Option<&dyn DeletableAnimeTracker> {
        Some(self)
    }

    fn as_deletable_manga(&self) -> Option<&dyn DeletableMangaTracker> {
        Some(self)
    }
}

#[async_trait]
impl AnimeTracker for AniList {
    fn status_list(&self) -> Vec<i64> {
        AnimeStatus::ORDER.iter().map(|s| s.to_db()).collect()
    }

    fn watching_status(&self) -> i64 {
        AnimeStatus::Watching.to_db()
    }

    fn rewatching_status(&self) -> i64 {
        AnimeStatus::Rewatching.to_db()
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        AnimeStatus::from_db(status).map(AnimeStatus::label_key)
    }

    fn index_to_score(&self, index: usize) -> f64 {
        self.score_format().index_to_score(index).unwrap_or(0.0)
    }

    fn display_score(&self, t: &AnimeTrack) -> String {
        self.score_format().display(t.score)
    }

    async fn update(
        &self,
        t: &AnimeTrack,
        did_watch_episode: bool,
    ) -> Result<AnimeTrack, TrackError> {
        let mut next = t.clone();
        service::update_anime(self, &mut next, did_watch_episode, now_ms()).await?;
        Ok(next)
    }

    async fn bind(&self, t: &mut AnimeTrack, has_seen: bool) -> Result<(), TrackError> {
        Ok(service::bind_anime(self, t, has_seen).await?)
    }

    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        Ok(self.search_anime(query).await?)
    }

    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        let mut next = t.clone();
        service::refresh_anime(self, &mut next).await?;
        Ok(next)
    }
}

#[async_trait]
impl MangaTracker for AniList {
    fn status_list(&self) -> Vec<i64> {
        MangaStatus::ORDER.iter().map(|s| s.to_db()).collect()
    }

    fn reading_status(&self) -> i64 {
        MangaStatus::Reading.to_db()
    }

    fn rereading_status(&self) -> i64 {
        MangaStatus::Rereading.to_db()
    }

    fn status_label(&self, status: i64) -> Option<&'static str> {
        MangaStatus::from_db(status).map(MangaStatus::label_key)
    }

    fn index_to_score(&self, index: usize) -> f64 {
        self.score_format().index_to_score(index).unwrap_or(0.0)
    }

    fn display_score(&self, t: &MangaTrack) -> String {
        self.score_format().display(t.score)
    }

    async fn update(
        &self,
        t: &MangaTrack,
        did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        let mut next = t.clone();
        service::update_manga(self, &mut next, did_read_chapter, now_ms()).await?;
        Ok(next)
    }

    async fn bind(&self, t: &mut MangaTrack, has_read: bool) -> Result<(), TrackError> {
        Ok(service::bind_manga(self, t, has_read).await?)
    }

    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        Ok(self.search_manga(query).await?)
    }

    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        let mut next = t.clone();
        service::refresh_manga(self, &mut next).await?;
        Ok(next)
    }
}

#[async_trait]
impl DeletableAnimeTracker for AniList {
    async fn delete(&self, t: &AnimeTrack) -> Result<(), TrackError> {
        Ok(service::delete_anime(self, t).await?)
    }
}

#[async_trait]
impl DeletableMangaTracker for AniList {
    async fn delete(&self, t: &MangaTrack) -> Result<(), TrackError> {
        Ok(service::delete_manga(self, t).await?)
    }
}
