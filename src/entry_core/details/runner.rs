//! The anime store runner: one tokio task consumes `EntryCmd`, every result goes back as an `EntryEvent` through an
//! `async-channel`, and the GTK side drains it. `Shutdown` (the page was popped) aborts the live subscriptions and
//! cancels the page's `CancellationToken`, which ends a refresh that is still fetching. Write commands run in their
//! own task, on the blocking pool, and never look at the token, so a pop does not cancel them. No GTK here.
//! Upstream: `AnimeScreenModel.kt:204-283` (init), `fetchAllFromSource`.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use async_channel::{Receiver, Sender};
use kuro_data::DataResult;
use kuro_data::anime_relation::AnimeRelationGroup;
use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_data::source::StubSourceRepo;
use kuro_db::handle::Db;
use kuro_db::codec::FetchType;
use kuro_platform::kind::Kind;
use kuro_prefs::library::LibraryPrefs;
use kuro_prefs::store::PrefStore;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use super::actions::Mark;
use super::details_error::{DetailsError, join_failed};
use super::entry_state::RefreshPlan;
use super::model::{EntryModel, EntryLoadContext};
use crate::entry_core::refresh_rules::{RefreshGate, Trigger, last_empty_open_fetch, record_empty_open_fetch};
use super::season_tile::SeasonExtra;
use super::set_default::Scope;
use super::store::EntryStore;
use crate::entry_core::backend::stub_source_info;
use crate::entry_core::source_info::SourceInfo;
use crate::entry_core::errors::DetailError;
use crate::entry_core::library_flow::{self, Hooks, RemoveOutcome, Toggled};
use crate::entry_core::runner_kit::{RunnerHandle, abort_all, watch_pair};

/// What a fetch hands back: the episodes the sync added, which a manual refresh queues for download (T-REF-4).
pub type EntryFetch = Pin<Box<dyn Future<Output = Result<Vec<Item>, DetailError>> + Send>>;

/// What the page needs from outside its own database: the source line and the source fetch.
pub trait EntryBackend: Send + Sync + 'static {
    /// The source line of the info box. `stub` is true when the extension is not installed.
    fn source_info(&self, source_id: i64) -> SourceInfo;
    /// `fetchAllFromSource`: fetches what `plan` names and stores it, so the page sees it through the subscriptions.
    /// `trigger` says whether the user asked (`manualFetch`). It returns the episodes the sync added.
    fn fetch(&self, entry_id: i64, plan: RefreshPlan, trigger: Trigger) -> EntryFetch;
    /// Queues new episodes for download. Only a manual refresh reaches this (E-REF-5). The app has no anime download
    /// queue wired yet, so the default does nothing.
    fn enqueue_new_items(&self, _anime_id: i64, _episodes: &[Item]) {}
    /// Binds the enhanced trackers of an entry that just joined the library (`bindEnhancedTrackers`).
    fn bind_enhanced(&self, _anime_id: i64) {}
    /// Deletes the cover files of an entry that just left the library (`removeCovers`), after the write succeeded.
    /// `true` when a file was removed.
    fn remove_cover(&self, _anime_id: i64) -> bool {
        false
    }
    /// How many downloaded episodes the entry has.
    fn downloaded_count(&self, _anime_id: i64) -> usize {
        0
    }
    /// Deletes every download of the entry.
    fn delete_downloads(&self, _anime_id: i64) {}
    /// Whether the app can queue and delete episode downloads (`onDownloadEpisode != null`): the Download menu and the
    /// download rows of the selection bar show only then.
    fn has_download_handler(&self) -> bool {
        false
    }
    /// The ids of the entry's episodes that are on disk.
    fn downloaded_items(&self, _anime_id: i64) -> HashSet<i64> {
        HashSet::new()
    }
    /// Queues these episodes. With `start_now` the one episode goes to the front and the queue starts. `false` when
    /// the downloads layer is not there: nothing was queued.
    fn enqueue_items(&self, _anime_id: i64, _episode_ids: &[i64], _start_now: bool) -> bool {
        false
    }
    /// Deletes the downloads of these episodes (the Delete row, and a mark seen with "remove after seen").
    fn delete_items(&self, _anime_id: i64, _episode_ids: &[i64]) {}
    /// Whether a tracker is logged in: the tracking button opens the tracking sheet then, the tracking settings
    /// otherwise (E-25). The default is no; the app's backends read the published tracker session.
    fn has_logged_in_tracker(&self) -> bool {
        false
    }
    /// The address of the source's site, when it has one: the entry urls of such a source are paths on it.
    fn home_url(&self, _source_id: i64) -> Option<String> {
        None
    }
    /// Whether the source is an HTTP source: only then does the action row offer WebView, Share and Open in browser (E-26).
    fn is_http_source(&self, source_id: i64) -> bool {
        self.home_url(source_id).is_some()
    }
    /// The web address of an entry of the source: its url when that is one, else the url joined to the site's address.
    fn web_url(&self, source_id: i64, url: &str) -> Option<String> {
        crate::browse::nav::entry_web_url(self.home_url(source_id).as_deref(), url)
    }
    /// Whether the source is configurable: only then does the overflow menu offer Settings (T-39a-057).
    fn is_configurable_source(&self, _source_id: i64) -> bool {
        false
    }
    /// Plays an episode in an external player. `false` when it cannot (the app resolves no video URL yet): the page
    /// then opens the internal player.
    fn play_external(&self, _anime_id: i64, _episode_id: i64) -> bool {
        false
    }
    /// The language code of a source (`source.lang`), for the language badge of a season. Empty when it is not known.
    fn source_language(&self, _source_id: i64) -> String {
        String::new()
    }
}

/// The backend of an app with no anime source registry yet: every source reads as not installed, the local source
/// reads as installed, and every fetch reports that the source is missing.
pub struct StubEntryBackend {
    stubs: StubSourceRepo,
}

impl StubEntryBackend {
    pub fn new(db: Arc<Db>) -> Self {
        Self {
            stubs: StubSourceRepo::new(db, Kind::Anime),
        }
    }
}

impl EntryBackend for StubEntryBackend {
    fn has_logged_in_tracker(&self) -> bool {
        crate::entry_core::tracking::session::has_logged_in()
    }
    fn source_info(&self, source_id: i64) -> SourceInfo {
        stub_source_info(&self.stubs, Kind::Anime, source_id)
    }

    fn fetch(&self, _anime_id: i64, _plan: RefreshPlan, _trigger: Trigger) -> EntryFetch {
        Box::pin(async { Err(DetailError::SourceNotInstalled) })
    }

    fn source_language(&self, source_id: i64) -> String {
        self.stubs.get(source_id).ok().flatten().map(|stub| stub.lang).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryCmd {
    Load { entry_id: i64 },
    /// A manual refresh: every part of the entry is fetched again, and the new episodes may auto-download.
    Refresh { entry_id: i64 },
    SetFlagBits { entry_id: i64, flag: i64, mask: i64 },
    /// The scanlator sheet's OK: the excluded set becomes exactly this.
    SetExcludedScanlators { entry_id: i64, excluded: std::collections::BTreeSet<String> },
    /// The skip-intro length in seconds; 0 turns the skip off (`setSkipIntroLength`).
    SetSkipIntroLength { entry_id: i64, length: i32 },
    /// A masked write of the season flag word (a pick of the season list options).
    SetSeasonFlagBits { entry_id: i64, flag: i64, mask: i64 },
    /// "Use as default for all titles": the default of this scope only goes to every favorite. Answered by
    /// `EntryEvent::DefaultsApplied`.
    ApplyDefaultsToLibrary { scope: Scope },
    /// The library button: remove an entry that is in the library, or add one that is not. With `check_duplicates` a
    /// library entry with the same title stops the add and the page is asked what to do; Add anyway sends it false.
    ToggleLibrary { entry_id: i64, check_duplicates: bool },
    /// Sets or clears the bookmark or fillermark of exactly these episodes (`actions::bulk_mark_targets`).
    SetMark { ids: Vec<i64>, mark: Mark, on: bool },
    /// Marks these episodes seen or unseen.
    SetSeen { ids: Vec<i64>, seen: bool },
    /// `setFetchInterval`: the wheel value; the stored value is its negative (0 is automatic).
    SetFetchInterval { entry_id: i64, wheel: i32 },
    /// Deletes every download of the entry (the Delete button of the prompt).
    DeleteDownloads { entry_id: i64 },
    /// Queues these episodes for download. With `start_now` the one episode goes to the front.
    Download { entry_id: i64, ids: Vec<i64>, start_now: bool },
    /// Deletes the downloads of these episodes.
    DeleteEpisodeDownloads { entry_id: i64, ids: Vec<i64> },
    /// Reads the categories and the entry's links, for the category editor: answered by `EntryEvent::Categories`.
    LoadCategories { entry_id: i64 },
    /// Reads the tracking count again, after the tracking page bound, edited or removed a tracker. Answered by
    /// `EntryEvent::Tracking`.
    ReadTracking { entry_id: i64, source_id: i64 },
    /// The category editor's OK: replaces the entry's category links.
    SetCategories { entry_id: i64, category_ids: Vec<i64> },
    /// The answer of the add dialog: the entry joins the library in these categories.
    AddWithCategories { entry_id: i64, category_ids: Vec<i64> },
    Shutdown,
}

#[derive(Debug)]
pub enum EntryEvent {
    /// The load finished: the model, or `None` when the row does not exist.
    Loaded(Option<Box<EntryModel>>),
    /// The live subscription published a newer entry and episode list.
    Snapshot { entry: Box<Entry>, episodes: Vec<Item> },
    /// The refresh started by the first open ended, with or without an error.
    RefreshEnded(Result<(), DetailError>),
    /// The entry joined the library, with the library defaults written to its flags.
    Added,
    /// The entry left the library. A failed write sends nothing (E-LIB-9).
    Removed(RemoveOutcome),
    /// A store read or write failed. The text is what the user sees (`DetailsError::user_message`); the detail is logged.
    Failed(String),
    /// The related groups changed (or were read for the first time).
    Related(Vec<AnimeRelationGroup>),
    /// The available scanlators or the excluded ones changed.
    Scanlators(super::store::Scanlators),
    /// The result of `ApplyDefaultsToLibrary`: how many entries took the defaults.
    DefaultsApplied { scope: Scope, updated: usize },
    /// The entry being added has the same title as these library entries.
    Duplicate { existing: Vec<Entry> },
    /// The download count and source language of each season, for the tiles of a `Seasons` entry.
    SeasonFacts(Vec<SeasonExtra>),
    /// The ids of the episodes on disk changed (a download finished or a delete ran).
    Downloads(HashSet<i64>),
    /// The answer of `LoadCategories`: every category and the ids the entry is linked to.
    Categories { all: Vec<(i64, String)>, linked: Vec<i64> },
    /// The library button of an add with no default category: the category dialog asks which ones (T-39b-026).
    AddCategories { all: Vec<(i64, String)>, linked: Vec<i64> },
    /// The tracking count, read again (`EntryCmd::ReadTracking`).
    Tracking(usize),
}

/// The page's handle on the runner. Dropping it stops the runner and the subscriptions.
pub type EntryRunner = RunnerHandle<EntryCmd, EntryEvent>;

struct Shared {
    store: EntryStore,
    prefs: PrefStore,
    backend: Arc<dyn EntryBackend>,
    ctx: EntryLoadContext,
    events: Sender<EntryEvent>,
    cancel: CancellationToken,
    subscriptions: Mutex<Vec<AbortHandle>>,
    gate: RefreshGate,
}

/// Starts the runner on `handle`.
pub fn spawn(
    handle: &Handle,
    store: EntryStore,
    prefs: PrefStore,
    backend: Arc<dyn EntryBackend>,
    ctx: EntryLoadContext,
) -> EntryRunner {
    let (runner, cmd_rx, event_tx, cancel) = EntryRunner::new(|| EntryCmd::Shutdown);
    let shared = Arc::new(Shared {
        store,
        prefs,
        backend,
        ctx,
        events: event_tx,
        cancel,
        subscriptions: Mutex::new(Vec::new()),
        gate: RefreshGate::default(),
    });
    handle.spawn(run(shared, cmd_rx));
    runner
}

async fn run(shared: Arc<Shared>, cmds: Receiver<EntryCmd>) {
    while let Ok(cmd) = cmds.recv().await {
        match cmd {
            EntryCmd::Shutdown => {
                shared.cancel.cancel();
                break;
            }
            EntryCmd::Load { entry_id } => shared.clone().load(entry_id).await,
            EntryCmd::Refresh { entry_id } => {
                let all = RefreshPlan { info: true, episodes: true, seasons: true };
                tokio::spawn(shared.clone().refresh(entry_id, all, Trigger::Manual));
            }
            write => {
                tokio::spawn(shared.clone().write(write));
            }
        }
    }
    shared.drop_subscriptions();
}

/// Unix time in milliseconds, for the empty-list guard of the open refresh.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Shared {
    fn emit(&self, event: EntryEvent) {
        let _ = self.events.try_send(event);
    }

    fn drop_subscriptions(&self) {
        abort_all(&self.subscriptions);
    }

    /// Tells the page a store read or write failed: the detail goes to the log, the user message to the page.
    fn failed(&self, error: impl Into<DetailsError>) {
        self.emit(EntryEvent::Failed(error.into().record().user_message()));
    }

    /// Runs blocking store work on the blocking pool. The work keeps running if the awaiting task is aborted. A task
    /// that did not finish is a typed error, not a panic.
    async fn blocking<T: Send + 'static>(&self, work: impl FnOnce(EntryStore) -> DataResult<T> + Send + 'static) -> Result<T, DetailsError> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || work(store)).await {
            Ok(result) => result.map_err(DetailsError::from),
            Err(e) => Err(join_failed(&e)),
        }
    }

    /// The load sequence (`EntryModel::load`), then the live subscription, then the first-open refresh.
    async fn load(self: Arc<Self>, entry_id: i64) {
        let ctx = EntryLoadContext {
            last_empty_open_fetch_ms: last_empty_open_fetch(self.store.kind(), entry_id),
            now_ms: now_ms(),
            ..self.ctx
        };
        let (prefs, backend) = (self.prefs.clone(), Arc::clone(&self.backend));
        let loaded = self
            .blocking(move |store| -> DataResult<Option<EntryModel>> {
                let mut model = EntryModel::load(&store, &prefs, entry_id, ctx, |source| backend.source_info(source))?;
                if let Some(m) = &mut model {
                    m.downloaded = backend.downloaded_items(entry_id);
                }
                Ok(model)
            })
            .await;
        match loaded {
            Ok(Some(model)) => {
                let plan = model.plan;
                if plan.episodes && model.episodes.is_empty() {
                    record_empty_open_fetch(self.store.kind(), entry_id, ctx.now_ms);
                }
                let seasons = model.entry.fetch_type() == FetchType::Seasons;
                self.emit(EntryEvent::Loaded(Some(Box::new(model))));
                self.subscribe(entry_id);
                if seasons {
                    self.emit_season_facts(entry_id).await;
                }
                if plan.any() {
                    tokio::spawn(self.clone().refresh(entry_id, plan, Trigger::Open));
                }
            }
            Ok(None) => self.emit(EntryEvent::Loaded(None)),
            Err(e) => {
                self.failed(e);
                self.emit(EntryEvent::Loaded(None));
            }
        }
    }

    /// Step 10 and 11 of the init block, or a manual refresh. Single-flight per entry: a request that arrives while one
    /// runs joins it and fetches nothing (E-CONC-1), and the running one reports the end. A cancelled refresh ends
    /// silently: nobody is left to read its event.
    async fn refresh(self: Arc<Self>, entry_id: i64, plan: RefreshPlan, trigger: Trigger) {
        let Some(running) = self.gate.begin(entry_id) else { return };
        let fetched = tokio::select! {
            () = self.cancel.cancelled() => return,
            result = self.backend.fetch(entry_id, plan, trigger) => result,
        };
        drop(running);
        let result = match fetched {
            Ok(new_episodes) => {
                self.auto_download(entry_id, trigger, new_episodes).await;
                Ok(())
            }
            Err(e) => Err(e),
        };
        let refreshed = result.is_ok();
        self.emit(EntryEvent::RefreshEnded(result));
        // a refresh of a `Seasons` entry may have added seasons
        if refreshed {
            self.emit_season_facts(entry_id).await;
        }
    }

    /// The download count and source language of each season of the entry, read off the GTK thread. Nothing is sent
    /// for an entry with no seasons.
    async fn emit_season_facts(&self, entry_id: i64) {
        let backend = Arc::clone(&self.backend);
        let facts = self
            .blocking(move |store| {
                Ok(store
                    .seasons(entry_id)?
                    .iter()
                    .map(|season| SeasonExtra {
                        id: season.entry.id,
                        download_count: i64::try_from(backend.downloaded_count(season.entry.id)).unwrap_or(i64::MAX),
                        source_language: backend.source_language(season.entry.source),
                    })
                    .collect::<Vec<_>>())
            })
            .await;
        match facts {
            Ok(facts) if !facts.is_empty() => self.emit(EntryEvent::SeasonFacts(facts)),
            Ok(_) => {}
            Err(e) => self.failed(e),
        }
    }

    /// Queues the new episodes of a manual refresh when the preferences and the entry's categories allow it (T-REF-4).
    /// The open-time refresh never gets here with episodes to queue (E-REF-5).
    async fn auto_download(&self, entry_id: i64, trigger: Trigger, new_episodes: Vec<Item>) {
        if new_episodes.is_empty() {
            return;
        }
        let prefs = self.prefs.clone();
        let targets = self.blocking(move |store| store.new_item_targets(&prefs, entry_id, trigger, &new_episodes)).await;
        match targets {
            Ok(targets) if !targets.is_empty() => {
                let backend = Arc::clone(&self.backend);
                let _ = tokio::task::spawn_blocking(move || backend.enqueue_new_items(entry_id, &targets)).await;
            }
            Ok(_) => {}
            Err(e) => self.failed(e),
        }
    }

    /// The library button (`library_flow`). A favorite is removed: the write first, then the cover and the download count.
    /// An entry that is not one is checked for a same-named library entry (with `check_duplicates`, T-39a-047), routed by
    /// the default category (T-39b-026), and added with the library defaults written to its flags (T-39b-012). A failed
    /// write does nothing else (E-LIB-9).
    async fn toggle_library(&self, entry_id: i64, check_duplicates: bool) {
        let prefs = self.prefs.clone();
        let default_category = i64::from(LibraryPrefs::new(prefs.clone()).default_category(self.store.kind()).get());
        let (backend, now) = (Arc::clone(&self.backend), now_ms());
        let toggled = self
            .blocking(move |store| {
                Self::with_hooks(&backend, entry_id, |hooks| {
                    library_flow::toggle(&store.library(&prefs), hooks, entry_id, default_category, check_duplicates, now)
                })
            })
            .await;
        match toggled {
            Ok(Toggled::Nothing) => {}
            Ok(Toggled::Removed(outcome)) => self.emit(EntryEvent::Removed(outcome)),
            Ok(Toggled::Duplicate(existing)) => self.emit(EntryEvent::Duplicate { existing }),
            Ok(Toggled::ChooseCategory { all, linked }) => self.emit(EntryEvent::AddCategories { all, linked }),
            Ok(Toggled::Added) => self.emit(EntryEvent::Added),
            Err(e) => self.failed(e),
        }
    }

    /// Adds the entry in these categories: the links and the favorite in order (`moveAnimeToCategoriesAndAddToLibrary`).
    async fn add_with_categories(&self, entry_id: i64, category_ids: Vec<i64>) {
        let (prefs, backend, now) = (self.prefs.clone(), Arc::clone(&self.backend), now_ms());
        let added = self
            .blocking(move |store| {
                Self::with_hooks(&backend, entry_id, |hooks| library_flow::add(&store.library(&prefs), hooks, entry_id, &category_ids, now))
            })
            .await;
        match added {
            Ok(true) => self.emit(EntryEvent::Added),
            Ok(false) => {}
            Err(e) => self.failed(e),
        }
    }

    /// The library button's backend hooks for one entry (`library_flow::Hooks`).
    fn with_hooks<T>(backend: &Arc<dyn EntryBackend>, entry_id: i64, f: impl FnOnce(&Hooks<'_>) -> T) -> T {
        let remove_cover = || backend.remove_cover(entry_id);
        let downloaded_count = || backend.downloaded_count(entry_id);
        let bind_enhanced = || backend.bind_enhanced(entry_id);
        f(&Hooks { remove_cover: &remove_cover, downloaded_count: &downloaded_count, bind_enhanced: &bind_enhanced })
    }

    /// Reads the episodes on disk again and tells the page.
    async fn emit_downloads(&self, entry_id: i64) {
        let backend = Arc::clone(&self.backend);
        if let Ok(on_disk) = tokio::task::spawn_blocking(move || backend.downloaded_items(entry_id)).await {
            self.emit(EntryEvent::Downloads(on_disk));
        }
    }

    /// The entry and its episodes as one snapshot (`watch_pair`), and the related groups.
    fn subscribe(self: &Arc<Self>, entry_id: i64) {
        let shared = Arc::clone(self);
        let pair = watch_pair(self.store.watch_anime(entry_id), self.store.watch_episodes(entry_id), move |entry: &Entry, episodes: &[Item]| {
            shared.emit(EntryEvent::Snapshot { entry: Box::new(entry.clone()), episodes: episodes.to_vec() });
        });
        let mut related_sub = self.store.watch_related(entry_id);
        let shared = Arc::clone(self);
        let related_task = tokio::spawn(async move {
            while let Some(published) = related_sub.changed().await {
                if let Ok(groups) = published {
                    shared.emit(EntryEvent::Related(groups.as_ref().clone()));
                }
            }
        });
        let mut subs = self.subscriptions.lock().unwrap_or_else(PoisonError::into_inner);
        subs.extend(pair);
        subs.push(related_task.abort_handle());
        if self.store.kind().is_manga() {
            subs.extend(self.watch_scanlators(entry_id));
        }
    }

    /// The available and the excluded scanlators have their own tables: an exclusion or a new chapter source changes
    /// them, and the sheet and the filter tint follow. Published once both have arrived.
    fn watch_scanlators(self: &Arc<Self>, entry_id: i64) -> [tokio::task::AbortHandle; 2] {
        let sets: Arc<Mutex<(Option<Vec<String>>, Option<std::collections::BTreeSet<String>>)>> = Arc::default();
        let publish = {
            let (shared, sets) = (Arc::clone(self), Arc::clone(&sets));
            move || {
                if let (Some(available), Some(excluded)) = &*sets.lock().unwrap_or_else(PoisonError::into_inner) {
                    shared.emit(EntryEvent::Scanlators(super::store::Scanlators { available: available.clone(), excluded: excluded.clone() }));
                }
            }
        };
        let (mut available_sub, mut excluded_sub) = (self.store.watch_available(entry_id), self.store.watch_excluded(entry_id));
        let available = tokio::spawn({
            let (sets, publish) = (Arc::clone(&sets), publish.clone());
            async move {
                while let Some(Ok(set)) = available_sub.changed().await {
                    sets.lock().unwrap_or_else(PoisonError::into_inner).0 = Some(set.iter().cloned().collect());
                    publish();
                }
            }
        });
        let excluded = tokio::spawn(async move {
            while let Some(Ok(set)) = excluded_sub.changed().await {
                sets.lock().unwrap_or_else(PoisonError::into_inner).1 = Some(set.as_ref().clone());
                publish();
            }
        });
        [available.abort_handle(), excluded.abort_handle()]
    }

    /// One write command. Runs in its own task.
    async fn write(self: Arc<Self>, cmd: EntryCmd) {
        let result = match cmd {
            EntryCmd::SetFlagBits { entry_id, flag, mask } => {
                self.blocking(move |s| s.set_flag_bits(entry_id, flag, mask)).await
            }
            EntryCmd::SetExcludedScanlators { entry_id, excluded } => self.blocking(move |s| s.set_excluded_scanlators(entry_id, &excluded).map(|()| true)).await,
            EntryCmd::SetSkipIntroLength { entry_id, length } => {
                self.blocking(move |s| s.set_skip_intro_length(entry_id, length)).await
            }
            EntryCmd::SetSeasonFlagBits { entry_id, flag, mask } => {
                self.blocking(move |s| s.set_season_flag_bits(entry_id, flag, mask)).await
            }
            EntryCmd::ApplyDefaultsToLibrary { scope } => {
                let prefs = self.prefs.clone();
                let applied = self
                    .blocking(move |s| match scope {
                        Scope::Episode => s.apply_episode_defaults_to_library(&prefs),
                        Scope::Season => s.apply_season_defaults_to_library(&prefs),
                    })
                    .await;
                match applied {
                    Ok(updated) => return self.emit(EntryEvent::DefaultsApplied { scope, updated }),
                    Err(e) => return self.failed(e),
                }
            }
            EntryCmd::AddWithCategories { entry_id, category_ids } => return self.add_with_categories(entry_id, category_ids).await,
            EntryCmd::ToggleLibrary { entry_id, check_duplicates } => return self.toggle_library(entry_id, check_duplicates).await,
            EntryCmd::SetMark { ids, mark, on } => self.blocking(move |s| s.set_marks(&ids, mark, on).map(|()| true)).await,
            EntryCmd::SetSeen { ids, seen } => self.blocking(move |s| s.set_seen(&ids, seen).map(|()| true)).await,
            EntryCmd::SetFetchInterval { entry_id, wheel } => {
                self.blocking(move |s| s.set_fetch_interval(entry_id, wheel).map(|()| true)).await
            }
            EntryCmd::DeleteDownloads { entry_id } => {
                let backend = Arc::clone(&self.backend);
                let _ = tokio::task::spawn_blocking(move || backend.delete_downloads(entry_id)).await;
                return self.emit_downloads(entry_id).await;
            }
            EntryCmd::Download { entry_id, ids, start_now } => {
                let backend = Arc::clone(&self.backend);
                let _ = tokio::task::spawn_blocking(move || backend.enqueue_items(entry_id, &ids, start_now)).await;
                return self.emit_downloads(entry_id).await;
            }
            EntryCmd::DeleteEpisodeDownloads { entry_id, ids } => {
                let backend = Arc::clone(&self.backend);
                let _ = tokio::task::spawn_blocking(move || backend.delete_items(entry_id, &ids)).await;
                return self.emit_downloads(entry_id).await;
            }
            EntryCmd::LoadCategories { entry_id } => match self.blocking(move |s| s.categories(entry_id)).await {
                Ok((all, linked)) => return self.emit(EntryEvent::Categories { all, linked }),
                Err(e) => Err(e),
            },
            EntryCmd::SetCategories { entry_id, category_ids } => {
                self.blocking(move |s| s.set_categories(entry_id, &category_ids).map(|()| true)).await
            }
            EntryCmd::ReadTracking { entry_id, source_id } => match self.blocking(move |s| s.track_summary(entry_id, source_id)).await {
                Ok(summary) => return self.emit(EntryEvent::Tracking(summary.count as usize)),
                Err(e) => Err(e),
            },
            EntryCmd::Load { .. } | EntryCmd::Refresh { .. } | EntryCmd::Shutdown => return,
        };
        if let Err(e) = result {
            self.failed(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use kuro_db::flags::item as ep;

    use super::super::entry_state::LoadState;
    use super::super::store::fixture::with_episodes;
    use super::*;

    /// A backend that counts fetches and holds them open. A cancelled fetch is dropped before it counts as finished.
    struct Fake {
        hold_ms: u64,
        started: AtomicUsize,
        finished: Arc<AtomicUsize>,
        /// The episodes every fetch reports as new.
        new_episodes: Vec<Item>,
        triggers: Mutex<Vec<Trigger>>,
        queued: Mutex<Vec<Vec<f64>>>,
        covers: AtomicUsize,
        downloads: usize,
        deleted: AtomicUsize,
    }

    impl Fake {
        fn new(hold_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                hold_ms,
                started: AtomicUsize::new(0),
                finished: Arc::default(),
                new_episodes: Vec::new(),
                triggers: Mutex::default(),
                queued: Mutex::default(),
                covers: AtomicUsize::new(0),
                downloads: 0,
                deleted: AtomicUsize::new(0),
            })
        }
    }

    impl EntryBackend for Fake {
        fn source_info(&self, _: i64) -> SourceInfo {
            SourceInfo { name: "Fake".into(), stub: false }
        }

        fn fetch(&self, _: i64, _: RefreshPlan, trigger: Trigger) -> EntryFetch {
            self.started.fetch_add(1, Ordering::SeqCst);
            self.triggers.lock().unwrap().push(trigger);
            let (hold, finished, new) = (self.hold_ms, Arc::clone(&self.finished), self.new_episodes.clone());
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(hold)).await;
                finished.fetch_add(1, Ordering::SeqCst);
                Ok(new)
            })
        }

        fn enqueue_new_items(&self, _: i64, episodes: &[Item]) {
            self.queued.lock().unwrap().push(episodes.iter().map(|e| e.number).collect());
        }

        fn remove_cover(&self, _: i64) -> bool {
            self.covers.fetch_add(1, Ordering::SeqCst);
            true
        }

        fn downloaded_count(&self, _: i64) -> usize {
            self.downloads
        }

        fn delete_downloads(&self, _: i64) {
            self.deleted.fetch_add(1, Ordering::SeqCst);
        }

        fn source_language(&self, _: i64) -> String {
            "ja".to_owned()
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    }

    async fn until(rx: &Receiver<EntryEvent>, mut pick: impl FnMut(&EntryEvent) -> bool) -> EntryEvent {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("an event in time").expect("open");
            if pick(&event) {
                return event;
            }
        }
    }

    fn start(rt: &tokio::runtime::Runtime, store: EntryStore, backend: Arc<Fake>) -> EntryRunner {
        spawn(rt.handle(), store, PrefStore::in_memory(), backend, EntryLoadContext::default())
    }

    #[test]
    fn a_fixture_entry_loads_to_success_as_the_first_event() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 3, 0);
        rt.block_on(async {
            let runner = start(&rt, store, Fake::new(0));
            runner.send(EntryCmd::Load { entry_id: id });
            let rx = runner.events();
            let EntryEvent::Loaded(Some(model)) = rx.recv().await.unwrap() else { panic!("the model first") };
            assert_eq!(model.flow.load, LoadState::Success);
            assert_eq!(model.episodes.len(), 3);
            assert_eq!(model.source.name, "Fake");
        });
    }

    #[test]
    fn a_missing_row_loads_as_none() {
        let rt = rt();
        let (_dir, store, _) = with_episodes(true, true, 0, 0);
        rt.block_on(async {
            let runner = start(&rt, store, Fake::new(0));
            runner.send(EntryCmd::Load { entry_id: 4242 });
            assert!(matches!(runner.events().recv().await.unwrap(), EntryEvent::Loaded(None)));
        });
    }

    #[test]
    fn the_first_open_refresh_runs_once_and_reports_its_end() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, false, 2, 0);
        let backend = Fake::new(20);
        rt.block_on(async {
            let runner = start(&rt, store, backend.clone());
            runner.send(EntryCmd::Load { entry_id: id });
            let rx = runner.events();
            let EntryEvent::Loaded(Some(model)) = until(&rx, |e| matches!(e, EntryEvent::Loaded(_))).await else { panic!() };
            assert!(model.flow.refreshing);
            let EntryEvent::RefreshEnded(result) = until(&rx, |e| matches!(e, EntryEvent::RefreshEnded(_))).await else { panic!() };
            assert_eq!(result, Ok(()));
            assert_eq!(backend.started.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn a_refresh_requested_while_one_runs_joins_it() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let backend = Fake::new(300);
        rt.block_on(async {
            let runner = start(&rt, store, backend.clone());
            let rx = runner.events();
            runner.send(EntryCmd::Refresh { entry_id: id });
            runner.send(EntryCmd::Refresh { entry_id: id });
            until(&rx, |e| matches!(e, EntryEvent::RefreshEnded(_))).await;
            assert_eq!(backend.started.load(Ordering::SeqCst), 1, "the second request joined the first");
        });
    }

    #[test]
    fn a_flag_write_reaches_the_page_as_a_snapshot_from_the_subscription() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 2, 0);
        rt.block_on(async {
            let runner = start(&rt, store, Fake::new(0));
            runner.send(EntryCmd::Load { entry_id: id });
            let rx = runner.events();
            until(&rx, |e| matches!(e, EntryEvent::Loaded(_))).await;
            runner.send(EntryCmd::SetFlagBits { entry_id: id, flag: ep::SHOW_UNDONE, mask: ep::UNDONE_MASK });
            let event = until(&rx, |e| matches!(e, EntryEvent::Snapshot { entry, .. } if entry.item_flags & ep::UNDONE_MASK == ep::SHOW_UNDONE)).await;
            let EntryEvent::Snapshot { episodes, .. } = event else { unreachable!() };
            assert_eq!(episodes.len(), 2);
        });
    }

    #[test]
    #[ignore = "slow: run on demand"]
    fn dispose_during_a_refresh_cancels_it_and_a_queued_write_still_lands() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, false, 1, 0);
        let backend = Fake::new(400);
        let finished = Arc::clone(&backend.finished);
        rt.block_on(async {
            let runner = start(&rt, store.clone(), backend.clone());
            let rx = runner.events();
            runner.send(EntryCmd::Load { entry_id: id });
            until(&rx, |e| matches!(e, EntryEvent::Loaded(_))).await;
            // the refresh starts after `Loaded`: wait until the fetch has really started, or a dispose that wins the
            // race would cancel it before it began
            let started = std::time::Instant::now() + Duration::from_secs(5);
            while backend.started.load(Ordering::SeqCst) == 0 {
                assert!(std::time::Instant::now() < started, "the fetch never started");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            runner.send(EntryCmd::SetFlagBits { entry_id: id, flag: ep::SHOW_BOOKMARKED, mask: ep::BOOKMARKED_MASK });
            drop(runner); // the page was popped while the fetch is held open
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while store.entry(id).unwrap().unwrap().item_flags & ep::BOOKMARKED_MASK != ep::SHOW_BOOKMARKED {
                assert!(std::time::Instant::now() < deadline, "the queued write never landed");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
            assert_eq!(backend.started.load(Ordering::SeqCst), 1, "the fetch had started");
            assert_eq!(finished.load(Ordering::SeqCst), 0, "dispose cancelled the fetch before it finished");
        });
    }

    fn episode(number: f64) -> Item {
        Item { number, ..Item::create(Kind::Anime) }
    }

    #[test]
    fn only_a_manual_refresh_queues_the_new_episodes_and_only_when_the_preference_is_on() {
        use kuro_domain::download::prefs::DownloadPrefs;
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let prefs = PrefStore::in_memory();
        DownloadPrefs::new(&prefs).download_new_episodes.set(true).unwrap();
        let backend = Arc::new(Fake { new_episodes: vec![episode(7.0), episode(8.0)], ..Arc::into_inner(Fake::new(0)).unwrap() });
        rt.block_on(async {
            let runner = spawn(rt.handle(), store.clone(), prefs, backend.clone(), EntryLoadContext::default());
            let rx = runner.events();
            runner.send(EntryCmd::Refresh { entry_id: id });
            until(&rx, |e| matches!(e, EntryEvent::RefreshEnded(Ok(())))).await;
            assert_eq!(*backend.triggers.lock().unwrap(), vec![Trigger::Manual]);
            assert_eq!(*backend.queued.lock().unwrap(), vec![vec![7.0, 8.0]], "a manual refresh queues what the sync added");
        });
        // the open-time refresh reports the same episodes and queues none (E-REF-5)
        let (_dir2, store2, id2) = with_episodes(true, false, 1, 0);
        let prefs = PrefStore::in_memory();
        DownloadPrefs::new(&prefs).download_new_episodes.set(true).unwrap();
        let backend = Arc::new(Fake { new_episodes: vec![episode(7.0)], ..Arc::into_inner(Fake::new(0)).unwrap() });
        rt.block_on(async {
            let runner = spawn(rt.handle(), store2, prefs, backend.clone(), EntryLoadContext::default());
            let rx = runner.events();
            runner.send(EntryCmd::Load { entry_id: id2 });
            until(&rx, |e| matches!(e, EntryEvent::RefreshEnded(_))).await;
            assert_eq!(*backend.triggers.lock().unwrap(), vec![Trigger::Open]);
            assert!(backend.queued.lock().unwrap().is_empty(), "the open refresh never auto-downloads");
        });
        // the preference is off: a manual refresh queues nothing
        let (_dir3, store3, id3) = with_episodes(true, true, 1, 0);
        let backend = Arc::new(Fake { new_episodes: vec![episode(7.0)], ..Arc::into_inner(Fake::new(0)).unwrap() });
        rt.block_on(async {
            let runner = spawn(rt.handle(), store3, PrefStore::in_memory(), backend.clone(), EntryLoadContext::default());
            let rx = runner.events();
            runner.send(EntryCmd::Refresh { entry_id: id3 });
            until(&rx, |e| matches!(e, EntryEvent::RefreshEnded(_))).await;
            assert!(backend.queued.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn removing_a_favorite_writes_first_then_deletes_the_cover_and_asks_about_downloads() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let backend = Arc::new(Fake { downloads: 2, ..Arc::into_inner(Fake::new(0)).unwrap() });
        rt.block_on(async {
            let runner = start(&rt, store.clone(), backend.clone());
            let rx = runner.events();
            runner.send(EntryCmd::ToggleLibrary { entry_id: id, check_duplicates: true });
            let EntryEvent::Removed(outcome) = until(&rx, |e| matches!(e, EntryEvent::Removed(_))).await else { unreachable!() };
            assert_eq!(outcome, RemoveOutcome { cover_removed: true, ask_delete_downloads: true });
            let row = store.entry(id).unwrap().unwrap();
            assert!(!row.favorite && row.date_added == 0);
            assert_eq!(backend.covers.load(Ordering::SeqCst), 1);
            runner.send(EntryCmd::DeleteDownloads { entry_id: id });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while backend.deleted.load(Ordering::SeqCst) == 0 {
                assert!(std::time::Instant::now() < deadline, "Delete never reached the downloads layer");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }

    #[test]
    fn adding_a_non_favorite_writes_favorite_and_the_library_defaults() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(false, true, 1, 0);
        let prefs = PrefStore::in_memory();
        kuro_domain::entries::defaults::set_item_settings_default(&prefs, Kind::Anime, ep::SHOW_UNDONE);
        rt.block_on(async {
            let runner = spawn(rt.handle(), store.clone(), prefs, Fake::new(0), EntryLoadContext::default());
            let rx = runner.events();
            runner.send(EntryCmd::ToggleLibrary { entry_id: id, check_duplicates: true });
            until(&rx, |e| matches!(e, EntryEvent::Added)).await;
            let row = store.entry(id).unwrap().unwrap();
            assert!(row.favorite);
            assert_eq!(row.item_flags & ep::UNDONE_MASK, ep::SHOW_UNDONE, "defaults are written at add time");
        });
    }

    #[test]
    fn mark_and_seen_commands_write_what_they_are_given() {
        use super::super::actions::Mark;
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 2, 0);
        let ids: Vec<i64> = store.episodes(id).unwrap().iter().map(|e| e.id).collect();
        rt.block_on(async {
            let runner = start(&rt, store.clone(), Fake::new(0));
            runner.send(EntryCmd::SetMark { ids: vec![ids[0]], mark: Mark::Fillermark, on: true });
            runner.send(EntryCmd::SetSeen { ids: vec![ids[1]], seen: true });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let rows = store.episodes(id).unwrap();
                let a = rows.iter().find(|e| e.id == ids[0]).unwrap().fillermark;
                let b = rows.iter().find(|e| e.id == ids[1]).unwrap().done;
                if a && b {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "the writes never landed");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
    }

    #[test]
    fn the_stub_backend_reads_unknown_sources_as_missing_and_fetches_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).unwrap();
        let db = Arc::new(Db::new(&path, writer).unwrap());
        StubSourceRepo::new(Arc::clone(&db), Kind::Anime).upsert(7, "en", "Player").unwrap();
        let backend = StubEntryBackend::new(db);
        assert_eq!(backend.source_info(7), SourceInfo { name: "Player (EN)".into(), stub: true });
        assert!(backend.source_info(99).stub);
        let local = backend.source_info(kuro_source::local::LOCAL_SOURCE_ID);
        assert!(!local.stub && !local.name.is_empty());
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let plan = RefreshPlan { info: true, episodes: false, seasons: false };
        assert_eq!(rt.block_on(backend.fetch(1, plan, Trigger::Manual)), Err(DetailError::SourceNotInstalled));
    }

    #[test]
    fn set_as_default_with_the_checkbox_reports_how_many_favorites_took_that_scope_only() {
        use kuro_db::flags::season as sf;
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        store.set_season_flag_bits(id, sf::SHOW_STARTED, sf::STARTED_MASK).unwrap();
        let prefs = PrefStore::in_memory();
        kuro_domain::entries::defaults::set_item_settings_default(&prefs, Kind::Anime, ep::SHOW_DONE | ep::SORT_ASC | ep::SORTING_NUMBER);
        rt.block_on(async {
            let runner = spawn(rt.handle(), store.clone(), prefs, Fake::new(0), EntryLoadContext::default());
            runner.send(EntryCmd::ApplyDefaultsToLibrary { scope: Scope::Episode });
            let event = until(&runner.events(), |e| matches!(e, EntryEvent::DefaultsApplied { .. })).await;
            let EntryEvent::DefaultsApplied { scope, updated } = event else { unreachable!() };
            assert_eq!((scope, updated), (Scope::Episode, 1));
            let row = store.entry(id).unwrap().unwrap();
            assert_eq!(row.item_flags & ep::UNDONE_MASK, ep::SHOW_DONE);
            assert_eq!(row.season_flags() & sf::STARTED_MASK, sf::SHOW_STARTED, "the episode scope leaves the season flags alone");
        });
    }

    #[test]
    fn an_add_with_a_same_named_library_entry_waits_for_the_answer_and_add_anyway_adds() {
        use kuro_data::entry::EntryRepo;
        let rt = rt();
        let (_dir, store, id) = with_episodes(false, true, 0, 0);
        let twin = EntryRepo::new(store.db()).insert(&Entry { source: 8, url: "/b".into(), title: "frieren".into(), favorite: true, ..Entry::create(Kind::Anime) }).unwrap();
        rt.block_on(async {
            let runner = start(&rt, store.clone(), Fake::new(0));
            let rx = runner.events();
            runner.send(EntryCmd::ToggleLibrary { entry_id: id, check_duplicates: true });
            let EntryEvent::Duplicate { existing } = until(&rx, |e| matches!(e, EntryEvent::Duplicate { .. })).await else { unreachable!() };
            assert_eq!(existing.iter().map(|a| a.id).collect::<Vec<_>>(), vec![twin]);
            assert!(!store.entry(id).unwrap().unwrap().favorite, "nothing is written until the user answers");
            runner.send(EntryCmd::ToggleLibrary { entry_id: id, check_duplicates: false });
            until(&rx, |e| matches!(e, EntryEvent::Added)).await;
            assert!(store.entry(id).unwrap().unwrap().favorite);
        });
    }

    #[test]
    fn an_add_without_a_duplicate_adds_at_once() {
        let rt = rt();
        let (_dir, store, id) = with_episodes(false, true, 0, 0);
        rt.block_on(async {
            let runner = start(&rt, store.clone(), Fake::new(0));
            runner.send(EntryCmd::ToggleLibrary { entry_id: id, check_duplicates: true });
            until(&runner.events(), |e| matches!(e, EntryEvent::Added)).await;
            assert!(store.entry(id).unwrap().unwrap().favorite);
        });
    }

    #[test]
    fn related_groups_are_published_after_the_load_and_again_when_they_change() {
        use kuro_data::anime_relation::AnimeRelationRepo;
        use kuro_data::entry::EntryRepo;
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        let other = EntryRepo::new(store.db()).insert(&Entry { source: 8, url: "/b".into(), title: "Other".into(), ..Entry::create(Kind::Anime) }).unwrap();
        rt.block_on(async {
            let runner = start(&rt, store.clone(), Fake::new(0));
            let rx = runner.events();
            runner.send(EntryCmd::Load { entry_id: id });
            until(&rx, |e| matches!(e, EntryEvent::Related(groups) if groups.is_empty())).await;
            let group = AnimeRelationGroup { name: "Sequel".into(), anime: vec![Entry { id: other, ..Entry::create(Kind::Anime) }] };
            AnimeRelationRepo::new(store.db()).replace_relations(id, &[group], 1).unwrap();
            let EntryEvent::Related(groups) = until(&rx, |e| matches!(e, EntryEvent::Related(g) if !g.is_empty())).await else { unreachable!() };
            assert_eq!((groups[0].name.as_str(), groups[0].anime[0].id), ("Sequel", other));
        });
    }

    #[test]
    fn a_seasons_entry_gets_the_download_count_and_language_of_each_season() {
        use kuro_data::entry::EntryRepo;
        let rt = rt();
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        store.set_fetch_type_for_test(id, FetchType::Seasons);
        let season = EntryRepo::new(store.db())
            .insert(&Entry { source: 7, url: "/s1".into(), title: "S1".into(), parent_id: Some(id), season_number: Some(1.0), ..Entry::create(Kind::Anime) })
            .unwrap();
        rt.block_on(async {
            let mut fake = Fake::new(0);
            Arc::get_mut(&mut fake).unwrap().downloads = 3;
            let runner = start(&rt, store.clone(), fake);
            let rx = runner.events();
            runner.send(EntryCmd::Load { entry_id: id });
            let EntryEvent::SeasonFacts(facts) = until(&rx, |e| matches!(e, EntryEvent::SeasonFacts(_))).await else { unreachable!() };
            assert_eq!(facts, vec![SeasonExtra { id: season, download_count: 3, source_language: "ja".into() }]);
        });
    }
}
