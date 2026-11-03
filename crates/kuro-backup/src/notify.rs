//! Backup and restore notices (area 21, T-21a-028, T-21a-029, T-21a-030; B-15). Builders are pure:
//! they return a `Notice` that the caller shows through `kuro_platform::notifier::Notifier`, with its click and its
//! "Open folder" button ([`Notice::desktop`]).

use std::path::{Path, PathBuf};

use kuro_platform::notifier::{self as platform, APP_NAME, Notifier, Tap, send};

use crate::error::BackupError;
use crate::restore::job::RestoreEnd;

/// The button that shows a created backup in the file manager.
pub const OPEN_FOLDER: &str = "Open folder";

/// The body of a restore notice while `hide_notification_content` is on.
pub const GENERIC_RESTORE_TEXT: &str = "Restore in progress";
pub const ID_BACKUP_PROGRESS: &str = "kurobayomi-backup-progress";
pub const ID_BACKUP_RESULT: &str = "kurobayomi-backup-result";
pub const ID_AUTO_WARNING: &str = "kurobayomi-auto-backup-warning";
/// Restore ids: progress and every final notice (complete, canceled, failed) share one id,
/// so the final notice replaces the progress one.
pub const ID_RESTORE_PROGRESS: &str = "restore-progress";
pub const ID_RESTORE_RESULT: &str = "restore-complete";
/// Consecutive failed automatic runs before the one warning (E-10).
pub const FAILURE_STREAK_WARNING: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    /// Restore completed with errors, restore failed.
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    ShowInFolder(PathBuf),
    OpenErrorLog(PathBuf),
    OpenSettings,
    CancelRestore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: &'static str,
    pub title: String,
    /// `None` when the body must not show (restore content with `hide_notification_content`).
    pub body: Option<String>,
    pub urgency: Urgency,
    pub action: Option<Action>,
    /// `(done, total)` for a determinate restore bar; `None` for indeterminate or final notices.
    pub progress: Option<(u32, u32)>,
    /// The notice names library content (T-01a-034): `show_hiding` replaces it when `hide_notification_content` is on.
    pub sensitive: bool,
}

impl Notice {
    fn new(id: &'static str, title: impl Into<String>, body: Option<String>, urgency: Urgency) -> Self {
        Self { id, title: title.into(), body, urgency, action: None, progress: None, sensitive: false }
    }

    /// The desktop notice: a created backup is revealed by a click and by "Open folder", an error log by a click. The
    /// settings and cancel actions are left to their own screens.
    pub fn desktop(&self) -> platform::Notice {
        let (tap, buttons) = match &self.action {
            Some(Action::ShowInFolder(path)) => (Some(Tap::Reveal(path.clone())), vec![(OPEN_FOLDER.to_owned(), Tap::Reveal(path.clone()))]),
            Some(Action::OpenErrorLog(path)) => (Some(Tap::Reveal(path.clone())), Vec::new()),
            Some(Action::OpenSettings | Action::CancelRestore) | None => (None, Vec::new()),
        };
        platform::Notice {
            id: self.id.to_owned(),
            title: self.title.clone(),
            body: self.body.clone(),
            low_priority: self.urgency == Urgency::Low,
            tap,
            buttons,
            timeout: None,
        }
    }

    /// Shows the notice. A missing daemon is the platform's failure to log; it never fails the job (E-22).
    pub fn show(&self, notifier: &dyn Notifier) {
        notifier.notify(&self.desktop());
    }

    /// Shows the notice, or only the app name and a generic text when it is sensitive and `hide_content` is set.
    pub fn show_hiding(&self, notifier: &dyn Notifier, hide_content: bool) {
        if hide_content && self.sensitive {
            send(notifier, hide_content, self.sensitive, self.id, APP_NAME, &self.title, "", GENERIC_RESTORE_TEXT);
        } else {
            self.show(notifier);
        }
    }
}

/// Closes the progress notice, then shows the manual result: "Backup created" or "Backup failed".
pub fn manual_finished(notifier: &dyn Notifier, result: &Result<PathBuf, BackupError>) {
    manual_finished_report(notifier, result, 0);
}

/// [`manual_finished`] with the number of extension packages that were skipped, which the success notice names.
pub fn manual_finished_report(notifier: &dyn Notifier, result: &Result<PathBuf, BackupError>, skipped: usize) {
    notifier.withdraw(ID_BACKUP_PROGRESS);
    match result {
        Ok(path) => manual_created_skipping(path, skipped).show(notifier),
        Err(e) => manual_failed(e).show(notifier),
    }
}

/// Closes the progress notice. An automatic run is silent on success; a failure shows the warning only at the
/// third consecutive failure (`streak`).
pub fn auto_finished(notifier: &dyn Notifier, result: &Result<PathBuf, BackupError>, streak: u32) {
    notifier.withdraw(ID_BACKUP_PROGRESS);
    if result.is_err() {
        if let Some(warning) = failure_warning(streak) {
            warning.show(notifier);
        }
    }
}

/// "Creating backup", indeterminate, shown at the start of every run.
pub fn create_progress() -> Notice {
    Notice::new(ID_BACKUP_PROGRESS, "Creating backup", None, Urgency::Low)
}

/// Manual success: "Backup created" with the display path and "Show in folder".
pub fn manual_created(path: &Path) -> Notice {
    let mut n = Notice::new(ID_BACKUP_RESULT, "Backup created", Some(path.display().to_string()), Urgency::Normal);
    n.action = Some(Action::ShowInFolder(path.to_path_buf()));
    n
}

/// Manual success naming the skipped extension packages: a second body line "N extensions were skipped" (T-21a-019).
pub fn manual_created_skipping(path: &Path, skipped: usize) -> Notice {
    let mut n = manual_created(path);
    if skipped > 0 {
        let noun = if skipped == 1 { "extension was" } else { "extensions were" };
        n.body = Some(format!("{}\n{skipped} {noun} skipped", path.display()));
    }
    n
}

/// Manual failure: "Backup failed" with the key text, never the internal detail.
pub fn manual_failed(error: &BackupError) -> Notice {
    Notice::new(ID_BACKUP_RESULT, "Backup failed", Some(error.user_text().to_owned()), Urgency::Normal)
}

/// Automatic failure: silent until the third consecutive failure, then one warning per streak (E-10).
pub fn failure_warning(streak: u32) -> Option<Notice> {
    if streak != FAILURE_STREAK_WARNING {
        return None;
    }
    let mut n = Notice::new(
        ID_AUTO_WARNING,
        "Automatic backup failed",
        Some("Check the backup folder and free space.".into()),
        Urgency::Normal,
    );
    n.action = Some(Action::OpenSettings);
    Some(n)
}

/// Restore progress: "Restoring backup" or "Syncing library", with the body hidden when asked (B-15, T-21a-030).
pub fn restore_progress(content: &str, done: u32, total: u32, sync: bool, hide_content: bool) -> Notice {
    let title = if sync { "Syncing library" } else { "Restoring backup" };
    let body = (!hide_content).then(|| content.to_owned());
    let mut n = Notice::new(ID_RESTORE_PROGRESS, title, body, Urgency::Low);
    n.action = Some(Action::CancelRestore);
    n.progress = Some((done, total));
    n.sensitive = true;
    n
}

pub fn restore_failed(reason: &str) -> Notice {
    Notice::new(ID_RESTORE_RESULT, "Restoring backup failed", Some(reason.to_owned()), Urgency::High)
}

/// Canceled restore: the failure title with "Canceled restore". No summary is shown.
pub fn restore_canceled() -> Notice {
    Notice::new(ID_RESTORE_RESULT, "Restoring backup failed", Some("Canceled restore".into()), Urgency::Normal)
}

/// The completion notice for a finished run: the summary when it completed, the canceled notice otherwise.
pub fn restore_end(end: &RestoreEnd) -> Notice {
    match end {
        RestoreEnd::Completed(s) => restore_done(
            s.elapsed_ms,
            u32::try_from(s.errors).unwrap_or(u32::MAX),
            u32::try_from(s.skipped_settings).unwrap_or(u32::MAX),
            s.error_log.as_deref(),
        ),
        RestoreEnd::Canceled { .. } => restore_canceled(),
    }
}

/// Restore finished: "Restore completed", with the summary line. "Tap to see details" appears only when there are
/// errors and a log exists (T-21a-029). The sync variant is dead (no caller sets sync, D21), so it has no parameter.
pub fn restore_done(elapsed_ms: i64, errors: u32, skipped: u32, log: Option<&Path>) -> Notice {
    let title = "Restore completed";
    let urgency = if errors > 0 { Urgency::High } else { Urgency::Normal };
    let mut n = Notice::new(ID_RESTORE_RESULT, title, Some(done_text(elapsed_ms, errors, skipped)), urgency);
    if let (true, Some(log)) = (errors > 0, log) {
        n.action = Some(Action::OpenErrorLog(log.to_path_buf()));
    }
    n
}

/// `%1$02d min, %2$02d sec` (B-15).
pub fn restore_duration(elapsed_ms: i64) -> String {
    let total_secs = elapsed_ms.max(0) / 1000;
    format!("{:02} min, {:02} sec", total_secs / 60, total_secs % 60)
}

/// "Done in %1$s with %2$s error" for one error, "errors" for any other count (English plural rule).
/// The completion text. The skip count is named only when it is not zero (T-22a-052, PROPOSAL wording).
pub fn done_text(elapsed_ms: i64, errors: u32, skipped: u32) -> String {
    let time = restore_duration(elapsed_ms);
    let noun = if errors == 1 { "error" } else { "errors" };
    let base = format!("Done in {time} with {errors} {noun}");
    match skipped {
        0 => base,
        1 => format!("{base}, 1 setting skipped"),
        n => format!("{base}, {n} settings skipped"),
    }
}

/// Notifier that records its calls, shared by the tests of the job and the loop.
#[cfg(test)]
pub(crate) mod recorder {
    use std::sync::Mutex;

    use kuro_platform::notifier::Notifier;

    #[derive(Default)]
    pub struct Recorder {
        pub shown: Mutex<Vec<(String, String, Option<String>)>>,
        pub withdrawn: Mutex<Vec<String>>,
        /// "show:<id>" and "withdraw:<id>" in call order.
        pub calls: Mutex<Vec<String>>,
    }

    impl Notifier for Recorder {
        fn show(&self, id: &str, title: &str, body: Option<&str>) {
            self.calls.lock().unwrap().push(format!("show:{id}"));
            self.shown.lock().unwrap().push((id.into(), title.into(), body.map(str::to_owned)));
        }
        fn withdraw(&self, id: &str) {
            self.calls.lock().unwrap().push(format!("withdraw:{id}"));
            self.withdrawn.lock().unwrap().push(id.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::recorder::Recorder;
    use super::*;

    #[test]
    fn manual_finish_closes_progress_then_reports() {
        let rec = Recorder::default();
        manual_finished(&rec, &Ok(PathBuf::from("/d/a.tachibk")));
        manual_finished(&rec, &Err(BackupError::CreateFile));
        assert_eq!(*rec.withdrawn.lock().unwrap(), vec![ID_BACKUP_PROGRESS.to_string(), ID_BACKUP_PROGRESS.to_string()]);
        let shown = rec.shown.lock().unwrap();
        assert_eq!(shown[0].1, "Backup created");
        assert_eq!(shown[1].1, "Backup failed");
    }

    #[test]
    fn automatic_success_is_silent_and_failure_warns_on_third() {
        let rec = Recorder::default();
        auto_finished(&rec, &Ok(PathBuf::from("/d/a.tachibk")), 0);
        assert!(rec.shown.lock().unwrap().is_empty());
        auto_finished(&rec, &Err(BackupError::CreateFile), 2);
        assert!(rec.shown.lock().unwrap().is_empty());
        auto_finished(&rec, &Err(BackupError::CreateFile), 3);
        assert_eq!(rec.shown.lock().unwrap()[0].0, ID_AUTO_WARNING);
    }

    #[test]
    fn restore_duration_format() {
        assert_eq!(restore_duration(65_000), "01 min, 05 sec");
        assert_eq!(restore_duration(0), "00 min, 00 sec");
        assert_eq!(restore_duration(3_599_999), "59 min, 59 sec");
    }

    #[test]
    fn plural_errors() {
        assert_eq!(done_text(65_000, 1, 0), "Done in 01 min, 05 sec with 1 error");
        assert_eq!(done_text(65_000, 2, 0), "Done in 01 min, 05 sec with 2 errors");
        assert_eq!(done_text(0, 0, 0), "Done in 00 min, 00 sec with 0 errors");
        assert_eq!(done_text(0, 0, 1), "Done in 00 min, 00 sec with 0 errors, 1 setting skipped");
        assert_eq!(done_text(0, 0, 3), "Done in 00 min, 00 sec with 0 errors, 3 settings skipped");
    }

    #[test]
    fn hide_content_is_cut_by_the_settings_diet_and_hides_the_restore_title_when_true() {
        assert!(restore_progress("One Piece", 1, 2, false, false).body.is_some(), "the default is off");
        assert_eq!(restore_progress("One Piece", 1, 2, false, true).body, None);
    }

    #[test]
    fn hide_content_restore_only() {
        let hidden = restore_progress("One Piece", 3, 10, false, true);
        assert_eq!(hidden.body, None);
        assert_eq!(hidden.title, "Restoring backup");
        let shown = restore_progress("One Piece", 3, 10, true, false);
        assert_eq!(shown.body.as_deref(), Some("One Piece"));
        assert_eq!(shown.title, "Syncing library");
        // Backup texts never depend on the setting.
        assert_eq!(create_progress().body, None);
        assert_eq!(failure_warning(3).unwrap().body.as_deref(), Some("Check the backup folder and free space."));
    }

    #[test]
    fn a_sensitive_notice_shown_hiding_loses_its_content() {
        let rec = Recorder::default();
        let progress = restore_progress("One Piece", 3, 10, false, false);
        progress.show_hiding(&rec, true);
        progress.show_hiding(&rec, false);
        manual_failed(&BackupError::CreateFile).show_hiding(&rec, true);
        let shown = rec.shown.lock().unwrap();
        assert_eq!(shown[0].1, APP_NAME);
        assert_eq!(shown[0].2.as_deref(), Some(GENERIC_RESTORE_TEXT));
        assert_eq!(shown[1].1, "Restoring backup");
        assert_eq!(shown[1].2.as_deref(), Some("One Piece"));
        assert_eq!(shown[2].1, "Backup failed", "a notice that is not sensitive is never hidden");
    }

    #[test]
    fn details_action_only_with_errors_and_log() {
        let log = Path::new("/home/u/log.txt");
        assert_eq!(restore_done(1, 2, 0, Some(log)).action, Some(Action::OpenErrorLog(log.into())));
        assert_eq!(restore_done(1, 0, 0, Some(log)).action, None);
        assert_eq!(restore_done(1, 2, 0, None).action, None);
    }

    #[test]
    fn restore_ids_and_urgency_follow_design_section_8() {
        assert_eq!((ID_RESTORE_PROGRESS, ID_RESTORE_RESULT), ("restore-progress", "restore-complete"));
        assert_eq!(restore_progress("x", 1, 2, false, false).urgency, Urgency::Low);
        assert_eq!(restore_done(1, 0, 0, None).urgency, Urgency::Normal);
        assert_eq!(restore_done(1, 2, 0, None).urgency, Urgency::High);
        assert_eq!(restore_canceled().urgency, Urgency::Normal);
        assert_eq!(restore_failed("x").urgency, Urgency::High);
    }

    #[test]
    fn automatic_warning_only_on_third_failure() {
        assert!(failure_warning(1).is_none());
        assert!(failure_warning(2).is_none());
        assert_eq!(failure_warning(3).unwrap().title, "Automatic backup failed");
        assert!(failure_warning(4).is_none());
    }

    #[test]
    fn canceled_restore_uses_the_failure_title_and_text() {
        let n = restore_canceled();
        assert_eq!((n.id, n.title.as_str(), n.body.as_deref()), (ID_RESTORE_RESULT, "Restoring backup failed", Some("Canceled restore")));
        assert_eq!(n.action, None);
    }

    #[test]
    fn end_notice_follows_the_run_outcome() {
        let log = PathBuf::from("/home/u/aniyomi_restore_error.txt");
        let done = restore_end(&RestoreEnd::Completed(crate::restore::job::Summary { elapsed_ms: 65_000, errors: 1, error_log: Some(log.clone()), skipped_settings: 0, skipped_extensions: 0 }));
        assert_eq!(done.title, "Restore completed");
        assert_eq!(done.body.as_deref(), Some("Done in 01 min, 05 sec with 1 error"));
        assert_eq!(done.action, Some(Action::OpenErrorLog(log)));
        let canceled = restore_end(&RestoreEnd::Canceled { error_log: None });
        assert_eq!(canceled.title, "Restoring backup failed");
    }

    #[test]
    fn manual_created_has_show_in_folder() {
        let p = Path::new("/home/u/kurobayomi_2026-10-09_12-00-00.tachibk");
        let n = manual_created(p);
        assert_eq!(n.title, "Backup created");
        assert_eq!(n.action, Some(Action::ShowInFolder(p.into())));
        assert_eq!(n.body.as_deref(), Some("/home/u/kurobayomi_2026-10-09_12-00-00.tachibk"));
    }

    #[test]
    fn the_completion_notice_names_the_skipped_extensions_only_when_there_are_some() {
        let p = Path::new("/home/u/a.tachibk");
        assert_eq!(manual_created_skipping(p, 0), manual_created(p));
        assert_eq!(manual_created_skipping(p, 1).body.as_deref(), Some("/home/u/a.tachibk\n1 extension was skipped"));
        assert_eq!(manual_created_skipping(p, 3).body.as_deref(), Some("/home/u/a.tachibk\n3 extensions were skipped"));
        let rec = Recorder::default();
        manual_finished_report(&rec, &Ok(p.to_path_buf()), 2);
        assert_eq!(rec.shown.lock().unwrap()[0].2.as_deref(), Some("/home/u/a.tachibk\n2 extensions were skipped"));
    }

    #[test]
    fn manual_failure_uses_key_text() {
        let n = manual_failed(&BackupError::CreateFile);
        assert_eq!(n.body.as_deref(), Some("Couldn't create a backup file"));
    }

    #[test]
    fn progress_reuses_one_id_and_shows_to_platform() {
        let rec = Recorder::default();
        create_progress().show(&rec);
        restore_progress("x", 1, 2, false, false).show(&rec);
        let shown = rec.shown.lock().unwrap();
        assert_eq!(shown[0].0, ID_BACKUP_PROGRESS);
        assert_eq!(shown[1].1, "Restoring backup");
        assert_eq!(shown[1].2.as_deref(), Some("x"));
    }

    #[test]
    fn a_created_backup_is_revealed_by_a_click_and_by_open_folder_and_progress_is_quiet() {
        // T-35a-024
        let path = Path::new("/home/u/backups/kurobayomi-2026.tachibk");
        let created = manual_created(path).desktop();
        let reveal = kuro_platform::notifier::Tap::Reveal(path.to_path_buf());
        assert_eq!((created.id.as_str(), created.title.as_str()), (ID_BACKUP_RESULT, "Backup created"));
        assert_eq!(created.tap, Some(reveal.clone()));
        assert_eq!(created.buttons, vec![(OPEN_FOLDER.to_owned(), reveal)]);
        assert!(!created.low_priority);
        let progress = create_progress().desktop();
        assert!(progress.low_priority && progress.buttons.is_empty() && progress.tap.is_none());
        let log = Path::new("/cache/restore.txt");
        assert_eq!(restore_done(1000, 2, 0, Some(log)).desktop().tap, Some(kuro_platform::notifier::Tap::Reveal(log.to_path_buf())));
    }
}
