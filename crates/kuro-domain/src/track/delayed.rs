//! The delayed tracking queue and its retry worker (`DelayedMangaTrackingStore.kt`, `DelayedAnimeTrackingStore.kt`,
//! `Delayed*TrackingUpdateJob.kt`; T-10a-029). Progress that could not reach a tracker waits here, keyed by track row id
//! and entry kind, and the worker replays it once the device is online.
//!
//! The replay itself (`TrackChapter` / `TrackEpisode` with `setup_job_on_failure = false`) is behind [`TrackReplay`], so the
//! worker has no tracker code. Decision: the queue is this JSON file. `kuro-track` has its own
//! `track_retry_queue` table with a different policy (doubling backoff to 5 hours, items kept after the last run); the app
//! must use one of the two, and this one follows the acceptance text.

use kuro_platform::kind::Kind;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Runs of the worker before it gives up (`runAttemptCount > 3` stops it): attempts 0 to 3.
pub const MAX_RUNS: u32 = 4;

/// Minutes slept after the first, second and third run (WorkManager exponential backoff from 5 minutes).
pub const BACKOFF_MINUTES: [u64; 3] = [5, 10, 20];

/// The file name inside the data directory.
pub const QUEUE_FILE: &str = "tracking_queue.json";

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Queue {
    #[serde(default)]
    manga: BTreeMap<i64, f32>,
    #[serde(default)]
    anime: BTreeMap<i64, f32>,
}

impl Queue {
    fn map(&mut self, kind: Kind) -> &mut BTreeMap<i64, f32> {
        match kind {
            Kind::Manga => &mut self.manga,
            Kind::Anime => &mut self.anime,
        }
    }
}

/// The queue file: `{"manga":{"12":150.5},"anime":{"3":7.0}}`. Nothing is read until the first use. Every change rewrites
/// the file through a sibling `.tmp` file and a rename, so a crash leaves either the old or the new file; a stray `.tmp`
/// is never read and the next write replaces it.
pub struct DelayedTrackingStore {
    path: PathBuf,
    state: Mutex<Option<Queue>>,
}

impl DelayedTrackingStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, state: Mutex::new(None) }
    }

    fn tmp_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    }

    /// An unreadable or missing file is an empty queue: the progress is only a retry hint, and the track row has it.
    fn load(&self) -> Queue {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write(&self, queue: &Queue) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.tmp_path();
        std::fs::write(&tmp, serde_json::to_vec(queue)?)?;
        std::fs::rename(&tmp, &self.path)
    }

    fn with<T>(&self, f: impl FnOnce(&mut Queue) -> T) -> T {
        let mut guard = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let queue = guard.get_or_insert_with(|| self.load());
        f(queue)
    }

    /// Queues `last` for the track, only when it is greater than the stored value (default 0), as an `f32` (EC-46, EC-54).
    pub fn add(&self, kind: Kind, track_id: i64, last: f64) -> io::Result<()> {
        let value = last as f32;
        self.with(|queue| {
            if value > queue.map(kind).get(&track_id).copied().unwrap_or(0.0) {
                queue.map(kind).insert(track_id, value);
                self.write(queue)
            } else {
                Ok(())
            }
        })
    }

    pub fn remove(&self, kind: Kind, track_id: i64) -> io::Result<()> {
        self.with(|queue| {
            if queue.map(kind).remove(&track_id).is_some() {
                self.write(queue)
            } else {
                Ok(())
            }
        })
    }

    /// The queued `(track id, progress)` pairs of one kind, in id order.
    pub fn items(&self, kind: Kind) -> Vec<(i64, f32)> {
        self.with(|queue| queue.map(kind).iter().map(|(id, v)| (*id, *v)).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.with(|queue| queue.manga.is_empty() && queue.anime.is_empty())
    }
}

/// What one replay of a queued item came to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReplayOutcome {
    /// Pushed (or nothing left to push). The entry leaves the queue.
    Done,
    /// The track row no longer exists. The entry leaves the queue.
    TrackMissing,
    /// The tracker is logged out. Kept until the final run, then dropped (EC-47).
    LoggedOut,
    /// Failed again. Kept.
    Failed,
}

/// The two things the worker needs from the tracking code.
#[async_trait]
pub trait TrackReplay: Send + Sync {
    /// Returns when the device is online.
    async fn wait_online(&self);
    /// `TrackChapter`/`TrackEpisode` for one queued item, with `setup_job_on_failure = false`.
    async fn replay(&self, kind: Kind, track_id: i64, progress: f32) -> ReplayOutcome;
}

struct Control {
    /// Set by `schedule` while a run is going: the backoff starts again after the in-flight item completes.
    restart: AtomicBool,
    wake: Notify,
}

/// The tokio task that replays the queue (`DelayedTrackingUpdateJob`). One task at a time.
pub struct DelayedTrackingWorker {
    store: Arc<DelayedTrackingStore>,
    replay: Arc<dyn TrackReplay>,
    control: Arc<Control>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl DelayedTrackingWorker {
    pub fn new(store: Arc<DelayedTrackingStore>, replay: Arc<dyn TrackReplay>) -> Self {
        Self {
            store,
            replay,
            control: Arc::new(Control { restart: AtomicBool::new(false), wake: Notify::new() }),
            task: Mutex::new(None),
        }
    }

    /// Starts the worker, or when it is running restarts its backoff after the in-flight item completes (`REPLACE`).
    /// Needs a tokio runtime.
    pub fn schedule(&self) {
        let mut task = self.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if task.as_ref().is_some_and(|t| !t.is_finished()) {
            self.control.restart.store(true, Ordering::SeqCst);
            self.control.wake.notify_one();
            return;
        }
        self.control.restart.store(false, Ordering::SeqCst);
        let (store, replay, control) = (Arc::clone(&self.store), Arc::clone(&self.replay), Arc::clone(&self.control));
        *task = Some(tokio::spawn(run(store, replay, control)));
    }

    /// Whether a run is going or sleeping between runs.
    pub fn is_running(&self) -> bool {
        self.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for DelayedTrackingWorker {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            task.abort();
        }
    }
}

async fn run(store: Arc<DelayedTrackingStore>, replay: Arc<dyn TrackReplay>, control: Arc<Control>) {
    let mut attempt: u32 = 0;
    'runs: loop {
        replay.wait_online().await;
        let last_run = attempt + 1 >= MAX_RUNS;
        for kind in Kind::ALL {
            for (track_id, progress) in store.items(kind) {
                let outcome = replay.replay(kind, track_id, progress).await;
                let drop_entry = match outcome {
                    ReplayOutcome::Done | ReplayOutcome::TrackMissing => true,
                    ReplayOutcome::LoggedOut => last_run,
                    ReplayOutcome::Failed => false,
                };
                if drop_entry {
                    if let Err(e) = store.remove(kind, track_id) {
                        tracing::warn!(error = %e, "cannot update the tracking queue file");
                    }
                }
                if control.restart.swap(false, Ordering::SeqCst) {
                    attempt = 0;
                    continue 'runs;
                }
            }
        }
        if store.is_empty() || last_run {
            return;
        }
        let sleep = Duration::from_secs(BACKOFF_MINUTES[attempt as usize] * 60);
        tokio::select! {
            () = tokio::time::sleep(sleep) => attempt += 1,
            () = control.wake.notified() => {
                control.restart.store(false, Ordering::SeqCst);
                attempt = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use tokio::time::Instant;

    use super::*;

    fn store() -> (tempfile::TempDir, Arc<DelayedTrackingStore>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(DelayedTrackingStore::new(dir.path().join(QUEUE_FILE)));
        (dir, store)
    }

    #[test]
    fn add_keeps_only_the_greater_progress_as_f32_and_the_kinds_stay_apart() {
        let (dir, store) = store();
        store.add(Kind::Manga, 12, 10.0).unwrap();
        store.add(Kind::Manga, 12, 9.5).unwrap();
        store.add(Kind::Manga, 12, 10.1).unwrap();
        store.add(Kind::Anime, 12, 3.0).unwrap();
        store.add(Kind::Manga, 13, 0.0).unwrap();
        assert_eq!(store.items(Kind::Manga), vec![(12, 10.1_f32)], "EC-46, EC-54: greater only, truncated to f32");
        assert_eq!(store.items(Kind::Anime), vec![(12, 3.0)], "the same id in the other kind is another entry");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join(QUEUE_FILE)).unwrap()).unwrap();
        assert_eq!(json["anime"]["12"], 3.0);
        assert!(json["manga"].is_object());
    }

    #[test]
    fn the_queue_is_read_back_from_the_file_and_a_stray_tmp_file_is_ignored_and_replaced() {
        let (dir, store) = store();
        store.add(Kind::Manga, 1, 4.0).unwrap();
        std::fs::write(dir.path().join("tracking_queue.json.tmp"), "{ broken").unwrap();
        let reopened = DelayedTrackingStore::new(dir.path().join(QUEUE_FILE));
        assert_eq!(reopened.items(Kind::Manga), vec![(1, 4.0)], "the stray tmp file is not read");
        reopened.add(Kind::Anime, 2, 1.0).unwrap();
        assert!(!dir.path().join("tracking_queue.json.tmp").exists(), "the next write replaced it");
        reopened.remove(Kind::Manga, 1).unwrap();
        assert_eq!(DelayedTrackingStore::new(dir.path().join(QUEUE_FILE)).items(Kind::Manga), vec![]);
        assert!(!reopened.is_empty());
    }

    #[test]
    fn a_missing_or_corrupt_file_is_an_empty_queue() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DelayedTrackingStore::new(dir.path().join("none.json")).is_empty());
        std::fs::write(dir.path().join("bad.json"), "not json").unwrap();
        assert!(DelayedTrackingStore::new(dir.path().join("bad.json")).is_empty());
    }

    /// Replays by a script: the outcome per track id, the times of each call, and an online switch.
    struct Script {
        outcome: Mutex<BTreeMap<i64, ReplayOutcome>>,
        calls: Mutex<Vec<(Duration, i64)>>,
        start: Instant,
        runs_waited: AtomicUsize,
    }

    impl Script {
        fn new(outcomes: &[(i64, ReplayOutcome)]) -> Arc<Self> {
            Arc::new(Self {
                outcome: Mutex::new(outcomes.iter().copied().collect()),
                calls: Mutex::default(),
                start: Instant::now(),
                runs_waited: AtomicUsize::new(0),
            })
        }
        fn minutes(&self) -> Vec<u64> {
            self.calls.lock().unwrap().iter().map(|(t, _)| t.as_secs() / 60).collect()
        }
    }

    #[async_trait]
    impl TrackReplay for Script {
        async fn wait_online(&self) {
            self.runs_waited.fetch_add(1, Ordering::SeqCst);
        }
        async fn replay(&self, _kind: Kind, track_id: i64, _progress: f32) -> ReplayOutcome {
            self.calls.lock().unwrap().push((self.start.elapsed(), track_id));
            *self.outcome.lock().unwrap().get(&track_id).unwrap_or(&ReplayOutcome::Done)
        }
    }

    async fn settle(worker: &DelayedTrackingWorker, minutes: u64) {
        for _ in 0..(minutes * 60) {
            tokio::time::advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;
            if !worker.is_running() {
                return;
            }
        }
    }

    /// EC-48: runs at 0, 5, 15 and 35 minutes, then it gives up.
    #[tokio::test(start_paused = true)]
    async fn worker_backoff_schedule() {
        let (_dir, store) = store();
        store.add(Kind::Manga, 1, 5.0).unwrap();
        let script = Script::new(&[(1, ReplayOutcome::Failed)]);
        let worker = DelayedTrackingWorker::new(store.clone(), script.clone());
        worker.schedule();
        settle(&worker, 60).await;
        assert_eq!(script.minutes(), vec![0, 5, 15, 35], "5, 10, 20 minutes between four runs");
        assert!(!worker.is_running(), "it gives up after four runs");
        assert_eq!(store.items(Kind::Manga), vec![(1, 5.0)], "a failing entry is kept");
        assert_eq!(script.runs_waited.load(Ordering::SeqCst), 4, "each run waits for the network");
    }

    #[tokio::test(start_paused = true)]
    async fn done_and_missing_entries_leave_the_queue_and_an_empty_queue_stops_the_worker() {
        let (_dir, store) = store();
        store.add(Kind::Manga, 1, 5.0).unwrap();
        store.add(Kind::Anime, 2, 6.0).unwrap();
        let script = Script::new(&[(1, ReplayOutcome::Done), (2, ReplayOutcome::TrackMissing)]);
        let worker = DelayedTrackingWorker::new(store.clone(), script.clone());
        worker.schedule();
        settle(&worker, 1).await;
        assert!(store.is_empty());
        assert_eq!(script.minutes(), vec![0, 0], "one run, no sleep");
        assert!(!worker.is_running());
    }

    /// EC-47: a logged-out entry survives the early runs and is dropped on the final one.
    #[tokio::test(start_paused = true)]
    async fn a_logged_out_entry_is_dropped_after_the_final_run_only() {
        let (_dir, store) = store();
        store.add(Kind::Manga, 1, 5.0).unwrap();
        store.add(Kind::Manga, 2, 5.0).unwrap();
        let script = Script::new(&[(1, ReplayOutcome::LoggedOut), (2, ReplayOutcome::Failed)]);
        let worker = DelayedTrackingWorker::new(store.clone(), script.clone());
        worker.schedule();
        settle(&worker, 5).await;
        assert_eq!(store.items(Kind::Manga).len(), 2, "kept after the first run");
        settle(&worker, 60).await;
        assert_eq!(store.items(Kind::Manga), vec![(2, 5.0)], "logged out dropped, failing kept");
    }

    #[tokio::test(start_paused = true)]
    async fn schedule_while_running_restarts_the_backoff() {
        let (_dir, store) = store();
        store.add(Kind::Manga, 1, 5.0).unwrap();
        let script = Script::new(&[(1, ReplayOutcome::Failed)]);
        let worker = DelayedTrackingWorker::new(store.clone(), script.clone());
        worker.schedule();
        settle(&worker, 6).await;
        assert_eq!(script.minutes(), vec![0, 5]);
        worker.schedule();
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        let at_restart = script.start.elapsed().as_secs() / 60;
        settle(&worker, 60).await;
        let minutes = script.minutes();
        assert_eq!(&minutes[..3], &[0, 5, at_restart], "a new run starts at once");
        assert_eq!(&minutes[3..], &[at_restart + 5, at_restart + 15, at_restart + 35], "and the backoff starts over");
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_schedule_on_a_running_worker_does_not_start_a_second_task() {
        let (_dir, store) = store();
        store.add(Kind::Manga, 1, 5.0).unwrap();
        let script = Script::new(&[(1, ReplayOutcome::Failed)]);
        let worker = DelayedTrackingWorker::new(store, script.clone());
        worker.schedule();
        tokio::task::yield_now().await;
        worker.schedule();
        settle(&worker, 1).await;
        assert!(worker.is_running());
        assert!(script.minutes().len() <= 2, "one task: {:?}", script.minutes());
    }
}
