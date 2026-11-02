//! Reader domain model: `ReaderChapter`, `ReaderPage`, `InsertPage`, `ViewerChapters` and `ChapterTransition`
//! (`ReaderChapter.kt`, `ReaderPage.kt`, `InsertPage.kt`, `ViewerChapters.kt`, `ChapterTransition.kt`, `Page.kt`).
//!
//! Everything here lives on the GTK main thread, so shared ownership is `Rc` and mutation is `Cell` or `RefCell`.
//! Background loaders report through events and never touch these types (plan 43, section 2).

use kuro_i18n::{MsgKey, tr};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use kuro_data::item::Item;
use tokio::sync::watch;

/// A value many watchers follow (a `StateFlow`): `set` wakes the receivers only when the value changed, and a
/// receiver that missed several values sees the latest one.
#[derive(Debug)]
pub struct Observable<T>(watch::Sender<T>);

impl<T: Copy + PartialEq> Observable<T> {
    pub fn new(value: T) -> Self {
        Self(watch::channel(value).0)
    }

    pub fn get(&self) -> T {
        *self.0.borrow()
    }

    pub fn set(&self, value: T) {
        self.0.send_if_modified(|v| std::mem::replace(v, value) != value);
    }

    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.0.subscribe()
    }
}

/// Why a chapter or a page failed to load. Shown to the user as `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub message: String,
}

impl LoadError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LoadError {}

/// What a chapter's page loader must be able to do when the chapter is let go (`PageLoader.recycle`).
pub trait PageLoader {
    fn recycle(&self);
    /// `isLocal`: false for the HTTP loader, whose chapters a downloaded copy can replace.
    fn is_local(&self) -> bool;
    /// `loadPage`: asks for page `index` (and the pages after it) to be fetched. The guard returned cancels what it
    /// queued when dropped. Local loaders have nothing to fetch.
    fn load_page(&self, _index: usize) -> Option<Box<dyn Any>> {
        None
    }
    /// The Retry of a failed page: queues it again at the highest priority.
    fn retry_page(&self, _index: usize) {}
    /// The cached image file of a fetched page that is ready. A file that has gone queues the page again.
    fn image_file(&self, _index: usize) -> Option<PathBuf> {
        None
    }
    /// Copies the loader's status of `page` into the page's cells (the worker never touches them).
    fn sync_page(&self, _page: &ReaderPage) {}
}

/// Identity of a chapter: its row id and url, the fields `ChapterImpl.equals` compares.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChapterKey {
    pub id: i64,
    pub url: String,
}

/// `ReaderChapter.State`. `pages` exist only in `Loaded`.
#[derive(Clone, Debug)]
pub enum ChapterState {
    Wait,
    Loading,
    Error(Rc<LoadError>),
    Loaded(Rc<Vec<Rc<ReaderPage>>>),
}

/// A chapter open in the reader.
pub struct ReaderChapter {
    pub chapter: Item,
    state: watch::Sender<ChapterState>,
    loader: RefCell<Option<Box<dyn PageLoader>>>,
    /// The page the viewer scrolls to when the chapter is shown.
    pub requested_page: Cell<i64>,
    /// The read flag as the reader sees it. Starts from `chapter.done` and only the reader changes it.
    pub read: Cell<bool>,
    /// The last page the reader saw, 0-based. Starts from `chapter.progress`.
    pub last_page_read: Cell<i64>,
    /// The bookmark flag as the reader sees it. Starts from `chapter.bookmark`; the top bar's toggle changes it.
    pub bookmark: Cell<bool>,
    refs: Cell<i32>,
}

impl ReaderChapter {
    pub fn new(chapter: Item) -> Rc<Self> {
        Rc::new(Self {
            read: Cell::new(chapter.done),
            last_page_read: Cell::new(chapter.progress),
            bookmark: Cell::new(chapter.bookmark),
            chapter,
            state: watch::channel(ChapterState::Wait).0,
            loader: RefCell::new(None),
            requested_page: Cell::new(0),
            refs: Cell::new(0),
        })
    }

    /// `isLocal` of the attached loader, `None` when no loader is attached.
    pub fn attached_is_local(&self) -> Option<bool> {
        self.loader.borrow().as_ref().map(|l| l.is_local())
    }

    /// Recycles the attached loader and goes back to `Wait`, without touching the reference count. Used when a
    /// downloaded copy should replace an online loader (`ReaderViewModel.preload`, step 2).
    pub fn reset(&self) {
        if let Some(loader) = self.loader.borrow_mut().take() {
            loader.recycle();
        }
        self.set_state(ChapterState::Wait);
    }

    pub fn key(&self) -> ChapterKey {
        ChapterKey { id: self.chapter.id, url: self.chapter.url.clone() }
    }

    /// The current state. Do not hold it across `set_state`.
    pub fn state(&self) -> watch::Ref<'_, ChapterState> {
        self.state.borrow()
    }

    pub fn set_state(&self, state: ChapterState) {
        self.state.send_replace(state);
    }

    /// The state as a watch receiver (`ReaderChapter.stateFlow`): `Wait`, `Loading`, `Error` or `Loaded`.
    pub fn subscribe_state(&self) -> watch::Receiver<ChapterState> {
        self.state.subscribe()
    }

    /// The pages, or `None` unless the chapter is `Loaded`.
    pub fn pages(&self) -> Option<Rc<Vec<Rc<ReaderPage>>>> {
        match &*self.state.borrow() {
            ChapterState::Loaded(pages) => Some(Rc::clone(pages)),
            _ => None,
        }
    }

    /// Stores the loaded pages: points each at this chapter and moves the state to `Loaded`. An empty list is an
    /// error state, as in `ChapterLoader`. Indices must already be `0..n`.
    pub fn finish_loading(self: &Rc<Self>, pages: Vec<Rc<ReaderPage>>) -> Result<(), LoadError> {
        if pages.is_empty() {
            let error = LoadError::new(tr(MsgKey::ui_page_list_is_empty));
            self.set_state(ChapterState::Error(Rc::new(error.clone())));
            return Err(error);
        }
        debug_assert!(pages.iter().enumerate().all(|(i, p)| p.index as usize == i), "page indices are 0..n");
        for page in &pages {
            page.set_chapter(self);
        }
        self.set_state(ChapterState::Loaded(Rc::new(pages)));
        Ok(())
    }

    /// Attaches the loader. A previous loader is recycled first, so it is never leaked.
    pub fn set_loader(&self, loader: Box<dyn PageLoader>) {
        if let Some(old) = self.loader.borrow_mut().replace(loader) {
            old.recycle();
        }
    }

    pub fn has_loader(&self) -> bool {
        self.loader.borrow().is_some()
    }

    /// Asks the loader to fetch page `index` (see [`PageLoader::load_page`]). `None` without a loader, or when there is
    /// nothing to fetch.
    pub fn request_page(&self, index: usize) -> Option<Box<dyn Any>> {
        self.loader.borrow().as_ref().and_then(|l| l.load_page(index))
    }

    /// Queues a failed page again (the Retry of its card).
    pub fn retry_page(&self, index: usize) {
        if let Some(loader) = self.loader.borrow().as_ref() {
            loader.retry_page(index);
        }
    }

    /// The cached image file of a ready fetched page.
    pub fn page_file(&self, index: usize) -> Option<PathBuf> {
        self.loader.borrow().as_ref().and_then(|l| l.image_file(index))
    }

    /// Copies the loader's status of `page` into its cells.
    pub fn sync_page(&self, page: &ReaderPage) {
        if let Some(loader) = self.loader.borrow().as_ref() {
            loader.sync_page(page);
        }
    }

    /// `ref()`.
    pub fn add_ref(&self) {
        self.refs.set(self.refs.get() + 1);
    }

    /// `unref()`. Recycles the loader and goes back to `Wait` only when the count reaches exactly zero. A negative
    /// count is a bug: it asserts in debug builds and, as upstream, never recycles in release builds (E-REF-1).
    /// Returns true when the chapter was reset.
    pub fn release(&self) -> bool {
        let count = self.refs.get() - 1;
        self.refs.set(count);
        debug_assert!(count >= 0, "chapter released more often than referenced (E-REF-1)");
        if count != 0 {
            return false;
        }
        if let Some(loader) = self.loader.borrow_mut().take() {
            loader.recycle();
        }
        self.set_state(ChapterState::Wait);
        true
    }

    #[cfg(test)]
    pub fn references(&self) -> i32 {
        self.refs.get()
    }
}

// Equality is the chapter's identity, not its load state.
impl PartialEq for ReaderChapter {
    fn eq(&self, other: &Self) -> bool {
        self.chapter.id == other.chapter.id && self.chapter.url == other.chapter.url
    }
}

impl Eq for ReaderChapter {}

impl Hash for ReaderChapter {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.chapter.id.hash(state);
        self.chapter.url.hash(state);
    }
}

impl fmt::Debug for ReaderChapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReaderChapter").field("id", &self.chapter.id).field("url", &self.chapter.url).finish()
    }
}

/// `Page.State`, in the upstream order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PageStatus {
    Queued,
    LoadPage,
    DownloadImage,
    Ready,
    Error,
}

/// Where a page's bytes come from. `Shared` is an `InsertPage`: it reads through its parent.
#[derive(Debug)]
pub enum PageSource {
    Http,
    File(PathBuf),
    ArchiveEntry { archive: PathBuf, entry: String },
    Shared(Rc<ReaderPage>),
}

/// A page of a chapter. `index` is 0-based and contiguous; every label shows `number()`.
#[derive(Debug)]
pub struct ReaderPage {
    pub index: u32,
    pub url: String,
    pub image_url: RefCell<Option<String>>,
    pub status: Observable<PageStatus>,
    /// 0 to 100, or -1 when the length is unknown.
    pub progress: Observable<i32>,
    pub source: PageSource,
    chapter: RefCell<Weak<ReaderChapter>>,
}

impl ReaderPage {
    /// A page with no chapter yet (`ChapterLoader` sets it). A page fetched over the network starts `Queued`; one whose
    /// bytes are already on disk (a download, a local folder or archive) is `Ready` at once, as the archive and
    /// directory loaders set it upstream, or nothing would ever leave the spinner.
    pub fn new(index: u32, url: impl Into<String>, image_url: Option<String>, source: PageSource) -> Rc<Self> {
        let status = if matches!(source, PageSource::Http) { PageStatus::Queued } else { PageStatus::Ready };
        Rc::new(Self {
            index,
            url: url.into(),
            image_url: RefCell::new(image_url),
            status: Observable::new(status),
            progress: Observable::new(0),
            source,
            chapter: RefCell::new(Weak::new()),
        })
    }

    /// `InsertPage(parent)`: same index, urls and chapter as the parent, `Ready` at once and sharing its bytes.
    pub fn insert_page(parent: &Rc<ReaderPage>) -> Rc<Self> {
        Rc::new(Self {
            index: parent.index,
            url: parent.url.clone(),
            image_url: RefCell::new(parent.image_url.borrow().clone()),
            status: Observable::new(PageStatus::Ready),
            progress: Observable::new(0),
            source: PageSource::Shared(Rc::clone(parent)),
            chapter: RefCell::new(parent.chapter.borrow().clone()),
        })
    }

    pub fn is_insert_page(&self) -> bool {
        matches!(self.source, PageSource::Shared(_))
    }

    /// An inserted page never saves reading progress (`ReaderViewModel.onPageSelected`).
    pub fn writes_progress(&self) -> bool {
        !self.is_insert_page()
    }

    /// `Page.number`: the 1-based label.
    pub fn number(&self) -> u32 {
        self.index + 1
    }

    pub fn set_chapter(&self, chapter: &Rc<ReaderChapter>) {
        *self.chapter.borrow_mut() = Rc::downgrade(chapter);
    }

    pub fn chapter(&self) -> Option<Rc<ReaderChapter>> {
        self.chapter.borrow().upgrade()
    }

    #[cfg(test)]
    /// `Page.update(bytesRead, contentLength)`: integer percent, or -1 for an unknown length.
    pub fn update_progress(&self, bytes_read: i64, content_length: i64) {
        let percent = if content_length > 0 { (100 * bytes_read / content_length) as i32 } else { -1 };
        self.progress.set(percent);
    }
}

/// The chapter being read and its neighbours.
#[derive(Clone, Debug)]
pub struct ViewerChapters {
    pub curr: Rc<ReaderChapter>,
    pub prev: Option<Rc<ReaderChapter>>,
    pub next: Option<Rc<ReaderChapter>>,
}

impl ViewerChapters {
    fn each(&self) -> impl Iterator<Item = &Rc<ReaderChapter>> {
        std::iter::once(&self.curr).chain(self.prev.iter()).chain(self.next.iter())
    }

    /// `ref()` on the current chapter and each neighbour.
    pub fn add_refs(&self) {
        self.each().for_each(|c| c.add_ref());
    }

    /// `unref()` on the current chapter and each neighbour.
    pub fn release_all(&self) {
        self.each().for_each(|c| {
            c.release();
        });
    }

    /// Swaps the chapter set. References are added for `new` before `old` is released, so a chapter in both sets
    /// never reaches zero and is never recycled.
    pub fn replace(old: Option<&ViewerChapters>, new: &ViewerChapters) {
        new.add_refs();
        if let Some(old) = old {
            old.release_all();
        }
    }
}

/// A move between chapters. The direction is part of the identity, so `Prev(a, b)` and `Next(a, b)` differ.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChapterTransition {
    Prev { from: Rc<ReaderChapter>, to: Option<Rc<ReaderChapter>> },
    Next { from: Rc<ReaderChapter>, to: Option<Rc<ReaderChapter>> },
}

impl ChapterTransition {
    pub fn from(&self) -> &Rc<ReaderChapter> {
        match self {
            Self::Prev { from, .. } | Self::Next { from, .. } => from,
        }
    }

    pub fn to(&self) -> Option<&Rc<ReaderChapter>> {
        match self {
            Self::Prev { to, .. } | Self::Next { to, .. } => to.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    fn chapter(id: i64) -> Rc<ReaderChapter> {
        ReaderChapter::new(Item { id, url: format!("/c/{id}"), ..Item::create(kuro_platform::kind::Kind::Manga) })
    }

    fn hash_of(t: &ChapterTransition) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    struct CountingLoader(Rc<Cell<u32>>);
    impl PageLoader for CountingLoader {
        fn is_local(&self) -> bool {
            true
        }

        fn recycle(&self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn pages(n: u32) -> Vec<Rc<ReaderPage>> {
        (0..n).map(|i| ReaderPage::new(i, format!("p{i}"), None, PageSource::Http)).collect()
    }

    #[test]
    fn a_new_chapter_waits_and_pages_exist_only_when_loaded() {
        let c = chapter(1);
        assert!(matches!(&*c.state(), ChapterState::Wait));
        assert!(c.pages().is_none());
        c.set_state(ChapterState::Loading);
        assert!(c.pages().is_none());
        c.set_state(ChapterState::Error(Rc::new(LoadError::new("boom"))));
        assert!(c.pages().is_none());
        c.finish_loading(pages(3)).unwrap();
        assert_eq!(c.pages().unwrap().len(), 3);
        assert_eq!(c.requested_page.get(), 0);
    }

    #[test]
    fn finishing_with_no_pages_is_an_error_state() {
        let c = chapter(1);
        let e = c.finish_loading(vec![]).unwrap_err();
        assert_eq!(e.message, "Page list is empty");
        assert!(matches!(&*c.state(), ChapterState::Error(_)));
    }

    #[test]
    fn a_page_on_disk_is_ready_at_once_and_a_fetched_one_waits() {
        assert_eq!(ReaderPage::new(0, "p", None, PageSource::Http).status.get(), PageStatus::Queued);
        let file = ReaderPage::new(0, "p", None, PageSource::File(std::path::PathBuf::from("/tmp/p.png")));
        assert_eq!(file.status.get(), PageStatus::Ready);
        let entry = ReaderPage::new(0, "p", None, PageSource::ArchiveEntry { archive: "/tmp/c.cbz".into(), entry: "1.png".into() });
        assert_eq!(entry.status.get(), PageStatus::Ready, "a downloaded chapter's pages show without a fetch");
    }

    #[test]
    fn page_status_has_five_ordered_values_and_starts_queued() {
        use PageStatus::*;
        assert!(Queued < LoadPage && LoadPage < DownloadImage && DownloadImage < Ready && Ready < Error);
        let p = ReaderPage::new(4, "u", None, PageSource::Http);
        assert_eq!(p.status.get(), Queued);
        assert_eq!(p.progress.get(), 0);
    }

    #[test]
    fn number_is_index_plus_one_for_every_loaded_page_and_each_page_knows_its_chapter() {
        let c = chapter(1);
        c.finish_loading(pages(4)).unwrap();
        for (i, page) in c.pages().unwrap().iter().enumerate() {
            assert_eq!(page.index as usize, i);
            assert_eq!(page.number() as usize, i + 1);
            assert_eq!(page.chapter().unwrap().chapter.id, 1);
        }
    }

    #[test]
    fn progress_is_integer_percent_or_minus_one() {
        let p = ReaderPage::new(0, "", None, PageSource::Http);
        p.update_progress(50, 200);
        assert_eq!(p.progress.get(), 25);
        p.update_progress(1, 3);
        assert_eq!(p.progress.get(), 33);
        p.update_progress(10, 0);
        assert_eq!(p.progress.get(), -1);
    }

    #[test]
    fn an_insert_page_from_a_ready_parent_is_ready_shares_its_bytes_and_never_writes_progress() {
        let c = chapter(1);
        let loaded = pages(2);
        loaded[1].status.set(PageStatus::Ready);
        *loaded[1].image_url.borrow_mut() = Some("img".into());
        c.finish_loading(loaded).unwrap();
        let parent = Rc::clone(&c.pages().unwrap()[1]);
        let insert = ReaderPage::insert_page(&parent);
        assert_eq!(insert.status.get(), PageStatus::Ready);
        assert_eq!((insert.index, insert.url.as_str()), (1, "p1"));
        assert_eq!(insert.image_url.borrow().as_deref(), Some("img"));
        assert!(matches!(&insert.source, PageSource::Shared(p) if Rc::ptr_eq(p, &parent)));
        assert_eq!(insert.chapter().unwrap().chapter.id, 1);
        assert!(!insert.writes_progress());
        assert!(parent.writes_progress());
    }

    #[test]
    fn release_resets_and_recycles_only_when_the_count_reaches_exactly_zero() {
        let c = chapter(1);
        let recycled = Rc::new(Cell::new(0));
        c.set_loader(Box::new(CountingLoader(Rc::clone(&recycled))));
        c.finish_loading(pages(1)).unwrap();
        c.add_ref();
        c.add_ref();
        assert!(!c.release());
        assert_eq!(recycled.get(), 0);
        assert!(c.pages().is_some());
        assert!(c.release());
        assert_eq!(recycled.get(), 1);
        assert!(!c.has_loader());
        assert!(matches!(&*c.state(), ChapterState::Wait));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "E-REF-1")]
    fn a_negative_count_asserts_in_debug_builds() {
        let c = chapter(1);
        c.add_ref();
        c.release();
        c.release();
    }

    #[test]
    fn replacing_a_loader_recycles_the_old_one_first() {
        let c = chapter(1);
        let first = Rc::new(Cell::new(0));
        c.set_loader(Box::new(CountingLoader(Rc::clone(&first))));
        c.set_loader(Box::new(CountingLoader(Rc::new(Cell::new(0)))));
        assert_eq!(first.get(), 1);
    }

    #[test]
    fn viewer_chapters_replace_keeps_shared() {
        let (a, b, c, d) = (chapter(1), chapter(2), chapter(3), chapter(4));
        let recycled = Rc::new(Cell::new(0));
        for ch in [&a, &b, &c, &d] {
            ch.set_loader(Box::new(CountingLoader(Rc::clone(&recycled))));
            ch.finish_loading(pages(1)).unwrap();
        }
        let old = ViewerChapters { curr: Rc::clone(&b), prev: Some(Rc::clone(&a)), next: Some(Rc::clone(&c)) };
        ViewerChapters::replace(None, &old);
        // next step: b and c stay, a leaves, d arrives
        let new = ViewerChapters { curr: Rc::clone(&c), prev: Some(Rc::clone(&b)), next: Some(Rc::clone(&d)) };
        ViewerChapters::replace(Some(&old), &new);
        assert!(b.pages().is_some() && c.pages().is_some(), "shared chapters were never recycled");
        assert!(d.pages().is_some());
        assert!(a.pages().is_none() && !a.has_loader(), "only the chapter that left is recycled");
        assert_eq!(recycled.get(), 1);
        assert_eq!((a.references(), b.references(), c.references(), d.references()), (0, 1, 1, 1));
        new.release_all();
        assert_eq!(recycled.get(), 4);
    }

    #[test]
    fn chapters_are_equal_by_id_and_url_not_by_state() {
        let a = chapter(1);
        let same = chapter(1);
        same.set_state(ChapterState::Loading);
        assert_eq!(*a, *same);
        assert_ne!(*a, *chapter(2));
    }

    #[test]
    fn transition_equality_direction() {
        let (a, b) = (chapter(1), chapter(2));
        let prev = |f: &Rc<ReaderChapter>, t: &Rc<ReaderChapter>| ChapterTransition::Prev { from: Rc::clone(f), to: Some(Rc::clone(t)) };
        let next = |f: &Rc<ReaderChapter>, t: &Rc<ReaderChapter>| ChapterTransition::Next { from: Rc::clone(f), to: Some(Rc::clone(t)) };
        assert_eq!(prev(&a, &b), prev(&a, &b));
        assert_eq!(hash_of(&prev(&a, &b)), hash_of(&prev(&a, &b)));
        assert_ne!(prev(&a, &b), next(&a, &b));
        assert_ne!(prev(&a, &b), prev(&b, &a), "the swapped pair is a different card");
        assert_eq!(prev(&a, &b), prev(&chapter(1), &chapter(2)), "equal by chapter identity, not by instance");
    }

    #[test]
    fn a_transition_carries_a_from_chapter_and_an_optional_to() {
        let a = chapter(1);
        let end = ChapterTransition::Next { from: Rc::clone(&a), to: None };
        assert!(Rc::ptr_eq(end.from(), &a));
        assert!(end.to().is_none());
        let back = ChapterTransition::Prev { from: Rc::clone(&a), to: Some(chapter(0)) };
        assert_eq!(back.to().unwrap().chapter.id, 0);
    }
}
