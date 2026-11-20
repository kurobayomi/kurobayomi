//! Test doubles for the ports of a pass: an in-memory library store, a sync that takes every fetched item as new,
//! and a host that records what it was asked to show or queue.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use kuro_platform::kind::Kind;

use crate::library_update::error::UpdateError;
use crate::library_update::fetch_interval::Window;
use crate::library_update::pass::{EntryUpdate, Failure, Host, ItemSync};
use crate::library_update::queue::{FetchType, LibraryEntry, UpdateStrategy};
use crate::library_update::source::{Fetched, SourceItem};
use crate::library_update::store::LibraryStore;

#[derive(Default)]
pub struct FakeStore {
    pub library: Vec<LibraryEntry>,
    pub favorites: Mutex<HashSet<i64>>,
    pub seasons: HashMap<i64, Vec<LibraryEntry>>,
    pub categories: HashMap<i64, Vec<i64>>,
    pub read: HashMap<i64, Vec<f64>>,
    pub library_error: Option<UpdateError>,
    /// When set, favourite checks after this many answer `false` (the entry was removed mid-pass).
    pub revoke_after_checks: Option<usize>,
    pub checks: AtomicUsize,
}

/// A manga row: favourite, one category, source `source`, no chapters read.
pub fn entry(id: i64, source: i64, title: &str) -> LibraryEntry {
    LibraryEntry {
        id,
        source,
        url: format!("/{id}"),
        title: title.into(),
        kind: Kind::Manga,
        category: 0,
        update_strategy: UpdateStrategy::AlwaysUpdate,
        status: 1,
        favorite: true,
        total: 5,
        read: 0,
        next_update_ms: 0,
        fetch_type: FetchType::Episodes,
        parent_id: None,
        fetch_interval: 7,
        last_update_ms: 0,
    }
}

/// A chapter with the given number and source position.
pub fn item(number: f64, source_order: i64) -> SourceItem {
    SourceItem {
        url: format!("/c/{number}"),
        name: format!("Chapter {number}"),
        number,
        source_order,
        ..SourceItem::default()
    }
}

impl FakeStore {
    /// A store whose every library entry is a favourite.
    pub fn with(library: Vec<LibraryEntry>) -> Self {
        let favorites = library.iter().map(|e| e.id).collect();
        Self { library, favorites: Mutex::new(favorites), ..Self::default() }
    }

}

impl LibraryStore for FakeStore {
    async fn library(&self, kind: Kind) -> Result<Vec<LibraryEntry>, UpdateError> {
        if let Some(error) = &self.library_error {
            return Err(error.clone());
        }
        Ok(self.library.iter().filter(|e| e.kind == kind).cloned().collect())
    }

    async fn season_children(&self, parent_id: i64) -> Vec<LibraryEntry> {
        self.seasons.get(&parent_id).cloned().unwrap_or_default()
    }

    async fn is_favorite(&self, id: i64, _kind: Kind) -> Result<bool, UpdateError> {
        let check = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
        if self.revoke_after_checks.is_some_and(|limit| check > limit) {
            return Ok(false);
        }
        Ok(self.favorites.lock().unwrap().contains(&id))
    }

    async fn categories(&self, id: i64, _kind: Kind) -> Result<Vec<i64>, UpdateError> {
        Ok(self.categories.get(&id).cloned().unwrap_or_default())
    }

    async fn read_numbers(&self, id: i64, _kind: Kind) -> Result<Vec<f64>, UpdateError> {
        Ok(self.read.get(&id).cloned().unwrap_or_default())
    }
}

/// Every fetched item is new.
#[derive(Default)]
pub struct FakeSync {
    pub calls: AtomicUsize,
}

impl ItemSync for FakeSync {
    async fn sync(
        &self,
        _entry: &LibraryEntry,
        fetched: &Fetched,
        _window: Window,
        _now_ms: i64,
    ) -> Result<Vec<SourceItem>, UpdateError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(fetched.items.clone())
    }
}

#[derive(Default)]
pub struct FakeHost {
    pub last_timestamp: AtomicI64,
    pub added: AtomicUsize,
    /// How often the progress notice was asked for.
    pub progress_updates: AtomicUsize,
    pub cancelled_progress: AtomicUsize,
    pub summaries: Mutex<Vec<Vec<EntryUpdate>>>,
    pub hidden_flags: Mutex<Vec<bool>>,
    pub enqueued: Mutex<Vec<(i64, usize)>>,
    pub starts: AtomicUsize,
    pub errors: Mutex<Vec<usize>>,
    /// The failures of each error log the pass asked to write; the fake answers with a fixed log path.
    pub logged: Mutex<Vec<Vec<Failure>>>,
    /// The log path each failure notice was shown with.
    pub error_logs: Mutex<Vec<Option<PathBuf>>>,
    pub queue_warnings: AtomicUsize,
}

impl Host for FakeHost {
    fn set_last_update_ms(&self, now_ms: i64) {
        self.last_timestamp.store(now_ms, Ordering::SeqCst);
    }

    fn add_new_updates(&self, _kind: Kind, count: usize) {
        self.added.fetch_add(count, Ordering::SeqCst);
    }

    fn show_progress(&self, _kind: Kind) {
        self.progress_updates.fetch_add(1, Ordering::SeqCst);
    }

    fn cancel_progress(&self, _kind: Kind) {
        self.cancelled_progress.fetch_add(1, Ordering::SeqCst);
    }

    fn show_summary(&self, _kind: Kind, updates: &[EntryUpdate], hide_content: bool) {
        self.summaries.lock().unwrap().push(updates.to_vec());
        self.hidden_flags.lock().unwrap().push(hide_content);
    }

    fn enqueue_downloads(&self, _kind: Kind, entry_id: i64, items: &[SourceItem]) {
        self.enqueued.lock().unwrap().push((entry_id, items.len()));
    }

    fn start_downloads(&self, _kind: Kind) {
        self.starts.fetch_add(1, Ordering::SeqCst);
    }

    fn write_error_log(&self, _kind: Kind, failures: &[Failure]) -> Option<PathBuf> {
        self.logged.lock().unwrap().push(failures.to_vec());
        Some(PathBuf::from("update_errors.txt"))
    }

    fn show_error(&self, _kind: Kind, failed: usize, log: Option<&Path>) {
        self.errors.lock().unwrap().push(failed);
        self.error_logs.lock().unwrap().push(log.map(Path::to_path_buf));
    }

    fn show_queue_warning(&self, _kind: Kind) {
        self.queue_warnings.fetch_add(1, Ordering::SeqCst);
    }
}
