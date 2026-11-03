//! Restore screen model (area 22, T-22a-033). The file is decoded and checked off the UI thread
//! with `validate::preflight_file`, and the result goes in with `apply_check`. The GTK page renders the states, the
//! card texts and the action enable rule from here, so none of it lives in widget code.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::options::RestoreOptions;
use crate::error::BackupError;
use crate::schema::Backup;
use crate::validate::{Checked, Preflight};

use crate::text::{self, Text};

/// Status line while the check runs (ui-parity 1.3).
pub const CHECKING_TEXT: &str = text::CHECKING_BACKUP.english;
/// Missing-components card: first line (ui-parity 1.2).
pub const MISSING_HINT: &str = text::MISSING_HINT.english;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The check has not finished. The options are hidden and the action is disabled.
    Validating,
    /// The file cannot be restored. `message` is the key text of the error (D19 fix: never the internal detail).
    Invalid { path: PathBuf, message: String },
    /// The file can be restored. `missing` may list sources and trackers the user must set up first.
    Ready(Preflight),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestoreScreen {
    pub check: Check,
    pub options: RestoreOptions,
    /// The backup decoded by the check, kept for the restore job (T-22a-041). Dropped with the screen.
    backup: Option<Backup>,
}

impl Default for RestoreScreen {
    fn default() -> Self {
        Self { check: Check::Validating, options: RestoreOptions::default(), backup: None }
    }
}

impl RestoreScreen {
    /// Applies the check result. A decode error is `Invalid`; any other result is `Ready`.
    pub fn apply_check(&mut self, path: &Path, result: Result<Preflight, BackupError>) {
        self.backup = None;
        self.check = match result {
            Ok(preflight) => Check::Ready(preflight),
            Err(e) => Check::Invalid { path: path.to_path_buf(), message: e.user_text().to_owned() },
        };
    }

    /// Applies a check that also decoded the backup, so the job can reuse it (T-22a-041).
    pub fn apply_checked(&mut self, path: &Path, result: Result<Checked, BackupError>) {
        match result {
            Ok(checked) => {
                self.apply_check(path, Ok(checked.preflight));
                self.backup = Some(checked.backup);
            }
            Err(e) => self.apply_check(path, Err(e)),
        }
    }

    /// Hands the decoded backup to the restore job. `None` when the check has not run or failed, or the job already
    /// took it.
    pub fn take_backup(&mut self) -> Option<Backup> {
        self.backup.take()
    }

    /// The option checkboxes are shown only once the file is ready (ui-parity 1.3).
    pub fn options_visible(&self) -> bool {
        matches!(self.check, Check::Ready(_))
    }

    /// The Restore action is enabled iff the file is ready and at least one option is on (X-204).
    pub fn can_restore(&self) -> bool {
        self.options_visible() && self.options.can_restore()
    }

    /// Text of the error card, or `None` when there is no card. Plain text: the GTK page sets the headings in bold.
    pub fn card_text(&self) -> Option<String> {
        self.card_text_with(&|t| t.english.to_owned())
    }

    /// [`card_text`](Self::card_text) with the headings looked up by `show`, which the GTK layer backs with the
    /// catalogue (T-22a-043).
    pub fn card_text_with(&self, show: &dyn Fn(Text) -> String) -> Option<String> {
        match &self.check {
            Check::Validating => None,
            Check::Invalid { path, message } => Some(format!(
                "{}\n{}\n\n{}\n{message}",
                show(text::INVALID_BACKUP_FILE),
                path.display(),
                show(text::INVALID_BACKUP_FILE_ERROR)
            )),
            Check::Ready(p) => missing_text(p, show),
        }
    }
}

/// The one stop path of a restore (T-22a-046, behaviour 8.1). The close dialog, the notification Cancel button and the
/// banner Cancel button all call `request`. The job reads `requested` at each check point.
#[derive(Debug, Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    pub fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What closing the restore window does (T-22a-046, design 11). A running restore asks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    Close,
    /// Show the two-choice dialog: `CLOSE_CHOICES`.
    Ask,
}

/// The two choices of the close dialog while a restore runs.
pub const CLOSE_CHOICES: [&str; 2] = [text::KEEP_RUNNING.english, text::CANCEL_AND_CLOSE.english];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    KeepRunning,
    CancelAndClose,
}

/// Closing the window: at once when no restore runs, otherwise through the dialog.
pub fn on_close_requested(restore_running: bool) -> CloseAction {
    if restore_running { CloseAction::Ask } else { CloseAction::Close }
}

/// The dialog result. Both choices close the window. Cancel also requests the stop, which the job honours at its next
/// check point.
pub fn on_close_choice(choice: CloseChoice, stop: &StopFlag) -> CloseAction {
    if choice == CloseChoice::CancelAndClose {
        stop.request();
    }
    CloseAction::Close
}

/// The missing-components card (X-203). Headings are separated from the lists by a blank line; each entry is `- name`.
fn missing_text(p: &Preflight, show: &dyn Fn(Text) -> String) -> Option<String> {
    if p.missing_sources.is_empty() && p.missing_trackers.is_empty() {
        return None;
    }
    let mut lines = vec![show(text::MISSING_HINT)];
    if !p.missing_sources.is_empty() {
        lines.push(String::new());
        lines.push(show(text::MISSING_SOURCES));
        lines.extend(p.missing_sources.iter().map(|s| format!("- {s}")));
    }
    if !p.missing_trackers.is_empty() {
        lines.push(String::new());
        lines.push(show(text::TRACKERS_NOT_LOGGED_IN));
        lines.extend(p.missing_trackers.iter().map(|t| format!("- {t}")));
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(sources: &[&str], trackers: &[&str]) -> Check {
        Check::Ready(Preflight {
            missing_sources: sources.iter().map(|s| s.to_string()).collect(),
            missing_trackers: trackers.iter().map(|t| t.to_string()).collect(),
        })
    }

    fn screen(check: Check) -> RestoreScreen {
        RestoreScreen { check, ..Default::default() }
    }

    #[test]
    fn validating_hides_options_and_disables_restore() {
        let s = RestoreScreen::default();
        assert!(!s.options_visible());
        assert!(!s.can_restore());
        assert_eq!(s.card_text(), None);
    }

    #[test]
    fn invalid_card_shows_path_and_key_text() {
        let mut s = RestoreScreen::default();
        s.apply_check(Path::new("/home/u/bad file.tachibk"), Err(BackupError::Corrupt("truncated".into())));
        assert_eq!(s.check, Check::Invalid { path: "/home/u/bad file.tachibk".into(), message: "Backup file is corrupted".into() });
        assert_eq!(
            s.card_text().as_deref(),
            Some("Invalid backup file:\n/home/u/bad file.tachibk\n\nFull error:\nBackup file is corrupted")
        );
        assert!(!s.can_restore());
    }

    #[test]
    fn ready_without_missing_has_no_card_and_restores() {
        let s = screen(ready(&[], &[]));
        assert!(s.options_visible());
        assert_eq!(s.card_text(), None);
        assert!(s.can_restore());
    }

    #[test]
    fn missing_sources_only() {
        let s = screen(ready(&["Ghost", "Other"], &[]));
        assert_eq!(s.card_text().unwrap(), format!("{MISSING_HINT}\n\nMissing sources:\n- Ghost\n- Other"));
        assert!(s.can_restore(), "missing components do not block the restore");
    }

    #[test]
    fn trackers_only() {
        let s = screen(ready(&[], &["Alpha"]));
        assert_eq!(s.card_text().unwrap(), format!("{MISSING_HINT}\n\nTrackers not logged into:\n- Alpha"));
    }

    #[test]
    fn sources_and_trackers_are_separated_by_a_blank_line() {
        let s = screen(ready(&["Ghost"], &["Alpha"]));
        assert_eq!(
            s.card_text().unwrap(),
            format!("{MISSING_HINT}\n\nMissing sources:\n- Ghost\n\nTrackers not logged into:\n- Alpha")
        );
    }

    #[test]
    fn action_enable_matrix() {
        for (ready_file, any_option, expected) in [(true, true, true), (true, false, false), (false, true, false), (false, false, false)] {
            let mut s = if ready_file { screen(ready(&[], &[])) } else { RestoreScreen::default() };
            s.options = RestoreOptions::from_bits([any_option, false, false, false, false, false, false]);
            assert_eq!(s.can_restore(), expected, "ready={ready_file} any={any_option}");
        }
    }

    #[test]
    fn toggling_the_last_option_off_disables_restore() {
        let mut s = screen(ready(&[], &[]));
        s.options = RestoreOptions::from_bits([true, false, false, false, false, false, false]);
        assert!(s.options.set(0, false));
        assert!(!s.can_restore());
    }

    #[test]
    fn the_decoded_backup_is_handed_to_the_job_once_and_dropped_on_failure() {
        let mut s = RestoreScreen::default();
        let checked = Checked { backup: Backup::default(), preflight: Preflight::default() };
        s.apply_checked(Path::new("/b.tachibk"), Ok(checked.clone()));
        assert!(s.options_visible());
        assert_eq!(s.take_backup(), Some(checked.backup));
        assert_eq!(s.take_backup(), None, "the job takes it once");
        s.apply_checked(Path::new("/b.tachibk"), Err(BackupError::NoEntries));
        assert_eq!(s.take_backup(), None);
    }

    #[test]
    fn closing_during_a_restore_asks_and_cancel_uses_the_shared_stop_path() {
        assert_eq!(on_close_requested(false), CloseAction::Close);
        assert_eq!(on_close_requested(true), CloseAction::Ask);
        assert_eq!(CLOSE_CHOICES, ["Keep running", "Cancel restore and close"]);
        let stop = StopFlag::default();
        assert_eq!(on_close_choice(CloseChoice::KeepRunning, &stop), CloseAction::Close);
        assert!(!stop.requested(), "keep running does not stop the job");
        let banner = stop.clone();
        assert_eq!(on_close_choice(CloseChoice::CancelAndClose, &stop), CloseAction::Close);
        assert!(banner.requested(), "a clone sees the request, as the banner and notification do");
    }

    #[test]
    fn default_options_match_the_parity_table() {
        let s = RestoreScreen::default();
        assert_eq!(s.options.to_bits(), [true, true, true, true, true, true, false]);
        assert_eq!(RestoreOptions::LABELS, ["Library", "Categories", "App settings", "Extension stores", "Custom buttons", "Source settings", "Extensions"]);
    }
}
