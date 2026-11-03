//! How a restore run ends (area 22, T-22a-032). A completed run reports its summary. A
//! canceled run reports `Canceled` and still writes the error log of the errors collected so far (F-03, D17 fix).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use super::RestoreError;
use super::driver::{Targets, restore_backup};
use super::error_log::write_error_log;
use super::options::RestoreOptions;
use super::progress::ProgressEvent;
use super::sections::Run;
use crate::schema::Backup;

/// Summary of a completed run. `error_log` is `None` when there were no errors or the log could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub elapsed_ms: i64,
    pub errors: usize,
    pub error_log: Option<PathBuf>,
    /// Backup preference keys the allowlist skipped (T-22a-052).
    pub skipped_settings: usize,
    /// APK extension payloads that were skipped, not installed (T-22a-029). They are not errors.
    pub skipped_extensions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreEnd {
    Completed(Summary),
    Canceled { error_log: Option<PathBuf> },
}

/// Runs the restore and concludes it. `log_dir` is the folder that holds the error log.
pub fn run_backup(
    backup: Backup,
    opts: RestoreOptions,
    now: DateTime<Local>,
    targets: Targets<'_>,
    cancel: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(ProgressEvent),
    log_dir: &Path,
) -> RestoreEnd {
    let started = Instant::now();
    let run = restore_backup(backup, opts, now, targets, cancel, emit);
    conclude(&run, started.elapsed(), log_dir)
}

/// Writes the error log (a write failure is logged as a warning, D15) and picks the end state.
pub fn conclude(run: &Run, elapsed: Duration, log_dir: &Path) -> RestoreEnd {
    let error_log = match write_error_log(log_dir, &run.errors) {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!("restore error log not written: {e}");
            None
        }
    };
    match run.stop {
        Some(RestoreError::Cancelled) => RestoreEnd::Canceled { error_log },
        None => RestoreEnd::Completed(Summary {
            elapsed_ms: i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
            errors: run.errors.len(),
            error_log,
            skipped_settings: run.skipped_settings,
            skipped_extensions: run.skipped_extensions,
        }),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};

    use super::*;
    use crate::restore::sections::ErrorLine;

    fn line(message: &str) -> ErrorLine {
        ErrorLine { at: Local.with_ymd_and_hms(2026, 10, 9, 9, 0, 0).single().unwrap(), message: message.into() }
    }

    #[test]
    fn cancel_writes_the_errors_so_far_and_reports_canceled() {
        let dir = tempfile::tempdir().unwrap();
        let run = Run { errors: vec![line("Berserk [Mock]: boom")], stop: Some(RestoreError::Cancelled), skipped_settings: 0, skipped_extensions: 0 };
        let end = conclude(&run, Duration::from_secs(3), dir.path());
        let RestoreEnd::Canceled { error_log: Some(path) } = end else { panic!("expected Canceled with a log") };
        assert!(std::fs::read_to_string(path).unwrap().contains("Berserk [Mock]: boom"));
    }

    #[test]
    fn cancel_without_errors_has_no_log() {
        let dir = tempfile::tempdir().unwrap();
        let run = Run { errors: vec![], stop: Some(RestoreError::Cancelled), skipped_settings: 0, skipped_extensions: 0 };
        assert_eq!(conclude(&run, Duration::ZERO, dir.path()), RestoreEnd::Canceled { error_log: None });
    }

    #[test]
    fn completed_run_reports_elapsed_count_and_log() {
        let dir = tempfile::tempdir().unwrap();
        let run = Run { errors: vec![line("a"), line("b")], stop: None, skipped_settings: 0, skipped_extensions: 0 };
        let RestoreEnd::Completed(s) = conclude(&run, Duration::from_millis(65_000), dir.path()) else { panic!("expected Completed") };
        assert_eq!((s.elapsed_ms, s.errors), (65_000, 2));
        assert_eq!(s.error_log, Some(dir.path().join(crate::restore::error_log::LOG_FILE)));
    }

    #[test]
    fn unwritable_log_still_concludes_without_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let run = Run { errors: vec![line("a")], stop: None, skipped_settings: 0, skipped_extensions: 0 };
        let RestoreEnd::Completed(s) = conclude(&run, Duration::ZERO, &blocker.join("sub")) else { panic!("expected Completed") };
        assert_eq!((s.errors, s.error_log), (1, None));
    }
}
