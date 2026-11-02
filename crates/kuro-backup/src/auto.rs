//! Automatic backup scheduler (area 21, T-21a-026, T-21a-031). `decide` is a pure function over the due rule,
//! deferrals and retry backoff (p1-topic-autobackup-scheduling.md section 2). `run_loop` ticks while the app is open
//! and calls `decide`; it never runs a backup unless the job slot is free.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kuro_platform::notifier::Notifier;
use tokio::sync::watch;

use crate::error::BackupError;
use crate::job::JobSlot;
use crate::notify;

/// Loop tick, equal to the 10-minute flex of the Android job (B-18).
pub const TICK: Duration = Duration::from_secs(600);
/// Base of the retry backoff and the deferral retry (B-18).
pub const RETRY_BASE: Duration = Duration::from_secs(600);
/// The first check after start waits so the library can load (topic section 2.1).
pub const FIRST_TICK_DELAY: Duration = Duration::from_secs(60);
/// Battery threshold on battery power (topic section 2.2, Proposal).
pub const DEFAULT_MIN_BATTERY_PERCENT: u8 = 15;

const HOUR_MS: i64 = 3_600_000;
const RETRY_BASE_MS: i64 = 600_000;

/// What the scheduler needs from the settings and the app state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoConfig {
    pub interval_hours: u32,
    /// `state.backup.last_auto_epoch_ms`; 0 means never.
    pub last_auto_epoch_ms: i64,
    /// `state.backup.enabled_at_epoch_ms`, the first due base when the run has never happened (E-08).
    pub enabled_at_epoch_ms: i64,
    pub min_battery_percent: u8,
}

/// The power source at one moment. `percent` is `None` when the level cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerState {
    pub on_battery: bool,
    pub percent: Option<u8>,
}

/// Failed automatic runs in a row and when the last one started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetryState {
    pub failures: u32,
    pub last_attempt_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    RestoreRunning,
    BatteryLow { percent: u8 },
    ManualRunning,
    Backoff { until_ms: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoDecision {
    Off,
    NotDue { next_at_ms: i64 },
    Defer(Reason),
    Run,
}

/// `10 min * 2^(failures - 1)`, capped at the interval (E-10).
pub fn backoff_ms(failures: u32, interval_hours: u32) -> i64 {
    let cap = i64::from(interval_hours) * HOUR_MS;
    let shift = failures.saturating_sub(1).min(62);
    RETRY_BASE_MS.saturating_mul(1i64 << shift).min(cap)
}

/// The due rule and deferrals (decision table, edge-cases section 4.6).
pub fn decide(
    now_ms: i64,
    cfg: &AutoConfig,
    restore_running: bool,
    power: PowerState,
    manual_running: bool,
    retry: &RetryState,
) -> AutoDecision {
    if cfg.interval_hours == 0 {
        return AutoDecision::Off;
    }
    // E-31: a last run in the future counts as now, so the schedule does not freeze.
    let last = cfg.last_auto_epoch_ms.min(now_ms);
    // E-08: before the first run, the due time counts from enabling.
    let base = if last == 0 { cfg.enabled_at_epoch_ms } else { last };
    let due = base.saturating_add(i64::from(cfg.interval_hours) * HOUR_MS);
    if now_ms < due {
        return AutoDecision::NotDue { next_at_ms: due };
    }
    if restore_running {
        return AutoDecision::Defer(Reason::RestoreRunning);
    }
    if manual_running {
        return AutoDecision::Defer(Reason::ManualRunning);
    }
    if let (true, Some(percent)) = (power.on_battery, power.percent) {
        if percent < cfg.min_battery_percent {
            return AutoDecision::Defer(Reason::BatteryLow { percent });
        }
    }
    if retry.failures > 0 {
        let until_ms = retry.last_attempt_ms.saturating_add(backoff_ms(retry.failures, cfg.interval_hours));
        if now_ms < until_ms {
            return AutoDecision::Defer(Reason::Backoff { until_ms });
        }
    }
    AutoDecision::Run
}

/// What the loop needs from the app. Implemented over the settings, the database and the platform.
pub trait AutoHost: Send + Sync + 'static {
    fn now_ms(&self) -> i64;
    /// Re-read on every tick, so a run started elsewhere is seen (topic section 4).
    fn last_auto_ms(&self) -> i64;
    fn restore_running(&self) -> bool;
    fn power(&self) -> PowerState;
    /// Writes one automatic backup into the automatic folder. Runs on a blocking thread.
    fn run_backup(&self) -> Result<PathBuf, BackupError>;
    /// Persists `state.backup.last_auto_epoch_ms`. Called only after a verified run (B-10).
    fn save_last_auto_ms(&self, ms: i64);
    /// Called after each failed automatic run with the failure streak length. The loop shows the notices itself
    /// (progress, and the one warning at the third failure); this is for the host's own state.
    fn report_failure(&self, streak: u32);
}

/// What one `--run-backup` invocation did (T-21a-041). The binary exits 0 for `Ran` and `Skipped`, 1 for `Failed`.
#[derive(Debug)]
pub enum OnceOutcome {
    /// A backup was written to this path.
    Ran(PathBuf),
    /// The run was not due, deferred by a rule of `decide`, or the GUI loop holds the lock.
    Skipped,
    Failed(BackupError),
}

/// The lock file of the automatic folder. The GUI loop and `--run-backup` both take it, so they never overlap (R-07).
pub const LOCK_FILE: &str = ".lock";

/// One automatic run for `kurobayomi --run-backup`. It uses the same `decide` as the loop, with no retry history, since
/// a one-shot process keeps none. A locked folder skips the run.
pub fn run_once<H: AutoHost>(cfg: &AutoConfig, host: &H, auto_dir: &Path) -> OnceOutcome {
    let lock = match fs::create_dir_all(auto_dir).and_then(|()| File::create(auto_dir.join(LOCK_FILE))) {
        Ok(file) => file,
        Err(e) => return OnceOutcome::Failed(BackupError::Io(e)),
    };
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return OnceOutcome::Skipped,
        Err(TryLockError::Error(e)) => return OnceOutcome::Failed(BackupError::Io(e)),
    }
    let now = host.now_ms();
    let mut current = cfg.clone();
    current.last_auto_epoch_ms = host.last_auto_ms();
    let decision = decide(now, &current, host.restore_running(), host.power(), false, &RetryState::default());
    if decision != AutoDecision::Run {
        return OnceOutcome::Skipped;
    }
    match host.run_backup() {
        Ok(path) => {
            host.save_last_auto_ms(now);
            OnceOutcome::Ran(path)
        }
        Err(e) => {
            host.report_failure(1);
            OnceOutcome::Failed(e)
        }
    }
}

/// Runs until the config sender is dropped. The config change wakes the loop at once (R-05). Interval 0 never runs.
pub async fn run_loop<H: AutoHost>(mut cfg: watch::Receiver<AutoConfig>, slot: JobSlot, host: Arc<H>, notifier: Arc<dyn Notifier>) {
    let mut retry = RetryState::default();
    let mut wait = FIRST_TICK_DELAY;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            changed = cfg.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
        wait = TICK;
        let mut current = cfg.borrow().clone();
        current.last_auto_epoch_ms = host.last_auto_ms();
        let now = host.now_ms();
        let decision = decide(now, &current, host.restore_running(), host.power(), slot.is_held(), &retry);
        if decision != AutoDecision::Run {
            continue;
        }
        let Some(guard) = slot.try_acquire() else {
            continue;
        };
        notify::create_progress().show(notifier.as_ref());
        let worker = Arc::clone(&host);
        let joined = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            worker.run_backup()
        })
        .await;
        let outcome = joined.unwrap_or_else(|_| Err(BackupError::Io(io::Error::other("backup worker panicked"))));
        let streak = if outcome.is_ok() { 0 } else { retry.failures + 1 };
        notify::auto_finished(notifier.as_ref(), &outcome, streak);
        match outcome {
            Ok(path) => {
                tracing::info!(file = %path.display(), "automatic backup written");
                retry = RetryState::default();
                host.save_last_auto_ms(now);
            }
            Err(e) => {
                tracing::warn!(error = %e, "automatic backup failed");
                retry.failures += 1;
                retry.last_attempt_ms = now;
                host.report_failure(retry.failures);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    const NOW: i64 = 1_000_000_000_000;
    const H: i64 = HOUR_MS;

    fn cfg(interval: u32, last: i64, enabled: i64) -> AutoConfig {
        AutoConfig { interval_hours: interval, last_auto_epoch_ms: last, enabled_at_epoch_ms: enabled, min_battery_percent: 15 }
    }

    fn ac() -> PowerState {
        PowerState { on_battery: false, percent: Some(80) }
    }

    fn bat(percent: Option<u8>) -> PowerState {
        PowerState { on_battery: true, percent }
    }

    fn no_retry() -> RetryState {
        RetryState::default()
    }

    #[test]
    fn interval_zero_is_off() {
        assert_eq!(decide(NOW, &cfg(0, 1, 1), false, ac(), false, &no_retry()), AutoDecision::Off);
    }

    #[test]
    fn never_run_is_due_at_enable_plus_interval() {
        let d = decide(NOW, &cfg(12, 0, NOW - H), false, ac(), false, &no_retry());
        assert_eq!(d, AutoDecision::NotDue { next_at_ms: NOW - H + 12 * H });
    }

    #[test]
    fn last_exactly_one_interval_ago_runs() {
        assert_eq!(decide(NOW, &cfg(6, NOW - 6 * H, 0), false, ac(), false, &no_retry()), AutoDecision::Run);
        assert_eq!(decide(NOW, &cfg(6, NOW - 6 * H + 1, 0), false, ac(), false, &no_retry()), AutoDecision::NotDue { next_at_ms: NOW + 1 });
    }

    #[test]
    fn restore_running_defers() {
        let d = decide(NOW, &cfg(6, NOW - 7 * H, 0), true, ac(), false, &no_retry());
        assert_eq!(d, AutoDecision::Defer(Reason::RestoreRunning));
    }

    #[test]
    fn battery_under_threshold_defers_and_at_threshold_runs() {
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, bat(Some(14)), false, &no_retry()), AutoDecision::Defer(Reason::BatteryLow { percent: 14 }));
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, bat(Some(15)), false, &no_retry()), AutoDecision::Run);
    }

    #[test]
    fn ac_power_ignores_low_percent() {
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, PowerState { on_battery: false, percent: Some(5) }, false, &no_retry()), AutoDecision::Run);
    }

    #[test]
    fn unknown_battery_percent_runs() {
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, bat(None), false, &no_retry()), AutoDecision::Run);
    }

    #[test]
    fn manual_running_defers() {
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, ac(), true, &no_retry()), AutoDecision::Defer(Reason::ManualRunning));
    }

    impl OnceOutcome {
        fn is_ran(&self) -> bool {
            matches!(self, OnceOutcome::Ran(_))
        }
    }

    #[test]
    fn run_once_with_a_clock_past_due_writes_exactly_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let h = host(NOW - 7 * H, false);
        let outcome = run_once(&cfg(6, 0, 0), h.as_ref(), dir.path());
        assert!(matches!(outcome, OnceOutcome::Ran(_)));
        assert_eq!(h.runs.load(Ordering::SeqCst), 1);
        assert_eq!(*h.last.lock().unwrap(), NOW, "the successful run is recorded");
        assert!(!run_once(&cfg(6, 0, 0), h.as_ref(), dir.path()).is_ran(), "not due again straight away");
        assert_eq!(h.runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn run_once_skips_when_the_folder_is_locked_or_not_due_and_fails_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let h = host(NOW - 7 * H, false);
        let held = File::create(dir.path().join(LOCK_FILE)).unwrap();
        held.lock().unwrap();
        assert!(matches!(run_once(&cfg(6, 0, 0), h.as_ref(), dir.path()), OnceOutcome::Skipped));
        drop(held);
        assert_eq!(h.runs.load(Ordering::SeqCst), 0, "a locked folder runs nothing");
        let fresh = host(NOW - H, false);
        assert!(matches!(run_once(&cfg(6, NOW - H, 0), fresh.as_ref(), dir.path()), OnceOutcome::Skipped));
        let broken = host(NOW - 7 * H, true);
        assert!(matches!(run_once(&cfg(6, 0, 0), broken.as_ref(), dir.path()), OnceOutcome::Failed(_)));
        assert_eq!(*broken.last.lock().unwrap(), NOW - 7 * H, "a failed run does not advance the last run");
    }

    #[test]
    fn backoff_attempt_three_waits_forty_minutes() {
        let retry = RetryState { failures: 3, last_attempt_ms: NOW - 30 * 60_000 };
        let d = decide(NOW, &cfg(6, NOW - 7 * H, 0), false, ac(), false, &retry);
        assert_eq!(d, AutoDecision::Defer(Reason::Backoff { until_ms: NOW - 30 * 60_000 + 40 * 60_000 }));
        let retry = RetryState { failures: 3, last_attempt_ms: NOW - 41 * 60_000 };
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, ac(), false, &retry), AutoDecision::Run);
    }

    #[test]
    fn backoff_is_capped_at_interval() {
        assert_eq!(backoff_ms(10, 6), 6 * H);
        assert_eq!(backoff_ms(1, 6), 10 * 60_000);
        assert_eq!(backoff_ms(2, 168), 20 * 60_000);
    }

    #[test]
    fn future_last_run_counts_as_now() {
        let d = decide(NOW, &cfg(6, NOW + 5 * H, 0), false, ac(), false, &no_retry());
        assert_eq!(d, AutoDecision::NotDue { next_at_ms: NOW + 6 * H });
    }

    #[test]
    fn interval_cut_while_due_runs_on_next_tick() {
        assert_eq!(decide(NOW, &cfg(12, NOW - 7 * H, 0), false, ac(), false, &no_retry()), AutoDecision::NotDue { next_at_ms: NOW - 7 * H + 12 * H });
        assert_eq!(decide(NOW, &cfg(6, NOW - 7 * H, 0), false, ac(), false, &no_retry()), AutoDecision::Run);
    }

    #[test]
    fn invalid_interval_three_loads_as_twelve() {
        assert_eq!(crate::settings::normalize_interval(3), 12);
    }

    #[test]
    fn restore_check_comes_after_due_check() {
        assert_eq!(decide(NOW, &cfg(6, NOW, 0), true, ac(), false, &no_retry()), AutoDecision::NotDue { next_at_ms: NOW + 6 * H });
    }

    struct FakeHost {
        last: Mutex<i64>,
        runs: AtomicU32,
        fail: bool,
        failures: AtomicU32,
    }

    impl AutoHost for FakeHost {
        fn now_ms(&self) -> i64 {
            NOW
        }
        fn last_auto_ms(&self) -> i64 {
            *self.last.lock().unwrap()
        }
        fn restore_running(&self) -> bool {
            false
        }
        fn power(&self) -> PowerState {
            ac()
        }
        fn run_backup(&self) -> Result<PathBuf, BackupError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            if self.fail { Err(BackupError::CreateFile) } else { Ok(PathBuf::from("/x.tachibk")) }
        }
        fn save_last_auto_ms(&self, ms: i64) {
            *self.last.lock().unwrap() = ms;
        }
        fn report_failure(&self, streak: u32) {
            self.failures.store(streak, Ordering::SeqCst);
        }
    }

    fn quiet() -> Arc<dyn Notifier> {
        Arc::new(crate::notify::recorder::Recorder::default())
    }

    fn host(last: i64, fail: bool) -> Arc<FakeHost> {
        Arc::new(FakeHost { last: Mutex::new(last), runs: AtomicU32::new(0), fail, failures: AtomicU32::new(0) })
    }

    #[tokio::test(start_paused = true)]
    async fn loop_runs_once_when_due_and_not_again_until_next_interval() {
        let h = host(NOW - 7 * H, false);
        let (tx, rx) = watch::channel(cfg(6, 0, 0));
        let slot = JobSlot::new();
        let task = tokio::spawn(run_loop(rx, slot, Arc::clone(&h), quiet()));
        tokio::time::sleep(Duration::from_secs(3600)).await;
        drop(tx);
        task.await.unwrap();
        assert_eq!(h.runs.load(Ordering::SeqCst), 1);
        assert_eq!(*h.last.lock().unwrap(), NOW);
    }

    #[tokio::test(start_paused = true)]
    async fn interval_zero_never_runs() {
        let h = host(0, false);
        let (tx, rx) = watch::channel(cfg(0, 0, 0));
        let task = tokio::spawn(run_loop(rx, JobSlot::new(), Arc::clone(&h), quiet()));
        tokio::time::sleep(Duration::from_secs(7 * 24 * 3600)).await;
        drop(tx);
        task.await.unwrap();
        assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_run_keeps_last_and_reports_streak() {
        let h = host(NOW - 7 * H, true);
        let (tx, rx) = watch::channel(cfg(6, 0, 0));
        let task = tokio::spawn(run_loop(rx, JobSlot::new(), Arc::clone(&h), quiet()));
        tokio::time::sleep(Duration::from_secs(120)).await;
        drop(tx);
        task.await.unwrap();
        assert_eq!(h.runs.load(Ordering::SeqCst), 1);
        assert_eq!(*h.last.lock().unwrap(), NOW - 7 * H, "E-10: a failed run does not advance last");
        assert_eq!(h.failures.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn held_slot_skips_the_tick() {
        let h = host(NOW - 7 * H, false);
        let slot = JobSlot::new();
        let _manual = slot.try_acquire().unwrap();
        let (tx, rx) = watch::channel(cfg(6, 0, 0));
        let task = tokio::spawn(run_loop(rx, slot.clone(), Arc::clone(&h), quiet()));
        tokio::time::sleep(Duration::from_secs(3600)).await;
        drop(tx);
        task.await.unwrap();
        assert_eq!(h.runs.load(Ordering::SeqCst), 0, "auto tick is skipped while a manual run holds the slot");
    }

    #[tokio::test(start_paused = true)]
    async fn run_shows_progress_and_closes_it_for_success_and_failure() {
        use crate::notify::{ID_BACKUP_PROGRESS, recorder::Recorder};
        for fail in [false, true] {
            let h = host(NOW - 7 * H, fail);
            let rec = Arc::new(Recorder::default());
            let (tx, rx) = watch::channel(cfg(6, 0, 0));
            let task = tokio::spawn(run_loop(rx, JobSlot::new(), Arc::clone(&h), rec.clone()));
            tokio::time::sleep(Duration::from_secs(120)).await;
            drop(tx);
            task.await.unwrap();
            let calls = rec.calls.lock().unwrap();
            assert_eq!(calls[0], format!("show:{ID_BACKUP_PROGRESS}"));
            assert_eq!(calls[1], format!("withdraw:{ID_BACKUP_PROGRESS}"));
            assert_eq!(rec.shown.lock().unwrap().len(), 1, "automatic runs are silent apart from the progress notice");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_ticks_show_no_progress() {
        use crate::notify::recorder::Recorder;
        let h = host(NOW, false);
        let rec = Arc::new(Recorder::default());
        let (tx, rx) = watch::channel(cfg(6, 0, 0));
        let task = tokio::spawn(run_loop(rx, JobSlot::new(), Arc::clone(&h), rec.clone()));
        tokio::time::sleep(Duration::from_secs(3600)).await;
        drop(tx);
        task.await.unwrap();
        assert!(rec.calls.lock().unwrap().is_empty());
    }
}
