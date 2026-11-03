//! Ordered section execution (area 22, T-22a-007, fixes D3). Sections run one after another in `ORDER`, each item
//! waits for the one before, and the cancel flag is checked before every section and every item.

use chrono::{DateTime, Local};

use super::RestoreError;
use super::options::RestoreOptions;
use super::progress::{Progress, ProgressEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Categories,
    AppSettings,
    SourceSettings,
    AnimeEntries,
    MangaEntries,
    AnimeStores,
    MangaRepos,
    Extensions,
}

/// Run order. Categories first, so entries see the created categories (R-01, R-02).
pub const ORDER: [Section; 8] = [
    Section::Categories,
    Section::AppSettings,
    Section::SourceSettings,
    Section::AnimeEntries,
    Section::MangaEntries,
    Section::AnimeStores,
    Section::MangaRepos,
    Section::Extensions,
];

/// Item counts read from the decoded backup (seasons included in the entry counts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub anime_entries: u32,
    pub manga_entries: u32,
    pub anime_stores: u32,
    pub manga_repos: u32,
}

/// Items per section for these options, in run order. A section that is off has zero items. Categories have two
/// items (anime, then manga), matching the amount rule with the D2 fix.
pub fn plan(opts: RestoreOptions, c: Counts) -> Vec<(Section, u32)> {
    let when = |on: bool, n: u32| if on { n } else { 0 };
    vec![
        (Section::Categories, when(opts.categories, 2)),
        (Section::AppSettings, when(opts.app_settings, 1)),
        (Section::SourceSettings, when(opts.source_settings, 1)),
        (Section::AnimeEntries, when(opts.library_entries, c.anime_entries)),
        (Section::MangaEntries, when(opts.library_entries, c.manga_entries)),
        (Section::AnimeStores, when(opts.extension_stores, c.anime_stores)),
        (Section::MangaRepos, when(opts.extension_stores, c.manga_repos)),
        (Section::Extensions, when(opts.extensions, 1)),
    ]
}

/// One finished item. `error` is recorded in the log and the run goes on; `title` labels the progress step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub error: Option<String>,
}

/// Restores one item of a section. Return `Err` only for a stop (cancel or fatal); item errors go in `Item::error`.
pub trait Restorer {
    fn item(&mut self, section: Section, index: u32) -> Result<Item, RestoreError>;
    /// Backup preference keys the allowlist skipped (T-22a-052). Read once, when the run ends.
    fn skipped_settings(&self) -> usize {
        0
    }
    /// Backup extension payloads that were not installed, because APK payloads do not run on Linux (T-22a-029).
    fn skipped_extensions(&self) -> usize {
        0
    }
}

/// One collected error: when it happened and the message of its log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorLine {
    pub at: DateTime<Local>,
    pub message: String,
}

/// What the run produced. `stop` is set when the run ended early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub errors: Vec<ErrorLine>,
    pub stop: Option<RestoreError>,
    /// Keys the app-preference allowlist skipped (T-22a-052).
    pub skipped_settings: usize,
    /// APK extension payloads that were skipped, not installed (T-22a-029).
    pub skipped_extensions: usize,
}

/// Runs `plan` in order. Each finished item advances `progress` once and emits one event.
pub fn run_sections(
    plan: &[(Section, u32)],
    restorer: &mut dyn Restorer,
    cancel: &dyn Fn() -> bool,
    progress: &mut Progress,
    emit: &mut dyn FnMut(ProgressEvent),
) -> Run {
    let mut errors = Vec::new();
    for &(section, items) in plan {
        if cancel() {
            return Run { errors, stop: Some(RestoreError::Cancelled), skipped_settings: restorer.skipped_settings(), skipped_extensions: restorer.skipped_extensions() };
        }
        for index in 0..items {
            if cancel() {
                return Run { errors, stop: Some(RestoreError::Cancelled), skipped_settings: restorer.skipped_settings(), skipped_extensions: restorer.skipped_extensions() };
            }
            match restorer.item(section, index) {
                Ok(item) => {
                    if let Some(message) = item.error {
                        errors.push(ErrorLine { at: Local::now(), message });
                    }
                    emit(progress.advance(&item.title));
                }
                Err(stop) => return Run { errors, stop: Some(stop), skipped_settings: restorer.skipped_settings(), skipped_extensions: restorer.skipped_extensions() },
            }
        }
    }
    Run { errors, stop: None, skipped_settings: restorer.skipped_settings(), skipped_extensions: restorer.skipped_extensions() }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::cell::RefCell;

    use super::*;
    use crate::restore::progress::amount;

    #[derive(Default)]
    struct Log {
        seen: Rc<RefCell<Vec<(Section, u32)>>>,
        fail_at: Option<usize>,
        error_at: Option<usize>,
    }

    impl Restorer for Log {
        fn item(&mut self, section: Section, index: u32) -> Result<Item, RestoreError> {
            let n = self.seen.borrow().len();
            self.seen.borrow_mut().push((section, index));
            if self.fail_at == Some(n) {
                return Err(RestoreError::Cancelled);
            }
            let error = (self.error_at == Some(n)).then(|| format!("entry {n} failed"));
            Ok(Item { title: format!("{section:?} {index}"), error })
        }
    }

    fn counts() -> Counts {
        Counts { anime_entries: 2, manga_entries: 3, anime_stores: 1, manga_repos: 2 }
    }

    fn run_plain(opts: RestoreOptions, c: Counts, log: &mut Log) -> (Run, Vec<ProgressEvent>) {
        let plan = plan(opts, c);
        let mut progress = Progress::new(amount(opts, c));
        let mut events = Vec::new();
        let run = run_sections(&plan, log, &|| false, &mut progress, &mut |e| events.push(e));
        (run, events)
    }

    #[test]
    fn order_is_the_fixed_nine_steps() {
        let mut log = Log::default();
        run_plain(RestoreOptions::default(), counts(), &mut log);
        let sections: Vec<Section> = log.seen.borrow().iter().map(|(s, _)| *s).collect();
        let mut unique = sections.clone();
        unique.dedup();
        assert_eq!(unique, vec![
            Section::Categories,
            Section::AppSettings,
            Section::SourceSettings,
            Section::AnimeEntries,
            Section::MangaEntries,
            Section::AnimeStores,
            Section::MangaRepos,
        ], "extensions is off by default");
        assert_eq!(ORDER.len(), 8);
    }

    #[test]
    fn categories_run_before_any_entry() {
        let mut log = Log::default();
        run_plain(RestoreOptions::default(), counts(), &mut log);
        let first_entry = log.seen.borrow().iter().position(|(s, _)| matches!(s, Section::AnimeEntries | Section::MangaEntries)).unwrap();
        let last_category = log.seen.borrow().iter().rposition(|(s, _)| *s == Section::Categories).unwrap();
        assert!(last_category < first_entry, "D3: categories finish before entries start");
    }

    #[test]
    fn final_progress_equals_amount_for_every_combination() {
        for n in 0u32..128 {
            let bits: [bool; 7] = std::array::from_fn(|i| n & (1 << i) != 0);
            let opts = RestoreOptions::from_bits(bits);
            let mut log = Log::default();
            let (run, events) = run_plain(opts, counts(), &mut log);
            assert_eq!(run.stop, None);
            let total = amount(opts, counts());
            assert_eq!(events.len() as u32, total);
            if let Some(last) = events.last() {
                assert_eq!(last.done, total);
                assert_eq!(last.total, total);
            }
        }
    }

    #[test]
    fn categories_emit_two_steps() {
        let opts = RestoreOptions { library_entries: false, categories: true, app_settings: false, extension_stores: false, custom_buttons: false, source_settings: false, extensions: false };
        let mut log = Log::default();
        let (_, events) = run_plain(opts, counts(), &mut log);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn cancel_before_an_entry_stops_the_run() {
        let opts = RestoreOptions::default();
        let plan = plan(opts, counts());
        let mut progress = Progress::new(amount(opts, counts()));
        let mut log = Log::default();
        let seen = Rc::clone(&log.seen);
        let calls = Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            seen.borrow().len() >= 4
        };
        let run = run_sections(&plan, &mut log, &cancel, &mut progress, &mut |_| {});
        assert_eq!(run.stop, Some(RestoreError::Cancelled));
        assert_eq!(log.seen.borrow().len(), 4, "no item runs after the flag is set");
        assert!(progress.done() < progress.total());
    }

    #[test]
    fn item_error_is_recorded_and_run_continues() {
        let mut log = Log { error_at: Some(3), ..Default::default() };
        let (run, events) = run_plain(RestoreOptions::default(), counts(), &mut log);
        let messages: Vec<&str> = run.errors.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, vec!["entry 3 failed"]);
        assert_eq!(run.stop, None);
        assert_eq!(events.len() as u32, amount(RestoreOptions::default(), counts()));
    }

    #[test]
    fn fatal_restorer_error_stops_and_keeps_errors() {
        let mut log = Log { error_at: Some(0), fail_at: Some(5), ..Default::default() };
        let (run, _) = run_plain(RestoreOptions::default(), counts(), &mut log);
        assert_eq!(run.errors.len(), 1);
        assert_eq!(run.stop, Some(RestoreError::Cancelled));
        assert_eq!(log.seen.borrow().len(), 6);
    }

    #[test]
    fn empty_plan_finishes_with_no_events() {
        let opts = RestoreOptions { library_entries: false, categories: false, app_settings: false, extension_stores: false, custom_buttons: false, source_settings: false, extensions: false };
        let mut log = Log::default();
        let (run, events) = run_plain(opts, Counts::default(), &mut log);
        assert_eq!(run, Run { errors: vec![], stop: None, skipped_settings: 0, skipped_extensions: 0 });
        assert!(events.is_empty());
    }
}
