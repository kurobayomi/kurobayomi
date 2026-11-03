//! Page state machines of the reader and the downloader (`HttpPageLoader.kt`, `MangaDownloader.kt`; T-15a-027).
//! The source and cache calls are traits, so this module holds the
//! state rules, the reader's priority queue and the image URL step, and makes no network call of its own.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use async_trait::async_trait;

use crate::error::SourceError;
use crate::model::{Page, PageState};

/// Pages after the current one that the reader queues ahead of time (`preloadSize`).
pub const PRELOAD_SIZE: usize = 4;
/// Priority of a page queued ahead of time.
pub const PRIORITY_PRELOAD: u8 = 0;
/// Priority of the page on screen.
pub const PRIORITY_CURRENT: u8 = 1;
/// Priority of a page the user asked to retry.
pub const PRIORITY_RETRY: u8 = 2;

/// `getImageUrl` of the source: the image URL of a page. Called only for a page that has none yet.
#[async_trait]
pub trait PageImages: Send + Sync {
    async fn image_url(&self, page: &Page) -> Result<String, SourceError>;
}

/// The chapter image cache as the reader sees it.
#[async_trait]
pub trait ImageCache: Send + Sync {
    /// Whether the image at `image_url` is cached (`isImageInCache`).
    fn is_cached(&self, image_url: &str) -> bool;
    /// Fetches the image of `page` and stores it under `image_url` (`putImageToCache`).
    async fn store(&self, page: &Page, image_url: &str) -> Result<(), SourceError>;
}

fn state_of(page: &Page) -> PageState {
    *page.status.state.borrow()
}

/// Whether a page may move from `from` to `to`. Staying in a state is always allowed. The reader table of section 8
/// adds the download steps of section 9 and the direct paths for a cached image.
pub fn transition_allowed(from: PageState, to: PageState) -> bool {
    use PageState::{DownloadImage, Error, LoadPage, Queue, Ready};
    from == to
        || matches!(
            (from, to),
            (Queue, LoadPage | DownloadImage | Ready)
                | (LoadPage, DownloadImage | Ready | Error)
                | (DownloadImage, Ready | Error)
                | (Ready, Queue)
                | (Error, Queue)
        )
}

/// Moves a page to `to`. The table is checked in debug builds; release builds set the state as given.
pub fn advance(page: &Page, to: PageState) {
    let from = state_of(page);
    debug_assert!(transition_allowed(from, to), "page moved from {from:?} to {to:?}");
    page.set_state(to);
}

/// The image URL step, shared by the reader and the downloader. A page with a URL keeps it. Otherwise the page is
/// `LoadPage` while the source resolves the URL. A failure is `Error` and is returned, never a panic. Nothing is
/// reported to the user here.
pub async fn resolve_image_url(page: &mut Page, images: &dyn PageImages) -> Result<String, SourceError> {
    if let Some(url) = page.image_url.clone().filter(|url| !url.is_empty()) {
        return Ok(url);
    }
    advance(page, PageState::LoadPage);
    let url = match images.image_url(&*page).await {
        Ok(url) if !url.is_empty() => url,
        Ok(_) => {
            advance(page, PageState::Error);
            return Err(SourceError::MissingImageUrl);
        }
        Err(error) => {
            advance(page, PageState::Error);
            return Err(error);
        }
    };
    page.image_url = Some(url.clone());
    Ok(url)
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    priority: u8,
    seq: u64,
    index: usize,
}

impl Ord for Entry {
    /// Higher priority first, then the earlier entry first (`PriorityPage.compareTo`).
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority).then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The reader's request queue: page indexes ordered by priority, then by arrival.
#[derive(Debug, Default)]
pub struct ReaderQueue {
    heap: BinaryHeap<Entry>,
    next_seq: u64,
}

impl ReaderQueue {
    pub fn push(&mut self, index: usize, priority: u8) {
        self.heap.push(Entry { priority, seq: self.next_seq, index });
        self.next_seq += 1;
    }

    /// The next page index to load.
    pub fn pop(&mut self) -> Option<usize> {
        self.heap.pop().map(|entry| entry.index)
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Drops every request. A closed reader leaves no queued work behind (RC-03).
    pub fn clear(&mut self) {
        self.heap.clear();
    }
}

/// `loadPage(page)` of the reader: a page that is stale in the cache or in `Error` is queued again; a queued page is
/// queued at the current priority, and the pages after it that are queued are queued for preloading.
pub fn load_page(queue: &mut ReaderQueue, pages: &[Page], index: usize, cache: &dyn ImageCache) {
    let Some(page) = pages.get(index) else {
        return;
    };
    if state_of(page) == PageState::Ready && page.image_url.as_deref().is_some_and(|url| !cache.is_cached(url)) {
        advance(page, PageState::Queue);
    }
    if state_of(page) == PageState::Error {
        advance(page, PageState::Queue);
    }
    if state_of(page) == PageState::Queue {
        queue.push(index, PRIORITY_CURRENT);
    }
    let end = pages.len().min(index + 1 + PRELOAD_SIZE);
    for (offset, next) in pages[index + 1..end].iter().enumerate() {
        if state_of(next) == PageState::Queue {
            queue.push(index + 1 + offset, PRIORITY_PRELOAD);
        }
    }
}

/// `retryPage(page)` of the reader: an `Error` page goes back to `Queue` and is requested at the retry priority.
pub fn retry_page(queue: &mut ReaderQueue, pages: &[Page], index: usize) {
    let Some(page) = pages.get(index) else {
        return;
    };
    if state_of(page) == PageState::Error {
        advance(page, PageState::Queue);
    }
    queue.push(index, PRIORITY_RETRY);
}

/// One step of the reader worker: takes the next request that is still `Queue` (a request for a page that moved on is
/// dropped) and loads that page. Returns the index and the outcome, or `None` when nothing is left to load.
pub async fn run_next(
    queue: &mut ReaderQueue,
    pages: &mut [Page],
    images: &dyn PageImages,
    cache: &dyn ImageCache,
) -> Option<(usize, Result<(), SourceError>)> {
    loop {
        let index = queue.pop()?;
        let Some(page) = pages.get_mut(index) else {
            continue;
        };
        if state_of(page) != PageState::Queue {
            continue;
        }
        let outcome = load_one(page, images, cache).await;
        return Some((index, outcome));
    }
}

/// `internalLoadPage` of the reader. A cancelled call leaves the state where it was.
async fn load_one(page: &mut Page, images: &dyn PageImages, cache: &dyn ImageCache) -> Result<(), SourceError> {
    let url = resolve_image_url(page, images).await?;
    if !cache.is_cached(&url) {
        advance(page, PageState::DownloadImage);
        if let Err(error) = cache.store(&*page, &url).await {
            advance(page, PageState::Error);
            return Err(error);
        }
    }
    advance(page, PageState::Ready);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use super::*;

    fn pages(count: i32) -> Vec<Page> {
        (0..count).map(|i| Page::new(i, format!("https://src.test/{i}"))).collect()
    }

    fn url_of(index: i32) -> String {
        format!("https://img.test/{index}.jpg")
    }

    struct Urls;

    #[async_trait]
    impl PageImages for Urls {
        async fn image_url(&self, page: &Page) -> Result<String, SourceError> {
            Ok(url_of(page.index))
        }
    }

    struct BrokenUrls;

    #[async_trait]
    impl PageImages for BrokenUrls {
        async fn image_url(&self, _page: &Page) -> Result<String, SourceError> {
            Err(SourceError::Http(503))
        }
    }

    struct EmptyUrls;

    #[async_trait]
    impl PageImages for EmptyUrls {
        async fn image_url(&self, _page: &Page) -> Result<String, SourceError> {
            Ok(String::new())
        }
    }

    /// A cache that holds the URLs it was given and counts stores.
    #[derive(Default)]
    struct Cache {
        held: Mutex<HashSet<String>>,
        stores: AtomicUsize,
        fail: bool,
    }

    impl Cache {
        fn holding(urls: &[String]) -> Self {
            Self { held: Mutex::new(urls.iter().cloned().collect()), ..Self::default() }
        }
    }

    #[async_trait]
    impl ImageCache for Cache {
        fn is_cached(&self, image_url: &str) -> bool {
            self.held.lock().expect("lock").contains(image_url)
        }

        async fn store(&self, _page: &Page, image_url: &str) -> Result<(), SourceError> {
            self.stores.fetch_add(1, AtomicOrdering::SeqCst);
            if self.fail {
                return Err(SourceError::Transport("reset".into()));
            }
            self.held.lock().expect("lock").insert(image_url.to_owned());
            Ok(())
        }
    }

    fn state(page: &Page) -> PageState {
        state_of(page)
    }

    #[test]
    fn the_life_cycle_table_allows_the_documented_steps_and_no_others() {
        use PageState::{DownloadImage, Error, LoadPage, Queue, Ready};
        assert!(transition_allowed(Queue, LoadPage));
        assert!(transition_allowed(LoadPage, DownloadImage));
        assert!(transition_allowed(DownloadImage, Ready));
        assert!(transition_allowed(DownloadImage, Error));
        assert!(transition_allowed(Ready, Queue), "EC-IM-02: an evicted page is queued again");
        assert!(transition_allowed(Error, Queue), "EC-IM-03 and EC-IM-04");
        assert!(!transition_allowed(Queue, Error));
        assert!(!transition_allowed(Ready, DownloadImage));
        assert!(!transition_allowed(DownloadImage, Queue));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "page moved")]
    fn an_illegal_step_is_caught_in_debug_builds() {
        let page = Page::new(0, "u");
        advance(&page, PageState::Error);
    }

    #[test]
    fn ec_im_01_a_ready_page_whose_image_is_cached_is_not_queued() {
        let pages = {
            let mut pages = pages(1);
            pages[0].image_url = Some(url_of(0));
            pages[0].set_state(PageState::Ready);
            pages
        };
        let cache = Cache::holding(&[url_of(0)]);
        let mut queue = ReaderQueue::default();
        load_page(&mut queue, &pages, 0, &cache);
        assert!(queue.is_empty(), "no network call and no work");
        assert_eq!(state(&pages[0]), PageState::Ready);
    }

    #[tokio::test]
    async fn ec_im_02_a_ready_page_whose_cache_was_evicted_is_downloaded_again() {
        let mut pages = pages(1);
        pages[0].image_url = Some(url_of(0));
        pages[0].set_state(PageState::Ready);
        let cache = Cache::default();
        let mut queue = ReaderQueue::default();
        load_page(&mut queue, &pages, 0, &cache);
        assert_eq!(state(&pages[0]), PageState::Queue);
        let (index, outcome) = run_next(&mut queue, &mut pages, &Urls, &cache).await.expect("one request");
        assert_eq!((index, outcome), (0, Ok(())));
        assert_eq!(cache.stores.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(state(&pages[0]), PageState::Ready);
    }

    #[tokio::test]
    async fn ec_im_03_an_error_page_is_queued_again_when_subscribed() {
        let mut pages = pages(1);
        pages[0].set_state(PageState::Error);
        let cache = Cache::default();
        let mut queue = ReaderQueue::default();
        load_page(&mut queue, &pages, 0, &cache);
        let (_, outcome) = run_next(&mut queue, &mut pages, &Urls, &cache).await.expect("request");
        assert_eq!(outcome, Ok(()));
        assert_eq!(state(&pages[0]), PageState::Ready);
    }

    #[test]
    fn ec_im_04_a_user_retry_is_queued_at_priority_two_ahead_of_the_current_page() {
        let pages = pages(2);
        pages[0].set_state(PageState::Queue);
        pages[1].set_state(PageState::Error);
        let cache = Cache::default();
        let mut queue = ReaderQueue::default();
        load_page(&mut queue, &pages, 0, &cache);
        retry_page(&mut queue, &pages, 1);
        assert_eq!(state(&pages[1]), PageState::Queue);
        assert_eq!(queue.pop(), Some(1), "priority 2 beats priority 1");
        assert_eq!(queue.pop(), Some(0));
    }

    #[test]
    fn the_queue_orders_by_priority_then_arrival() {
        let mut queue = ReaderQueue::default();
        queue.push(5, PRIORITY_PRELOAD);
        queue.push(6, PRIORITY_PRELOAD);
        queue.push(7, PRIORITY_CURRENT);
        queue.push(8, PRIORITY_RETRY);
        queue.push(9, PRIORITY_CURRENT);
        let order: Vec<usize> = std::iter::from_fn(|| queue.pop()).collect();
        assert_eq!(order, vec![8, 7, 9, 5, 6]);
    }

    #[test]
    fn a_page_queues_itself_at_current_and_the_next_four_queued_pages_for_preload() {
        let pages = pages(8);
        for page in &pages {
            page.set_state(PageState::Queue);
        }
        let cache = Cache::default();
        let mut queue = ReaderQueue::default();
        load_page(&mut queue, &pages, 0, &cache);
        assert_eq!(queue.len(), 5, "current page plus four preloads, not six");
        let order: Vec<usize> = std::iter::from_fn(|| queue.pop()).collect();
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn a_request_for_a_page_that_moved_on_is_dropped() {
        let mut pages = pages(2);
        pages[0].set_state(PageState::Queue);
        let cache = Cache::default();
        let mut queue = ReaderQueue::default();
        queue.push(0, PRIORITY_CURRENT);
        queue.push(0, PRIORITY_PRELOAD);
        assert!(run_next(&mut queue, &mut pages, &Urls, &cache).await.is_some());
        assert!(run_next(&mut queue, &mut pages, &Urls, &cache).await.is_none(), "the duplicate is skipped");
        assert_eq!(cache.stores.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_url_step_is_error_and_is_returned_not_panicked() {
        let mut pages = pages(1);
        pages[0].set_state(PageState::Queue);
        let cache = Cache::default();
        let mut queue = ReaderQueue::default();
        queue.push(0, PRIORITY_CURRENT);
        let (_, outcome) = run_next(&mut queue, &mut pages, &BrokenUrls, &cache).await.expect("request");
        assert_eq!(outcome, Err(SourceError::Http(503)));
        assert_eq!(state(&pages[0]), PageState::Error);
        assert_eq!(cache.stores.load(AtomicOrdering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_page_without_an_image_url_after_the_step_is_missing_image_url() {
        let mut page = Page::new(0, "u");
        let error = resolve_image_url(&mut page, &EmptyUrls).await.expect_err("empty URL");
        assert_eq!(error, SourceError::MissingImageUrl);
        assert_eq!(state(&page), PageState::Error);
    }

    #[tokio::test]
    async fn a_store_failure_is_error_and_the_page_keeps_its_url() {
        let mut pages = pages(1);
        pages[0].set_state(PageState::Queue);
        let cache = Cache { fail: true, ..Cache::default() };
        let mut queue = ReaderQueue::default();
        queue.push(0, PRIORITY_CURRENT);
        let (_, outcome) = run_next(&mut queue, &mut pages, &Urls, &cache).await.expect("request");
        assert!(outcome.is_err());
        assert_eq!(state(&pages[0]), PageState::Error);
        assert_eq!(pages[0].image_url.as_deref(), Some(url_of(0).as_str()));
    }

    #[test]
    fn closing_the_reader_clears_every_queued_request() {
        let mut queue = ReaderQueue::default();
        queue.push(0, PRIORITY_CURRENT);
        queue.push(1, PRIORITY_PRELOAD);
        queue.clear();
        assert!(queue.is_empty(), "RC-03: no queue entries survive a cancel");
    }
}
