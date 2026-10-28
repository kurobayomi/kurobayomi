//! The reader session: progress on page change, completion, chapter switching, adjacent navigation, preload,
//! and the per-manga mode writes. It runs on the GTK thread and talks to the database only
//! through [`ReaderStore`]. T-43a-023 (initial index), T-43a-025, T-43a-026, T-43a-027, T-43a-028, T-43a-029,
//! T-43a-036, T-43a-086 (event channel).

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use kuro_data::entry::{Entry, EntryRepo};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::Db;
use kuro_domain::history::recorder::SessionTimer;
use kuro_platform::kind::Kind;
use kuro_prefs::reader::{READING_MODE_MASK, ReadingMode};

use super::chapters::{ListError, ListOptions, build_reader_list};
use super::delete_queue::{self, DeleteQueue, Pending};
use super::download_ahead;
use super::flags;
use super::history_rules::{self, now_ms};
use super::loader::{LoadFailure, Loader, load_chapter};
use super::model::{ChapterState, LoadError, PageStatus, ReaderChapter, ReaderPage, ViewerChapters};
use super::nav::{self, Step};
use super::restore::SavedSession;
use super::strings;

/// The writes a session makes. The production implementation is the writer thread (`writer::WriterHandle`).
pub trait ReaderStore {
    fn save_progress(&self, chapter_id: i64, last_page_read: i64, read: bool);
    /// Writes the bookmark and reports whether it was stored. `Err` makes the session revert the flag (T-43a-035).
    fn set_bookmark(&self, chapter_id: i64, bookmark: bool) -> Result<(), String>;
    /// Marks a chapter read without moving its last page (the duplicate rule, T-43a-031).
    fn mark_read(&self, chapter_id: i64);
    /// Adds `duration` ms of read time and sets the last read time (T-43a-033).
    fn upsert_history(&self, chapter_id: i64, read_at: i64, duration: i64);
    fn set_viewer_flags(&self, manga_id: i64, value: i64, mask: i64);
    /// Keeps where the reader is, for the crash restore (T-43a-039). A store with nowhere to keep it ignores it.
    fn save_restore(&self, _session: SavedSession) {}
    /// Blocks until every write sent so far has run. A chapter switch calls it before the new chapter loads.
    fn flush(&self);
}

/// Events from the reader to its UI. Delivered in order and never dropped (E-EVT-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderEvent {
    ReloadViewerChapters,
    PageChanged,
    SetCoverResult(bool),
    SavedImage(PathBuf),
    ShareImage(PathBuf),
    CopyImage(PathBuf),
}

/// What an adjacent chapter load ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdjacentOutcome {
    /// Another adjacent load is running; this press is ignored (one at a time).
    Busy,
    /// There is no chapter in that direction (the button is disabled).
    NoNeighbour,
    /// The new chapter is current; the caller moves to its page 0.
    Moved(usize),
    /// The load failed. The current chapter stays, and the message goes in a toast (E-NAV-1).
    Failed(String),
}

/// Initial chapter index for the requested chapter id. A missing id gives the generic message (E-INIT-4).
pub fn initial_index(ids: &[i64], requested: i64) -> Result<usize, String> {
    ids.iter().position(|&id| id == requested).ok_or_else(|| strings::requested_chapter_not_found(requested))
}

/// The message for a list that could not be built. The downloaded-only text is the proposal in
/// `p1-edge-cases-and-tests.md` for E-LIST-2, flagged there for strings review.
pub fn list_error_message(error: &ListError) -> String {
    match error {
        ListError::ChapterNotFound(id) => strings::requested_chapter_not_found(*id),
        ListError::CurrentChapterMissing => strings::chapter_not_downloaded().to_owned(),
    }
}

/// What opening the reader needs from the database: the manga, its reader list and the initial index
/// (T-43a-023). A missing manga gives "Unknown err" (E-INIT-3). `Err` carries the message the card shows.
pub struct OpenedList {
    pub manga: Entry,
    pub chapters: Vec<Rc<ReaderChapter>>,
    pub index: usize,
}

pub fn open_list(
    db: Arc<Db>,
    manga_id: i64,
    requested: i64,
    opts: impl FnOnce(&Entry) -> ListOptions,
    is_downloaded: impl Fn(&Item) -> bool,
) -> Result<OpenedList, String> {
    let manga = EntryRepo::new(Arc::clone(&db)).by_id(manga_id).map_err(|_| strings::unknown_err().to_owned())?;
    let all = ItemRepo::new(db).by_entry_id(manga_id, true).map_err(|e| e.to_string())?;
    let list = build_reader_list(&all, requested, &opts(&manga), is_downloaded).map_err(|e| list_error_message(&e))?;
    let ids: Vec<i64> = list.iter().map(|c| c.id).collect();
    let index = initial_index(&ids, requested)?;
    let chapters = list.into_iter().map(ReaderChapter::new).collect();
    Ok(OpenedList { manga, chapters, index })
}

pub struct ReaderSession {
    manga_id: i64,
    chapters: Vec<Rc<ReaderChapter>>,
    current: Option<usize>,
    viewer_flags: i64,
    incognito: bool,
    preserve_position: bool,
    store: Rc<dyn ReaderStore>,
    events: Sender<ReaderEvent>,
    adjacent_busy: bool,
    /// The visible page, 1-based, for the page position in the top bar (`currentPage`).
    current_page: u32,
    /// `mark_duplicate_read_chapter_read` contains `existing` (T-43a-031). Off unless the caller turns it on.
    mark_duplicates_read: bool,
    /// The read session timer (T-43a-033). Owned by the UI thread (E-HIST-2).
    history: SessionTimer,
    /// Whether `history` has a start, for [`Self::resume_history`]: the timer does not say.
    history_running: bool,
    /// The delete-after-read rule, when the app can delete downloads (T-43a-032).
    delete_after_read: Option<DeleteAfterRead>,
    /// Sends the completed chapter's number to the trackers (T-43a-030). Off unless the caller sets it.
    track_update: Option<TrackUpdate>,
    /// The download-ahead rule, with its amount read when the reader opened (T-43a-034).
    ahead: Option<AheadRule>,
}

/// Called with the manga id and the number of a chapter the reader completed. The caller applies the tracking switch.
pub type TrackUpdate = Rc<dyn Fn(i64, f64)>;

/// The download-ahead amount of a session and the lookup that says which chapters are downloaded.
struct AheadRule {
    amount: i32,
    is_downloaded: Rc<dyn Fn(&Item) -> bool>,
}

/// The delete-after-read state of a session: the slot count read when the reader opened, the chapters queued so far
/// and what hands a newly queued chapter to the download service, which keeps it across a restart.
struct DeleteAfterRead {
    slots: i64,
    queue: DeleteQueue,
    defer: Rc<dyn Fn(Pending)>,
}

impl ReaderSession {
    /// `incognito` is read once, when the session starts (E-INC-2 closes the reader when it changes).
    pub fn new(
        manga_id: i64,
        viewer_flags: i64,
        chapters: Vec<Rc<ReaderChapter>>,
        current: usize,
        incognito: bool,
        preserve_position: bool,
        store: Rc<dyn ReaderStore>,
        events: Sender<ReaderEvent>,
    ) -> Self {
        Self {
            manga_id,
            chapters,
            current: Some(current),
            viewer_flags,
            incognito,
            preserve_position,
            store,
            events,
            adjacent_busy: false,
            current_page: 1,
            mark_duplicates_read: false,
            history: SessionTimer::default(),
            history_running: false,
            delete_after_read: None,
            track_update: None,
            ahead: None,
        }
    }

    /// Turns on the tracker update for the completions this session sees (T-43a-030). Incognito sessions never call it.
    pub fn set_track_update(&mut self, update: TrackUpdate) {
        self.track_update = Some(update);
    }

    /// Turns on download-ahead (T-43a-034). `amount` is the preference value read when the reader opened; zero or less
    /// leaves it off.
    pub fn set_download_ahead(&mut self, amount: i32, is_downloaded: Rc<dyn Fn(&Item) -> bool>) {
        self.ahead = (amount > 0).then_some(AheadRule { amount, is_downloaded });
    }

    /// The chapter ids to queue for the page being read now (empty when download-ahead is off or its rules say wait).
    fn ahead_chapters(&self) -> Vec<i64> {
        let (Some(rule), Some(current)) = (&self.ahead, self.current) else { return Vec::new() };
        let Some(pages) = self.chapters.get(current).and_then(|c| c.pages()) else { return Vec::new() };
        let ids: Vec<i64> = self.chapters.iter().map(|c| c.chapter.id).collect();
        let downloaded = |i: usize| (rule.is_downloaded)(&self.chapters[i].chapter);
        download_ahead::chapters_to_queue(rule.amount, &ids, downloaded, current, self.current_page as usize, pages.len())
    }

    /// Turns on delete-after-read for the completions this session sees (T-43a-032). `slots` is the preference value:
    /// a negative one turns it off. `defer` gets each chapter at once, so it survives a restart (E-DEL-4).
    pub fn set_delete_after_read(&mut self, slots: i64, defer: Rc<dyn Fn(Pending)>) {
        self.delete_after_read = (slots >= 0).then(|| DeleteAfterRead { slots, queue: DeleteQueue::default(), defer });
    }

    /// Turns on the duplicate rule for the completions this session sees (T-43a-031).
    pub fn set_mark_duplicates_read(&mut self, on: bool) {
        self.mark_duplicates_read = on;
    }

    /// Starts the read timer for the current chapter (T-43a-033). Incognito never writes history.
    pub fn start_history(&mut self, now: i64) {
        self.history.start(now);
        self.history_running = true;
    }

    /// Starts the timer again when the reader is shown after a pause, unless one is already running (E-HIST-1).
    pub fn resume_history(&mut self, now: i64) {
        if !self.history_running {
            self.start_history(now);
        }
    }

    /// Writes the read time since the timer started for the current chapter and stops the timer (E-HIST-1). A flush
    /// with no running timer writes nothing new. Incognito writes nothing, and the timer stops there too
    /// (`SessionTimer::flush`: the session is clamped to 12 h and the start is cleared on every path).
    pub fn flush_history(&mut self, now: i64) {
        let item_id = self.current_chapter().map(|c| c.chapter.id);
        let update = self.history.flush(now, Kind::Manga, item_id.unwrap_or(-1), self.incognito);
        self.history_running = false;
        if let (Some(_), Some(update)) = (item_id, update) {
            self.store.upsert_history(update.item_id, update.at, update.session_ms);
        }
    }

    pub fn chapters(&self) -> &[Rc<ReaderChapter>] {
        &self.chapters
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current_page(&self) -> u32 {
        self.current_page
    }

    /// A transition card is on screen: no page is shown, so the counter has no number (`current_page` 0).
    pub fn clear_current_page(&mut self) {
        self.current_page = 0;
    }

    /// The chapter being read.
    pub fn current_chapter(&self) -> Option<&Rc<ReaderChapter>> {
        self.current.and_then(|i| self.chapters.get(i))
    }

    /// Replaces the chapter list (E-LIST-1). The chapter being read keeps its own object, with its loaded pages, and
    /// stays the current one; the list the caller built must contain it.
    pub fn replace_list(&mut self, mut list: Vec<Rc<ReaderChapter>>) {
        if let Some(current) = self.current_chapter().cloned() {
            let key = current.key();
            if let Some(slot) = list.iter_mut().find(|c| c.key() == key) {
                *slot = current;
            }
            self.current = list.iter().position(|c| c.key() == key);
        }
        self.chapters = list;
    }

    /// The current chapter and its neighbours, for the viewer.
    pub fn viewer_chapters(&self) -> Option<ViewerChapters> {
        let current = Rc::clone(self.current_chapter()?);
        let (previous, next) = self.neighbours();
        Some(ViewerChapters {
            curr: current,
            prev: previous.map(|i| Rc::clone(&self.chapters[i])),
            next: next.map(|i| Rc::clone(&self.chapters[i])),
        })
    }

    /// Flips the bookmark of the current chapter and writes it. Incognito does not gate it (`bookmarkCurrentChapter`).
    /// The flag changes at once. A failed write puts the previous flag back and returns the error (E-BMK-1). `None`
    /// when no chapter is current.
    pub fn toggle_bookmark(&self) -> Option<Result<bool, String>> {
        let chapter = self.current_chapter()?;
        let previous = chapter.bookmark.get();
        let value = !previous;
        chapter.bookmark.set(value);
        match self.store.set_bookmark(chapter.chapter.id, value) {
            Ok(()) => Some(Ok(value)),
            Err(error) => {
                chapter.bookmark.set(previous);
                Some(Err(error))
            }
        }
    }

    pub fn viewer_flags(&self) -> i64 {
        self.viewer_flags
    }

    /// The previous and next positions of the current chapter.
    pub fn neighbours(&self) -> (Option<usize>, Option<usize>) {
        nav::neighbours(self.chapters.len(), self.current)
    }

    /// The progress part of `onPageSelected` (7.1 and 7.2). Returns the chapter index when the page belongs to a
    /// chapter other than the current one, so the caller can switch to it. Insert pages change nothing.
    pub fn record_page(&mut self, chapter_index: usize, page: &ReaderPage) -> Option<usize> {
        if !page.writes_progress() {
            return None;
        }
        let chapter = Rc::clone(self.chapters.get(chapter_index)?);
        let pages_len = chapter.pages()?.len() as i64;
        let index = i64::from(page.index);
        self.current_page = page.number();
        chapter.requested_page.set(index);
        if !self.incognito && page.status.get() != PageStatus::Error {
            chapter.last_page_read.set(index);
            if index == pages_len - 1 {
                // landing on the last page completes the chapter; repeating it is harmless (E-PROG-2)
                let newly_read = !chapter.read.replace(true);
                // the hook order is: track update, delete-after-read, then duplicate marking (T-43a-030, T-43a-031,
                // T-43a-032). The chapter number goes as a double; the tracker skips one it is already at or past.
                if newly_read && let Some(update) = &self.track_update {
                    update(self.manga_id, chapter.chapter.number);
                }
                self.queue_delete_after(chapter_index);
                if newly_read && self.mark_duplicates_read {
                    self.mark_duplicates_read_of(chapter_index);
                }
            }
            self.store.save_progress(chapter.chapter.id, index, chapter.read.get());
            self.store.save_restore(SavedSession {
                manga_id: self.manga_id,
                chapter_id: chapter.chapter.id,
                page_index: index,
                saved_at: now_ms(),
            });
        }
        if self.current == Some(chapter_index) {
            return None;
        }
        Some(chapter_index)
    }

    /// Makes `index` the current chapter if it loads (`loadNewChapter`). A failure is logged and the current
    /// chapter stays current.
    pub fn switch_chapter(
        &mut self,
        index: usize,
        build: impl FnOnce(&Rc<ReaderChapter>) -> Result<Loader, LoadError>,
    ) -> Result<(), LoadFailure> {
        // the previous chapter's progress is durable before the new chapter is loaded (T-43a-025)
        self.store.flush();
        let now = now_ms();
        self.flush_history(now);
        let chapter = Rc::clone(&self.chapters[index]);
        let result = load_chapter(&chapter, || build(&chapter), self.preserve_position);
        // the timer runs on the chapter the reader now shows, which is the old one when the load failed
        self.start_history(now);
        match &result {
            Ok(()) => self.current = Some(index),
            Err(LoadFailure::Failed(error)) => tracing::warn!(error = %error, "chapter switch failed"),
            Err(LoadFailure::Cancelled) => {}
        }
        result
    }

    /// `loadAdjacent`: next or previous chapter, one at a time. A failure keeps the current page.
    pub fn load_adjacent(
        &mut self,
        step: Step,
        build: impl FnOnce(&Rc<ReaderChapter>) -> Result<Loader, LoadError>,
    ) -> AdjacentOutcome {
        if self.adjacent_busy {
            return AdjacentOutcome::Busy;
        }
        let Some(target) = nav::target(self.chapters.len(), self.current, step) else {
            return AdjacentOutcome::NoNeighbour;
        };
        self.load_at(target, build)
    }

    /// Opens the chapter at `index` of the reader's list, for the chapter list's jump. The outcomes are those of
    /// [`Self::load_adjacent`]; an index past the list is `NoNeighbour`.
    pub fn load_at(
        &mut self,
        target: usize,
        build: impl FnOnce(&Rc<ReaderChapter>) -> Result<Loader, LoadError>,
    ) -> AdjacentOutcome {
        if self.adjacent_busy {
            return AdjacentOutcome::Busy;
        }
        if target >= self.chapters.len() {
            return AdjacentOutcome::NoNeighbour;
        }
        self.adjacent_busy = true;
        let result = self.switch_chapter(target, build);
        self.adjacent_busy = false;
        match result {
            Ok(()) => AdjacentOutcome::Moved(target),
            Err(LoadFailure::Failed(error)) => AdjacentOutcome::Failed(error.message),
            Err(LoadFailure::Cancelled) => AdjacentOutcome::Failed(String::new()),
        }
    }

    /// `preload` (027): loads a chapter in the background for the viewers. A downloaded copy replaces an online
    /// loader first. Failure is silent and leaves the chapter in `Error`. Success sends a reload event.
    pub fn preload(
        &mut self,
        index: usize,
        is_downloaded: bool,
        build: impl FnOnce(&Rc<ReaderChapter>) -> Result<Loader, LoadError>,
    ) {
        let chapter = Rc::clone(&self.chapters[index]);
        if matches!(*chapter.state(), ChapterState::Loaded(_) | ChapterState::Loading) {
            return;
        }
        if chapter.attached_is_local() == Some(false) && is_downloaded {
            chapter.reset();
        }
        if !matches!(*chapter.state(), ChapterState::Wait | ChapterState::Error(_)) {
            return;
        }
        if load_chapter(&chapter, || build(&chapter), self.preserve_position).is_ok() {
            let _ = self.events.send(ReaderEvent::ReloadViewerChapters);
        }
    }

    /// A reading mode was picked for this manga: it is written to the manga's viewer flags (only the mode bits), the
    /// requested page becomes the saved page, and the viewers reload.
    pub fn set_reading_mode(&mut self, mode: ReadingMode) {
        // the manga's own mode (user, 2026-10-24): the Settings page holds the default for manga without one
        self.viewer_flags = crate::reader::flags::set_reading_mode(self.viewer_flags, mode);
        let mask = i64::from(kuro_prefs::reader::READING_MODE_MASK);
        self.store.set_viewer_flags(self.manga_id, self.viewer_flags & mask, mask);
        self.restore_saved_page();
        let _ = self.events.send(ReaderEvent::ReloadViewerChapters);
    }

    /// `onPageSelected` (T-043a-093). An insert page is ignored. Otherwise the progress is saved, a chapter switch
    /// runs when the page belongs to another chapter, then download-ahead and the flash run, in that order.
    /// `download_ahead` gets the chapter ids to queue, and is not called when there are none.
    /// Returns the chapter index the session switched to.
    pub fn on_page_selected(
        &mut self,
        page: &Rc<ReaderPage>,
        build: impl FnOnce(&Rc<ReaderChapter>) -> Result<Loader, LoadError>,
        download_ahead: &mut dyn FnMut(&[i64]),
        flash: &mut dyn FnMut(),
    ) -> Option<usize> {
        if page.is_insert_page() {
            return None;
        }
        let chapter = page.chapter()?;
        let index = self.index_of(&chapter)?;
        let switch = self.record_page(index, page);
        if let Some(target) = switch {
            let _ = self.switch_chapter(target, build);
        }
        let queue = self.ahead_chapters();
        if !queue.is_empty() {
            download_ahead(&queue);
        }
        flash();
        switch
    }

    /// `request_preload` from a viewer (T-043a-093): loads the chapter in the background when the list has it.
    pub fn on_request_preload(
        &mut self,
        chapter: &Rc<ReaderChapter>,
        is_downloaded: bool,
        build: impl FnOnce(&Rc<ReaderChapter>) -> Result<Loader, LoadError>,
    ) {
        if let Some(index) = self.index_of(chapter) {
            self.preload(index, is_downloaded, build);
        }
    }

    /// `deleteChapterIfNeeded`: queues the chapter `slots` before the completed one, when it has been read. It is handed to
    /// the download service at once, which deletes it when the reader closes (and after a crash, at the next flush).
    fn queue_delete_after(&mut self, completed: usize) {
        let Some(rule) = self.delete_after_read.as_mut() else { return };
        let Some(target) = delete_queue::delete_target(&self.chapters, completed, rule.slots) else { return };
        let chapter = &self.chapters[target];
        if !chapter.read.get() {
            return;
        }
        let entry = Pending { manga_id: self.manga_id, chapter_id: chapter.chapter.id };
        if rule.queue.push(entry) {
            (rule.defer)(entry);
        }
    }

    /// Marks the unread chapters that share the completed chapter's number as read (T-43a-031). The list is the
    /// reader's list, so only those chapters are touched (E-DUP-1).
    fn mark_duplicates_read_of(&mut self, completed: usize) {
        for index in history_rules::duplicate_indices(&self.chapters, completed) {
            let duplicate = &self.chapters[index];
            duplicate.read.set(true);
            self.store.mark_read(duplicate.chapter.id);
        }
    }

    fn index_of(&self, chapter: &Rc<ReaderChapter>) -> Option<usize> {
        self.chapters.iter().position(|c| Rc::ptr_eq(c, chapter))
    }

    #[cfg(test)]
    /// Sends an event on the reader's ordered channel.
    pub fn notify(&self, event: ReaderEvent) {
        let _ = self.events.send(event);
    }

    fn restore_saved_page(&self) {
        if let Some(chapter) = self.current.and_then(|i| self.chapters.get(i)) {
            chapter.requested_page.set(chapter.last_page_read.get());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::chapters::ListError;
    use crate::reader::model::{ChapterState, PageSource};
    use kuro_data::item::Item;
    use std::cell::RefCell;
    use std::sync::mpsc;

    #[derive(Default)]
    struct Recorder {
        progress: RefCell<Vec<(i64, i64, bool)>>,
        order: RefCell<Vec<&'static str>>,
        flags: RefCell<Vec<(i64, i64, i64)>>,
        bookmarks: RefCell<Vec<(i64, bool)>>,
        read_marks: RefCell<Vec<i64>>,
        history: RefCell<Vec<(i64, i64, i64)>>,
        restores: RefCell<Vec<SavedSession>>,
        fail_bookmarks: std::cell::Cell<bool>,
    }

    impl ReaderStore for Recorder {
        fn save_progress(&self, chapter_id: i64, last_page_read: i64, read: bool) {
            self.progress.borrow_mut().push((chapter_id, last_page_read, read));
        }
        fn mark_read(&self, chapter_id: i64) {
            self.read_marks.borrow_mut().push(chapter_id);
        }

        fn upsert_history(&self, chapter_id: i64, read_at: i64, duration: i64) {
            self.history.borrow_mut().push((chapter_id, read_at, duration));
        }

        fn save_restore(&self, session: SavedSession) {
            self.restores.borrow_mut().push(session);
        }

        fn set_bookmark(&self, chapter_id: i64, bookmark: bool) -> Result<(), String> {
            if self.fail_bookmarks.get() {
                return Err("Unknown error".to_owned());
            }
            self.bookmarks.borrow_mut().push((chapter_id, bookmark));
            Ok(())
        }
        fn set_viewer_flags(&self, manga_id: i64, value: i64, mask: i64) {
            self.flags.borrow_mut().push((manga_id, value, mask));
        }
        fn flush(&self) {
            self.order.borrow_mut().push("flush");
        }
    }

    fn chapter(id: i64, read: bool) -> Rc<ReaderChapter> {
        let mut c = Item::create(Kind::Manga);
        c.id = id;
        c.url = format!("/c/{id}");
        c.done = read;
        let ch = ReaderChapter::new(c);
        let pages = (0..3)
            .map(|i| ReaderPage::new(i, format!("p{i}"), None, PageSource::Http))
            .collect::<Vec<_>>();
        ch.finish_loading(pages).unwrap();
        ch
    }

    fn page(ch: &Rc<ReaderChapter>, index: u32) -> Rc<ReaderPage> {
        let pages = ch.pages().unwrap();
        let p = Rc::clone(&pages[index as usize]);
        p.set_chapter(ch);
        p
    }

    fn session(store: Rc<Recorder>, chapters: Vec<Rc<ReaderChapter>>, current: usize, incognito: bool) -> (ReaderSession, mpsc::Receiver<ReaderEvent>) {
        let (tx, rx) = mpsc::channel();
        (ReaderSession::new(7, 0x2, chapters, current, incognito, false, store, tx), rx)
    }

    fn never_built(_: &Rc<ReaderChapter>) -> Result<Loader, LoadError> {
        Err(LoadError::new("should not build"))
    }

    #[test]
    fn the_bookmark_toggle_flips_the_current_chapter_and_writes_even_in_incognito() {
        let rec = Rc::new(Recorder::default());
        let ch = chapter(4, false);
        let (s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, true);
        assert_eq!(s.toggle_bookmark(), Some(Ok(true)));
        assert!(ch.bookmark.get());
        assert_eq!(s.toggle_bookmark(), Some(Ok(false)));
        assert_eq!(rec.bookmarks.borrow().as_slice(), [(4, true), (4, false)]);
    }

    #[test]
    fn a_failed_bookmark_write_reverts_the_flag_and_reports_the_error() {
        let rec = Rc::new(Recorder::default());
        rec.fail_bookmarks.set(true);
        let ch = chapter(4, false);
        let (s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, false);
        assert_eq!(s.toggle_bookmark(), Some(Err("Unknown error".to_owned())));
        assert!(!ch.bookmark.get(), "the icon and the stored flag stay where they were");
    }

    #[test]
    fn a_flush_writes_the_time_since_the_timer_started_and_a_second_flush_keeps_the_duration() {
        let rec = Rc::new(Recorder::default());
        let (mut s, _rx) = session(Rc::clone(&rec), vec![chapter(4, false)], 0, false);
        s.start_history(1_000);
        s.flush_history(4_000);
        s.flush_history(9_000);
        assert_eq!(rec.history.borrow().as_slice(), [(4, 4_000, 3_000), (4, 9_000, 0)]);
    }

    #[test]
    fn resuming_after_a_pause_times_the_next_stretch_and_never_resets_a_running_timer() {
        let rec = Rc::new(Recorder::default());
        let (mut s, _rx) = session(Rc::clone(&rec), vec![chapter(4, false)], 0, false);
        s.start_history(1_000);
        s.resume_history(2_000);
        s.flush_history(4_000);
        s.resume_history(10_000);
        s.flush_history(15_000);
        assert_eq!(rec.history.borrow().as_slice(), [(4, 4_000, 3_000), (4, 15_000, 5_000)]);
    }

    #[test]
    fn incognito_writes_no_history_and_stops_the_timer() {
        let rec = Rc::new(Recorder::default());
        let (mut s, _rx) = session(Rc::clone(&rec), vec![chapter(4, false)], 0, true);
        s.start_history(1_000);
        s.flush_history(4_000);
        assert!(rec.history.borrow().is_empty());
        s.resume_history(10_000);
        s.flush_history(15_000);
        assert!(rec.history.borrow().is_empty(), "the second session is incognito too, and starts at its own resume");
    }

    #[test]
    fn a_session_never_counts_more_than_twelve_hours() {
        use kuro_domain::history::recorder::MAX_SESSION_MS;
        let rec = Rc::new(Recorder::default());
        let (mut s, _rx) = session(Rc::clone(&rec), vec![chapter(4, false)], 0, false);
        s.start_history(0);
        s.flush_history(MAX_SESSION_MS * 3);
        assert_eq!(rec.history.borrow().as_slice(), [(4, MAX_SESSION_MS * 3, MAX_SESSION_MS)]);
    }

    #[test]
    fn completing_a_chapter_marks_its_unread_duplicates_read_when_the_rule_is_on() {
        let rec = Rc::new(Recorder::default());
        let numbered = |id: i64, number: f64| {
            let mut c = Item::create(Kind::Manga);
            c.id = id;
            c.url = format!("/c/{id}");
            c.number = number;
            let ch = ReaderChapter::new(c);
            let pages = (0..3).map(|i| ReaderPage::new(i, format!("p{i}"), None, PageSource::Http)).collect::<Vec<_>>();
            ch.finish_loading(pages).unwrap();
            ch
        };
        let completed = numbered(1, 5.0);
        let chapters = vec![Rc::clone(&completed), numbered(2, 5.0), numbered(3, 6.0)];
        let (mut s, _rx) = session(Rc::clone(&rec), chapters, 0, false);
        s.set_mark_duplicates_read(true);
        s.record_page(0, &page(&completed, 2));
        assert_eq!(rec.read_marks.borrow().as_slice(), [2]);
    }

    #[test]
    fn progress_is_saved_on_each_page_and_the_last_page_completes_the_chapter() {
        let rec = Rc::new(Recorder::default());
        let ch = chapter(1, false);
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, false);
        assert_eq!(s.record_page(0, &page(&ch, 1)), None);
        assert_eq!(ch.last_page_read.get(), 1);
        assert!(!ch.read.get());
        s.record_page(0, &page(&ch, 2));
        assert!(ch.read.get());
        s.record_page(0, &page(&ch, 2));
        assert_eq!(rec.progress.borrow().as_slice(), [(1, 1, false), (1, 2, true), (1, 2, true)]);
    }

    #[test]
    fn each_page_change_saves_the_restore_record_and_incognito_saves_none() {
        let rec = Rc::new(Recorder::default());
        let ch = chapter(5, false);
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, false);
        s.record_page(0, &page(&ch, 1));
        s.record_page(0, &page(&ch, 2));
        let saved: Vec<(i64, i64, i64)> = rec.restores.borrow().iter().map(|r| (r.manga_id, r.chapter_id, r.page_index)).collect();
        assert_eq!(saved, [(7, 5, 1), (7, 5, 2)]);

        let private = Rc::new(Recorder::default());
        let (mut p, _rx2) = session(Rc::clone(&private), vec![Rc::clone(&ch)], 0, true);
        p.record_page(0, &page(&ch, 1));
        assert!(private.restores.borrow().is_empty(), "the in-memory restore keys are kept but not saved (E-INC-1)");
    }

    /// Four chapters, the first `read` of them already read; the session is on the last page of chapter 3.
    fn delete_session(read: usize, slots: i64, incognito: bool) -> (Rc<RefCell<Vec<Pending>>>, Vec<Rc<ReaderChapter>>) {
        let chapters: Vec<_> = (1..=4).map(|id| chapter(id, (id as usize) <= read)).collect();
        let (mut s, _rx) = session(Rc::new(Recorder::default()), chapters.clone(), 2, incognito);
        let handed: Rc<RefCell<Vec<Pending>>> = Rc::default();
        let sink = Rc::clone(&handed);
        s.set_delete_after_read(slots, Rc::new(move |entry| sink.borrow_mut().push(entry)));
        s.record_page(2, &page(&chapters[2], 2));
        // the page is recorded twice: a chapter is handed over once
        s.record_page(2, &page(&chapters[2], 2));
        (handed, chapters)
    }

    fn queued_ids(handed: &Rc<RefCell<Vec<Pending>>>) -> Vec<i64> {
        handed.borrow().iter().map(|p| p.chapter_id).collect()
    }

    #[test]
    fn completing_a_chapter_queues_the_one_slots_before_it_and_only_when_that_one_is_read() {
        let (one_back, _c) = delete_session(2, 1, false);
        assert_eq!(queued_ids(&one_back), [2], "slot 1 targets the chapter before the completed one");
        let (itself, _c) = delete_session(2, 0, false);
        assert_eq!(queued_ids(&itself), [3], "slot 0 targets the completed chapter, which is read now");
        let (unread_target, _c) = delete_session(0, 1, false);
        assert!(queued_ids(&unread_target).is_empty(), "an unread chapter is never queued");
    }

    #[test]
    fn a_negative_slot_count_an_early_position_and_incognito_queue_nothing() {
        let (off, _c) = delete_session(4, -1, false);
        assert!(queued_ids(&off).is_empty());
        let (also_off, _c) = delete_session(4, -2, false);
        assert!(queued_ids(&also_off).is_empty(), "-2 does not reach a chapter ahead");
        let (too_far, _c) = delete_session(4, 5, false);
        assert!(queued_ids(&too_far).is_empty());
        let (private, _c) = delete_session(4, 1, true);
        assert!(queued_ids(&private).is_empty(), "incognito writes nothing (E-INC-1)");
    }

    #[test]
    fn a_page_in_error_and_incognito_write_nothing_to_the_database() {
        let rec = Rc::new(Recorder::default());
        let ch = chapter(1, false);
        let errored = page(&ch, 1);
        errored.status.set(PageStatus::Error);
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, false);
        s.record_page(0, &errored);
        assert!(rec.progress.borrow().is_empty());

        let (mut private, _rx2) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, true);
        private.record_page(0, &page(&ch, 2));
        assert!(rec.progress.borrow().is_empty());
        assert_eq!(ch.last_page_read.get(), 0, "incognito keeps the saved page untouched");
        assert!(!ch.read.get());
    }

    #[test]
    fn an_insert_page_changes_nothing() {
        let rec = Rc::new(Recorder::default());
        let ch = chapter(1, false);
        let parent = page(&ch, 1);
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&ch)], 0, false);
        let insert = ReaderPage::insert_page(&parent);
        assert_eq!(s.record_page(0, &insert), None);
        assert!(rec.progress.borrow().is_empty());
        assert_eq!(ch.last_page_read.get(), 0);
    }

    #[test]
    fn switching_to_a_new_chapter_is_reported_by_record_page() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let b = chapter(2, false);
        let (mut s, _rx) = session(rec, vec![Rc::clone(&a), Rc::clone(&b)], 0, false);
        assert_eq!(s.record_page(1, &page(&b, 0)), Some(1));
    }

    #[test]
    fn adjacent_navigation_is_disabled_at_the_ends_and_moves_between_chapters() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let (mut s, _rx) = session(rec, vec![Rc::clone(&a)], 0, false);
        assert_eq!(s.load_adjacent(Step::Next, never_built), AdjacentOutcome::NoNeighbour);
        assert_eq!(s.load_adjacent(Step::Previous, never_built), AdjacentOutcome::NoNeighbour);
    }

    #[test]
    fn the_previous_chapters_writes_are_flushed_before_the_new_chapter_is_built() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let b = ReaderChapter::new(Item::create(Kind::Manga));
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&a), b], 0, false);
        let order = Rc::clone(&rec);
        let _ = s.switch_chapter(1, |_| {
            order.order.borrow_mut().push("build");
            Err(LoadError::new("offline"))
        });
        assert_eq!(rec.order.borrow().as_slice(), ["flush", "build"]);
    }

    #[test]
    fn a_failed_adjacent_load_keeps_the_current_chapter() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let b = ReaderChapter::new(Item::create(Kind::Manga));
        let (mut s, _rx) = session(rec, vec![Rc::clone(&a), b], 0, false);
        let outcome = s.load_adjacent(Step::Next, |_| Err(LoadError::new("Source not found")));
        assert_eq!(outcome, AdjacentOutcome::Failed("Source not found".into()));
        assert_eq!(s.current_index(), Some(0));
    }

    #[test]
    fn preload_of_a_loaded_or_loading_chapter_does_nothing_and_a_failed_preload_is_silent() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let b = ReaderChapter::new(Item::create(Kind::Manga));
        let (mut s, rx) = session(rec, vec![Rc::clone(&a), Rc::clone(&b)], 0, false);
        s.preload(0, false, never_built);
        assert!(rx.try_recv().is_err());
        s.preload(1, false, |_| Err(LoadError::new("offline")));
        assert!(rx.try_recv().is_err(), "a failed preload sends no event");
        assert!(matches!(*b.state(), ChapterState::Error(_)));
    }

    #[test]
    fn a_new_reading_mode_is_written_to_the_manga_and_reloads_the_viewers() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        a.last_page_read.set(2);
        let (mut s, rx) = session(Rc::clone(&rec), vec![Rc::clone(&a)], 0, false);
        s.set_reading_mode(ReadingMode::Webtoon);
        let mask = i64::from(kuro_prefs::reader::READING_MODE_MASK);
        assert_eq!(*rec.flags.borrow(), [(s.manga_id, i64::from(ReadingMode::Webtoon.flag_value()), mask)], "only the mode bits of this manga");
        assert_eq!(crate::reader::flags::reading_mode_of(s.viewer_flags()), ReadingMode::Webtoon);
        assert_eq!(a.requested_page.get(), 2);
        assert_eq!(rx.try_recv(), Ok(ReaderEvent::ReloadViewerChapters));
    }

    #[test]
    fn a_page_selected_runs_progress_then_switch_then_download_ahead_then_flash() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let b = chapter(2, false);
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&a), Rc::clone(&b)], 0, false);
        s.set_download_ahead(1, Rc::new(|_| true));
        let log = RefCell::new(Vec::new());
        let switched = s.on_page_selected(
            &page(&b, 1),
            |_| {
                log.borrow_mut().push("build");
                Err(LoadError::new("offline"))
            },
            &mut |ids| log.borrow_mut().push(if ids == [2] { "ahead [2]" } else { "ahead other" }),
            &mut || log.borrow_mut().push("flash"),
        );
        assert_eq!(switched, Some(1));
        assert_eq!(log.borrow().as_slice(), ["build", "ahead [2]", "flash"], "a failed switch keeps chapter 0 current");
        assert_eq!(rec.progress.borrow().as_slice(), [(2, 1, false)]);
    }

    #[test]
    fn download_ahead_is_silent_when_off_or_when_the_amount_is_not_positive() {
        let rec = Rc::new(Recorder::default());
        let (a, b) = (chapter(1, false), chapter(2, false));
        let (mut s, _rx) = session(rec, vec![Rc::clone(&a), Rc::clone(&b)], 0, false);
        let calls = std::cell::Cell::new(0);
        s.on_page_selected(&page(&a, 2), never_built, &mut |_: &[i64]| calls.set(calls.get() + 1), &mut || {});
        s.set_download_ahead(0, Rc::new(|_| true));
        s.on_page_selected(&page(&a, 2), never_built, &mut |_: &[i64]| calls.set(calls.get() + 1), &mut || {});
        assert_eq!(calls.get(), 0);
    }

    fn chapter_numbered(id: i64, number: f64) -> Rc<ReaderChapter> {
        let mut c = Item::create(Kind::Manga);
        c.id = id;
        c.url = format!("/c/{id}");
        c.number = number;
        let ch = ReaderChapter::new(c);
        let pages = (0..3).map(|i| ReaderPage::new(i, format!("p{i}"), None, PageSource::Http)).collect::<Vec<_>>();
        ch.finish_loading(pages).unwrap();
        ch
    }

    #[test]
    fn completing_a_chapter_sends_its_number_once_and_never_in_incognito() {
        for (incognito, expected) in [(false, vec![(7, 12.5)]), (true, vec![])] {
            let rec = Rc::new(Recorder::default());
            let a = chapter_numbered(1, 12.5);
            let (mut s, _rx) = session(rec, vec![Rc::clone(&a)], 0, incognito);
            let sent = Rc::new(RefCell::new(Vec::new()));
            let sink = Rc::clone(&sent);
            s.set_track_update(Rc::new(move |manga, number| sink.borrow_mut().push((manga, number))));
            s.record_page(0, &page(&a, 2));
            s.record_page(0, &page(&a, 2));
            assert_eq!(*sent.borrow(), expected);
        }
    }

    #[test]
    fn a_page_before_the_last_sends_nothing_to_the_trackers() {
        let rec = Rc::new(Recorder::default());
        let a = chapter_numbered(1, 3.0);
        let (mut s, _rx) = session(rec, vec![Rc::clone(&a)], 0, false);
        let sent = Rc::new(std::cell::Cell::new(0));
        let sink = Rc::clone(&sent);
        s.set_track_update(Rc::new(move |_, _| sink.set(sink.get() + 1)));
        s.record_page(0, &page(&a, 1));
        assert_eq!(sent.get(), 0);
    }

    #[test]
    fn an_insert_page_or_an_unknown_chapter_runs_none_of_the_hooks() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let (mut s, _rx) = session(Rc::clone(&rec), vec![Rc::clone(&a)], 0, false);
        let calls = std::cell::Cell::new(0);
        let parent = page(&a, 0);
        let insert = ReaderPage::insert_page(&parent);
        s.on_page_selected(&insert, never_built, &mut |_: &[i64]| calls.set(calls.get() + 1), &mut || calls.set(calls.get() + 1));
        let stray = chapter(9, false);
        s.on_page_selected(&page(&stray, 0), never_built, &mut |_: &[i64]| calls.set(calls.get() + 1), &mut || calls.set(calls.get() + 1));
        assert_eq!(calls.get(), 0);
        assert!(rec.progress.borrow().is_empty());
    }

    #[test]
    fn a_preload_request_for_a_chapter_in_the_list_reaches_the_preload() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let b = ReaderChapter::new(Item::create(Kind::Manga));
        let (mut s, rx) = session(rec, vec![Rc::clone(&a), Rc::clone(&b)], 0, false);
        s.on_request_preload(&b, false, |_| Err(LoadError::new("offline")));
        assert!(matches!(*b.state(), ChapterState::Error(_)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn events_arrive_in_send_order_without_loss() {
        let rec = Rc::new(Recorder::default());
        let a = chapter(1, false);
        let (s, rx) = session(rec, vec![a], 0, false);
        for _ in 0..1000 {
            s.notify(ReaderEvent::PageChanged);
        }
        assert_eq!(rx.try_iter().count(), 1000);
    }

    #[test]
    fn requested_chapter_missing_gives_the_generic_message() {
        assert_eq!(initial_index(&[4, 5], 5), Ok(1));
        assert_eq!(initial_index(&[4, 5], 9), Err("Requested chapter of id 9 not found in chapter list".into()));
    }

    #[test]
    fn list_errors_have_user_messages() {
        assert_eq!(
            list_error_message(&ListError::ChapterNotFound(3)),
            "Requested chapter of id 3 not found in chapter list"
        );
        assert!(list_error_message(&ListError::CurrentChapterMissing).starts_with("The chapter is not downloaded"));
    }

    #[test]
    fn e_list_1_a_rebuilt_list_keeps_the_open_chapter_object_and_its_place() {
        let rec = Rc::new(Recorder::default());
        let (one, two, three) = (chapter(1, false), chapter(2, false), chapter(3, false));
        let (mut s, _rx) = session(Rc::clone(&rec), vec![one, Rc::clone(&two), three], 1, false);
        // the skip rule drops chapter 3 and the rebuilt list holds a new object for chapter 2
        s.replace_list(vec![chapter(2, false), chapter(1, false)]);
        assert_eq!(s.current_index(), Some(0), "the open chapter keeps the current place in the new list");
        assert!(Rc::ptr_eq(s.current_chapter().expect("a current chapter"), &two), "its loaded object is kept");
        assert_eq!(s.chapters()[1].key().id, 1, "the other chapters are the rebuilt ones");
    }
}
