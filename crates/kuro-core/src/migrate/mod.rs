//! The migration framework (T-11a-001, T-11a-027; port of `mihon.core.migration`). A migration is a one-shot step
//! that runs at start when the stored version code is lower than the running one. [`select`] picks a [`Strategy`]
//! from the stored and the running code, [`plan`] lists the steps it runs, and [`run`] runs them in order.
//!
//! Rules of the port, from `plans/11-app-migrations`:
//! * `old == 0` runs only the `Always` steps; `old >= new` runs nothing; otherwise the `Always` steps and the steps
//!   whose version is in `old + 1 ..= new` run.
//! * Steps run one after the other, `Always` first, equal versions in list order. Every step runs even when an
//!   earlier one returned `Ok(false)`. The first `Err` stops the run and is returned.
//! * `run` is `Ok(true)` when at least one step returned `Ok(true)`, and `on_complete` runs only then. In a dry run
//!   no step body runs, each step is logged with the `(Dry-run)` prefix, `on_complete` does not run, and a non-empty
//!   plan still gives `Ok(true)`.
//! * [`run_blocking`] stores `last_version_code` from `on_complete`, so the stored code moves only after a run that
//!   changed something and finished without error.

pub mod list;
pub mod steps;

#[cfg(test)]
mod idempotency;

use std::future::Future;
use std::ops::RangeInclusive;
use std::pin::Pin;
use std::sync::Arc;

use kuro_prefs::migrate::LAST_VERSION_CODE;
use kuro_prefs::store::PrefStore;

use crate::sched::JobScheduler;

/// When a step becomes due.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    /// Every start.
    Always,
    /// The first start with a version code of at least this number.
    At(u32),
}

impl Version {
    pub fn is_always(self) -> bool {
        matches!(self, Self::Always)
    }

    /// The text Kotlin's `Float.toString` gave: `-1.0`, `64.0`. Log lines keep it so they can be compared.
    pub fn log_text(self) -> String {
        match self {
            Self::Always => "-1.0".to_owned(),
            Self::At(n) => format!("{n}.0"),
        }
    }
}

/// What a step fails with.
pub type StepError = Box<dyn std::error::Error + Send + Sync>;

/// What a step returns: `Ok(true)` when it changed something or registered a schedule, `Ok(false)` when it had
/// nothing to do.
pub type StepResult = Result<bool, StepError>;

pub type MigrationFuture<'a> = Pin<Box<dyn Future<Output = StepResult> + Send + 'a>>;

/// A step.
pub struct Migration {
    /// The class name of the Kotlin step, used in the log line.
    pub name: &'static str,
    pub version: Version,
    pub run: for<'a> fn(&'a MigrationContext) -> MigrationFuture<'a>,
}

impl Migration {
    /// A step from a function. The test helper `Migration.of` of the Kotlin code.
    pub const fn of(
        name: &'static str,
        version: Version,
        run: for<'a> fn(&'a MigrationContext) -> MigrationFuture<'a>,
    ) -> Self {
        Self { name, version, run }
    }
}

/// What a step can reach. A service that a test leaves out is `None`, and the step then returns `Ok(false)` after a
/// debug line (the `?: return false` of the Kotlin steps).
pub struct MigrationContext {
    /// No step body runs and nothing is stored.
    pub dry_run: bool,
    prefs: Option<PrefStore>,
    scheduler: Option<Arc<dyn JobScheduler>>,
}

impl MigrationContext {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run, prefs: None, scheduler: None }
    }

    pub fn with_prefs(mut self, prefs: PrefStore) -> Self {
        self.prefs = Some(prefs);
        self
    }

    pub fn with_scheduler(mut self, scheduler: Arc<dyn JobScheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    /// The raw settings store, usable before any typed section exists.
    pub fn prefs(&self) -> Option<&PrefStore> {
        self.prefs.as_ref()
    }

    pub fn scheduler(&self) -> Option<&dyn JobScheduler> {
        self.scheduler.as_deref()
    }
}

/// Which steps a start runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// A new profile: only the `Always` steps.
    Initial,
    /// The same build, or a downgrade: nothing.
    Noop,
    /// An upgrade: the `Always` steps and the versions in the range.
    Range(RangeInclusive<u32>),
}

/// `MigrationStrategyFactory.create`.
pub fn select(old: u32, new: u32) -> Strategy {
    if old == 0 {
        Strategy::Initial
    } else if old >= new {
        Strategy::Noop
    } else {
        Strategy::Range(old + 1..=new)
    }
}

/// The steps `strategy` runs, in run order: `Always` first, then by version, ties in list order.
pub fn plan<'a>(strategy: &Strategy, all: &'a [Migration]) -> Vec<&'a Migration> {
    let mut steps: Vec<&Migration> = all
        .iter()
        .filter(|m| match (strategy, m.version) {
            (Strategy::Noop, _) => false,
            (_, Version::Always) => true,
            (Strategy::Initial, Version::At(_)) => false,
            (Strategy::Range(range), Version::At(n)) => range.contains(&n),
        })
        .collect();
    steps.sort_by_key(|m| m.version);
    steps
}

/// A step failed. Nothing is stored, so the same range runs again at the next start.
#[derive(Debug, thiserror::Error)]
#[error("Migration {name} failed: {source}")]
pub struct MigrationError {
    pub name: &'static str,
    pub version: Version,
    #[source]
    pub source: StepError,
}

/// Runs the steps `strategy` selects. See the module text for the rules.
pub async fn run(
    strategy: &Strategy,
    ctx: &MigrationContext,
    all: &[Migration],
    on_complete: impl FnOnce(),
) -> Result<bool, MigrationError> {
    let steps = plan(strategy, all);
    if steps.is_empty() {
        return Ok(false);
    }
    let mut any = false;
    for step in steps {
        let version = step.version.log_text();
        if ctx.dry_run {
            tracing::debug!("(Dry-run) Running migration: {{ name = {}, version = {version} }}", step.name);
            any = true;
            continue;
        }
        tracing::debug!("Running migration: {{ name = {}, version = {version} }}", step.name);
        match (step.run)(ctx).await {
            Ok(changed) => any |= changed,
            Err(source) => {
                tracing::error!("Migration failed: {} (version {version})", step.name);
                return Err(MigrationError { name: step.name, version: step.version, source });
            }
        }
    }
    if any && !ctx.dry_run {
        on_complete();
    }
    Ok(any)
}

/// The start-up call: reads the stored code, runs the migrations on `runtime`, and stores `current` when a step
/// changed something and none failed. The one `block_on` of the start-up path; call it once, before any window is
/// built. Returns whether a step changed something.
pub fn run_blocking(
    runtime: &tokio::runtime::Handle,
    store: &PrefStore,
    current: u32,
    ctx: &MigrationContext,
    migrations: &[Migration],
) -> Result<bool, MigrationError> {
    let stored = store.pref(LAST_VERSION_CODE, 0i32);
    let old = u32::try_from(stored.get()).unwrap_or(0);
    tracing::info!("Migration from {old} to {current}");
    let strategy = select(old, current);
    runtime.block_on(run(&strategy, ctx, migrations, || {
        tracing::info!("Updating last version to {current}");
        if let Err(e) = stored.set(i32::try_from(current).unwrap_or(i32::MAX)) {
            tracing::error!("Cannot store the last version: {e}");
        }
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A step that returns a fixed result, taken from a table so a plain `fn` can serve every test.
    fn always_true<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
        Box::pin(async { Ok(true) })
    }

    fn always_false<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
        Box::pin(async { Ok(false) })
    }

    fn failing<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
        Box::pin(async { Err("boom".into()) })
    }

    fn of(version: Version, run: for<'a> fn(&'a MigrationContext) -> MigrationFuture<'a>) -> Migration {
        Migration::of("Test", version, run)
    }

    fn at(n: u32, run: for<'a> fn(&'a MigrationContext) -> MigrationFuture<'a>) -> Migration {
        of(Version::At(n), run)
    }

    /// Runs `all` under `select(old, new)`, counting the `on_complete` calls.
    fn run_counting(old: u32, new: u32, all: &[Migration]) -> (Result<bool, MigrationError>, usize) {
        let done = AtomicUsize::new(0);
        let ctx = MigrationContext::new(false);
        let result = futures_block(run(&select(old, new), &ctx, all, || {
            done.fetch_add(1, Ordering::SeqCst);
        }));
        (result, done.load(Ordering::SeqCst))
    }

    fn futures_block<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().build().expect("runtime").block_on(fut)
    }

    #[test]
    fn select_picks_the_strategy_from_the_two_codes() {
        assert_eq!(select(0, 1), Strategy::Initial);
        assert_eq!(select(1, 1), Strategy::Noop);
        assert_eq!(select(5, 3), Strategy::Noop, "a downgrade is the same as the same build");
        assert_eq!(select(1, 2), Strategy::Range(2..=2));
        assert_eq!(select(1, 10), Strategy::Range(2..=10));
        assert_eq!(select(0, 99), Strategy::Initial, "a fresh profile skips the versioned steps");
    }

    #[test]
    fn initial_version_runs_only_always_steps() {
        let all = [of(Version::Always, always_true), at(2, always_false)];
        assert_eq!(plan(&select(0, 1), &all).len(), 1);
        let (result, done) = run_counting(0, 1, &all);
        assert!(result.unwrap());
        assert_eq!(done, 1);
    }

    #[test]
    fn same_version_is_noop() {
        static BODIES: AtomicUsize = AtomicUsize::new(0);
        fn counted<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async {
                BODIES.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
        let all = [of(Version::Always, counted), at(2, always_false)];
        assert!(plan(&select(1, 1), &all).is_empty());
        let (result, done) = run_counting(1, 1, &all);
        assert!(!result.unwrap());
        assert_eq!((done, BODIES.load(Ordering::SeqCst)), (0, 0), "no callback and no step body");
    }

    #[test]
    fn empty_list_returns_false_without_callback() {
        let (result, done) = run_counting(1, 2, &[]);
        assert!(!result.unwrap());
        assert_eq!(done, 0);
    }

    #[test]
    fn small_range_includes_always_and_version_two() {
        let all = [of(Version::Always, always_true), at(2, always_true)];
        assert_eq!(plan(&select(1, 2), &all).len(), 2);
        let (result, done) = run_counting(1, 2, &all);
        assert!(result.unwrap());
        assert_eq!(done, 1);
    }

    #[test]
    fn large_range_includes_all() {
        let mut all = vec![of(Version::Always, always_true)];
        all.extend((2..=10).map(|n| at(n, always_true)));
        assert_eq!(plan(&select(1, 10), &all).len(), 10);
        let (result, done) = run_counting(1, 10, &all);
        assert!(result.unwrap());
        assert_eq!(done, 1);
    }

    #[test]
    fn range_excludes_versions_above_new() {
        let all = [of(Version::Always, always_true), at(2, always_true), at(3, always_false)];
        let versions: Vec<Version> = plan(&select(1, 2), &all).iter().map(|m| m.version).collect();
        assert_eq!(versions, vec![Version::Always, Version::At(2)], "version 3 is not due yet");
        let (result, done) = run_counting(1, 2, &all);
        assert!(result.unwrap());
        assert_eq!(done, 1);
    }

    #[test]
    fn range_is_inclusive_at_the_top_and_exclusive_at_the_stored_version() {
        let all = [at(64, always_true), at(65, always_true)];
        assert_eq!(plan(&select(63, 64), &all).len(), 1, "the step at the new code runs");
        let later: Vec<Version> = plan(&select(64, 65), &all).iter().map(|m| m.version).collect();
        assert_eq!(later, vec![Version::At(65)], "the step at the stored code does not run again");
    }

    #[test]
    fn plan_is_stable_sorted_with_always_first() {
        fn first<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async { Ok(true) })
        }
        let all = [
            Migration::of("Late", Version::At(70), always_true),
            Migration::of("A", Version::At(64), first),
            Migration::of("B", Version::At(64), always_true),
            Migration::of("Every", Version::Always, always_true),
        ];
        let names: Vec<&str> = plan(&select(1, 100), &all).iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["Every", "A", "B", "Late"], "equal versions keep their list order");
    }

    #[test]
    fn a_false_step_does_not_stop_the_run_and_any_true_wins() {
        let (result, done) = run_counting(1, 3, &[at(2, always_false), at(3, always_true)]);
        assert!(result.unwrap());
        assert_eq!(done, 1);
    }

    #[test]
    fn all_false_means_no_callback() {
        let (result, done) = run_counting(1, 3, &[at(2, always_false), at(3, always_false)]);
        assert!(!result.unwrap());
        assert_eq!(done, 0);
    }

    #[test]
    fn every_step_runs_after_an_earlier_true() {
        static RAN: AtomicUsize = AtomicUsize::new(0);
        fn counted<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async {
                RAN.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
        let all = [at(2, counted), at(3, always_false), at(4, counted)];
        run_counting(1, 4, &all).0.unwrap();
        assert_eq!(RAN.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_error_stops_the_chain_and_skips_the_callback() {
        static AFTER: AtomicUsize = AtomicUsize::new(0);
        fn after<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async {
                AFTER.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
        let (result, done) = run_counting(1, 4, &[at(2, always_true), at(3, failing), at(4, after)]);
        let err = result.unwrap_err();
        assert_eq!((err.name, err.version), ("Test", Version::At(3)));
        assert!(err.to_string().contains("boom"), "{err}");
        assert_eq!(AFTER.load(Ordering::SeqCst), 0, "the third body is not called");
        assert_eq!(done, 0, "nothing is completed after a failure");
    }

    #[test]
    fn dry_run_skips_bodies_logs_the_prefix_and_returns_true() {
        static BODY: AtomicUsize = AtomicUsize::new(0);
        fn body<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async {
                BODY.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            })
        }
        let ctx = MigrationContext::new(true);
        let all = [Migration::of("SetupBackupCreateMigration", Version::Always, body), at(64, body)];
        let done = AtomicUsize::new(0);
        let (result, log) = captured(|| {
            futures_block(run(&select(1, 100), &ctx, &all, || {
                done.fetch_add(1, Ordering::SeqCst);
            }))
        });
        assert!(result.unwrap(), "a non-empty dry-run plan is Ok(true)");
        assert_eq!(BODY.load(Ordering::SeqCst), 0, "no step body runs");
        assert_eq!(done.load(Ordering::SeqCst), 0, "a dry run completes nothing");
        assert!(log.contains("(Dry-run) Running migration: { name = SetupBackupCreateMigration, version = -1.0 }"), "{log}");
        assert!(log.contains("(Dry-run) Running migration: { name = Test, version = 64.0 }"), "{log}");
    }

    #[test]
    fn the_callback_runs_after_the_last_step() {
        static FINISHED: AtomicUsize = AtomicUsize::new(0);
        fn step<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async {
                FINISHED.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
        let seen = Mutex::new(None);
        let all = [at(2, step), at(3, step)];
        let ctx = MigrationContext::new(false);
        futures_block(run(&select(1, 3), &ctx, &all, || {
            *seen.lock().unwrap() = Some(FINISHED.load(Ordering::SeqCst));
        }))
        .unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(2), "both steps had finished");
    }

    /// Captures what the subscriber logs on this thread while `f` runs.
    fn captured<T>(f: impl FnOnce() -> T) -> (T, String) {
        #[derive(Clone, Default)]
        struct Logs(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Logs {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
            type Writer = Logs;
            fn make_writer(&'a self) -> Logs {
                self.clone()
            }
        }
        let logs = Logs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, || {
            // parallel tests can cache a log statement as unwatched before this thread's subscriber arrives: rebuild the cache
            tracing::callsite::rebuild_interest_cache();
            f()
        });
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (value, text)
    }

    fn start(store: &PrefStore, current: u32, migrations: &[Migration]) -> Result<bool, MigrationError> {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let ctx = MigrationContext::new(false).with_prefs(store.clone());
        run_blocking(runtime.handle(), store, current, &ctx, migrations)
    }

    fn stored(store: &PrefStore) -> i32 {
        store.pref(LAST_VERSION_CODE, 0i32).get()
    }

    #[test]
    fn a_first_run_stores_the_version_only_when_a_step_changed_something() {
        let store = PrefStore::in_memory();
        assert!(!start(&store, 5, &[]).unwrap());
        assert!(store.get_raw(LAST_VERSION_CODE).is_none(), "no step ran, so the key is not written");
        assert!(start(&store, 5, &[of(Version::Always, always_true)]).unwrap());
        assert_eq!(stored(&store), 5);
    }

    #[test]
    fn a_downgrade_and_the_same_build_never_write_the_key() {
        let store = PrefStore::in_memory();
        store.pref(LAST_VERSION_CODE, 0i32).set(9).unwrap();
        let all = [of(Version::Always, always_true)];
        assert!(!start(&store, 3, &all).unwrap(), "a downgrade runs nothing");
        assert_eq!(stored(&store), 9);
        assert!(!start(&store, 9, &all).unwrap(), "the same build runs nothing");
        assert_eq!(stored(&store), 9);
    }

    #[test]
    fn a_failing_step_in_the_middle_does_not_store_the_new_version_and_the_range_reruns() {
        let store = PrefStore::in_memory();
        store.pref(LAST_VERSION_CODE, 0i32).set(1).unwrap();
        let all = [at(2, always_true), at(3, failing)];
        assert!(start(&store, 3, &all).is_err());
        assert_eq!(stored(&store), 1, "the stored code stays at the old value");
        assert!(start(&store, 3, &all).is_err(), "the next start tries the same range again");
    }

    #[test]
    fn a_false_run_leaves_the_stored_version_unchanged() {
        let store = PrefStore::in_memory();
        store.pref(LAST_VERSION_CODE, 0i32).set(1).unwrap();
        assert!(!start(&store, 2, &[at(2, always_false)]).unwrap());
        assert_eq!(stored(&store), 1);
    }

    #[test]
    fn a_second_launch_with_the_new_code_is_a_noop() {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        fn counted<'a>(_: &'a MigrationContext) -> MigrationFuture<'a> {
            Box::pin(async {
                RUNS.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
        let store = PrefStore::in_memory();
        let all = [of(Version::Always, counted)];
        assert!(start(&store, 4, &all).unwrap());
        assert_eq!(RUNS.load(Ordering::SeqCst), 1);
        assert!(!start(&store, 4, &all).unwrap());
        assert_eq!(RUNS.load(Ordering::SeqCst), 1, "no step executes on the second launch");
    }

    #[test]
    fn a_dry_run_stores_nothing() {
        let store = PrefStore::in_memory();
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let ctx = MigrationContext::new(true).with_prefs(store.clone());
        let result = run_blocking(runtime.handle(), &store, 4, &ctx, &[of(Version::Always, always_true)]);
        assert!(result.unwrap());
        assert!(store.get_raw(LAST_VERSION_CODE).is_none());
    }
}
