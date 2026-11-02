//! HTTP page loader (`HttpPageLoader.kt`). T-43a-013, T-43a-014, T-43a-015.
//!
//! One worker thread takes pages from a priority queue, one page at a time. Priority is higher first, then FIFO:
//! retry 2, visible 1, preload 0. Loading the visible page also queues up to [`PRELOAD_AHEAD`] forward pages of
//! the same chapter. Dropping the [`PageGuard`] returned by `load_page` removes the entries it queued that are
//! still `Queued`. The worker starts on the first `load_page`, not in `get_pages`.
//!
//! The page list is cached on disk by chapter. A cached list is trusted on open and the source is not asked for it
//! (T-43a-092). It can go stale, because image urls expire or the source re-uploads a chapter. The refresh rule is
//! [`HttpLoader::refresh_if_stale`]: once an image of a list that came from the cache has failed, the source is asked
//! for the list; when it differs, the cache is replaced and the caller reloads the chapter to use it.
//! The reader does not call it: a failed page of a cached list drops that list instead (`sync_page`), so the next
//! open of the chapter fetches a fresh one off the GTK thread.
//!
//! Page status lives in the shared state, written by the worker. The GTK side copies it into the `ReaderPage`
//! cells with [`HttpLoader::apply_statuses`] (the loader never touches those cells from the worker, plan 43 section 2).

use std::cell::Cell;
use std::collections::BinaryHeap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use super::{LoadFailure, PageEntry};
use crate::reader::cache::DiskCache;
use crate::reader::model::{LoadError, PageSource, PageStatus, ReaderPage};

/// Forward pages of the same chapter queued with priority 0 (`preloadSize = 4`).
pub const PRELOAD_AHEAD: usize = 4;
const PRIORITY_PRELOAD: u8 = 0;
const PRIORITY_VISIBLE: u8 = 1;
const PRIORITY_RETRY: u8 = 2;

/// What the HTTP loader needs from a source. Errors are plain messages; the loader turns them into page or
/// chapter errors.
pub trait HttpPageSource: Send + Sync {
    /// The page list of a chapter, from the network.
    fn page_list(&self, chapter_url: &str) -> Result<Vec<PageEntry>, String>;
    /// The image url of a page whose list entry had none.
    fn image_url(&self, page_url: &str) -> Result<String, String>;
    /// The image bytes behind one image url.
    fn image(&self, image_url: &str) -> Result<Vec<u8>, String>;
}

/// One entry of the priority queue. The max-heap pops the highest priority first, and the lowest sequence
/// number (first queued) among equals.
#[derive(Debug, PartialEq, Eq)]
struct Queued {
    priority: u8,
    seq: u64,
    index: usize,
}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority).then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug)]
struct Slot {
    url: String,
    image_url: Option<String>,
    status: PageStatus,
}

#[derive(Default)]
struct State {
    slots: Vec<Slot>,
    queue: BinaryHeap<Queued>,
    next_seq: u64,
    shutdown: bool,
}

impl State {
    /// Queues one entry and returns its sequence number, the identity a [`PageGuard`] cancels by.
    fn push(&mut self, index: usize, priority: u8) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.queue.push(Queued { priority, seq, index });
        seq
    }

    /// The next page to load: the highest priority first, then FIFO. An entry whose page is no longer `Queued`
    /// (already loaded, or queued twice) is dropped here, so a manual retry of a ready page downloads nothing
    /// (E-HTTP-2, E-HTTP-3).
    fn dequeue(&mut self) -> Option<usize> {
        while let Some(item) = self.queue.pop() {
            if self.slots[item.index].status == PageStatus::Queued {
                return Some(item.index);
            }
        }
        None
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // a poisoned lock only means a worker panicked mid-update; the state is still usable
        self.state.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

/// Keeps the entries one `load_page` queued. Dropping it cancels the ones still `Queued`.
pub struct PageGuard {
    shared: Arc<Shared>,
    seqs: Vec<u64>,
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        let mut guard = self.shared.lock();
        let State { queue, slots, .. } = &mut *guard;
        let seqs = &self.seqs;
        queue.retain(|q| !(seqs.contains(&q.seq) && slots[q.index].status == PageStatus::Queued));
    }
}

pub struct HttpLoader {
    source: Arc<dyn HttpPageSource>,
    manga_id: i64,
    chapter_url: String,
    cache: Arc<DiskCache>,
    shared: Arc<Shared>,
    worker_started: Cell<bool>,
    recycled: Cell<bool>,
    /// The current page list came from the cache, so it may be stale.
    list_from_cache: Cell<bool>,
}

impl HttpLoader {
    pub fn new(source: Arc<dyn HttpPageSource>, manga_id: i64, chapter_url: String, cache: Arc<DiskCache>) -> Self {
        Self {
            source,
            manga_id,
            chapter_url,
            cache,
            shared: Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() }),
            worker_started: Cell::new(false),
            recycled: Cell::new(false),
            list_from_cache: Cell::new(false),
        }
    }

    pub fn is_recycled(&self) -> bool {
        self.recycled.get()
    }

    #[cfg(test)]
    /// Whether the worker thread has been started (it starts on the first `load_page`).
    pub fn worker_running(&self) -> bool {
        self.worker_started.get()
    }

    /// The page list from the page-list cache, else the source. A fetched list is cached at once. Indices are
    /// rebuilt as `0..n`: the loader does not trust the source's numbering.
    pub fn get_pages(&self) -> Result<Vec<Rc<ReaderPage>>, LoadFailure> {
        if self.is_recycled() {
            return Err(LoadFailure::Cancelled);
        }
        let cached = self.cache.get_page_list(self.manga_id, &self.chapter_url);
        self.list_from_cache.set(cached.is_some());
        let entries = match cached {
            Some(entries) => entries,
            None => {
                let entries = self
                    .source
                    .page_list(&self.chapter_url)
                    .map_err(|e| LoadFailure::Failed(LoadError::new(e)))?;
                if let Err(e) = self.cache.put_page_list(self.manga_id, &self.chapter_url, &entries) {
                    tracing::warn!(error = %e, "page list could not be cached");
                }
                entries
            }
        };
        {
            let mut state = self.shared.lock();
            state.slots = entries
                .iter()
                .map(|e| Slot { url: e.url.clone(), image_url: e.image_url.clone(), status: PageStatus::Queued })
                .collect();
        }
        Ok(entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| ReaderPage::new(i as u32, e.url, e.image_url, PageSource::Http))
            .collect())
    }

    #[cfg(test)]
    /// The stale-cache rule (T-43a-092). Does nothing, and asks the source for nothing, unless the list came from the
    /// cache and a page has failed. Then it fetches the source's list: when it differs from the cached one, the
    /// cache is replaced and `true` is returned, so the caller reloads the chapter to use the new list. The source
    /// call blocks, so the caller runs it off the display's frame path.
    pub fn refresh_if_stale(&self) -> Result<bool, LoadError> {
        let failed = self.shared.lock().slots.iter().any(|s| s.status == PageStatus::Error);
        if self.is_recycled() || !self.list_from_cache.get() || !failed {
            return Ok(false);
        }
        let fresh = self.source.page_list(&self.chapter_url).map_err(LoadError::new)?;
        if self.cache.get_page_list(self.manga_id, &self.chapter_url).as_deref() == Some(fresh.as_slice()) {
            return Ok(false);
        }
        self.cache.put_page_list(self.manga_id, &self.chapter_url, &fresh).map_err(|e| LoadError::new(e.to_string()))?;
        Ok(true)
    }

    /// Queues the page when it needs loading and up to [`PRELOAD_AHEAD`] forward `Queued` pages of this chapter.
    /// Also re-queues a `Ready` page whose cached image has gone, and an `Error` page (automatic retry).
    pub fn load_page(&self, index: usize) -> PageGuard {
        let mut queued: Vec<u64> = Vec::new();
        if self.is_recycled() {
            return PageGuard { shared: Arc::clone(&self.shared), seqs: queued };
        }
        self.ensure_worker();
        {
            let mut state = self.shared.lock();
            let len = state.slots.len();
            if index < len {
                let (status, image_url) = {
                    let slot = &state.slots[index];
                    (slot.status, slot.image_url.clone())
                };
                let evicted = status == PageStatus::Ready && image_url.as_ref().is_some_and(|u| !self.cache.has_image(u));
                if evicted || status == PageStatus::Error {
                    state.slots[index].status = PageStatus::Queued;
                }
                if state.slots[index].status == PageStatus::Queued {
                    queued.push(state.push(index, PRIORITY_VISIBLE));
                }
                let end = (index + 1 + PRELOAD_AHEAD).min(len);
                for next in (index + 1)..end {
                    if state.slots[next].status == PageStatus::Queued {
                        queued.push(state.push(next, PRIORITY_PRELOAD));
                    }
                }
            }
        }
        self.shared.wake.notify_one();
        PageGuard { shared: Arc::clone(&self.shared), seqs: queued }
    }

    /// Manual retry (E-HTTP-2): an `Error` page is re-queued at the highest priority. Other statuses are skipped
    /// by the worker when it dequeues the entry.
    pub fn retry_page(&self, index: usize) {
        self.ensure_worker();
        {
            let mut state = self.shared.lock();
            if index >= state.slots.len() {
                return;
            }
            if state.slots[index].status == PageStatus::Error {
                state.slots[index].status = PageStatus::Queued;
            }
            state.push(index, PRIORITY_RETRY);
        }
        self.shared.wake.notify_one();
    }

    #[cfg(test)]
    /// The status of a page, as the worker last set it.
    pub fn status(&self, index: usize) -> Option<PageStatus> {
        self.shared.lock().slots.get(index).map(|s| s.status)
    }

    /// The cached image file of a `Ready` page. A page whose file vanished goes back to `Queued` and is
    /// re-enqueued (E-IMG-3), and `None` is returned.
    pub fn image_file(&self, index: usize) -> Option<PathBuf> {
        let image_url = {
            let state = self.shared.lock();
            let slot = state.slots.get(index)?;
            if slot.status != PageStatus::Ready {
                return None;
            }
            slot.image_url.clone()?
        };
        if let Some(path) = self.cache.read_image(&image_url) {
            return Some(path);
        }
        {
            let mut state = self.shared.lock();
            state.slots[index].status = PageStatus::Queued;
            state.push(index, PRIORITY_VISIBLE);
        }
        self.shared.wake.notify_one();
        None
    }

    /// Copies the worker's status and image url of one page into its cells. Runs on the GTK thread. A failed page of a
    /// list that came from the cache drops that list (see [`HttpLoader::drop_stale_list`]).
    pub fn sync_page(&self, page: &ReaderPage) {
        let state = self.shared.lock();
        if let Some(slot) = state.slots.get(page.index as usize) {
            page.status.set(slot.status);
            if slot.image_url.is_some() {
                *page.image_url.borrow_mut() = slot.image_url.clone();
            }
            if slot.status == PageStatus::Error {
                self.drop_stale_list();
            }
        }
    }

    /// The cached list may be stale (image urls rotate or expire), so it is removed and the next open of the chapter asks
    /// the source for a fresh one. Once per loader, and only for a list that came from the cache.
    fn drop_stale_list(&self) {
        if self.list_from_cache.replace(false) {
            if let Err(error) = std::fs::remove_file(self.cache.page_list_path(self.manga_id, &self.chapter_url)) {
                tracing::warn!(%error, "the stale page list could not be removed");
            }
        }
    }

    /// Stops the worker after its current page and drops the queue. Idempotent.
    pub fn recycle(&self) {
        if self.recycled.replace(true) {
            return;
        }
        let mut state = self.shared.lock();
        state.shutdown = true;
        state.queue.clear();
        drop(state);
        self.shared.wake.notify_all();
    }

    fn ensure_worker(&self) {
        if self.worker_started.replace(true) {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let source = Arc::clone(&self.source);
        let cache = Arc::clone(&self.cache);
        std::thread::Builder::new()
            .name("kuro-http-page-loader".into())
            .spawn(move || worker(shared, source, cache))
            .expect("the page loader worker thread starts");
    }
}

/// The single worker: dequeue, skip entries whose page is no longer `Queued`, process, repeat.
fn worker(shared: Arc<Shared>, source: Arc<dyn HttpPageSource>, cache: Arc<DiskCache>) {
    loop {
        let index = {
            let mut state = shared.lock();
            loop {
                if state.shutdown {
                    return;
                }
                match state.dequeue() {
                    Some(index) => break index,
                    None => state = shared.wake.wait(state).unwrap_or_else(|poison| poison.into_inner()),
                }
            }
        };
        process(&shared, source.as_ref(), &cache, index);
    }
}

fn set_status(shared: &Shared, index: usize, status: PageStatus) {
    shared.lock().slots[index].status = status;
}

/// `internalLoadPage`: resolve the image url if needed, download unless cached, then `Ready`. A failure sets
/// `Error` and the worker goes on with the next page (E-HTTP-6).
fn process(shared: &Shared, source: &dyn HttpPageSource, cache: &DiskCache, index: usize) {
    let (page_url, known_image) = {
        let state = shared.lock();
        let slot = &state.slots[index];
        (slot.url.clone(), slot.image_url.clone())
    };
    let image_url = match known_image {
        Some(url) => url,
        None => {
            set_status(shared, index, PageStatus::LoadPage);
            match source.image_url(&page_url) {
                Ok(url) => {
                    shared.lock().slots[index].image_url = Some(url.clone());
                    url
                }
                Err(e) => return fail(shared, index, &e),
            }
        }
    };
    if !cache.has_image(&image_url) {
        set_status(shared, index, PageStatus::DownloadImage);
        let stored = source.image(&image_url).and_then(|bytes| cache.put_image(&image_url, &bytes).map_err(|e| e.to_string()));
        if let Err(e) = stored {
            return fail(shared, index, &e);
        }
    }
    set_status(shared, index, PageStatus::Ready);
}

fn fail(shared: &Shared, index: usize, message: &str) {
    tracing::warn!(index, error = message, "page failed to load");
    set_status(shared, index, PageStatus::Error);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::{Duration, Instant};

    /// A source that records what it was asked and fails on request.
    #[derive(Default)]
    struct Scripted {
        pages: Vec<PageEntry>,
        fail_list: bool,
        /// When set, the source's list has changed to this one.
        fresh: Mutex<Option<Vec<PageEntry>>>,
        fail_images: Vec<String>,
        lists: AtomicUsize,
        images: Mutex<Vec<String>>,
        inflight: AtomicUsize,
        max_inflight: AtomicUsize,
    }

    impl HttpPageSource for Scripted {
        fn page_list(&self, _chapter_url: &str) -> Result<Vec<PageEntry>, String> {
            self.lists.fetch_add(1, AtomicOrdering::SeqCst);
            if self.fail_list {
                return Err("offline".into());
            }
            Ok(self.fresh.lock().unwrap().clone().unwrap_or_else(|| self.pages.clone()))
        }

        fn image_url(&self, page_url: &str) -> Result<String, String> {
            Ok(format!("https://img/{page_url}"))
        }

        fn image(&self, image_url: &str) -> Result<Vec<u8>, String> {
            let now = self.inflight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            self.max_inflight.fetch_max(now, AtomicOrdering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            self.inflight.fetch_sub(1, AtomicOrdering::SeqCst);
            self.images.lock().unwrap().push(image_url.to_owned());
            if self.fail_images.iter().any(|f| f == image_url) {
                Err("boom".into())
            } else {
                Ok(b"img".to_vec())
            }
        }
    }

    fn entries(n: usize, with_image_url: bool) -> Vec<PageEntry> {
        (0..n)
            .map(|i| PageEntry {
                url: format!("p{i}"),
                image_url: with_image_url.then(|| format!("https://img/p{i}")),
            })
            .collect()
    }

    fn cache(tag: &str) -> Arc<DiskCache> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-http-tests").join(tag);
        let _ = std::fs::remove_dir_all(&root);
        Arc::new(DiskCache::open(root, crate::reader::cache::CACHE_LIMIT_BYTES).unwrap())
    }

    fn loader(source: Arc<Scripted>, tag: &str) -> HttpLoader {
        HttpLoader::new(source, 1, "/chapter/1".into(), cache(tag))
    }

    fn wait_for(loader: &HttpLoader, index: usize, want: PageStatus) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while loader.status(index) != Some(want) {
            assert!(Instant::now() < deadline, "page {index} never reached {want:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn prio_queue_order_and_fifo() {
        let mut state = State::default();
        for (index, priority) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 0)] {
            state.push(index, priority);
        }
        let order: Vec<usize> = std::iter::from_fn(|| state.queue.pop().map(|q| q.index)).collect();
        assert_eq!(order, [3, 1, 2, 0, 4]);
    }

    #[test]
    fn page_status_filter_on_dequeue() {
        let slot = |status| Slot { url: "p".into(), image_url: None, status };
        let mut state = State::default();
        state.slots = vec![slot(PageStatus::Ready), slot(PageStatus::Queued), slot(PageStatus::Error), slot(PageStatus::Queued)];
        // a retry entry (priority 2) for the ready page, a visible entry for page 1, an error page, and page 1 again
        for (index, priority) in [(0, 2), (1, 1), (2, 1), (1, 0), (3, 0)] {
            state.push(index, priority);
        }
        assert_eq!(state.dequeue(), Some(1), "the ready page is dropped, the queued one is taken");
        state.slots[1].status = PageStatus::Ready;
        assert_eq!(state.dequeue(), Some(3), "the error page and the second entry of a loaded page are dropped");
        assert_eq!(state.dequeue(), None);
    }

    #[test]
    fn get_pages_does_not_start_the_worker_and_rebuilds_indices() {
        let source = Arc::new(Scripted { pages: entries(3, false), ..Default::default() });
        let loader = loader(Arc::clone(&source), "no-worker");
        let pages = loader.get_pages().ok().unwrap();
        assert!(!loader.worker_running());
        assert_eq!(pages.iter().map(|p| p.index).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(pages.iter().all(|p| p.status.get() == PageStatus::Queued));
    }

    #[test]
    fn page_list_is_cached_when_fetched_and_read_back_without_the_source() {
        let source = Arc::new(Scripted { pages: entries(2, true), ..Default::default() });
        let loader = loader(Arc::clone(&source), "list-cache");
        loader.get_pages().ok().unwrap();
        loader.get_pages().ok().unwrap();
        assert_eq!(source.lists.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn a_stale_cached_list_is_replaced_once_an_image_fails_and_the_source_list_differs() {
        let source = Arc::new(Scripted {
            pages: entries(2, true),
            fail_images: vec!["https://img/p0".into()],
            ..Default::default()
        });
        let shared = cache("stale-list");
        let open = || HttpLoader::new(Arc::clone(&source) as Arc<dyn HttpPageSource>, 1, "/chapter/1".into(), Arc::clone(&shared));
        let first = open();
        first.get_pages().ok().unwrap();
        assert_eq!(first.refresh_if_stale(), Ok(false), "a list fresh from the source is never refreshed");

        let reopened = open();
        assert_eq!(reopened.get_pages().ok().unwrap().len(), 2);
        assert_eq!(source.lists.load(AtomicOrdering::SeqCst), 1, "the reopen asks the source for nothing");
        assert_eq!(reopened.refresh_if_stale(), Ok(false), "no failure yet, so the cached list is trusted");
        assert_eq!(source.lists.load(AtomicOrdering::SeqCst), 1);

        let _guard = reopened.load_page(0);
        wait_for(&reopened, 0, PageStatus::Error);
        assert_eq!(reopened.refresh_if_stale(), Ok(false), "the source list is the same, so the cache stays");
        assert_eq!(source.lists.load(AtomicOrdering::SeqCst), 2);

        *source.fresh.lock().unwrap() = Some(entries(3, true));
        assert_eq!(reopened.refresh_if_stale(), Ok(true), "a different list replaces the cache");
        assert_eq!(shared.get_page_list(1, "/chapter/1").unwrap().len(), 3);
        assert_eq!(open().get_pages().ok().unwrap().len(), 3, "the next open reads the new list");
    }

    #[test]
    fn a_failed_page_of_a_cached_list_drops_the_list_so_the_next_open_fetches_it() {
        let source = Arc::new(Scripted { pages: entries(2, true), fail_images: vec!["https://img/p0".into()], ..Default::default() });
        let shared = cache("drop-stale");
        let open = || HttpLoader::new(Arc::clone(&source) as Arc<dyn HttpPageSource>, 1, "/chapter/1".into(), Arc::clone(&shared));
        let fresh = open();
        let pages = fresh.get_pages().ok().unwrap();
        let _guard = fresh.load_page(0);
        wait_for(&fresh, 0, PageStatus::Error);
        fresh.sync_page(&pages[0]);
        assert!(shared.get_page_list(1, "/chapter/1").is_some(), "a list fresh from the source stays");

        let reopened = open();
        let pages = reopened.get_pages().ok().unwrap();
        let _guard = reopened.load_page(0);
        wait_for(&reopened, 0, PageStatus::Error);
        reopened.sync_page(&pages[1]);
        assert!(shared.get_page_list(1, "/chapter/1").is_some(), "a page that did not fail drops nothing");
        reopened.sync_page(&pages[0]);
        assert_eq!(pages[0].status.get(), PageStatus::Error);
        assert!(shared.get_page_list(1, "/chapter/1").is_none(), "the failed page drops the cached list");
        assert_eq!(source.lists.load(AtomicOrdering::SeqCst), 1);
        open().get_pages().ok().unwrap();
        assert_eq!(source.lists.load(AtomicOrdering::SeqCst), 2, "the next open asks the source again");
    }

    #[test]
    fn a_failed_page_list_is_a_chapter_error() {
        let source = Arc::new(Scripted { fail_list: true, ..Default::default() });
        let loader = loader(source, "list-fail");
        assert!(matches!(loader.get_pages(), Err(LoadFailure::Failed(e)) if e.message == "offline"));
    }

    #[test]
    fn visible_page_loads_then_forward_pages_preload_with_one_worker() {
        let source = Arc::new(Scripted { pages: entries(8, false), ..Default::default() });
        let loader = loader(Arc::clone(&source), "preload");
        loader.get_pages().ok().unwrap();
        let _guard = loader.load_page(0);
        assert!(loader.worker_running());
        wait_for(&loader, 4, PageStatus::Ready);
        assert_eq!(loader.status(5), Some(PageStatus::Queued), "only four pages ahead are queued");
        assert_eq!(loader.status(6), Some(PageStatus::Queued));
        assert_eq!(source.max_inflight.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn the_last_page_queues_nothing_ahead() {
        let source = Arc::new(Scripted { pages: entries(2, true), ..Default::default() });
        let loader = loader(Arc::clone(&source), "last");
        loader.get_pages().ok().unwrap();
        let _guard = loader.load_page(1);
        wait_for(&loader, 1, PageStatus::Ready);
        assert_eq!(source.images.lock().unwrap().as_slice(), ["https://img/p1"]);
    }

    #[test]
    fn page_without_image_url_goes_load_page_then_download_then_ready() {
        let source = Arc::new(Scripted { pages: entries(1, false), ..Default::default() });
        let loader = loader(Arc::clone(&source), "resolve");
        loader.get_pages().ok().unwrap();
        let _guard = loader.load_page(0);
        wait_for(&loader, 0, PageStatus::Ready);
        assert!(loader.image_file(0).is_some());
        assert_eq!(source.images.lock().unwrap().as_slice(), ["https://img/p0"]);
    }

    #[test]
    fn a_non_cancellation_failure_sets_error_and_the_worker_continues() {
        let source = Arc::new(Scripted {
            pages: entries(2, true),
            fail_images: vec!["https://img/p0".into()],
            ..Default::default()
        });
        let loader = loader(source, "fail");
        loader.get_pages().ok().unwrap();
        let _guard = loader.load_page(0);
        wait_for(&loader, 0, PageStatus::Error);
        loader.retry_page(1);
        wait_for(&loader, 1, PageStatus::Ready);
    }

    #[test]
    fn cancelling_the_guard_removes_still_queued_entries_and_keeps_others() {
        let source = Arc::new(Scripted { pages: entries(4, true), ..Default::default() });
        let loader = loader(Arc::clone(&source), "cancel");
        loader.get_pages().ok().unwrap();
        // no worker yet: the queue is only what the test pushes, so the outcome is deterministic
        let mut seqs = Vec::new();
        {
            let mut state = loader.shared.lock();
            seqs.push(state.push(1, PRIORITY_VISIBLE));
            seqs.push(state.push(2, PRIORITY_PRELOAD));
            state.push(3, PRIORITY_PRELOAD);
        }
        let guard = PageGuard { shared: Arc::clone(&loader.shared), seqs };
        drop(guard);
        let left: Vec<usize> = std::iter::from_fn(|| loader.shared.lock().queue.pop().map(|q| q.index)).collect();
        assert_eq!(left, [3]);
    }

    #[test]
    fn dropping_an_older_guard_leaves_the_newer_guards_entries_queued() {
        let source = Arc::new(Scripted { pages: entries(8, true), ..Default::default() });
        let loader = loader(Arc::clone(&source), "overlap");
        loader.get_pages().ok().unwrap();
        // the worker is not started, so nothing is dequeued; the queue holds what the guards pushed
        let older = {
            let mut state = loader.shared.lock();
            PageGuard { shared: Arc::clone(&loader.shared), seqs: vec![state.push(2, PRIORITY_VISIBLE), state.push(3, PRIORITY_PRELOAD)] }
        };
        let newer = {
            let mut state = loader.shared.lock();
            PageGuard { shared: Arc::clone(&loader.shared), seqs: vec![state.push(2, PRIORITY_VISIBLE), state.push(3, PRIORITY_PRELOAD)] }
        };
        drop(older);
        let mut left: Vec<(usize, u64)> = std::iter::from_fn(|| loader.shared.lock().queue.pop().map(|q| (q.index, q.seq))).collect();
        left.sort();
        assert_eq!(left, [(2, newer.seqs[0]), (3, newer.seqs[1])]);
    }

    #[test]
    fn a_ready_page_whose_cache_file_is_gone_is_queued_again() {
        let source = Arc::new(Scripted { pages: entries(1, true), ..Default::default() });
        let loader = loader(Arc::clone(&source), "evicted");
        loader.get_pages().ok().unwrap();
        let _guard = loader.load_page(0);
        wait_for(&loader, 0, PageStatus::Ready);
        std::fs::remove_file(loader.cache.image_path("https://img/p0")).unwrap();
        // closed first, so the worker cannot race the assertion and download the page again
        loader.recycle();
        assert!(loader.image_file(0).is_none());
        assert_eq!(loader.status(0), Some(PageStatus::Queued));
    }

    #[test]
    fn reading_an_image_through_the_loader_keeps_it_through_eviction() {
        let source = Arc::new(Scripted { pages: entries(2, true), ..Default::default() });
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-http-tests/lru-read");
        let _ = std::fs::remove_dir_all(&root);
        // the limit holds two of the three bytes-long images
        let cache = Arc::new(DiskCache::open(root, 6).unwrap());
        let loader = HttpLoader::new(source, 1, "/chapter/1".into(), Arc::clone(&cache));
        loader.get_pages().ok().unwrap();
        let _guard = loader.load_page(0);
        wait_for(&loader, 0, PageStatus::Ready);
        wait_for(&loader, 1, PageStatus::Ready);
        loader.recycle();
        let base = std::time::SystemTime::now() - Duration::from_secs(1000);
        crate::reader::cache::set_mtime(&cache.image_path("https://img/p0"), base);
        crate::reader::cache::set_mtime(&cache.image_path("https://img/p1"), base + Duration::from_secs(5));
        assert!(loader.image_file(0).is_some(), "reading p0 makes it the most recent");
        cache.put_image("https://img/other", b"img").unwrap();
        assert!(cache.has_image("https://img/p0"));
        assert!(!cache.has_image("https://img/p1"));
    }

    #[test]
    fn recycle_is_idempotent_and_refuses_later_page_lists() {
        let source = Arc::new(Scripted { pages: entries(1, true), ..Default::default() });
        let loader = loader(source, "recycle");
        loader.recycle();
        loader.recycle();
        assert!(loader.is_recycled());
        assert!(matches!(loader.get_pages(), Err(LoadFailure::Cancelled)));
    }
}
