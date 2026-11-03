//! Fakes and fixtures for the source tests (T-17a-038). Test-only. Each fake keeps
//! its calls, so a test can check what was asked and in which order.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use kuro_data::entry::{Entry, EntryUpdate, SeasonEntry};
use kuro_data::item::Item;
use kuro_db::subscribe::Subscription;
use kuro_source::SourceError;
use kuro_source::anime::{SAnime, SAnimeEpisodeUpdate, SAnimeSeasonUpdate, SEpisode};
use kuro_source::filter_list::FilterList;
use kuro_source::model::{MangasPage, Page, SChapter, SManga};
use kuro_source::source::{CatalogueSource, MangaSource};
use serde::Deserialize;
use tokio::sync::watch;

use crate::error::DomainError;
use crate::source::model::{Id, Source};
use crate::source::registry::InstalledSource;
use crate::entries::update::CacheFiles;
use crate::source::remote_update::{AnimeRemote, ListSync, RemoteStore};
use crate::source::repo::{ExtensionCatalog, StubSourcePort};
use crate::source::stub::StubSource;

/// The calls a test saw, in order. Shared between fakes so one log shows the whole sequence.
#[derive(Default)]
pub struct CallLog(Mutex<Vec<String>>);

impl CallLog {
    pub fn push(&self, call: impl Into<String>) {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(call.into());
    }

    pub fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

/// An in-memory stub table (`StubStore`): a map, a counter of writes, and a switch that makes every call fail (E-B2).
#[derive(Default)]
pub struct MemStubStore {
    rows: Mutex<HashMap<Id, StubSource>>,
    writes: AtomicUsize,
    reads: AtomicUsize,
    fail: bool,
}

impl MemStubStore {
    pub fn with_rows(rows: &[StubSource]) -> Self {
        Self {
            rows: Mutex::new(rows.iter().map(|row| (row.id, row.clone())).collect()),
            ..Self::default()
        }
    }

    /// A store whose every read and write fails, for the DB-unavailable start (E-B2).
    pub fn unavailable() -> Self {
        Self { fail: true, ..Self::default() }
    }

    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn failure(&self) -> Result<(), DomainError> {
        if self.fail { Err(DomainError::Store("stub table unavailable")) } else { Ok(()) }
    }
}

impl StubSourcePort for MemStubStore {
    fn get(&self, id: Id) -> Result<Option<StubSource>, DomainError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.failure()?;
        Ok(self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&id).cloned())
    }

    fn all(&self) -> Result<Vec<StubSource>, DomainError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.failure()?;
        let mut rows: Vec<StubSource> =
            self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).values().cloned().collect();
        rows.sort_by_key(|row| row.id);
        Ok(rows)
    }

    fn upsert(&self, id: Id, lang: &str, name: &str) -> Result<(), DomainError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.failure()?;
        self.rows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, StubSource { id, lang: lang.to_owned(), name: name.to_owned() });
        Ok(())
    }

    fn subscribe_all(&self) -> Subscription<Vec<StubSource>> {
        let rows = self.all().unwrap_or_default();
        Subscription::over(watch::channel(Some(Ok(Arc::new(rows)))).1)
    }
}

/// An extension index that publishes its rows on demand (`ExtensionCatalog`).
#[derive(Default)]
pub struct FakeCatalog {
    rows: RwLock<HashMap<Id, StubSource>>,
}

impl FakeCatalog {
    /// Replaces the index with `rows`, as an extension snapshot would.
    pub fn publish(&self, rows: &[StubSource]) {
        *self.rows.write().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            rows.iter().map(|row| (row.id, row.clone())).collect();
    }
}

impl ExtensionCatalog for FakeCatalog {
    fn index_source(&self, id: Id) -> Option<StubSource> {
        self.rows.read().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&id).cloned()
    }
}

/// Records the `(old, new)` pairs a registry reports as renames (`SourceRenamer`).
#[derive(Default)]
pub struct RenameLog(Mutex<Vec<(String, String)>>);

impl RenameLog {
    pub fn record(&self, old: &str, new: &str) {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((old.to_owned(), new.to_owned()));
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

/// A live catalogue manga source with a fixed name, language and `supportsLatest` (for the identity adapter).
pub struct LiveCatalogue {
    id: i64,
    name: String,
    lang: String,
    latest: bool,
}

impl LiveCatalogue {
    pub fn new(id: i64, name: &str, lang: &str, latest: bool) -> Self {
        Self { id, name: name.to_owned(), lang: lang.to_owned(), latest }
    }

    fn page(&self) -> Result<MangasPage, SourceError> {
        Ok(MangasPage { mangas: vec![SManga::new("/m", "M")], has_next_page: false })
    }
}

#[async_trait]
impl MangaSource for LiveCatalogue {
    fn id(&self) -> i64 {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn lang(&self) -> &str {
        &self.lang
    }

    async fn get_manga_details(&self, manga: &SManga) -> Result<SManga, SourceError> {
        Ok(manga.clone())
    }

    async fn get_chapter_list(&self, _manga: &SManga) -> Result<Vec<SChapter>, SourceError> {
        Ok(Vec::new())
    }

    async fn get_page_list(&self, _chapter: &SChapter) -> Result<Vec<Page>, SourceError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl CatalogueSource for LiveCatalogue {
    fn supports_latest(&self) -> bool {
        self.latest
    }

    fn filter_list(&self) -> FilterList {
        FilterList::default()
    }

    async fn popular(&self, _page: i32) -> Result<MangasPage, SourceError> {
        self.page()
    }

    async fn search(&self, _page: i32, _query: &str, _filters: &FilterList) -> Result<MangasPage, SourceError> {
        self.page()
    }

    async fn latest(&self, _page: i32) -> Result<MangasPage, SourceError> {
        self.page()
    }
}

/// An installed source as the registry sees it, with fixed flags.
pub struct Installed {
    pub id: Id,
    pub http: bool,
    pub catalogue: bool,
    pub source: Source,
}

impl InstalledSource for Installed {
    fn id(&self) -> Id {
        self.id
    }
    fn is_http(&self) -> bool {
        self.http
    }
    fn is_catalogue(&self) -> bool {
        self.catalogue
    }
    fn source(&self) -> Source {
        self.source.clone()
    }
}

/// The anime source calls of a remote update. It records each request and answers with canned data or a failure.
pub struct FakeAnimeSource {
    pub log: Arc<CallLog>,
    pub episodes: Vec<SEpisode>,
    pub seasons: Vec<SAnime>,
    pub anime: SAnime,
    pub failure: Option<SourceError>,
    /// The `(fetch_details, fetch_list)` flags of each call.
    pub asked: Mutex<Vec<(bool, bool)>>,
    /// The episode URLs the source was given, in the order it received them.
    pub received_urls: Mutex<Vec<String>>,
}

impl FakeAnimeSource {
    pub fn new(log: Arc<CallLog>, anime: SAnime) -> Self {
        Self {
            log,
            episodes: Vec::new(),
            seasons: Vec::new(),
            anime,
            failure: None,
            asked: Mutex::new(Vec::new()),
            received_urls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AnimeRemote for FakeAnimeSource {
    async fn episode_update(
        &self,
        _anime: SAnime,
        episodes: Vec<SEpisode>,
        fetch_details: bool,
        fetch_episodes: bool,
    ) -> Result<SAnimeEpisodeUpdate, SourceError> {
        self.log.push("source.episodes");
        self.asked.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((fetch_details, fetch_episodes));
        *self.received_urls.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            episodes.iter().map(|episode| episode.url.clone()).collect();
        if let Some(err) = &self.failure {
            return Err(clone_error(err));
        }
        Ok(SAnimeEpisodeUpdate { anime: self.anime.clone(), episodes: self.episodes.clone() })
    }

    async fn season_update(
        &self,
        _anime: SAnime,
        seasons: Vec<SAnime>,
        fetch_details: bool,
        fetch_seasons: bool,
    ) -> Result<SAnimeSeasonUpdate, SourceError> {
        self.log.push("source.seasons");
        self.asked.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((fetch_details, fetch_seasons));
        *self.received_urls.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            seasons.iter().map(|season| season.url.clone()).collect();
        if let Some(err) = &self.failure {
            return Err(clone_error(err));
        }
        Ok(SAnimeSeasonUpdate { anime: self.anime.clone(), seasons: self.seasons.clone() })
    }
}

/// `SourceError` is not `Clone`, so the fake rebuilds the failure kinds the tests use.
fn clone_error(err: &SourceError) -> SourceError {
    match err {
        SourceError::Cancelled => SourceError::Cancelled,
        SourceError::NotInstalled(id) => SourceError::NotInstalled(*id),
        SourceError::Http(code) => SourceError::Http(*code),
        other => SourceError::Transport(other.to_string()),
    }
}

/// One list-sync call: the number of raw rows, the manual flag and the fetch window.
pub type SyncCall = (usize, bool, (i64, i64));

/// The list sync of a remote update. It records the arguments it was given and returns canned new rows.
pub struct FakeListSync {
    pub log: Arc<CallLog>,
    pub calls: Mutex<Vec<SyncCall>>,
    pub new_episodes: Vec<Item>,
    pub new_seasons: Vec<Entry>,
}

impl FakeListSync {
    pub fn new(log: Arc<CallLog>) -> Self {
        Self { log, calls: Mutex::new(Vec::new()), new_episodes: Vec::new(), new_seasons: Vec::new() }
    }
}

impl ListSync for FakeListSync {
    fn sync_episodes(
        &self,
        raw: &[SEpisode],
        _anime: &Entry,
        manual: bool,
        window: (i64, i64),
    ) -> Result<Vec<Item>, DomainError> {
        self.log.push("sync.episodes");
        self.calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((raw.len(), manual, window));
        Ok(self.new_episodes.clone())
    }

    fn sync_seasons(
        &self,
        raw: &[SAnime],
        _anime: &Entry,
        manual: bool,
        window: (i64, i64),
    ) -> Result<Vec<Entry>, DomainError> {
        self.log.push("sync.seasons");
        self.calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((raw.len(), manual, window));
        Ok(self.new_seasons.clone())
    }
}

/// An in-memory anime, its episodes and seasons, with a switch that makes the metadata write fail (E-S4). Every write it
/// accepts is kept in `updates`.
pub struct MemRemoteStore {
    pub log: Arc<CallLog>,
    pub anime: Mutex<Entry>,
    pub episodes: Vec<Item>,
    pub seasons: Vec<SeasonEntry>,
    pub fail_update: bool,
    pub updates: Mutex<Vec<EntryUpdate>>,
}

impl MemRemoteStore {
    pub fn new(log: Arc<CallLog>, anime: Entry) -> Self {
        Self {
            log,
            anime: Mutex::new(anime),
            episodes: Vec::new(),
            seasons: Vec::new(),
            fail_update: false,
            updates: Mutex::new(Vec::new()),
        }
    }
}

impl RemoteStore for MemRemoteStore {
    fn anime(&self, _id: i64) -> Result<Entry, DomainError> {
        self.log.push("read.anime");
        Ok(self.anime.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone())
    }

    fn episodes(&self, _anime_id: i64) -> Result<Vec<Item>, DomainError> {
        self.log.push("read.episodes");
        Ok(self.episodes.clone())
    }

    fn seasons(&self, _parent_id: i64) -> Result<Vec<SeasonEntry>, DomainError> {
        self.log.push("read.seasons");
        Ok(self.seasons.clone())
    }

    fn update_entry(&self, update: &EntryUpdate) -> Result<(), DomainError> {
        self.log.push("write.anime");
        if self.fail_update {
            return Err(DomainError::Store("metadata write failed"));
        }
        self.updates.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(update.clone());
        let mut anime = self.anime.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(title) = &update.title {
            anime.title = title.clone();
        }
        Ok(())
    }
}

/// Cover and background files that record their deletions (`CacheFiles`).
pub struct FakeCacheFiles {
    pub log: Arc<CallLog>,
    pub custom_cover: bool,
    pub custom_background: bool,
}

impl CacheFiles for FakeCacheFiles {
    fn has_custom_cover(&self, _anime: &Entry) -> bool {
        self.custom_cover
    }
    fn has_custom_background(&self, _anime: &Entry) -> bool {
        self.custom_background
    }
    fn delete_cover(&self, _anime: &Entry) {
        self.log.push("files.delete_cover");
    }
    fn delete_background(&self, _anime: &Entry) {
        self.log.push("files.delete_background");
    }
}

/// One row of `tests/fixtures/sources/manga_rows.json`.
#[derive(Deserialize)]
struct FixtureRow {
    id: Id,
    lang: String,
    name: String,
    #[serde(default)]
    count: i64,
    #[serde(default)]
    stub: bool,
}

#[derive(Deserialize)]
struct MangaRows {
    enabled_rows: Vec<FixtureRow>,
    migration_rows: Vec<FixtureRow>,
}

fn fixture() -> MangaRows {
    serde_json::from_str(include_str!("../../tests/fixtures/sources/manga_rows.json")).expect("manga_rows.json parses")
}

/// The six enabled source rows, in repository order.
pub fn enabled_rows() -> Vec<Source> {
    fixture()
        .enabled_rows
        .into_iter()
        .map(|row| Source::new(row.id, &row.name, &row.lang))
        .collect()
}

/// The five rows of section 3 with their favourite counts (a to e).
pub fn migration_rows() -> Vec<(Source, i64)> {
    fixture()
        .migration_rows
        .into_iter()
        .map(|row| {
            let source = Source { is_stub: row.stub, ..Source::new(row.id, &row.name, &row.lang) };
            (source, row.count)
        })
        .collect()
}
