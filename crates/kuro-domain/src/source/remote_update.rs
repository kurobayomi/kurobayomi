//! Remote entry update for anime (`UpdateAnimeFromRemote.kt`, area 17 section 9; T-17a-027 to T-17a-032). It reads the
//! stored episodes or seasons in source order, asks the source for fresh metadata and the list, writes the metadata,
//! runs the list sync on the ORIGINAL anime, then re-reads the anime and returns it with the new rows.
//!
//! The cover and background rules are pure functions of the stored row.
//! The source calls, the list sync and the cover files are ports, so the orchestration is testable without a source or
//! a cover cache. The episode and season sync (T-10a-009 and T-10a-010) implement [`ListSync`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate, SeasonEntry};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::handle::Db;
use kuro_source::SourceError;
use kuro_source::anime::{AnimeSource, AnimeStatus, SAnime, SAnimeEpisodeUpdate, SAnimeSeasonUpdate, SEpisode};

use crate::error::{DomainError, NoEpisodes, NoSeasons};
use crate::app::source::LOCAL_SOURCE_ID;
use crate::entries::update::{CacheFiles, prepare_update, remote_from_anime};
pub use crate::entries::update::CacheAction;
use crate::source::model::Id;

/// Which parts of the remote data to fetch, and the window and manual flags (`awaitEpisodesUpdate` and
/// `awaitSeasonsUpdate` arguments). The default is nothing fetched, a `(0, 0)` window and no manual fetch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FetchFlags {
    pub fetch_details: bool,
    pub fetch_episodes: bool,
    pub fetch_seasons: bool,
    pub manual_fetch: bool,
    pub fetch_window: (i64, i64),
}

/// The result of an episode refresh: the anime as stored after the write, and the episodes the sync added
/// (`RemoteAnimeEpisodeUpdate`). Both fields are required.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteEpisodeUpdate {
    pub anime: Entry,
    pub new_episodes: Vec<Item>,
}

/// The result of a season refresh (`RemoteAnimeSeasonUpdate`). Both fields are required.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteSeasonUpdate {
    pub anime: Entry,
    pub new_seasons: Vec<Entry>,
}

/// Why a remote update failed. `Cancelled` is not a failure: it is never logged as an error (E-S5).
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(transparent)]
    Source(SourceError),
    #[error("{0}")]
    Db(DomainError),
    /// A failure to fetch that is not a source error. The Rust path has none today; the variant is kept for callers.
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    NoEpisodes(#[from] NoEpisodes),
    #[error(transparent)]
    NoSeasons(#[from] NoSeasons),
}

impl RemoteError {
    /// A source error, with cancellation kept apart from failures (E-S5).
    fn from_source(error: SourceError) -> Self {
        match error {
            SourceError::Cancelled => Self::Cancelled,
            other => Self::Source(other),
        }
    }
}

/// The source calls of a remote update: `getAnimeEpisodeUpdate` and `getAnimeSeasonUpdate`. A stub answers
/// `NotInstalled` before anything is written (E-S3).
#[async_trait]
pub trait AnimeRemote: Send + Sync {
    async fn episode_update(
        &self,
        anime: SAnime,
        episodes: Vec<SEpisode>,
        fetch_details: bool,
        fetch_episodes: bool,
    ) -> Result<SAnimeEpisodeUpdate, SourceError>;

    async fn season_update(
        &self,
        anime: SAnime,
        seasons: Vec<SAnime>,
        fetch_details: bool,
        fetch_seasons: bool,
    ) -> Result<SAnimeSeasonUpdate, SourceError>;
}

/// Every live anime source is an [`AnimeRemote`]: the calls are the source's own combined updates.
#[async_trait]
impl<T: AnimeSource + ?Sized> AnimeRemote for T {
    async fn episode_update(
        &self,
        anime: SAnime,
        episodes: Vec<SEpisode>,
        fetch_details: bool,
        fetch_episodes: bool,
    ) -> Result<SAnimeEpisodeUpdate, SourceError> {
        self.anime_episode_update(anime, episodes, fetch_details, fetch_episodes).await
    }

    async fn season_update(
        &self,
        anime: SAnime,
        seasons: Vec<SAnime>,
        fetch_details: bool,
        fetch_seasons: bool,
    ) -> Result<SAnimeSeasonUpdate, SourceError> {
        self.anime_season_update(anime, seasons, fetch_details, fetch_seasons).await
    }
}

/// The remote side of a stub: every call is `NotInstalled` with the stub's id.
pub struct StubRemote(pub Id);

#[async_trait]
impl AnimeRemote for StubRemote {
    async fn episode_update(
        &self,
        _anime: SAnime,
        _episodes: Vec<SEpisode>,
        _fetch_details: bool,
        _fetch_episodes: bool,
    ) -> Result<SAnimeEpisodeUpdate, SourceError> {
        Err(SourceError::NotInstalled(self.0))
    }

    async fn season_update(
        &self,
        _anime: SAnime,
        _seasons: Vec<SAnime>,
        _fetch_details: bool,
        _fetch_seasons: bool,
    ) -> Result<SAnimeSeasonUpdate, SourceError> {
        Err(SourceError::NotInstalled(self.0))
    }
}

/// The list sync of a remote update (`SyncEpisodesWithSource`, `SyncSeasonsWithSource`). It writes the new list and
/// returns the rows that are new. The empty-list guard runs before this call, in [`UpdateFromRemote`].
pub trait ListSync: Send + Sync {
    fn sync_episodes(
        &self,
        raw: &[SEpisode],
        anime: &Entry,
        manual: bool,
        window: (i64, i64),
    ) -> Result<Vec<Item>, DomainError>;

    fn sync_seasons(
        &self,
        raw: &[SAnime],
        anime: &Entry,
        manual: bool,
        window: (i64, i64),
    ) -> Result<Vec<Entry>, DomainError>;
}

/// The reads and the metadata write of a remote update. [`DbRemoteStore`] is the database form.
pub trait RemoteStore: Send + Sync {
    fn anime(&self, id: i64) -> Result<Entry, DomainError>;
    fn episodes(&self, anime_id: i64) -> Result<Vec<Item>, DomainError>;
    fn seasons(&self, parent_id: i64) -> Result<Vec<SeasonEntry>, DomainError>;
    /// Writes the metadata. An error stops the update before the list sync (E-S4).
    fn update_entry(&self, update: &EntryUpdate) -> Result<(), DomainError>;
}

/// The source view of a stored anime (`Anime.toSAnime`).
pub fn to_s_anime(anime: &Entry) -> SAnime {
    SAnime {
        url: anime.url.clone(),
        title: anime.title.clone(),
        artist: anime.artist.clone(),
        author: anime.author.clone(),
        description: anime.description.clone(),
        genre: anime.genre.as_ref().map(|genres| genres.join(", ")),
        status: AnimeStatus::from_i32(anime.status as i32),
        thumbnail_url: anime.thumbnail_url.clone(),
        background_url: anime.background_url.clone(),
        update_strategy: crate::entries::source_convert::to_source_strategy(anime.update_strategy),
        fetch_type: kuro_source::anime::FetchType::from(anime.fetch_type() as i64),
        season_number: anime.season_number(),
        initialized: anime.initialized,
        memo: anime.memo(),
    }
}

/// The source view of a stored episode (`Episode.toSEpisode`).
pub fn to_s_episode(episode: &Item) -> SEpisode {
    SEpisode {
        url: episode.url.clone(),
        name: episode.name.clone(),
        episode_number: episode.number as f32,
        fillermark: episode.fillermark,
        scanlator: episode.scanlator.clone(),
        date_upload: episode.date_upload,
        summary: episode.summary.clone(),
        preview_url: episode.preview_url.clone(),
        memo: episode.memo(),
    }
}

/// The database form of [`RemoteStore`], over the entry and item repositories.
pub struct DbRemoteStore {
    entries: EntryRepo,
    items: ItemRepo,
}

impl DbRemoteStore {
    pub fn new(db: Arc<Db>) -> Self {
        Self { entries: EntryRepo::new(db.clone()), items: ItemRepo::new(db) }
    }
}

impl RemoteStore for DbRemoteStore {
    fn anime(&self, id: i64) -> Result<Entry, DomainError> {
        Ok(self.entries.by_id(id)?)
    }
    fn episodes(&self, anime_id: i64) -> Result<Vec<Item>, DomainError> {
        Ok(self.items.by_entry_id(anime_id, false)?)
    }
    fn seasons(&self, parent_id: i64) -> Result<Vec<SeasonEntry>, DomainError> {
        Ok(self.entries.seasons_by_parent(parent_id)?)
    }
    fn update_entry(&self, update: &EntryUpdate) -> Result<(), DomainError> {
        Ok(self.entries.update(update)?)
    }
}

/// The remote update, with its ports. The clock is injected so the cover timestamps are testable.
pub struct UpdateFromRemote {
    store: Arc<dyn RemoteStore>,
    sync: Arc<dyn ListSync>,
    files: Arc<dyn CacheFiles>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// One lock per anime id, so two refreshes of one anime run one after the other (T-17a-047, E-C4).
    anime_locks: Mutex<HashMap<i64, Arc<AsyncMutex<()>>>>,
}

impl UpdateFromRemote {
    /// Uses the system clock for the cover timestamps (`Instant.now()`, in milliseconds since the epoch).
    pub fn new(store: Arc<dyn RemoteStore>, sync: Arc<dyn ListSync>, files: Arc<dyn CacheFiles>) -> Self {
        Self::with_clock(store, sync, files, Arc::new(|| chrono::Utc::now().timestamp_millis()))
    }

    pub fn with_clock(
        store: Arc<dyn RemoteStore>,
        sync: Arc<dyn ListSync>,
        files: Arc<dyn CacheFiles>,
        now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self { store, sync, files, now_ms, anime_locks: Mutex::new(HashMap::new()) }
    }

    /// The lock that serialises the refreshes of one anime.
    fn anime_lock(&self, id: i64) -> Arc<AsyncMutex<()>> {
        let mut locks = self.anime_locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(locks.entry(id).or_default())
    }

    /// `awaitEpisodesUpdate`. The source is asked before anything is written, so a stub writes nothing (E-S3).
    pub async fn episodes<A: AnimeRemote + ?Sized>(
        &self,
        anime: &Entry,
        source: &A,
        flags: FetchFlags,
    ) -> Result<RemoteEpisodeUpdate, RemoteError> {
        let lock = self.anime_lock(anime.id);
        let _serial = lock.lock().await;
        let result = self.episodes_inner(anime, source, flags).await;
        log_failure(&result);
        result
    }

    async fn episodes_inner<A: AnimeRemote + ?Sized>(
        &self,
        anime: &Entry,
        source: &A,
        flags: FetchFlags,
    ) -> Result<RemoteEpisodeUpdate, RemoteError> {
        let mut stored = self.store.episodes(anime.id).map_err(RemoteError::Db)?;
        stored.sort_by_key(|episode| episode.source_order);
        let raw: Vec<SEpisode> = stored.iter().map(to_s_episode).collect();
        let update = source
            .episode_update(to_s_anime(anime), raw, flags.fetch_details, flags.fetch_episodes)
            .await
            .map_err(RemoteError::from_source)?;
        self.apply_metadata(anime, &update.anime, flags.manual_fetch)?;
        if update.episodes.is_empty() && anime.source != LOCAL_SOURCE_ID {
            return Err(NoEpisodes.into());
        }
        let new_episodes = self
            .sync
            .sync_episodes(&update.episodes, anime, flags.manual_fetch, flags.fetch_window)
            .map_err(RemoteError::Db)?;
        let anime = self.store.anime(anime.id).map_err(RemoteError::Db)?;
        Ok(RemoteEpisodeUpdate { anime, new_episodes })
    }

    /// `awaitSeasonsUpdate`, the season twin of [`UpdateFromRemote::episodes`].
    pub async fn seasons<A: AnimeRemote + ?Sized>(
        &self,
        anime: &Entry,
        source: &A,
        flags: FetchFlags,
    ) -> Result<RemoteSeasonUpdate, RemoteError> {
        let lock = self.anime_lock(anime.id);
        let _serial = lock.lock().await;
        let result = self.seasons_inner(anime, source, flags).await;
        log_failure(&result);
        result
    }

    async fn seasons_inner<A: AnimeRemote + ?Sized>(
        &self,
        anime: &Entry,
        source: &A,
        flags: FetchFlags,
    ) -> Result<RemoteSeasonUpdate, RemoteError> {
        let mut stored = self.store.seasons(anime.id).map_err(RemoteError::Db)?;
        stored.sort_by_key(|season| season.entry.season_source_order());
        let raw: Vec<SAnime> = stored.iter().map(|season| to_s_anime(&season.entry)).collect();
        let update = source
            .season_update(to_s_anime(anime), raw, flags.fetch_details, flags.fetch_seasons)
            .await
            .map_err(RemoteError::from_source)?;
        self.apply_metadata(anime, &update.anime, flags.manual_fetch)?;
        if update.seasons.is_empty() && anime.source != LOCAL_SOURCE_ID {
            return Err(NoSeasons.into());
        }
        let new_seasons = self
            .sync
            .sync_seasons(&update.seasons, anime, flags.manual_fetch, flags.fetch_window)
            .map_err(RemoteError::Db)?;
        let anime = self.store.anime(anime.id).map_err(RemoteError::Db)?;
        Ok(RemoteSeasonUpdate { anime, new_seasons })
    }

    /// `awaitUpdateFromSource`: decides the cache actions, deletes the files they name, then writes the metadata. A
    /// failed write is an error, so the list sync does not run (E-S4, the documented difference from Kotlin).
    fn apply_metadata(&self, local: &Entry, remote: &SAnime, manual: bool) -> Result<(), RemoteError> {
        let update = prepare_update(local, &remote_from_anime(remote), manual, self.files.as_ref(), (self.now_ms)());
        self.store.update_entry(&update).map_err(RemoteError::Db)
    }
}

/// Logs a failed refresh at error level. A cancellation is not an error and is not logged (E-S5).
fn log_failure<T>(result: &Result<T, RemoteError>) {
    match result {
        Ok(_) | Err(RemoteError::Cancelled) => {}
        Err(err) => tracing::error!(%err, "remote entry update failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_flags_default_to_nothing_fetched_and_a_zero_window() {
        let flags = FetchFlags::default();
        assert!(!flags.fetch_details && !flags.fetch_episodes && !flags.fetch_seasons && !flags.manual_fetch);
        assert_eq!(flags.fetch_window, (0, 0));
    }

    #[test]
    fn a_cancellation_is_kept_apart_from_a_source_failure() {
        assert!(matches!(RemoteError::from_source(SourceError::Cancelled), RemoteError::Cancelled));
        assert!(matches!(RemoteError::from_source(SourceError::Http(503)), RemoteError::Source(_)));
    }
}

#[cfg(test)]
mod orchestration_tests {
    use std::sync::Arc;

    use kuro_platform::kind::Kind;

    use super::*;
    use crate::source::fakes::{CallLog, FakeAnimeSource, FakeCacheFiles, FakeListSync, MemRemoteStore};

    const NOW: i64 = 1_700_000_000_000;

    struct Rig {
        log: Arc<CallLog>,
        source: FakeAnimeSource,
        store: Arc<MemRemoteStore>,
        sync: Arc<FakeListSync>,
        files: Arc<FakeCacheFiles>,
    }

    fn anime(source: i64, favorite: bool) -> Entry {
        let mut anime = Entry::create(Kind::Anime);
        anime.id = 5;
        anime.source = source;
        anime.url = "/show".into();
        anime.title = "Stored".into();
        anime.favorite = favorite;
        anime.thumbnail_url = Some("thumb-a".into());
        anime.background_url = Some("bg-a".into());
        anime
    }

    fn episode(url: &str, source_order: i64) -> Item {
        let mut episode = Item::create(Kind::Anime);
        episode.url = url.into();
        episode.source_order = source_order;
        episode
    }

    fn s_anime(title: &str) -> SAnime {
        SAnime { url: "/show".into(), title: title.into(), ..SAnime::default() }
    }

    fn s_episode(url: &str) -> SEpisode {
        SEpisode { url: url.into(), name: url.into(), ..SEpisode::default() }
    }

    fn make_rig(stored: Entry) -> Rig {
        let log = Arc::new(CallLog::default());
        let source = FakeAnimeSource::new(log.clone(), s_anime("Fresh"));
        let store = Arc::new(MemRemoteStore::new(log.clone(), stored));
        let sync = Arc::new(FakeListSync::new(log.clone()));
        let files = Arc::new(FakeCacheFiles { log: log.clone(), custom_cover: false, custom_background: false });
        Rig { log, source, store, sync, files }
    }

    fn updater(rig: &Rig) -> UpdateFromRemote {
        UpdateFromRemote::with_clock(rig.store.clone(), rig.sync.clone(), rig.files.clone(), Arc::new(|| NOW))
    }

    fn calls(rig: &Rig) -> Vec<String> {
        rig.log.calls()
    }

    #[tokio::test]
    async fn the_episode_refresh_runs_read_source_write_sync_then_reread() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.episodes = vec![s_episode("/e1")];
        rig.sync = Arc::new(FakeListSync {
            new_episodes: vec![episode("/e1", 0)],
            ..FakeListSync::new(rig.log.clone())
        });
        let result = updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect("episodes");
        assert_eq!(
            calls(&rig),
            ["read.episodes", "source.episodes", "write.anime", "sync.episodes", "read.anime"],
            "T-17a-027: the order of operations"
        );
        assert_eq!(result.new_episodes.len(), 1);
        assert_eq!(result.anime.title, "Fresh", "the anime is re-read after the write");
    }

    #[tokio::test]
    async fn a_refresh_waits_for_the_running_refresh_of_the_same_anime_only() {
        // E-C4: the lock is per anime id. While anime 5 is held, a second refresh of 5 waits and a refresh of 6 runs.
        let stored = anime(7, false);
        let rig = make_rig(stored.clone());
        let updater = updater(&rig);
        let held = updater.anime_lock(stored.id);
        let _running = held.lock().await;
        let wait = std::time::Duration::from_millis(50);
        let same = tokio::time::timeout(wait, updater.episodes(&stored, &rig.source, FetchFlags::default())).await;
        assert!(same.is_err(), "E-C4: the second refresh of anime 5 waits for the first");
        let other = Entry { id: 6, ..stored.clone() };
        let free = tokio::time::timeout(wait, updater.episodes(&other, &rig.source, FetchFlags::default())).await;
        assert!(free.is_ok(), "a refresh of another anime is not blocked");
    }

    #[tokio::test]
    async fn two_concurrent_refreshes_of_one_anime_run_one_whole_cycle_after_the_other() {
        // T-17a-047 / E-C4: each refresh holds the anime's lock for its read, source, write, sync and re-read.
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.episodes = vec![s_episode("/e1")];
        rig.sync = Arc::new(FakeListSync { new_episodes: vec![episode("/e1", 0)], ..FakeListSync::new(rig.log.clone()) });
        let updater = updater(&rig);
        let (first, second) = tokio::join!(
            updater.episodes(&stored, &rig.source, FetchFlags::default()),
            updater.episodes(&stored, &rig.source, FetchFlags::default()),
        );
        first.expect("first refresh");
        second.expect("second refresh");
        let cycle = ["read.episodes", "source.episodes", "write.anime", "sync.episodes", "read.anime"];
        let expected: Vec<&str> = cycle.iter().chain(cycle.iter()).copied().collect();
        assert_eq!(calls(&rig), expected, "E-C4: the second refresh starts after the first one has finished");
    }

    #[tokio::test]
    async fn episodes_reach_the_source_in_source_order() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.episodes = vec![s_episode("/first")];
        rig.store = Arc::new(MemRemoteStore {
            episodes: vec![episode("/third", 2), episode("/first", 0), episode("/second", 1)],
            ..MemRemoteStore::new(rig.log.clone(), stored.clone())
        });
        updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect("episodes");
        assert_eq!(*rig.source.received_urls.lock().unwrap(), ["/first", "/second", "/third"], "E-S7");
    }

    #[tokio::test]
    async fn an_empty_list_from_a_non_local_source_fails_after_the_metadata_write() {
        let stored = anime(7, false);
        let rig = make_rig(stored.clone());
        let err = updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect_err("E-S1");
        assert!(matches!(err, RemoteError::NoEpisodes(_)));
        assert_eq!(calls(&rig), ["read.episodes", "source.episodes", "write.anime"], "metadata already written, no sync");
    }

    #[tokio::test]
    async fn an_empty_list_from_the_local_source_succeeds_with_no_new_episodes() {
        let stored = anime(LOCAL_SOURCE_ID, false);
        let rig = make_rig(stored.clone());
        let result = updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect("E-S2");
        assert!(result.new_episodes.is_empty());
        assert!(calls(&rig).contains(&"sync.episodes".to_owned()));
    }

    #[tokio::test]
    async fn a_stub_fails_with_not_installed_and_writes_nothing() {
        let stored = anime(7, false);
        let rig = make_rig(stored.clone());
        let err = updater(&rig).episodes(&stored, &StubRemote(7), FetchFlags::default()).await.expect_err("E-S3");
        assert!(matches!(err, RemoteError::Source(SourceError::NotInstalled(7))));
        assert!(!calls(&rig).iter().any(|call| call.starts_with("write") || call.starts_with("sync")), "no DB write");
    }

    #[tokio::test]
    async fn a_failed_metadata_write_stops_the_episode_sync() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.store = Arc::new(MemRemoteStore { fail_update: true, ..MemRemoteStore::new(rig.log.clone(), stored.clone()) });
        let err = updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect_err("E-S4");
        assert!(matches!(err, RemoteError::Db(_)));
        assert!(!calls(&rig).contains(&"sync.episodes".to_owned()), "the documented difference: no sync");
    }

    #[tokio::test]
    async fn a_cancellation_is_not_a_failure_and_writes_nothing() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.failure = Some(SourceError::Cancelled);
        let err = updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect_err("E-S5");
        assert!(matches!(err, RemoteError::Cancelled));
        assert!(!calls(&rig).iter().any(|call| call.starts_with("write")), "nothing written after a cancel");
    }

    #[tokio::test]
    async fn an_empty_season_list_from_a_non_local_source_fails_with_no_seasons() {
        let stored = anime(7, false);
        let rig = make_rig(stored.clone());
        let err = updater(&rig).seasons(&stored, &rig.source, FetchFlags::default()).await.expect_err("E-S6");
        assert!(matches!(err, RemoteError::NoSeasons(_)));
    }

    #[tokio::test]
    async fn the_season_refresh_runs_read_source_write_sync_then_reread() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.seasons = vec![s_anime("/s1")];
        let result = updater(&rig).seasons(&stored, &rig.source, FetchFlags::default()).await.expect("seasons");
        assert_eq!(calls(&rig), ["read.seasons", "source.seasons", "write.anime", "sync.seasons", "read.anime"]);
        assert!(result.new_seasons.is_empty());
    }

    #[tokio::test]
    async fn the_fetch_flags_and_the_window_reach_the_source_and_the_sync_unchanged() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.episodes = vec![s_episode("/e1")];
        let flags = FetchFlags {
            fetch_details: true,
            fetch_episodes: true,
            fetch_seasons: false,
            manual_fetch: true,
            fetch_window: (5, 9),
        };
        updater(&rig).episodes(&stored, &rig.source, flags).await.expect("episodes");
        assert_eq!(*rig.source.asked.lock().unwrap(), [(true, true)], "T-17a-031: the source sees the flags");
        assert_eq!(*rig.sync.calls.lock().unwrap(), [(1, true, (5, 9))], "the sync sees manual and the window");
    }

    #[tokio::test]
    async fn a_manual_fetch_rewrites_the_cover_timestamp_even_when_the_url_is_unchanged() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source = FakeAnimeSource::new(rig.log.clone(), SAnime { thumbnail_url: Some("thumb-a".into()), ..s_anime("T") });
        rig.source.episodes = vec![s_episode("/e1")];
        let manual = FetchFlags { manual_fetch: true, ..FetchFlags::default() };
        updater(&rig).episodes(&stored, &rig.source, manual).await.expect("manual");
        let update = rig.store.updates.lock().unwrap()[0].clone();
        assert_eq!(update.cover_last_modified, Some(NOW), "E-R3: the timestamp is rewritten");
        assert!(calls(&rig).contains(&"files.delete_cover".to_owned()), "the non-custom cover is deleted");
    }

    #[tokio::test]
    async fn a_non_favourite_takes_the_remote_title_and_a_favourite_keeps_its_own() {
        let stored = anime(7, false);
        let mut rig = make_rig(stored.clone());
        rig.source.episodes = vec![s_episode("/e1")];
        let result = updater(&rig).episodes(&stored, &rig.source, FetchFlags::default()).await.expect("episodes");
        assert_eq!(result.anime.title, "Fresh");

        let favourite = anime(7, true);
        let mut rig = make_rig(favourite.clone());
        rig.source.episodes = vec![s_episode("/e1")];
        let result = updater(&rig).episodes(&favourite, &rig.source, FetchFlags::default()).await.expect("episodes");
        assert_eq!(result.anime.title, "Stored", "E-T1: a favourite keeps its title");
    }
}
