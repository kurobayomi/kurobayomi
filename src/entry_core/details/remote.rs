//! The source side of an anime refresh (T-39b-021). `RemoteAnimeBackend` runs a fetch through the domain's
//! `UpdateFromRemote`: it asks the source, writes the metadata with the cover and background rules (T-REF-5), syncs the
//! episode list and returns the episodes the sync added, which a manual refresh may queue for download (T-REF-4).
//!
//! Where a source comes from is `AnimeSourceLookup`. `LiveSources` answers for the sources this build ships
//! (`builtin_anime_sources`, none yet); an id it does not know reports that the source is not installed. A `Seasons`
//! entry syncs its season list through the domain's `SyncSeasons`, and with "update seasons on refresh" on, the episodes
//! of its seasons are fetched after it (`season_fetch`).
//!
//! Known gap: no cover cache is wired, so a changed cover is stamped but its file is not deleted.

use std::collections::HashSet;
use std::sync::Arc;

use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_db::handle::Db;
use kuro_db::codec::FetchType;
use kuro_domain::error::DomainError;
use async_trait::async_trait;
use kuro_domain::items::season_sync::SyncSeasons;
use kuro_domain::items::source_sync::EntrySyncLocks;
use kuro_domain::items::sync::{NoRenames, SourceItem, SyncItems, SyncItemsError, SyncOptions, source_item_from_episode};
use kuro_domain::source::remote_update::{AnimeRemote, DbRemoteStore, FetchFlags, ListSync, RemoteError, RemoteStore, UpdateFromRemote};
use kuro_platform::kind::Kind;
use kuro_prefs::store::PrefStore;
use kuro_source::SourceError;
use kuro_source::anime::{AnimeSource, SAnime, SAnimeEpisodeUpdate, SAnimeSeasonUpdate, SEpisode};
use tokio::runtime::Handle;

use super::entry_state::RefreshPlan;
use super::download_backend::EntryDownloads;
use crate::entry_core::refresh_rules::Trigger;
use super::runner::{EntryBackend, EntryFetch, StubEntryBackend};
use super::season_fetch::fetch_episodes_of;
use super::store::EntryStore;
use crate::browse::errors::limited;
use crate::entry_core::backend::NoCoverFiles;
use crate::entry_core::source_info::SourceInfo;
use crate::entry_core::errors::DetailError;

/// Finds the live anime source of an id.
pub trait AnimeSourceLookup: Send + Sync + 'static {
    /// The source, or `None` when its extension is not installed.
    fn remote(&self, source_id: i64) -> Option<Arc<dyn AnimeRemote>>;
    /// The bare name of an installed source, for the sentences of a failed refresh. `None` reads "This source".
    fn name(&self, _source_id: i64) -> Option<String> {
        None
    }
    /// The source line of the info box for an installed source. `None` falls back to the stub reading.
    fn info(&self, _source_id: i64) -> Option<SourceInfo> {
        None
    }
    /// Whether the source is configurable (`source is ConfigurableAnimeSource`): only then does the overflow menu offer
    /// Settings (T-39a-057).
    fn is_configurable(&self, _source_id: i64) -> bool {
        false
    }
    /// Whether the source is an `UnmeteredSource`: a season refresh fetches its seasons in parallel.
    fn is_unmetered(&self, _source_id: i64) -> bool {
        false
    }
}

/// The lookup over live sources. The flags come from the source itself (`is_configurable`, `is_unmetered`); the trait has
/// no HTTP marker, so it reads false and the page offers no WebView.
pub struct LiveSources {
    sources: Vec<Arc<dyn AnimeSource>>,
}

impl LiveSources {
    pub fn new(sources: Vec<Arc<dyn AnimeSource>>) -> Self {
        Self { sources }
    }

    #[cfg(test)]
    /// The sources this build ships.
    pub fn builtin() -> Self {
        Self::new(builtin_anime_sources())
    }

    fn find(&self, source_id: i64) -> Option<&Arc<dyn AnimeSource>> {
        self.sources.iter().find(|s| s.id() == source_id)
    }
}

/// A live source as the domain's remote update asks it: its own combined updates. A panicking source fails its own
/// refresh (`limited`: the panic guard and the 30 second limit), so the page always gets its refresh result and offers
/// Retry.
struct SourceRemote(Arc<dyn AnimeSource>);

#[async_trait]
impl AnimeRemote for SourceRemote {
    async fn episode_update(&self, entry: SAnime, episodes: Vec<SEpisode>, fetch_details: bool, fetch_episodes: bool) -> Result<SAnimeEpisodeUpdate, SourceError> {
        limited(self.0.anime_episode_update(entry, episodes, fetch_details, fetch_episodes)).await
    }

    async fn season_update(&self, entry: SAnime, seasons: Vec<SAnime>, fetch_details: bool, fetch_seasons: bool) -> Result<SAnimeSeasonUpdate, SourceError> {
        limited(self.0.anime_season_update(entry, seasons, fetch_details, fetch_seasons)).await
    }
}

impl AnimeSourceLookup for LiveSources {
    fn remote(&self, source_id: i64) -> Option<Arc<dyn AnimeRemote>> {
        self.find(source_id).map(|source| Arc::new(SourceRemote(Arc::clone(source))) as Arc<dyn AnimeRemote>)
    }

    fn info(&self, source_id: i64) -> Option<SourceInfo> {
        self.find(source_id).map(|source| SourceInfo { name: source.display_name(), stub: false })
    }

    fn name(&self, source_id: i64) -> Option<String> {
        self.find(source_id).map(|source| source.name().to_owned())
    }

    fn is_configurable(&self, source_id: i64) -> bool {
        self.find(source_id).is_some_and(|source| source.is_configurable())
    }

    fn is_unmetered(&self, source_id: i64) -> bool {
        self.find(source_id).is_some_and(|source| source.is_unmetered())
    }
}

/// The list sync of `UpdateFromRemote` over the domain's async episode sync and its season sync. It runs on a tokio
/// worker, so the wait for the episode sync is `block_in_place`.
struct EpisodeListSync {
    sync: SyncItems,
    seasons: SyncSeasons,
    handle: Handle,
    mark_duplicate_done: bool,
}

impl ListSync for EpisodeListSync {
    fn sync_episodes(&self, raw: &[SEpisode], entry: &Entry, manual: bool, window: (i64, i64)) -> Result<Vec<Item>, DomainError> {
        let opts = SyncOptions {
            now_ms: chrono::Utc::now().timestamp_millis(),
            manual,
            window,
            mark_duplicate_done: self.mark_duplicate_done,
            is_local: entry.source == kuro_source::local::LOCAL_SOURCE_ID,
        };
        let items: Vec<SourceItem> = raw.iter().map(source_item_from_episode).collect();
        let run = self.sync.run(items, entry, opts, None);
        tokio::task::block_in_place(|| self.handle.block_on(run))
            .map_err(|e| match e {
                SyncItemsError::Db(e) => DomainError::Db(e),
                SyncItemsError::NoItems => DomainError::Invalid("the source returned no episodes"),
            })
    }

    fn sync_seasons(&self, raw: &[SAnime], entry: &Entry, manual: bool, window: (i64, i64)) -> Result<Vec<Entry>, DomainError> {
        self.seasons.run(raw, entry, manual, window, chrono::Utc::now().timestamp_millis())
    }
}

/// The backend of the anime page over a source lookup.
pub struct RemoteAnimeBackend {
    stubs: StubEntryBackend,
    lookup: Arc<dyn AnimeSourceLookup>,
    store: Arc<DbRemoteStore>,
    update: Arc<UpdateFromRemote>,
    episodes: EntryStore,
    downloads: Option<EntryDownloads>,
}

impl RemoteAnimeBackend {
    /// `handle` is the runtime the fetches run on, which must be a multi-threaded one.
    pub fn new(db: Arc<Db>, _prefs: &PrefStore, lookup: Arc<dyn AnimeSourceLookup>, handle: Handle) -> Self {
        let mark_duplicate_done = crate::misc::diet_defaults::LIBRARY_MARK_DUPLICATE_EPISODE_SEEN;
        let sync = SyncItems::new(Arc::clone(&db), Arc::new(EntrySyncLocks::default()), Arc::new(NoRenames));
        let list_sync = EpisodeListSync { sync, seasons: SyncSeasons::new(Arc::clone(&db)), handle, mark_duplicate_done };
        let store = Arc::new(DbRemoteStore::new(Arc::clone(&db)));
        let update = Arc::new(UpdateFromRemote::new(store.clone(), Arc::new(list_sync), Arc::new(NoCoverFiles)));
        Self { stubs: StubEntryBackend::new(Arc::clone(&db)), lookup, store, update, episodes: EntryStore::new(db, Kind::Anime), downloads: None }
    }

    /// Gives the backend the downloads of the page: the downloads folder and the app's queue.
    pub fn with_downloads(mut self, downloads: EntryDownloads) -> Self {
        self.downloads = Some(downloads);
        self
    }

    /// The entry and the episodes with these ids (all of them when `ids` is `None`), with the source line.
    fn entry_episodes(&self, entry_id: i64, ids: Option<&[i64]>) -> Option<(Entry, Vec<Item>, SourceInfo)> {
        let entry = self.episodes.entry(entry_id).ok().flatten()?;
        let episodes: Vec<Item> = self.episodes.episodes(entry_id).ok()?.into_iter().filter(|e| ids.is_none_or(|ids| ids.contains(&e.id))).collect();
        let info = self.source_info(entry.source);
        Some((entry, episodes, info))
    }
}

impl EntryBackend for RemoteAnimeBackend {
    fn remove_cover(&self, entry_id: i64) -> bool {
        use kuro_images::art::ArtKind;
        let db = self.episodes.db();
        // both files go; the timestamp moves when either did
        let cover = crate::library::mount::clear_cached(&db, entry_id, ArtKind::Cover);
        crate::library::mount::clear_cached(&db, entry_id, ArtKind::Background) + cover > 0
    }

    fn has_logged_in_tracker(&self) -> bool {
        crate::entry_core::tracking::session::has_logged_in()
    }

    fn bind_enhanced(&self, entry_id: i64) {
        crate::updates::track_binding::bind_entry(&self.episodes.db(), Kind::Anime, entry_id);
    }
    fn source_info(&self, source_id: i64) -> SourceInfo {
        self.lookup.info(source_id).unwrap_or_else(|| self.stubs.source_info(source_id))
    }

    fn fetch(&self, entry_id: i64, plan: RefreshPlan, trigger: Trigger) -> EntryFetch {
        let (store, update, lookup) = (Arc::clone(&self.store), Arc::clone(&self.update), Arc::clone(&self.lookup));
        let season_episodes = crate::misc::diet_defaults::LIBRARY_UPDATE_SEASON;
        Box::pin(async move {
            let entry = store.anime(entry_id).map_err(domain_error)?;
            let Some(remote) = lookup.remote(entry.source) else {
                return Err(DetailError::SourceNotInstalled);
            };
            let name = lookup.name(entry.source).unwrap_or_default();
            let flags = FetchFlags {
                fetch_details: plan.info,
                fetch_episodes: plan.episodes,
                fetch_seasons: plan.seasons,
                manual_fetch: trigger == Trigger::Manual,
                fetch_window: (0, 0),
            };
            match entry.fetch_type() {
                FetchType::Episodes => update
                    .episodes(&entry, &*remote, flags)
                    .await
                    .map(|r| r.new_episodes)
                    .map_err(|e| detail_error(e, &name)),
                FetchType::Seasons => {
                    let seasons = update.seasons(&entry, &*remote, flags).await.map_err(|e| detail_error(e, &name))?.new_seasons;
                    // behaviour 6.4.1: with "update seasons on refresh" on, the episodes of the unfinished seasons follow
                    if season_episodes {
                        let episode_flags = FetchFlags { fetch_details: false, fetch_episodes: true, fetch_seasons: false, ..flags };
                        fetch_episodes_of(seasons, lookup.is_unmetered(entry.source), |season| {
                            let (update, remote) = (Arc::clone(&update), Arc::clone(&remote));
                            async move {
                                // one failing season is logged and does not stop the others; new episodes are not downloaded
                                match update.episodes(&season, &*remote, episode_flags).await {
                                    Ok(_) | Err(RemoteError::Cancelled | RemoteError::Source(SourceError::Cancelled)) => {}
                                    Err(error) => tracing::error!(season = season.id, %error, "the episodes of a season were not fetched"),
                                }
                            }
                        })
                        .await;
                    }
                    Ok(Vec::new())
                }
            }
        })
    }

    fn home_url(&self, source_id: i64) -> Option<String> {
        crate::browse::directory::home_url(Kind::Anime, source_id)
    }

    fn is_configurable_source(&self, source_id: i64) -> bool {
        self.lookup.is_configurable(source_id)
    }

    fn has_download_handler(&self) -> bool {
        self.downloads.as_ref().is_some_and(EntryDownloads::can_enqueue)
    }

    fn downloaded_items(&self, entry_id: i64) -> HashSet<i64> {
        let (Some(downloads), Some((entry, episodes, info))) = (&self.downloads, self.entry_episodes(entry_id, None)) else { return HashSet::new() };
        downloads.downloaded(&entry, &episodes, &info)
    }

    fn downloaded_count(&self, entry_id: i64) -> usize {
        self.downloaded_items(entry_id).len()
    }

    fn enqueue_items(&self, entry_id: i64, episode_ids: &[i64], start_now: bool) -> bool {
        let (Some(downloads), Some((entry, episodes, info))) = (&self.downloads, self.entry_episodes(entry_id, Some(episode_ids))) else { return false };
        downloads.enqueue(&entry, &episodes, &info, start_now)
    }

    fn delete_items(&self, entry_id: i64, episode_ids: &[i64]) {
        let (Some(downloads), Some((entry, episodes, info))) = (&self.downloads, self.entry_episodes(entry_id, Some(episode_ids))) else { return };
        let categories = self.episodes.category_ids(entry_id).unwrap_or_default();
        downloads.delete(&entry, &episodes, categories, &info);
    }

    fn delete_downloads(&self, entry_id: i64) {
        let on_disk: Vec<i64> = self.downloaded_items(entry_id).into_iter().collect();
        self.delete_items(entry_id, &on_disk);
    }
}

fn domain_error(e: DomainError) -> DetailError {
    DetailError::Other { class: "SQLiteException".to_owned(), message: Some(e.to_string()) }
}

/// The page's error for a failed remote update of the source called `source`.
fn detail_error(e: RemoteError, source: &str) -> DetailError {
    match e {
        RemoteError::Source(error) => DetailError::from_source(source, error),
        RemoteError::Cancelled => DetailError::Cancelled,
        RemoteError::Db(e) => domain_error(e),
        RemoteError::Fetch(message) => DetailError::Other { class: "Exception".to_owned(), message: Some(message) },
        RemoteError::NoEpisodes(_) => DetailError::NoEpisodes,
        RemoteError::NoSeasons(_) => DetailError::NoSeasons,
    }
}

#[cfg(test)]
use kuro_source::anime::registry::builtin_anime_sources;

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use kuro_source::anime::{SAnimeEpisodeUpdate, SAnimeSeasonUpdate};

    use super::super::store::fixture::with_episodes;
    use super::*;

    /// A source that answers with canned data, records the `(fetch_details, fetch_list)` flags of each call and can
    /// fail with an HTTP status.
    struct FakeSource {
        entry: SAnime,
        episodes: Vec<SEpisode>,
        seasons: Vec<SAnime>,
        http_failure: Option<u16>,
        asked: Mutex<Vec<(bool, bool)>>,
    }

    impl FakeSource {
        fn new(episodes: Vec<SEpisode>) -> Arc<Self> {
            Arc::new(Self { entry: SAnime { title: "Frieren".into(), ..SAnime::default() }, episodes, seasons: Vec::new(), http_failure: None, asked: Mutex::default() })
        }
    }

    #[async_trait]
    impl AnimeRemote for FakeSource {
        async fn episode_update(&self, _: SAnime, _: Vec<SEpisode>, fetch_details: bool, fetch_episodes: bool) -> Result<SAnimeEpisodeUpdate, SourceError> {
            self.asked.lock().unwrap().push((fetch_details, fetch_episodes));
            if let Some(code) = self.http_failure {
                return Err(SourceError::Http(code));
            }
            Ok(SAnimeEpisodeUpdate { anime: self.entry.clone(), episodes: self.episodes.clone() })
        }

        async fn season_update(&self, _: SAnime, _: Vec<SAnime>, _: bool, _: bool) -> Result<SAnimeSeasonUpdate, SourceError> {
            if self.seasons.is_empty() {
                return Err(SourceError::Unsupported("seasons"));
            }
            Ok(SAnimeSeasonUpdate { anime: self.entry.clone(), seasons: self.seasons.clone() })
        }
    }

    struct One(Arc<FakeSource>);

    impl AnimeSourceLookup for One {
        fn remote(&self, _: i64) -> Option<Arc<dyn AnimeRemote>> {
            Some(self.0.clone())
        }
    }

    fn s_episode(url: &str, number: f32) -> SEpisode {
        SEpisode { url: url.into(), name: format!("Episode {number}"), episode_number: number, ..SEpisode::default() }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    }

    fn backend(db: Arc<Db>, source: Arc<FakeSource>, handle: Handle) -> RemoteAnimeBackend {
        RemoteAnimeBackend::new(db, &PrefStore::in_memory(), Arc::new(One(source)), handle)
    }

    const PLAN: RefreshPlan = RefreshPlan { info: true, episodes: true, seasons: false };

    #[test]
    fn a_fetch_stores_what_the_source_lists_and_returns_the_new_episodes() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, false, 1, 0);
        let backend = backend(store.db(), FakeSource::new(vec![s_episode("/e1", 1.0), s_episode("/e2", 2.0)]), rt.handle().clone());
        let new = rt.block_on(backend.fetch(id, PLAN, Trigger::Manual)).expect("the fetch works");
        assert_eq!(new.iter().map(|e| e.number).collect::<Vec<_>>(), [2.0], "the one that was not stored before");
        assert_eq!(store.episodes(id).unwrap().len(), 2);
        assert!(store.entry(id).unwrap().unwrap().initialized, "the metadata write marks it initialized");
    }

    #[test]
    fn the_plan_reaches_the_source_call() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let source = FakeSource::new(vec![s_episode("/e1", 1.0)]);
        let backend = backend(store.db(), source.clone(), rt.handle().clone());
        let plan = RefreshPlan { info: false, episodes: true, seasons: false };
        rt.block_on(backend.fetch(id, plan, Trigger::Open)).unwrap();
        assert_eq!(*source.asked.lock().unwrap(), vec![(false, true)], "details are not asked for");
    }

    fn seasons_source() -> Arc<FakeSource> {
        let seasons = vec![
            SAnime { url: "/s1".into(), title: "Frieren Season 1".into(), ..SAnime::default() },
            SAnime { url: "/s2".into(), title: "Frieren Season 2".into(), ..SAnime::default() },
        ];
        Arc::new(FakeSource { seasons, ..Arc::into_inner(FakeSource::new(vec![s_episode("/e1", 1.0)])).unwrap() })
    }

    /// Refreshes a parent that lists seasons from `source`: the temp dir to keep alive, the ids of the stored seasons and the store.
    fn season_refresh(rt: &tokio::runtime::Runtime, source: &Arc<FakeSource>) -> (tempfile::TempDir, Vec<i64>, EntryStore) {
        let (dir, store, id) = with_episodes(true, true, 0, 0);
        store.set_fetch_type_for_test(id, FetchType::Seasons);
        let prefs = PrefStore::in_memory();
        let backend = RemoteAnimeBackend::new(store.db(), &prefs, Arc::new(One(source.clone())), rt.handle().clone());
        let plan = RefreshPlan { info: true, episodes: false, seasons: true };
        rt.block_on(backend.fetch(id, plan, Trigger::Manual)).expect("the refresh works");
        let ids = kuro_data::entry::EntryRepo::new(store.db()).seasons_by_parent(id).unwrap().into_iter().map(|s| s.entry.id).collect();
        (dir, ids, store)
    }

    #[test]
    fn a_seasons_refresh_leaves_the_episodes_alone_because_the_season_setting_is_cut() {
        let rt = rt();
        let source = seasons_source();
        let (_dir, ids, store) = season_refresh(&rt, &source);
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| store.episodes(*id).unwrap().is_empty()));
        assert!(source.asked.lock().unwrap().is_empty());
    }

    /// A live source that panics while it reads the details: the refresh fails with an error (the page's snackbar and
    /// Retry), it does not kill the refresh task.
    #[test]
    fn a_panicking_live_source_fails_the_refresh_instead_of_killing_it() {
        struct Panics;
        #[async_trait]
        impl AnimeSource for Panics {
            fn id(&self) -> i64 {
                7
            }
            fn name(&self) -> &str {
                "Panics"
            }
            fn supports_latest(&self) -> bool {
                false
            }
            async fn popular(&self, _: i32) -> Result<kuro_source::anime::AnimesPage, SourceError> {
                unreachable!()
            }
            async fn latest(&self, _: i32) -> Result<kuro_source::anime::AnimesPage, SourceError> {
                unreachable!()
            }
            async fn search(&self, _: i32, _: &str, _: &kuro_source::filter_list::FilterList) -> Result<kuro_source::anime::AnimesPage, SourceError> {
                unreachable!()
            }
            async fn anime_details(&self, _: &SAnime) -> Result<SAnime, SourceError> {
                panic!("a parser bug")
            }
            fn provides_hosters(&self) -> bool {
                false
            }
        }
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let backend = RemoteAnimeBackend::new(store.db(), &PrefStore::in_memory(), Arc::new(LiveSources::new(vec![Arc::new(Panics)])), rt.handle().clone());
        let result = rt.block_on(backend.fetch(id, PLAN, Trigger::Manual));
        assert!(matches!(&result, Err(DetailError::Plain(text)) if text.starts_with("Panics ") && text.contains("isn't working right now")), "{result:?}");
    }

    #[test]
    fn the_live_sources_know_only_their_own_ids() {
        let lookup = LiveSources::new(Vec::new());
        assert!(lookup.remote(7).is_none());
        assert!(!lookup.is_configurable(7) && !lookup.is_unmetered(7));
        assert!(LiveSources::builtin().info(7).is_none(), "no builtin source has this id");
    }

    #[test]
    fn an_error_of_each_kind_has_a_page_error() {
        assert_eq!(detail_error(RemoteError::Source(SourceError::NotInstalled(1)), "S"), DetailError::SourceNotInstalled);
        assert_eq!(detail_error(RemoteError::Cancelled, "S"), DetailError::Cancelled);
        assert!(matches!(detail_error(RemoteError::Source(SourceError::UnknownHost { host: "h".into() }), "S"), DetailError::Plain(_)));
        assert!(matches!(detail_error(RemoteError::Fetch("x".into()), "S"), DetailError::Other { .. }));
    }
}
