//! The pass runner (behaviour 2, 6.1, 6.5): one pass per kind at a time, a last-run timestamp, per-source groups
//! run in parallel (at most five), entries of one source run one after another, progress and summary to the host,
//! and a cancel token per running pass. The host posts notifications and queues downloads; this module decides.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Local;
use kuro_platform::kind::Kind;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::library_update::download::DownloadPolicy;
use crate::library_update::entry::{Deps, EntryContext, update_entry};
use crate::library_update::error::UpdateError;
use crate::library_update::fetch_interval::{Window, window_for};
use crate::library_update::politeness;
use crate::library_update::progress;
use crate::library_update::queue::{FetchType, LibraryEntry, QueueInput, QueueRestrictions, build_queue};
use crate::library_update::source::{Fetched, SourceItem, SourceRegistry};
use crate::library_update::store::LibraryStore;
use crate::library_update::text;
use crate::settings_ui::jobs::{JobOutcome, UniquePolicy};

/// Sources processed at the same time (`Semaphore(5)` in the Kotlin job).
pub const MAX_SOURCES_IN_PARALLEL: usize = 5;

/// Diffs the source list against the database and writes it (behaviour 5.4 and 5.5). Returns the new items.
pub trait ItemSync: Send + Sync {
    fn sync(
        &self,
        entry: &LibraryEntry,
        fetched: &Fetched,
        window: Window,
        now_ms: i64,
    ) -> impl Future<Output = Result<Vec<SourceItem>, UpdateError>> + Send;
}

/// What the pass tells the outside world: notifications, the app-state counters and the download queue.
pub trait Host: Send + Sync {
    /// `library_update_last_timestamp`, written at pass start even for an empty queue.
    fn set_last_update_ms(&self, now_ms: i64);
    /// Adds to `library_unread_updates_count` (manga) or `library_unseen_updates_count` (anime).
    fn add_new_updates(&self, kind: Kind, count: usize);
    /// The progress notification; `body` is `None` when content is hidden.
    fn show_progress(&self, kind: Kind, title: &str, body: Option<&str>);
    fn cancel_progress(&self, kind: Kind);
    /// The summary and per-entry notifications for the entries with new items.
    fn show_summary(&self, kind: Kind, updates: &[EntryUpdate], hide_content: bool);
    /// Queues the items with autostart off.
    fn enqueue_downloads(&self, kind: Kind, entry_id: i64, items: &[SourceItem]);
    fn start_downloads(&self, kind: Kind);
    /// Writes the error log of a pass that had failures (T-26a-095) and returns the file, or `None` when none was written.
    /// Called only when there are failures and the pass was not cancelled.
    fn write_error_log(&self, _kind: Kind, _failures: &[Failure]) -> Option<PathBuf> {
        None
    }
    /// The failure notice; `log` is the file its tap opens (T-26a-096).
    fn show_error(&self, kind: Kind, failed: usize, log: Option<&Path>);
    /// The large-queue warning: a source of this pass has more queued entries than the threshold (not unmetered).
    fn show_queue_warning(&self, kind: Kind);
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryUpdate {
    pub entry_id: i64,
    pub title: String,
    pub items: Vec<SourceItem>,
}

/// One failed entry of a pass: what the error log lists (behaviour 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub source_id: i64,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Auto,
    Manual,
}

/// The choices of one pass, read from the preferences by the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct PassSettings {
    pub kind: Kind,
    /// `Some(id)` for a manual run of one category.
    pub category: Option<i64>,
    pub include_categories: Vec<i64>,
    pub exclude_categories: Vec<i64>,
    pub restrictions: QueueRestrictions,
    pub include_season_children: bool,
    pub auto_update_metadata: bool,
    pub download: DownloadPolicy,
    pub hide_content: bool,
}

impl PassSettings {
    /// The defaults: all categories, the default restriction set, nothing else enabled.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            category: None,
            include_categories: Vec::new(),
            exclude_categories: Vec::new(),
            restrictions: QueueRestrictions::from_pref::<&str>(None),
            include_season_children: false,
            auto_update_metadata: false,
            download: DownloadPolicy::default(),
            hide_content: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassStatus {
    /// Another pass of this kind is running.
    Skipped,
    /// A scheduled pass found a manual pass running; try again later.
    Retry,
    Success,
    Cancelled,
    /// The library could not be read; the pass ended before any entry.
    Failed(String),
}

/// The events of one pass, in the order they happened. `EntryFinished::done` never decreases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassEvent {
    Started { total: usize },
    EntryStarted { entry_id: i64 },
    EntryFinished { entry_id: i64, done: usize },
    Finished { status: PassStatus },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PassOutcome {
    pub status: PassStatus,
    pub updates: Vec<EntryUpdate>,
    /// Entry title and user message per failed entry.
    pub failures: Vec<(String, String)>,
    pub queued_downloads: usize,
    pub events: Vec<PassEvent>,
}

impl PassOutcome {
    fn ended(status: PassStatus) -> Self {
        Self {
            status: status.clone(),
            updates: Vec::new(),
            failures: Vec::new(),
            queued_downloads: 0,
            events: vec![PassEvent::Finished { status }],
        }
    }
}

fn slot_index(kind: Kind) -> usize {
    match kind {
        Kind::Manga => 0,
        Kind::Anime => 1,
    }
}

struct Slot {
    trigger: Trigger,
    cancel: Arc<AtomicBool>,
}

/// One running pass per kind. Acquiring is the in-process lock; cancelling sets the running pass's token.
#[derive(Default)]
pub struct PassLocks {
    slots: Mutex<[Option<Slot>; 2]>,
}

impl PassLocks {
    fn lock(&self) -> MutexGuard<'_, [Option<Slot>; 2]> {
        self.slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The guard of a new pass, or the trigger of the pass that holds the kind.
    pub fn acquire(this: &Arc<Self>, kind: Kind, trigger: Trigger) -> Result<PassGuard, Trigger> {
        let mut slots = this.lock();
        let index = slot_index(kind);
        if let Some(slot) = &slots[index] {
            return Err(slot.trigger);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        slots[index] = Some(Slot { trigger, cancel: Arc::clone(&cancel) });
        Ok(PassGuard { locks: Arc::clone(this), kind, trigger, cancel })
    }

    /// Cancels the running pass of `kind`. Returns the trigger of that pass (a stopped automatic pass re-arms its
    /// loop), or `None` when nothing runs.
    pub fn stop(&self, kind: Kind) -> Option<Trigger> {
        let slots = self.lock();
        slots[slot_index(kind)].as_ref().map(|slot| {
            slot.cancel.store(true, Ordering::SeqCst);
            slot.trigger
        })
    }

    pub fn holder(&self, kind: Kind) -> Option<Trigger> {
        self.lock()[slot_index(kind)].as_ref().map(|slot| slot.trigger)
    }
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// Held while a pass runs. Dropping it frees the kind.
pub struct PassGuard {
    locks: Arc<PassLocks>,
    kind: Kind,
    trigger: Trigger,
    cancel: Arc<AtomicBool>,
}

impl PassGuard {

    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
}

impl Drop for PassGuard {
    fn drop(&mut self) {
        self.locks.lock()[slot_index(self.kind)] = None;
    }
}

/// Status of a request that found the kind busy: a scheduled pass waits for a manual one, anything else is skipped.
fn busy_status(trigger: Trigger, held_by: Trigger) -> PassStatus {
    match (trigger, held_by) {
        (Trigger::Auto, Trigger::Manual) => PassStatus::Retry,
        _ => PassStatus::Skipped,
    }
}

/// Entries grouped by source, in order of first appearance.
pub(crate) fn group_by_source(entries: Vec<LibraryEntry>) -> Vec<Vec<LibraryEntry>> {
    let mut groups: Vec<(i64, Vec<LibraryEntry>)> = Vec::new();
    for entry in entries {
        match groups.iter_mut().find(|(source, _)| *source == entry.source) {
            Some((_, list)) => list.push(entry),
            None => groups.push((entry.source, vec![entry])),
        }
    }
    groups.into_iter().map(|(_, list)| list).collect()
}

/// The state shared by the entry tasks of one pass.
struct Shared {
    total: usize,
    progress: Mutex<Progress>,
    updates: Mutex<Vec<EntryUpdate>>,
    failures: Mutex<Vec<Failure>>,
    events: Mutex<Vec<PassEvent>>,
    queued: AtomicUsize,
}

#[derive(Default)]
struct Progress {
    completed: usize,
    running: Vec<(i64, String)>,
}

impl Shared {
    fn publish<H: Host>(&self, host: &H, kind: Kind, hide_content: bool) {
        let (completed, titles) = {
            let progress = self.progress.lock().unwrap_or_else(|p| p.into_inner());
            (progress.completed, progress.running.iter().map(|(_, title)| title.clone()).collect::<Vec<_>>())
        };
        let title = text::progress_title(completed as u64, self.total as u64);
        let body = (!hide_content).then(|| text::progress_body(&titles));
        host.show_progress(kind, &title, body.as_deref());
    }
}

pub struct PassRunner<R, S, Y, H> {
    deps: Arc<Deps<R, S, Y, H>>,
    locks: Arc<PassLocks>,
}

impl<R, S, Y, H> PassRunner<R, S, Y, H>
where
    R: SourceRegistry + 'static,
    S: LibraryStore + 'static,
    Y: ItemSync + 'static,
    H: Host + 'static,
{
    pub fn new(deps: Deps<R, S, Y, H>) -> Self {
        Self { deps: Arc::new(deps), locks: Arc::new(PassLocks::default()) }
    }

    pub fn locks(&self) -> &Arc<PassLocks> {
        &self.locks
    }

    /// Runs a pass now, on the caller's task. A busy kind returns `Skipped`, or `Retry` for a scheduled pass that
    /// found a manual one running.
    pub async fn run(&self, settings: PassSettings, trigger: Trigger) -> PassOutcome {
        match PassLocks::acquire(&self.locks, settings.kind, trigger) {
            Ok(guard) => {
                let now = Local::now();
                self.run_with(guard, settings, window_for(&now), now.timestamp_millis()).await
            }
            Err(held_by) => PassOutcome::ended(busy_status(trigger, held_by)),
        }
    }

    /// The manual "start now": returns `false` without starting when a pass of the kind is running, otherwise runs
    /// the pass on a spawned task and returns `true`.
    pub fn start_now(self: &Arc<Self>, settings: PassSettings) -> bool {
        self.start_after(settings, std::future::ready(()))
    }

    /// [`Self::start_now`], with the pass waiting for `before` first. The lock is taken at once, so the pass counts as
    /// running (and a second start is refused) while it waits.
    pub fn start_after(self: &Arc<Self>, settings: PassSettings, before: impl Future<Output = ()> + Send + 'static) -> bool {
        let Ok(guard) = PassLocks::acquire(&self.locks, settings.kind, Trigger::Manual) else {
            return false;
        };
        let this = Arc::clone(self);
        tokio::spawn(async move {
            before.await;
            let now = Local::now();
            this.run_with(guard, settings, window_for(&now), now.timestamp_millis()).await;
        });
        true
    }

    /// Cancels the running pass of `kind`; returns its trigger.
    pub fn stop(&self, kind: Kind) -> Option<Trigger> {
        self.locks.stop(kind)
    }

    /// The pass with the lock already held, recorded as one job of its kind in the job registry (T-52a-056), so the
    /// worker page and the busy checks see it. `window` and `now_ms` are fixed for the whole pass.
    pub async fn run_with(&self, guard: PassGuard, settings: PassSettings, window: Window, now_ms: i64) -> PassOutcome {
        let (name, jobs) = (job_name(settings.kind), crate::settings_ui::jobs::global());
        // The lock is held here, so an active job of this name can only be one that never ended: it is replaced.
        let job = jobs.enqueue_unique(name, &["library-update"], UniquePolicy::Replace, None).id;
        jobs.start(job);
        let outcome = self.run_pass(guard, settings, window, now_ms).await;
        match &outcome.status {
            PassStatus::Success => jobs.finish(job, JobOutcome::Success),
            PassStatus::Retry => jobs.finish(job, JobOutcome::Retry),
            PassStatus::Cancelled => jobs.cancel_unique(name),
            PassStatus::Skipped | PassStatus::Failed(_) => jobs.finish(job, JobOutcome::Failure),
        }
        outcome
    }

    /// The pass body, with the lock already held.
    async fn run_pass(&self, guard: PassGuard, settings: PassSettings, window: Window, now_ms: i64) -> PassOutcome {
        let deps = Arc::clone(&self.deps);
        let kind = settings.kind;
        let cancel = guard.cancel_token();
        deps.host.set_last_update_ms(now_ms);

        let library = match deps.store.library(kind).await {
            Ok(library) => library,
            Err(error) => {
                deps.host.cancel_progress(kind);
                return PassOutcome::ended(PassStatus::Failed(error.user_message()));
            }
        };

        let mut season_children = HashMap::new();
        if settings.include_season_children {
            for parent in library.iter().filter(|e| e.fetch_type == FetchType::Seasons) {
                season_children.insert(parent.id, deps.store.season_children(parent.id).await);
            }
        }

        let off_sources = crate::source_switch::off_ids();
        let queue = build_queue(&QueueInput {
            library: &library,
            explicit_category: settings.category,
            include_categories: &settings.include_categories,
            exclude_categories: &settings.exclude_categories,
            restrictions: settings.restrictions,
            include_season_children: settings.include_season_children,
            release_window_upper_ms: window.upper_ms,
            season_children: &season_children,
            off_sources: &off_sources,
        });
        for (entry_id, reason) in &queue.skipped {
            tracing::debug!(entry_id, reason = reason.message(), "library update skipped entry");
        }

        let mut queued_per_source: HashMap<i64, usize> = HashMap::new();
        for entry in &queue.entries {
            *queued_per_source.entry(entry.source).or_default() += 1;
        }
        let per_source: Vec<(usize, bool)> = queued_per_source
            .into_iter()
            .map(|(source, count)| (count, deps.registry.get_or_stub(kind, source).is_unmetered()))
            .collect();
        if text::queue_needs_warning(&per_source) {
            deps.host.show_queue_warning(kind);
        }

        let total = queue.entries.len();
        let shared = Arc::new(Shared {
            total,
            progress: Mutex::new(Progress::default()),
            updates: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            events: Mutex::new(vec![PassEvent::Started { total }]),
            queued: AtomicUsize::new(0),
        });
        progress::publish(kind, PassEvent::Started { total });
        let settings = Arc::new(settings);
        let permits = Arc::new(Semaphore::new(MAX_SOURCES_IN_PARALLEL));
        let mut tasks = JoinSet::new();
        for group in group_by_source(queue.entries) {
            let deps = Arc::clone(&deps);
            let settings = Arc::clone(&settings);
            let shared = Arc::clone(&shared);
            let cancel = Arc::clone(&cancel);
            let permits = Arc::clone(&permits);
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await.expect("pass semaphore is never closed");
                for entry in group {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    {
                        let mut progress = shared.progress.lock().unwrap_or_else(|p| p.into_inner());
                        progress.running.push((entry.id, entry.title.clone()));
                        shared.events.lock().unwrap_or_else(|p| p.into_inner()).push(PassEvent::EntryStarted { entry_id: entry.id });
                    }
                    shared.publish(&deps.host, kind, settings.hide_content);

                    let cx = EntryContext { settings: &settings, window, now_ms, cancel: &cancel };
                    // The source's process-wide permit: no second request to it from the other kind's pass (T-26a-022).
                    let result = {
                        let _source = politeness::acquire(entry.source).await;
                        update_entry(&*deps, &cx, &entry).await
                    };
                    record_entry(&deps.host, kind, &shared, &entry, result);
                    {
                        let mut progress = shared.progress.lock().unwrap_or_else(|p| p.into_inner());
                        progress.running.retain(|(id, _)| *id != entry.id);
                        progress.completed += 1;
                        let done = progress.completed;
                        shared.events.lock().unwrap_or_else(|p| p.into_inner()).push(PassEvent::EntryFinished { entry_id: entry.id, done });
                        progress::publish(kind, PassEvent::EntryFinished { entry_id: entry.id, done });
                    }
                    shared.publish(&deps.host, kind, settings.hide_content);
                }
            });
        }
        while let Some(joined) = tasks.join_next().await {
            if let Err(error) = joined {
                tracing::error!(%error, "library update entry task ended abnormally");
            }
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        let updates = shared.updates.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let failed = shared.failures.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let failures: Vec<(String, String)> = failed.iter().map(|f| (f.title.clone(), f.message.clone())).collect();
        let queued_downloads = shared.queued.load(Ordering::SeqCst);
        deps.host.cancel_progress(kind);
        if !updates.is_empty() {
            deps.host.show_summary(kind, &updates, settings.hide_content);
        }
        if queued_downloads > 0 {
            deps.host.start_downloads(kind);
        }
        if !failures.is_empty() && !cancelled {
            let log = deps.host.write_error_log(kind, &failed);
            deps.host.show_error(kind, failures.len(), log.as_deref());
        }
        let status = if cancelled { PassStatus::Cancelled } else { PassStatus::Success };
        let mut events = shared.events.lock().unwrap_or_else(|p| p.into_inner()).clone();
        events.push(PassEvent::Finished { status: status.clone() });
        progress::publish(kind, PassEvent::Finished { status: status.clone() });
        PassOutcome { status, updates, failures, queued_downloads, events }
    }
}

/// Files one entry's result: new items, counters, or a failure. A cancelled entry is not a failure.
fn record_entry<H: Host>(
    host: &H,
    kind: Kind,
    shared: &Shared,
    entry: &LibraryEntry,
    result: Result<crate::library_update::entry::EntryOutcome, UpdateError>,
) {
    match result {
        Ok(outcome) => {
            if !outcome.new_items.is_empty() {
                // The counter write happens under the same lock that orders the updates, so concurrent entries
                // add to the app counter one at a time (T-26a-025).
                let mut updates = shared.updates.lock().unwrap_or_else(|p| p.into_inner());
                host.add_new_updates(kind, outcome.new_items.len());
                updates.push(EntryUpdate { entry_id: entry.id, title: entry.title.clone(), items: outcome.new_items });
            }
            shared.queued.fetch_add(outcome.downloads, Ordering::SeqCst);
        }
        Err(UpdateError::Cancelled) => {}
        Err(error) => {
            shared.failures.lock().unwrap_or_else(|p| p.into_inner()).push(Failure {
                source_id: entry.source,
                title: entry.title.clone(),
                message: error.user_message(),
            });
        }
    }
}

/// The name of the job a library pass of `kind` records under in the job registry.
pub fn job_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "library-update-manga",
        Kind::Anime => "library-update-anime",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_update::fake::{FakeHost, FakeStore, FakeSync, entry, item};
    use crate::library_update::source::fake::{FakeRegistry, FakeSource};

    type TestRunner = PassRunner<FakeRegistry, FakeStore, FakeSync, FakeHost>;

    fn runner(store: FakeStore, registry: FakeRegistry) -> Arc<TestRunner> {
        Arc::new(PassRunner::new(Deps { registry, store, sync: FakeSync::default(), host: FakeHost::default() }))
    }

    fn window() -> Window {
        Window { lower_ms: 0, upper_ms: 1_000_000 }
    }

    /// No item restrictions, so the fixture entries (nothing read) reach the source.
    fn settings() -> PassSettings {
        PassSettings { restrictions: QueueRestrictions::default(), ..PassSettings::new(Kind::Manga) }
    }

    #[tokio::test]
    async fn an_empty_library_succeeds_writes_the_timestamp_and_clears_progress() {
        // Q-01 and P-19: the timestamp is written even with nothing to update.
        let runner = runner(FakeStore::default(), FakeRegistry::default());
        let outcome = runner.run_with(PassLocks::acquire(runner.locks(), Kind::Manga, Trigger::Manual).unwrap(), settings(), window(), 4242).await;
        assert_eq!(outcome.status, PassStatus::Success);
        assert_eq!(runner.deps.host.last_timestamp.load(Ordering::SeqCst), 4242);
        assert_eq!(runner.deps.host.cancelled_progress.load(Ordering::SeqCst), 1);
        assert!(outcome.updates.is_empty() && outcome.failures.is_empty());
    }

    #[tokio::test]
    async fn a_busy_kind_refuses_a_manual_start_and_a_scheduled_pass_retries() {
        // P-09, P-10, R-01
        let runner = runner(FakeStore::with(vec![entry(1, 1, "A")]), FakeRegistry::default());
        let held = PassLocks::acquire(runner.locks(), Kind::Manga, Trigger::Manual).expect("first lock");
        assert!(!Arc::clone(&runner).start_now(settings()), "start now is refused while a pass runs");
        let scheduled = runner.run(settings(), Trigger::Auto).await;
        assert_eq!(scheduled.status, PassStatus::Retry);
        let manual = runner.run(settings(), Trigger::Manual).await;
        assert_eq!(manual.status, PassStatus::Skipped);
        drop(held);
        assert!(runner.locks().holder(Kind::Manga).is_none(), "dropping the guard frees the kind");
    }

    #[tokio::test]
    async fn an_automatic_anime_pass_yields_to_a_running_manual_anime_pass() {
        // T-26a-015: anime follows the manga rule (the deliberate fix of P-5).
        let runner = runner(FakeStore::default(), FakeRegistry::default());
        let _held = PassLocks::acquire(runner.locks(), Kind::Anime, Trigger::Manual).expect("anime lock");
        let scheduled = runner.run(PassSettings::new(Kind::Anime), Trigger::Auto).await;
        assert_eq!(scheduled.status, PassStatus::Retry);
    }

    #[tokio::test]
    async fn an_automatic_pass_of_the_other_kind_is_not_blocked() {
        // Kinds lock independently (SC-12).
        let runner = runner(FakeStore::default(), FakeRegistry::default());
        let _manga = PassLocks::acquire(runner.locks(), Kind::Manga, Trigger::Manual).expect("manga");
        let anime = PassSettings::new(Kind::Anime);
        assert_eq!(runner.run(anime, Trigger::Auto).await.status, PassStatus::Success);
    }

    #[tokio::test]
    async fn a_started_pass_refuses_a_second_start_at_once() {
        // R-01: the lock is taken before the task is spawned.
        let runner = runner(FakeStore::default(), FakeRegistry::default());
        assert!(Arc::clone(&runner).start_now(settings()));
        assert!(!Arc::clone(&runner).start_now(settings()));
    }

    #[tokio::test]
    async fn stop_reports_the_trigger_of_the_running_pass() {
        // SC-09 / SC-10: the caller re-arms only an automatic pass.
        let runner = runner(FakeStore::default(), FakeRegistry::default());
        assert_eq!(runner.stop(Kind::Manga), None);
        let _auto = PassLocks::acquire(runner.locks(), Kind::Manga, Trigger::Auto).expect("auto");
        assert_eq!(runner.stop(Kind::Manga), Some(Trigger::Auto));
    }

    #[tokio::test]
    async fn a_cancelled_pass_starts_no_entry_and_reports_no_failure() {
        // P-08 and T-26a-021: no source call after the cancel; cancellation is not an error notification.
        let mut registry = FakeRegistry::default();
        let source = FakeSource::ok(vec![item(1.0, 0)]);
        let calls = Arc::clone(&source.calls);
        registry.sources.insert(1, source);
        let runner = runner(FakeStore::with(vec![entry(1, 1, "A"), entry(2, 1, "B")]), registry);
        let guard = PassLocks::acquire(runner.locks(), Kind::Manga, Trigger::Manual).expect("lock");
        assert_eq!(runner.stop(Kind::Manga), Some(Trigger::Manual));
        let outcome = runner.run_with(guard, settings(), window(), 1).await;
        assert_eq!(outcome.status, PassStatus::Cancelled);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(outcome.failures.is_empty());
        assert!(runner.deps.host.errors.lock().unwrap().is_empty());
    }

    #[test]
    fn each_kind_records_its_own_job() {
        assert_ne!(job_name(Kind::Manga), job_name(Kind::Anime));
    }

    #[tokio::test]
    async fn a_library_read_failure_ends_the_pass_and_frees_the_lock() {
        // F-02
        let store = FakeStore { library_error: Some(UpdateError::Other("db gone".into())), ..FakeStore::default() };
        let runner = runner(store, FakeRegistry::default());
        let outcome = runner.run(settings(), Trigger::Manual).await;
        assert_eq!(outcome.status, PassStatus::Failed("db gone".into()));
        assert!(runner.locks().holder(Kind::Manga).is_none());
    }

    #[tokio::test]
    async fn new_items_are_counted_per_entry_and_summarised_per_entry() {
        // P-15, P-18, N-17 style counts.
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::ok(vec![item(1.0, 0), item(2.0, 1)]));
        registry.sources.insert(2, FakeSource::ok(vec![item(5.0, 0)]));
        let store = FakeStore::with(vec![entry(1, 1, "Alpha"), entry(2, 1, "Beta"), entry(3, 2, "Gamma")]);
        let runner = runner(store, registry);
        let outcome = runner.run(settings(), Trigger::Manual).await;
        assert_eq!(outcome.status, PassStatus::Success);
        assert_eq!(runner.deps.host.added.load(Ordering::SeqCst), 2 + 2 + 1);
        assert_eq!(outcome.updates.len(), 3, "one update per entry with new items");
        let summaries = runner.deps.host.summaries.lock().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].len(), 3);
        assert_eq!(runner.deps.host.hidden_flags.lock().unwrap().as_slice(), &[false]);
    }

    #[tokio::test]
    async fn one_failing_entry_does_not_stop_the_others_and_is_reported_once() {
        // P-06, F-07: failures are collected, the error notification shows the count.
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::err(UpdateError::Other("timeout".into())));
        registry.sources.insert(2, FakeSource::ok(vec![item(1.0, 0)]));
        let store = FakeStore::with(vec![entry(1, 1, "Broken"), entry(2, 2, "Fine")]);
        let runner = runner(store, registry);
        let outcome = runner.run(settings(), Trigger::Manual).await;
        assert_eq!(outcome.status, PassStatus::Success);
        assert_eq!(outcome.failures, vec![("Broken".to_owned(), "timeout".to_owned())]);
        assert_eq!(outcome.updates.len(), 1);
        assert_eq!(runner.deps.host.errors.lock().unwrap().as_slice(), &[1]);
    }

    #[tokio::test]
    async fn the_error_log_is_written_with_the_failures_and_its_path_goes_to_the_notice() {
        // T-26a-095, T-26a-096
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::err(UpdateError::Other("timeout".into())));
        let runner = runner(FakeStore::with(vec![entry(1, 1, "Broken")]), registry);
        runner.run(settings(), Trigger::Manual).await;
        let logged = runner.deps.host.logged.lock().unwrap();
        assert_eq!(logged.as_slice(), &[vec![Failure { source_id: 1, title: "Broken".into(), message: "timeout".into() }]]);
        assert_eq!(
            runner.deps.host.error_logs.lock().unwrap().as_slice(),
            &[Some(PathBuf::from("update_errors.txt"))],
            "the failure notice carries the log file its tap opens"
        );
    }

    #[tokio::test]
    async fn a_pass_without_failures_writes_no_error_log() {
        // P-16
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::ok(vec![item(1.0, 0)]));
        let runner = runner(FakeStore::with(vec![entry(1, 1, "Fine")]), registry);
        runner.run(settings(), Trigger::Manual).await;
        assert!(runner.deps.host.logged.lock().unwrap().is_empty());
        assert!(runner.deps.host.errors.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn entries_of_one_source_run_one_after_another_and_at_most_five_sources_overlap() {
        // P-01, P-02: a shared counter sees the global overlap; each source sees its own entries sequentially.
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let mut registry = FakeRegistry::default();
        let mut library = Vec::new();
        for source in 1..=7_i64 {
            let mut s = FakeSource::ok(vec![item(1.0, 0)]);
            s.active = Arc::clone(&active);
            s.max_active = Arc::clone(&max_active);
            registry.sources.insert(source, s);
            library.push(entry(source * 10, source, &format!("S{source}-a")));
            library.push(entry(source * 10 + 1, source, &format!("S{source}-b")));
        }
        let runner = runner(FakeStore::with(library), registry);
        let outcome = runner.run(settings(), Trigger::Manual).await;
        assert_eq!(outcome.status, PassStatus::Success);
        assert_eq!(outcome.updates.len(), 14);
        let global = max_active.load(Ordering::SeqCst);
        assert!(global <= MAX_SOURCES_IN_PARALLEL, "at most five sources at once, saw {global}");
        assert!(global > 1, "sources do overlap, saw {global}");
    }

    #[tokio::test]
    async fn a_queue_of_more_than_sixty_entries_from_one_source_warns_once() {
        // Q-18, T-26a-031: 60 is quiet, 61 warns once.
        for (count, warnings) in [(60_i64, 0), (61, 1)] {
            let mut registry = FakeRegistry::default();
            registry.sources.insert(1, FakeSource::ok(Vec::new()));
            let library: Vec<LibraryEntry> = (1..=count).map(|id| entry(id, 1, &format!("T{id}"))).collect();
            let runner = runner(FakeStore::with(library), registry);
            runner.run(settings(), Trigger::Manual).await;
            assert_eq!(runner.deps.host.queue_warnings.load(Ordering::SeqCst), warnings, "{count} queued");
        }
    }

    #[tokio::test]
    async fn downloads_start_once_and_only_when_something_was_queued() {
        // P-12
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::ok(vec![item(1.0, 0)]));
        let store = FakeStore::with(vec![entry(1, 1, "A"), entry(2, 1, "B")]);
        let runner = runner(store, registry);
        let mut with_downloads = settings();
        with_downloads.download.enabled = true;
        let outcome = runner.run(with_downloads, Trigger::Manual).await;
        assert_eq!(outcome.queued_downloads, 2);
        assert_eq!(runner.deps.host.starts.load(Ordering::SeqCst), 1);

        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::ok(vec![item(1.0, 0)]));
        let quiet = self::runner(FakeStore::with(vec![entry(1, 1, "A")]), registry);
        quiet.run(settings(), Trigger::Manual).await;
        assert_eq!(quiet.deps.host.starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn events_start_with_started_end_with_finished_and_done_only_grows() {
        // T-26a-007: PassEvent order; done is monotonic.
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::ok(vec![item(1.0, 0)]));
        registry.sources.insert(2, FakeSource::ok(vec![item(1.0, 0)]));
        let runner = runner(FakeStore::with(vec![entry(1, 1, "A"), entry(2, 2, "B")]), registry);
        let outcome = runner.run(settings(), Trigger::Manual).await;
        let events = &outcome.events;
        assert_eq!(events.first(), Some(&PassEvent::Started { total: 2 }));
        assert_eq!(events.last(), Some(&PassEvent::Finished { status: PassStatus::Success }));
        let done: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                PassEvent::EntryFinished { done, .. } => Some(*done),
                _ => None,
            })
            .collect();
        assert_eq!(done, vec![1, 2]);
        let started = events.iter().filter(|e| matches!(e, PassEvent::EntryStarted { .. })).count();
        assert_eq!(started, 2);
    }
}
