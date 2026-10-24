//! The reader screen: the initial load behind the source gate, the failure card with Retry, adjacent chapter
//! navigation and the crop toggle. T-43a-023, T-43a-024, T-43a-026, T-43a-037 (wiring), part of T-43a-077.
//!
//! [`ReaderCore`] holds the behaviour and has no widgets, so it is tested without a display. [`open`] builds the
//! widgets over it. Once the chapter has loaded, the screen builds the viewer of the manga's reading mode
//! (`viewers::make_viewer` behind `viewers::adapter::EngineViewer`) and attaches it to the `viewer` slot of the returned
//! [`ReaderScreen`]. A mode change destroys it and attaches the next one.
//!
//! ponytail: a chapter load runs on the GTK thread (a `ReaderChapter` is `Rc`, so it cannot move to a worker). The
//! spinner is shown and one frame is let through before the load starts, so the screen is never modal, but it does
//! not animate while an HTTP page list is fetched. Move the page-list fetch to the runtime when the viewers land.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use gtk::glib;
use gtk::prelude::*;
use kuro_core::sources::SourceGate;
use kuro_domain::download::prefs::DownloadPrefs;
use kuro_track::policy::auto_update_allowed;
use kuro_track::prefs::TrackPrefs;
use kuro_data::item::Item;
use kuro_db::Db;
use kuro_platform::toast::{ToastLength, Toaster};
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_prefs::reader::{MangaReaderPrefs, ReaderPrefs, ReadingMode};
use tokio::sync::watch;

use super::chapters::ListOptions;
use super::delete_queue::Pending;
use super::chrome::{self, MenuHandlers, TopBar, TopBarActions, build_bare_top_bar, build_error_card, build_top_bar};
use super::flags;
use super::history_rules;
use super::page_actions;
use super::page_save;
use super::saved_notice;
use super::restore;
use crate::misc::diet_defaults as diet;
use crate::widgets::page_header::{self, TwoLineTitle};
use super::keep_awake::{self, KeepAwake};
use super::loader::{LoadFailure, Loader, load_chapter};
use super::model::{LoadError, PageStatus, ReaderChapter, ReaderPage, ViewerChapters};
use super::nav::{self, Step};
use super::nav_overlay::{self, NavOverlay};
use super::navigator::{Navigator, NavigatorState};
use super::overlay::Tint;
use super::session::{AdjacentOutcome, ReaderEvent, ReaderSession, ReaderStore, TrackUpdate, open_list};
use super::settings_window;
use super::sheet::{self, Sheet};
use super::source_pages;
use super::sync::{self, Effect};
use super::state::{self, Dialog, EscapeAction, UiState};
use super::strings;
use super::theme;
use super::viewer::{self, Viewer, ViewerEvent, ViewerSlot};
use super::viewers::adapter::{self, EngineViewer};
use super::viewers::downloads::DownloadCheck;
use super::viewers::settings::ViewerSettings;
use super::viewers::{Emitter, ViewerContext, ViewerKind, make_viewer};

/// Picks the loader of a chapter (see `loader::choose_loader`).
pub type BuildLoader = Rc<dyn Fn(&Rc<ReaderChapter>) -> Result<Loader, LoadError>>;

/// What the reader needs from the app.
pub struct ReaderDeps {
    pub db: Arc<Db>,
    pub manga_id: i64,
    pub chapter_id: i64,
    pub prefs: ReaderPrefs,
    pub store: Rc<dyn ReaderStore>,
    pub build: BuildLoader,
    pub is_downloaded: Rc<dyn Fn(&Item) -> bool>,
    pub downloaded_only: bool,
    pub incognito: bool,
    /// The store behind `prefs`. Its change feed applies live settings to the open reader.
    pub prefs_store: kuro_prefs::store::PrefStore,
}

/// Deletes the downloads of the chapters read (T-43a-032). `defer` gets each chapter the moment it is queued, and `flush`
/// is called once, when the reader closes. Both run on the GTK thread and do their work elsewhere.
#[derive(Clone)]
pub struct DeleteSink {
    pub defer: Rc<dyn Fn(Pending)>,
    pub flush: Rc<dyn Fn()>,
}

/// What a reader opened by the app has beyond [`ReaderDeps`]: where it keeps its crash-restore record and its delete
/// queue, how it deletes downloads, and the download index for its transition cards. The default has none of them, so
/// a reader that does not need them (a local manga) is opened with [`open`] and writes nothing.
pub struct ReaderExtras {
    /// The platform data directory. Without it no restore record is kept.
    pub data_dir: Option<PathBuf>,
    /// Deletes the downloads of the chapters queued by delete-after-read. Without it the rule is off.
    pub delete_sink: Option<DeleteSink>,
    pub downloads: DownloadCheck,
    /// Sends a completed chapter's number to the trackers (T-43a-030). The reader calls it only while
    /// `pref_auto_update_manga_sync_key` is on and the session is not incognito. Without it nothing is sent.
    pub track_update: Option<TrackUpdate>,
    /// Queues the chapters download-ahead picked: the manga id and the chapter ids (T-43a-034). Without it the rule is off.
    pub download_ahead: Option<DownloadAheadSink>,
    /// Starts the source work Retry on the failure card waits for (an online chapter's page-list fetch), and gives its
    /// gate. Without it Retry loads again at once.
    pub retry_gate: Option<Rc<dyn Fn() -> SourceGate>>,
}

/// Queues the given chapters of a manga for download.
pub type DownloadAheadSink = Rc<dyn Fn(i64, &[i64])>;

impl Default for ReaderExtras {
    fn default() -> Self {
        Self { data_dir: None, delete_sink: None, downloads: DownloadCheck::default(), track_update: None, download_ahead: None, retry_gate: None }
    }
}

/// The crop button after a toggle: the new value, the icon that reflects it and the toast text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CropView {
    pub value: bool,
    pub icon: &'static str,
    pub toast: String,
}

pub const CROP_ICON_ON: &str = "image-crop-symbolic";
pub const CROP_ICON_OFF: &str = "image-x-generic-symbolic";

/// The crop icon for a state: crop when on, crop-off when off.
pub fn crop_icon(on: bool) -> &'static str {
    if on { CROP_ICON_ON } else { CROP_ICON_OFF }
}

/// The toast for a failed adjacent load. A blank message never gives an empty toast (E-TOAST-1).
pub fn failure_toast(message: &str) -> String {
    if message.trim().is_empty() { strings::unknown_error().to_owned() } else { message.to_owned() }
}

/// Resolves once the source manager has finished initialising. `false` when it went away without doing so.
pub async fn wait_ready(mut ready: watch::Receiver<bool>) -> bool {
    while !*ready.borrow_and_update() {
        if ready.changed().await.is_err() {
            return false;
        }
    }
    true
}

/// The reader's behaviour without widgets.
pub struct ReaderCore {
    deps: ReaderDeps,
    extras: ReaderExtras,
    session: RefCell<Option<ReaderSession>>,
    events: RefCell<Option<Receiver<ReaderEvent>>>,
    manga_title: RefCell<String>,
    /// The delete queue was handed to the sink; a later close does not hand it over again.
    deletes_flushed: Cell<bool>,
}

impl ReaderCore {
    pub fn new(deps: ReaderDeps) -> Self {
        Self::with_extras(deps, ReaderExtras::default())
    }

    pub fn with_extras(deps: ReaderDeps, extras: ReaderExtras) -> Self {
        Self {
            deps,
            extras,
            session: RefCell::new(None),
            events: RefCell::new(None),
            manga_title: RefCell::new(String::new()),
            deletes_flushed: Cell::new(false),
        }
    }

    /// The download index for the viewers' transition cards.
    pub fn downloads(&self) -> DownloadCheck {
        self.extras.downloads.clone()
    }

    /// Where the reader starts (T-43a-039): the explicit chapter, or the chapter and page of a fresh crash-restore record
    /// when none was asked for and `restore_after_crash` is on.
    pub fn start(&self) -> restore::Start {
        let saved = self.extras.data_dir.as_deref().and_then(|dir| restore::load(&restore::path_in(dir)));
        let enabled = diet::READER_RESTORE_AFTER_CRASH;
        restore::resolve(self.deps.manga_id, self.deps.chapter_id, saved, history_rules::now_ms(), enabled)
    }

    /// Tells the sink to delete what delete-after-read queued, once, when the reader closes (T-43a-032). The chapters were
    /// handed over as they were queued and wait in the service until now, so those of a reader that never closed are
    /// deleted by the next flush.
    pub fn flush_deletes(&self) {
        let Some(sink) = &self.extras.delete_sink else { return };
        if self.deletes_flushed.replace(true) {
            return;
        }
        (sink.flush)();
    }

    /// The reader's event feed for the viewers, once the initial load has made it. Taken once.
    pub fn take_events(&self) -> Option<Receiver<ReaderEvent>> {
        self.events.borrow_mut().take()
    }

    /// The settings a new viewer starts from, read from the preference store (T-44a-004).
    pub fn viewer_settings(&self) -> ViewerSettings {
        ViewerSettings::from_manga(&self.reader_prefs())
    }

    /// The chapter list rules, from the skip preferences of the reader.
    fn list_options(&self, manga: &Entry) -> ListOptions {
        ListOptions {
            skip_read: diet::READER_SKIP_READ,
            skip_filtered: diet::READER_SKIP_FILTERED,
            skip_dupe: diet::READER_SKIP_DUPE,
            downloaded_only: self.deps.downloaded_only,
            item_flags: manga.item_flags,
        }
    }

    /// Builds the chapter list again after a skip rule changed (E-LIST-1). The chapter being read stays open with its
    /// loaded pages.
    pub fn rebuild_list(&self) -> Result<(), String> {
        let Some(current) = self.session.borrow().as_ref().and_then(|s| s.current_chapter().map(|c| c.key().id)) else {
            return Ok(());
        };
        let opened = open_list(Arc::clone(&self.deps.db), self.deps.manga_id, current, |manga| self.list_options(manga), |chapter| {
            (self.deps.is_downloaded)(chapter)
        })?;
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.replace_list(opened.chapters);
        }
        Ok(())
    }

    /// The preference store handle a viewer follows for live changes (T-44a-005).
    /// The reader settings of the open manga: its own values, else the defaults of the Settings page.
    pub fn reader_prefs(&self) -> MangaReaderPrefs {
        MangaReaderPrefs::new(self.deps.prefs.clone(), self.deps.manga_id)
    }

    /// Loads the manga, builds the reader list and loads the requested chapter (T-43a-023). A failure leaves no
    /// session behind, so a retry repeats the whole load (T-43a-024).
    pub fn initial_load(&self) -> Result<(), LoadError> {
        *self.session.borrow_mut() = None;
        let start = self.start();
        let opened = open_list(Arc::clone(&self.deps.db), self.deps.manga_id, start.chapter_id, |manga| self.list_options(manga), |chapter| {
            (self.deps.is_downloaded)(chapter)
        })
        .map_err(LoadError::new)?;
        let preserve = diet::READER_PRESERVE_READING_POSITION;
        let chapter = Rc::clone(&opened.chapters[opened.index]);
        load_chapter(&chapter, || (self.deps.build)(&chapter), preserve).map_err(|failure| match failure {
            LoadFailure::Failed(error) => error,
            LoadFailure::Cancelled => LoadError::new(strings::unknown_error()),
        })?;
        if let (Some(page), Some(pages)) = (start.page_index, chapter.pages()) {
            // a restored page is where the viewer opens, kept inside the chapter
            chapter.requested_page.set(page.clamp(0, pages.len().saturating_sub(1) as i64));
        }
        *self.manga_title.borrow_mut() = opened.manga.title.clone();
        let (tx, rx) = mpsc::channel();
        let mut session = ReaderSession::new(
            self.deps.manga_id,
            opened.manga.viewer_flags,
            opened.chapters,
            opened.index,
            self.deps.incognito,
            preserve,
            Rc::clone(&self.deps.store),
            tx,
        );
        session.set_mark_duplicates_read(diet::LIBRARY_MARK_DUPLICATE_CHAPTER_READ);
        if let Some(sink) = &self.extras.delete_sink {
            // the slot count is read once, when the reader opens
            let slots = DownloadPrefs::new(&self.deps.prefs_store).remove_after_read_slots.get();
            session.set_delete_after_read(i64::from(slots), Rc::clone(&sink.defer));
        }
        if let Some(sink) = &self.extras.track_update {
            let (sink, prefs, incognito) = (Rc::clone(sink), TrackPrefs::new(self.deps.prefs_store.clone()), self.deps.incognito);
            session.set_track_update(Rc::new(move |manga_id, number| {
                if auto_update_allowed(&prefs, incognito) {
                    sink(manga_id, number);
                }
            }));
        }
        if self.extras.download_ahead.is_some() {
            // one chapter ahead, past a quarter of the open one (a fixed rule: the amount is not a setting)
            let amount = diet::DOWNLOAD_AUTO_WHILE_AMOUNT;
            let downloaded = Rc::clone(&self.deps.is_downloaded);
            session.set_download_ahead(amount, Rc::new(move |chapter| downloaded(chapter)));
        }
        session.start_history(history_rules::now_ms());
        *self.session.borrow_mut() = Some(session);
        *self.events.borrow_mut() = Some(rx);
        Ok(())
    }

    /// Whether the previous and next buttons are enabled: off at the ends and before the session exists.
    pub fn nav_enabled(&self) -> (bool, bool) {
        self.session.borrow().as_ref().map_or((false, false), |s| {
            let (previous, next) = s.neighbours();
            (previous.is_some(), next.is_some())
        })
    }

    /// Moves to the next or previous chapter. A failure keeps the current chapter (E-NAV-1).
    pub fn step(&self, step: Step) -> AdjacentOutcome {
        match self.session.borrow_mut().as_mut() {
            Some(session) => session.load_adjacent(step, |chapter| (self.deps.build)(chapter)),
            None => AdjacentOutcome::NoNeighbour,
        }
    }

    /// What the top bar shows for the current chapter: the manga title, the chapter name, the bookmark state, and
    /// whether the chapter comes from an HTTP source (which gets the web entries in the overflow menu).
    pub fn top_bar(&self) -> Option<TopBar> {
        let session = self.session.borrow();
        let chapter = session.as_ref()?.current_chapter()?;
        Some(TopBar {
            manga_title: self.manga_title.borrow().clone(),
            chapter_name: chapter.chapter.name.clone(),
            bookmarked: chapter.bookmark.get(),
            is_http_source: chapter.attached_is_local() == Some(false),
        })
    }

    /// The id of the chapter being read.
    pub fn current_chapter_id(&self) -> Option<i64> {
        self.session.borrow().as_ref()?.current_chapter().map(|c| c.chapter.id)
    }

    /// The source url of the chapter being read.
    pub fn chapter_url(&self) -> Option<String> {
        self.session.borrow().as_ref()?.current_chapter().map(|c| c.chapter.url.clone())
    }

    /// Flips the bookmark of the current chapter and writes it. Returns the new state.
    pub fn toggle_bookmark(&self) -> Option<Result<bool, String>> {
        self.session.borrow().as_ref()?.toggle_bookmark()
    }

    /// The page on screen: the current chapter's page at the visible position.
    pub fn visible_page(&self) -> Option<Rc<ReaderPage>> {
        let session = self.session.borrow();
        let session = session.as_ref()?;
        let pages = session.current_chapter()?.pages()?;
        let index = session.current_page().checked_sub(1)? as usize;
        pages.get(index).cloned()
    }

    /// The folder of the manga when it is a local one (its url is the folder's absolute path). `None` for any other source.
    pub fn local_folder(&self) -> Option<PathBuf> {
        let manga = EntryRepo::new(Arc::clone(&self.deps.db)).by_id(self.deps.manga_id).ok()?;
        kuro_local::is_local_source(manga.source).then(|| PathBuf::from(manga.url))
    }

    /// The toast after a local manga's cover write. The new thumbnail and its time go on the manga row, which the library
    /// reads its cover from.
    pub fn local_cover_result(&self, written: Result<kuro_local::images::CoverWrite, String>) -> String {
        let Ok(cover) = written else { return strings::notification_cover_update_failed() };
        let update = EntryUpdate {
            thumbnail_url: Some(cover.thumbnail_url),
            cover_last_modified: Some(cover.cover_last_modified),
            ..EntryUpdate::new(self.deps.manga_id)
        };
        match EntryRepo::new(Arc::clone(&self.deps.db)).update(&update) {
            Ok(()) => strings::cover_updated(),
            Err(_) => strings::notification_cover_update_failed(),
        }
    }

    /// The toast after a cover write (T-43a-058). The cover is written first, then the library check (E-COVER-1).
    pub fn cover_result(&self, written: Result<(), String>) -> String {
        if written.is_err() {
            return strings::notification_cover_update_failed();
        }
        let manga = match EntryRepo::new(Arc::clone(&self.deps.db)).by_id(self.deps.manga_id) {
            Ok(manga) => manga,
            Err(_) => return strings::notification_cover_update_failed(),
        };
        if !manga.favorite {
            return strings::notification_first_add_to_library();
        }
        let update = EntryUpdate { id: self.deps.manga_id, cover_last_modified: Some(history_rules::now_ms()), ..Default::default() };
        match EntryRepo::new(Arc::clone(&self.deps.db)).update(&update) {
            Ok(()) => strings::cover_updated(),
            Err(_) => strings::notification_cover_update_failed(),
        }
    }

    /// The manga title and the chapter name of a page, for the saved file's name (T-43a-054).
    pub fn page_names(&self, page: &ReaderPage) -> (String, String) {
        let chapter = page.chapter().map(|c| c.chapter.name.clone()).unwrap_or_default();
        (self.manga_title.borrow().clone(), chapter)
    }

    /// Writes the read time of the open chapter when the reader leaves the screen (T-43a-033).
    pub fn pause_history(&self) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.flush_history(history_rules::now_ms());
        }
    }

    /// Starts the read timer again when the reader is shown after a pause (T-43a-033).
    pub fn resume_history(&self) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.resume_history(history_rules::now_ms());
        }
    }

    /// The current chapter and its neighbours, for a viewer that is attached.
    pub fn viewer_chapters(&self) -> Option<ViewerChapters> {
        self.session.borrow().as_ref()?.viewer_chapters()
    }

    /// The slider's move: asks `viewer` to show the page at the 0-based `index` of the current chapter. `false`
    /// when there is no such page.
    pub fn move_viewer_to(&self, viewer: &dyn Viewer, index: usize) -> bool {
        let page = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.current_chapter().and_then(|c| c.pages()))
            .and_then(|pages| pages.get(index).cloned());
        match page {
            Some(page) => {
                viewer.move_to_page(&page);
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    /// The current chapter's position in the reader list.
    pub fn current_index(&self) -> Option<usize> {
        self.session.borrow().as_ref().and_then(|s| s.current_index())
    }

    /// The crop state for the current reading mode, for the button icon when the screen opens.
    pub fn crop_on(&self) -> bool {
        self.reader_prefs().crop_borders().get()
    }

    /// Flips the crop setting of the open manga and stores it (T-43a-037). One switch serves every mode, the long strip
    /// follows it.
    pub fn toggle_crop(&self) -> CropView {
        let crop = self.reader_prefs().crop_borders();
        let toggle = flags::toggle_crop(self.mode(), crop.get(), crop.get());
        if let Err(e) = crop.set(toggle.value) {
            tracing::warn!(error = %e, "the crop preference could not be saved");
        }
        CropView { value: toggle.value, icon: crop_icon(toggle.value), toast: strings::on_off(toggle.value) }
    }

    /// The resolved reading mode of the current manga (the default preference applies to a Default manga).
    pub fn reading_mode(&self) -> ReadingMode {
        self.mode()
    }

    /// The visible page (1-based, `-1` before any page) and the page total of the current chapter.
    pub fn page_counts(&self) -> (i32, i32) {
        self.session.borrow().as_ref().map_or((-1, 0), |s| {
            let total = s.current_index().and_then(|i| s.chapters().get(i)).and_then(|c| c.pages()).map_or(0, |p| p.len());
            (s.current_page() as i32, total as i32)
        })
    }

    /// Stores the picked reading mode for the open manga (its viewer flags) and reloads the viewers.
    pub fn apply_mode(&self, mode: ReadingMode) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.set_reading_mode(mode);
        }
    }

    pub fn theme_pref(&self) -> i32 {
        self.reader_prefs().reader_theme().get()
    }

    /// `on_page_selected` for a page a viewer shows (T-043a-093). Download-ahead and the flash are the hooks the
    /// caller passes in. Download-ahead gets the chapter ids to queue.
    pub fn page_selected(
        &self,
        page: &Rc<ReaderPage>,
        download_ahead: &mut dyn FnMut(&[i64]),
        flash: &mut dyn FnMut(),
    ) -> Option<usize> {
        let build = Rc::clone(&self.deps.build);
        self.session.borrow_mut().as_mut().and_then(|s| s.on_page_selected(page, |c| build(c), download_ahead, flash))
    }

    /// A viewer shows a transition card: the page counter has no number.
    pub fn transition_shown(&self) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.clear_current_page();
        }
    }

    /// Hands the chapters download-ahead picked to the sink, when the app gave one (T-43a-034).
    pub fn queue_ahead(&self, chapter_ids: &[i64]) {
        if let Some(sink) = &self.extras.download_ahead {
            sink(self.deps.manga_id, chapter_ids);
        }
    }

    /// A viewer asked for a chapter to be loaded in the background (T-043a-093).
    pub fn request_preload(&self, chapter: &Rc<ReaderChapter>) {
        let build = Rc::clone(&self.deps.build);
        let downloaded = (self.deps.is_downloaded)(&chapter.chapter);
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.on_request_preload(chapter, downloaded, |c| build(c));
        }
    }

    pub fn show_reading_mode(&self) -> bool {
        diet::READER_SHOW_READING_MODE
    }

    pub fn fullscreen_pref(&self) -> bool {
        diet::READER_FULLSCREEN
    }

    pub fn keep_awake_pref(&self) -> bool {
        diet::READER_KEEP_SCREEN_ON
    }

    fn mode(&self) -> kuro_prefs::reader::ReadingMode {
        // the manga's own mode, else the default of the Settings page
        let viewer_flags = self.session.borrow().as_ref().map_or(0, |s| s.viewer_flags());
        flags::resolve_reading_mode(viewer_flags, self.deps.prefs.default_reading_mode.get())
    }
}

/// The reader widget and the slot the viewers fill. `chrome` is what the viewers call: taps toggle the menu.
pub struct ReaderScreen {
    pub root: gtk::Box,
    pub viewer: gtk::Box,
    pub core: Rc<ReaderCore>,
    pub chrome: ReaderChrome,
}

/// The chrome controls for the viewers (area 44): a tap toggles the menu, and the menu drives the fullscreen state.
#[derive(Clone)]
pub struct ReaderChrome {
    screen: Weak<Screen>,
}

impl ReaderChrome {
    /// Routes a viewer event to the session and the chrome (T-043a-093). A long press waits for the page sheet
    /// (T-043a-045).
    pub fn route(&self, event: ViewerEvent) {
        let Some(screen) = self.screen.upgrade() else { return };
        match event {
            ViewerEvent::ToggleMenu => self.toggle_menu(),
            ViewerEvent::ShowMenu => self.show_menu(),
            ViewerEvent::HideMenu => self.hide_menu(),
            ViewerEvent::PageSelected(page) => {
                let tint = &screen.tint;
                // the flash counts only while the feature is on, so enabling it later does not flash at once
                let mut flash = || {
                    if diet::READER_FLASH_ON_PAGE_CHANGE {
                        tint.on_page_changed();
                    }
                };
                let switched = screen.core.page_selected(&page, &mut |ids| screen.core.queue_ahead(ids), &mut flash);
                if switched.is_some() {
                    // the page belongs to another chapter, which is now the current one: the viewer gets its neighbours
                    reload_chapters(&screen);
                }
                sync_top_bar(&screen);
                refresh(&screen);
            }
            ViewerEvent::TransitionShown => {
                screen.core.transition_shown();
                refresh(&screen);
            }
            ViewerEvent::RequestPreload(chapter) => screen.core.request_preload(&chapter),
            ViewerEvent::LongTap(page) => open_page_actions(&screen, page),
        }
    }

    /// Attaches a page viewer (area 44): the previous viewer is destroyed, `ReaderScreen::viewer` is cleared, and the
    /// new viewer gets the current chapters through `set_chapters`, where it adds its widget to the container. From
    /// then on the slider moves it.
    pub fn attach_viewer(&self, next: Box<dyn Viewer>) {
        let Some(screen) = self.screen.upgrade() else { return };
        let container = screen.viewer_box.clone();
        let chapters = screen.core.viewer_chapters();
        {
            let mut slot = screen.viewer_slot.borrow_mut();
            viewer::swap(
                &mut slot,
                next,
                || {
                    while let Some(child) = container.first_child() {
                        container.remove(&child);
                    }
                },
                |v| {
                    if let Some(chapters) = &chapters {
                        v.set_chapters(chapters);
                    }
                },
            );
        }
        move_references(&screen.viewer_refs, chapters);
    }

    pub fn toggle_menu(&self) {
        if let Some(screen) = self.screen.upgrade() {
            let visible = !screen.ui.borrow().menu_visible;
            set_menu(&screen, visible);
        }
    }

    pub fn show_menu(&self) {
        if let Some(screen) = self.screen.upgrade() {
            set_menu(&screen, true);
        }
    }

    pub fn hide_menu(&self) {
        if let Some(screen) = self.screen.upgrade() {
            if screen.keep_menu_until.get().is_some_and(|until| std::time::Instant::now() < until) {
                return;
            }
            set_menu(&screen, false);
        }
    }
}

/// The widget state the reader's handlers share. The root is held weakly, so the screen does not keep its own
/// widget alive.
struct Screen {
    core: Rc<ReaderCore>,
    /// The gate the initial load waits for; Retry replaces it with the retry gate's.
    ready: RefCell<watch::Receiver<bool>>,
    toaster: Toaster,
    on_close: Rc<dyn Fn()>,
    root: glib::WeakRef<gtk::Box>,
    stack: gtk::Stack,
    /// The widget the viewers fill (`ReaderScreen::viewer`).
    viewer_box: gtk::Box,
    /// The viewer attached by area 44. The slider moves it; with none attached the slider has nothing to move.
    viewer_slot: RefCell<ViewerSlot>,
    /// The chapter set the viewer holds references to. A new set is referenced before the old one is released (T-43a-002).
    viewer_refs: RefCell<Option<ViewerChapters>>,
    /// The reader menu's flag, read by the viewers (long tap, volume keys); `set_menu` keeps it current.
    menu_open: Rc<Cell<bool>>,
    /// The reading mode the attached viewer was built for, so a reload knows whether the mode changed.
    viewer_kind: Cell<Option<ViewerKind>>,
    /// The core's event feed, taken once the initial load has made it (T-043a-086).
    events: RefCell<Option<std::sync::mpsc::Receiver<ReaderEvent>>>,
    tint: Tint,
    /// The tap zones drawn over the pages (G-27).
    nav_overlay: NavOverlay,
    /// Holds the top bar, which is rebuilt when the chapter or its bookmark changes.
    top_slot: gtk::Box,
    /// The id of the chapter the top bar shows, `-1` before the first build.
    shown_chapter: Cell<i64>,
    top: gtk::Revealer,
    /// The bars hide by themselves (`auto_hide_bars`, read when the reader opens).
    auto_hide: bool,
    bottom: gtk::Revealer,
    navigator: Rc<Navigator>,
    /// Until then a hide request from a viewer is ignored: the slider and the chapter buttons move the pages, and
    /// that move must not read as a swipe that hides the bar.
    keep_menu_until: Cell<Option<std::time::Instant>>,
    /// The `reader` actions of the top bar's menu, installed on the root.
    menu_actions: gtk::gio::SimpleActionGroup,
    /// The title of the top bar shown now, which carries the page position. `None` while the bar is bare.
    position: RefCell<Option<TwoLineTitle>>,
    sheet: Rc<Sheet>,
    ui: RefCell<UiState>,
    keep_awake: KeepAwake,
    /// An adjacent load is waiting for its frame or running; further presses are ignored.
    stepping: Cell<bool>,
}

const PAGE_LOADING: &str = "loading";
const PAGE_CARD: &str = "card";
const PAGE_VIEWER: &str = "viewer";
/// How often a chapter whose page list is still being fetched is asked for again. The spinner shows meanwhile.
const PENDING_RETRY: std::time::Duration = std::time::Duration::from_millis(250);

/// `reader::open`: the reader page. It waits for the source manager, loads the manga and the requested chapter,
/// and shows the failure card with Retry and Close when that fails. The root widget is named `screen:reader`.
pub fn open(deps: ReaderDeps, gate: &SourceGate, toaster: Toaster, on_close: impl Fn() + 'static) -> ReaderScreen {
    open_with(deps, ReaderExtras::default(), gate, toaster, on_close)
}

/// [`open`] with the crash-restore, delete-after-read and download-index extras of an app-opened reader.
pub fn open_with(
    deps: ReaderDeps,
    extras: ReaderExtras,
    gate: &SourceGate,
    toaster: Toaster,
    on_close: impl Fn() + 'static,
) -> ReaderScreen {
    // Decodes started by an earlier reader are stale from now on (T-43b-059).
    crate::reader::decode_guard::new_session();
    // and so are the pages the last reader kept decoded
    crate::reader::page_view::clear_decoded_cache();
    let theme_value = MangaReaderPrefs::new(deps.prefs.clone(), deps.manga_id).reader_theme().get();
    let prefs_store = deps.prefs_store.clone();
    let core = Rc::new(ReaderCore::with_extras(deps, extras));
    // closing the reader first hands the delete-after-read queue over (T-43a-032)
    let on_close = {
        let core = Rc::clone(&core);
        move || {
            core.flush_deletes();
            on_close();
        }
    };

    let viewer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    viewer.set_vexpand(true);
    let loading = gtk::Spinner::builder().halign(gtk::Align::Center).valign(gtk::Align::Center).spinning(true).build();
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&loading, Some(PAGE_LOADING));
    stack.add_named(&viewer, Some(PAGE_VIEWER));

    // the overlays sit over the pages only, below the chrome and the sheets
    let page_layer = gtk::Overlay::new();
    let tint = Tint::new(&page_layer, &stack);
    let nav_overlay = NavOverlay::new(&page_layer);

    let navigator = Navigator::new();
    let top_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let top = gtk::Revealer::new();
    top.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    top.set_transition_duration(200);
    top.set_child(Some(&top_slot));
    top.set_reveal_child(true);
    // the page slider with the chapter buttons is the only thing in the floating bottom bar
    let bottom_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom_column.append(&navigator.root);
    // a press on the bar is picked by the bar (an overlay above the pages), so it never reaches the page's tap handler;
    // no extra gesture here, as claiming the press on the bar would also steal the slider's own drag
    let bottom = gtk::Revealer::new();
    bottom.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    bottom.set_transition_duration(200);
    bottom.set_child(Some(&bottom_column));
    bottom.set_reveal_child(true);

    let sheet = Sheet::new(&page_layer);
    // the bottom bar floats over the pages (as in Aniyomi): showing it never resizes the page area, so the pages do
    // not move and nothing reads the toggle as a scroll that hides it again
    // it goes on the page layer, so the reader's sheets still open above it
    bottom.set_valign(gtk::Align::End);
    page_layer.add_overlay(&bottom);
    // with auto-hide (`auto_hide_bars`) the top bar floats over the pages too, so hiding it never moves them; without it
    // the bar sits above the pages and never covers one
    let auto_hide = auto_hide_bars();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if auto_hide {
        top.set_valign(gtk::Align::Start);
        page_layer.add_overlay(&top);
    } else {
        column.append(&top);
    }
    column.append(sheet.root());
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_widget_name("screen:reader");
    root.set_focusable(true);
    root.append(&column);
    // only the pages take the reader's colours: the top bar keeps the GTK theme's, or its title turns white on a
    // light bar
    // the pages sit on their own background, so the colour filter blends with it and not with what is behind
    theme::apply_background(&stack, theme_value);

    let root_ref = glib::WeakRef::new();
    root_ref.set(Some(&root));
    let screen = Rc::new_cyclic(|me| Screen {
        core: Rc::clone(&core),
        ready: RefCell::new(gate.ready_watch()),
        toaster,
        on_close: Rc::new(on_close),
        root: root_ref,
        stack,
        viewer_box: viewer.clone(),
        viewer_slot: RefCell::new(None),
        viewer_refs: RefCell::new(None),
        viewer_kind: Cell::new(None),
        menu_open: Rc::new(Cell::new(state::UiState::default().menu_visible)),
        events: RefCell::new(None),
        tint,
        nav_overlay,
        top_slot,
        shown_chapter: Cell::new(-1),
        top,
        bottom,
        auto_hide,
        navigator: Rc::clone(&navigator),
        keep_menu_until: Cell::new(None),
        menu_actions: chrome::menu_actions(core.reading_mode(), core.crop_on(), menu_handlers(me)),
        position: RefCell::new(None),
        sheet: Rc::clone(&sheet),
        ui: RefCell::new(UiState::default()),
        keep_awake: KeepAwake::default(),
        stepping: Cell::new(false),
    });
    root.insert_action_group(chrome::ACTION_GROUP, Some(&screen.menu_actions));

    let weak = Rc::downgrade(&screen);
    let (step_weak, slider_weak) = (weak.clone(), weak.clone());
    navigator.connect(
        move |step| {
            if let Some(s) = step_weak.upgrade() {
                press(&s, step);
            }
        },
        move |index| {
            if let Some(s) = slider_weak.upgrade() {
                move_viewer(&s, index);
            }
        },
    );
    // a resize across the wide-window threshold changes the navigator padding
    crate::window_size::on_tablet_change({
        let weak = weak.clone();
        move |_| match weak.upgrade() {
            Some(s) => {
                refresh(&s);
                true
            }
            None => false,
        }
    });
    let dismissed = weak.clone();
    sheet.connect_dismissed(move || {
        if let Some(s) = dismissed.upgrade() {
            s.ui.borrow_mut().dialog = None;
            // the focus came into the sheet, so it goes back to the reader for the keys
            if let Some(root) = s.root.upgrade() {
                root.grab_focus();
            }
        }
    });

    let keys = gtk::EventControllerKey::new();
    {
        let screen = Rc::clone(&screen);
        keys.connect_key_pressed(move |_, key, _, state| handle_key(&screen, key, state));
    }
    root.add_controller(keys);
    // with auto-hide, the mouse at the top or the bottom edge (where the bars live) brings the bars back
    if screen.auto_hide {
        let motion = gtk::EventControllerMotion::new();
        let (weak, area) = (Rc::downgrade(&screen), root.clone());
        motion.connect_motion(move |_, _, y| {
            let at_edge = at_bar_edge(y, f64::from(area.height()));
            if let Some(screen) = weak.upgrade().filter(|s| at_edge && !s.ui.borrow().menu_visible) {
                set_menu(&screen, true);
            }
        });
        root.add_controller(motion);
    }
    {
        let screen = Rc::clone(&screen);
        root.connect_map(move |root| {
            root.grab_focus();
            // shown again after a pause (for example back from the manga details): time reading again (T-43a-033)
            screen.core.resume_history();
            sync_keep_awake(&screen, true);
        });
    }
    {
        // a reader taken out of the window without its close button still hands its queue over
        let core = Rc::clone(&core);
        // and the decoded pages (they stay cached after their views go) are freed with the reader, so a closed reader
        // keeps no pixels in memory
        root.connect_destroy(move |_| {
            core.flush_deletes();
            super::page_view::clear_decoded_cache();
        });
    }
    {
        let screen = Rc::clone(&screen);
        root.connect_unmap(move |_| {
            // leaving the reader (or hiding it) ends the read session, so its time is written (T-43a-033)
            screen.core.pause_history();
            sync_keep_awake(&screen, false);
        });
    }

    screen.tint.apply();
    sync::follow(&prefs_store, weak.clone(), apply_effect);
    start(&screen);
    ReaderScreen { root, viewer, core, chrome: ReaderChrome { screen: weak } }
}

fn window_of(screen: &Screen) -> Option<gtk::Window> {
    screen.root.upgrade().and_then(|root| root.root()).and_then(|root| root.downcast::<gtk::Window>().ok())
}

fn size_of(screen: &Screen) -> (i32, i32) {
    screen.root.upgrade().map_or((0, 0), |root| (root.width(), root.height()))
}

/// Takes or releases the keep-awake inhibit for the open reader (T-043a-051).
fn sync_keep_awake(screen: &Screen, reader_open: bool) {
    let Some(window) = window_of(screen) else { return };
    let want = keep_awake::should_inhibit(screen.core.keep_awake_pref(), reader_open);
    screen.keep_awake.sync(&window, want);
}

/// Applies one live setting change to the open reader (T-043a-080). Only the theme and the global incognito flag are
/// live now: the other reader settings are cut from the settings and hard-coded.
fn apply_effect(screen: &Screen, effect: Effect) {
    match effect {
        Effect::Theme => {
            theme::apply_background(&screen.stack, screen.core.theme_pref());
        }
        Effect::Incognito => {
            // turning the global flag off closes the reader (E-INC-2)
            let on = kuro_prefs::base::BasePrefs::new(screen.core.deps.prefs_store.clone()).incognito_mode.get();
            if sync::closes_on_incognito(on) {
                (screen.on_close)();
            }
        }
    }
}

/// Applies the chrome state: navigator, page position, menu state and the window fullscreen state.
fn refresh(screen: &Screen) {
    let (previous, next) = screen.core.nav_enabled();
    let (page, total) = screen.core.page_counts();
    let state = NavigatorState {
        total,
        current_page: page,
        previous_enabled: previous,
        next_enabled: next,
        rtl: screen.core.reading_mode() == ReadingMode::RightToLeft,
        loading: screen.stepping.get(),
    };
    screen.navigator.update(&state, size_of(screen));
    let ui = *screen.ui.borrow();
    update_position(screen);
    chrome::sync_menu_state(&screen.menu_actions, screen.core.reading_mode(), screen.core.crop_on());
    apply_fullscreen(screen, ui.menu_visible);
}

fn apply_fullscreen(screen: &Screen, menu_visible: bool) {
    let Some(window) = window_of(screen) else { return };
    let want = state::fullscreen_for_menu(menu_visible, screen.core.fullscreen_pref());
    if window.is_fullscreen() != want {
        if want { window.fullscreen() } else { window.unfullscreen() }
    }
}

/// How close to the top or bottom edge the pointer brings the hidden bars back, in pixels.
const EDGE_PX: f64 = 8.0;

/// Whether a pointer at `y` in an area `height` tall is at the top or the bottom edge, where the bars come back.
fn at_bar_edge(y: f64, height: f64) -> bool {
    y < EDGE_PX || (height > 0.0 && y > height - EDGE_PX)
}

/// Whether the reader's and the player's bars hide by themselves (`auto_hide_bars`, off by default).
fn auto_hide_bars() -> bool {
    crate::data::is_installed() && crate::data::app().base.auto_hide_bars.get()
}

fn set_menu(screen: &Screen, visible: bool) {
    screen.ui.borrow_mut().set_menu_visible(visible);
    screen.menu_open.set(visible);
    // with auto-hide the top bar follows the menu as the bottom bar does (a tap in the middle, a page turn); without it
    // the top bar stays
    screen.top.set_reveal_child(visible || !screen.auto_hide);
    screen.bottom.set_reveal_child(visible);
    refresh(screen);
}

fn handle_key(screen: &Rc<Screen>, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> glib::Propagation {
    if key == gtk::gdk::Key::F11 {
        if let Some(window) = window_of(screen) {
            if state::toggle_fullscreen(window.is_fullscreen()) { window.fullscreen() } else { window.unfullscreen() }
        }
        return glib::Propagation::Stop;
    }
    if key == gtk::gdk::Key::Escape {
        // first on the root, wherever the focus is: an open sheet is dismissed before anything else (T-043a-079)
        match state::escape_action(screen.sheet.is_open(), window_of(screen).is_some_and(|w| w.is_fullscreen())) {
            EscapeAction::DismissSheet => screen.sheet.dismiss(),
            EscapeAction::LeaveFullscreen => {
                if let Some(window) = window_of(screen) {
                    window.unfullscreen();
                }
            }
            EscapeAction::CloseReader => (screen.on_close)(),
        }
        return glib::Propagation::Stop;
    }
    // the keys of the sheet that is open stay with it
    if screen.sheet.is_open() {
        return glib::Propagation::Proceed;
    }
    // a text entry keeps every key it is typed (the zoom keys plus, minus and 0 included, T-43b-058)
    if text_has_focus(screen) {
        return glib::Propagation::Proceed;
    }
    if let Some(propagation) = shortcut(screen, key, modifiers) {
        return propagation;
    }
    // the reader's focus is on the root, so the keys the viewer owns (arrows, page keys) come through here
    if screen.viewer_slot.borrow().as_ref().is_some_and(|v| v.handle_key(key, modifiers)) {
        return glib::Propagation::Stop;
    }
    match key.to_unicode().and_then(nav::key_step) {
        Some(step) => {
            press(screen, step);
            glib::Propagation::Stop
        }
        None => glib::Propagation::Proceed,
    }
}

/// The page actions sheet (T-43a-045). A long press, a right-click and Shift+F10 open it for the page.
fn open_page_actions(screen: &Rc<Screen>, page: Rc<ReaderPage>) {
    let weak = Rc::downgrade(screen);
    let on_action: Rc<dyn Fn(page_actions::Action)> = Rc::new(move |action| {
        let Some(s) = weak.upgrade() else { return };
        if page_actions::closes_sheet(action) {
            s.sheet.dismiss();
        }
        run_page_action(&s, &page, action);
    });
    let content = page_actions::build(on_action);
    screen.ui.borrow_mut().open(Dialog::PageActions);
    screen.sheet.show(&content);
}

fn run_page_action(screen: &Rc<Screen>, page: &Rc<ReaderPage>, action: page_actions::Action) {
    match action {
        page_actions::Action::Copy => copy_page(screen, page),
        page_actions::Action::Save => save_page(screen, page),
        page_actions::Action::Share => share_page(screen, page),
        page_actions::Action::SetCover => set_cover(screen, page),
    }
}

fn warn_unknown(screen: &Screen) {
    screen.toaster.show(strings::unknown_error(), ToastLength::Short);
}

/// Where a page's bytes are, for an action that runs now. A page that is not `Ready` gives `None` and nothing is
/// shown: the actions do nothing for it (T-43a-045). A ready page whose bytes cannot be found shows the error toast.
fn ready_source(screen: &Screen, page: &Rc<ReaderPage>) -> Option<crate::reader::viewers::page_bytes::ByteSource> {
    if !page_actions::runs_for(page.status.get()) {
        return None;
    }
    let source = page_save::page_source(page);
    if source.is_none() {
        warn_unknown(screen);
    }
    source
}

/// Copies a ready page to the clipboard as an image (T-43a-057). Nothing is shown on success.
fn copy_page(screen: &Rc<Screen>, page: &Rc<ReaderPage>) {
    let Some(source) = ready_source(screen, page) else { return };
    let toaster = screen.toaster.clone();
    page_save::in_background(move || page_save::read_bytes(&source), move |result| {
        let texture = result.ok().and_then(|bytes| gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(bytes)).ok());
        match (texture, gtk::gdk::Display::default()) {
            (Some(texture), Some(display)) => display.clipboard().set_texture(&texture),
            _ => {
                toaster.show(strings::unknown_error(), ToastLength::Short);
            }
        }
    });
}

/// Saves a page into the pictures folder and shows the saved or error notice (T-43a-054, T-43a-055).
fn save_page(screen: &Rc<Screen>, page: &Rc<ReaderPage>) {
    let Some(source) = ready_source(screen, page) else { return };
    let (title, chapter) = screen.core.page_names(page);
    let base = page_save::page_file_base(&title, &chapter, page.number() as usize);
    let folder = diet::READER_CREATE_FOLDER_PER_MANGA;
    let Some(app) = saved_notice::application() else { return warn_unknown(screen) };
    // the notice of an earlier save goes first; the new one replaces it (fixed id)
    saved_notice::clear(&app);
    page_save::save_in_background(source, &title, &base, folder, move |saved, thumbnail| match saved {
        Ok(path) => saved_notice::show_saved(&app, &path, thumbnail),
        Err(error) => saved_notice::show_failed(&app, Some(&error)),
    });
}

/// Stages a copy of the page, opens it with the default handler and shows the share message with a "Show in file
/// manager" button (T-43a-056). The system share list does not exist here; that loss is the replacement the task names.
fn share_page(screen: &Rc<Screen>, page: &Rc<ReaderPage>) {
    let Some(source) = ready_source(screen, page) else { return };
    let (title, chapter) = screen.core.page_names(page);
    let base = page_save::page_file_base(&title, &chapter, page.number() as usize);
    let message = strings::share_page_info(&title, &chapter, page.number() as i32);
    let Ok(paths) = kuro_platform::system_paths::SystemPaths::system() else { return warn_unknown(screen) };
    let staging = kuro_images::share::staging_dir(&paths);
    let toaster = screen.toaster.clone();
    page_save::in_background(move || page_save::stage_for_share(&source, &base, &staging), move |result| {
        // the file opens with the default handler; the notice carries the share message and offers the file manager
        let opened = result.and_then(|path| {
            saved_notice::open_with_default(&path).map_err(|e| e.to_string())?;
            Ok(path)
        });
        match (opened, saved_notice::application()) {
            (Ok(path), Some(app)) => saved_notice::show_shared(&app, &message, &path),
            (Ok(_), None) => {}
            (Err(_), _) => {
                toaster.show(strings::unknown_error(), ToastLength::Short);
            }
        }
    });
}

/// Sets the page as the manga's cover after a confirm (T-43a-058). The result toast follows the write.
fn set_cover(screen: &Rc<Screen>, page: &Rc<ReaderPage>) {
    if !page_actions::runs_for(page.status.get()) {
        return;
    }
    let Some(window) = window_of(screen) else { return };
    let (weak, page) = (Rc::downgrade(screen), Rc::clone(page));
    sheet::confirm(&window, &strings::confirm_set_image_as_cover(), move || {
        let Some(s) = weak.upgrade() else { return };
        // checked again now that the confirm has been answered
        let Some(source) = ready_source(&s, &page) else { return };
        let weak = Rc::downgrade(&s);
        if let Some(folder) = s.core.local_folder() {
            // a local manga's cover is an image in its folder, and the library shows the manga's thumbnail
            page_save::in_background(move || page_save::write_local_cover(&source, &folder), move |written| {
                let Some(s) = weak.upgrade() else { return };
                let message = s.core.local_cover_result(written);
                s.toaster.show(message, ToastLength::Short);
            });
            return;
        }
        let Ok(paths) = kuro_platform::system_paths::SystemPaths::system() else { return warn_unknown(&s) };
        let cover = kuro_images::cover_paths::custom_file(&paths, kuro_images::cover_paths::Family::MangaCover, s.core.deps.manga_id);
        page_save::in_background(move || page_save::write_cover_file(&source, &cover), move |written| {
            let Some(s) = weak.upgrade() else { return };
            let message = s.core.cover_result(written);
            s.toaster.show(message, ToastLength::Short);
        });
    });
}

/// Whether a text field of the reader has the focus, when the reader's shortcuts stay off (T-43a-053).
fn text_has_focus(screen: &Screen) -> bool {
    let Some(window) = window_of(screen) else { return false };
    let Some(focus) = gtk::prelude::GtkWindowExt::focus(&window) else { return false };
    focus.is::<gtk::Entry>() || focus.is::<gtk::Text>() || focus.is::<gtk::TextView>() || focus.is::<gtk::SearchEntry>()
}

/// The desktop shortcuts (T-43a-053): Ctrl+D bookmark, Ctrl+C copy page, Ctrl+S save page, Shift+F10 page actions.
/// `None` when the key is not one of them.
fn shortcut(screen: &Rc<Screen>, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> Option<glib::Propagation> {
    if key == gtk::gdk::Key::F10 && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
        let page = screen.core.visible_page()?;
        open_page_actions(screen, page);
        return Some(glib::Propagation::Stop);
    }
    if !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
        return None;
    }
    match key.to_unicode()?.to_ascii_lowercase() {
        'd' => {
            if let Some(Err(_)) = screen.core.toggle_bookmark() {
                warn_unknown(screen);
            }
            let weak = Rc::downgrade(screen);
            glib::idle_add_local_once(move || {
                if let Some(s) = weak.upgrade() {
                    rebuild_top_bar(&s);
                }
            });
        }
        'c' => copy_page(screen, &screen.core.visible_page()?),
        's' => save_page(screen, &screen.core.visible_page()?),
        _ => return None,
    }
    Some(glib::Propagation::Stop)
}

fn toggle_crop(screen: &Screen) {
    let view = screen.core.toggle_crop();
    screen.toaster.show(view.toast, ToastLength::Short);
    refresh(screen);
}

/// The reader settings window (T-043a-046, T-043a-047). Its mode row applies to every manga at once; the other rows write
/// their keys, which the open reader follows.
fn open_settings(screen: &Rc<Screen>) {
    let Some(window) = window_of(screen) else { return };
    let mode_weak = Rc::downgrade(screen);
    let on_mode: Rc<dyn Fn(ReadingMode)> = Rc::new(move |mode| {
        if let Some(s) = mode_weak.upgrade() {
            s.core.apply_mode(mode);
            drain_events(&s);
            refresh(&s);
        }
    });
    settings_window::open(&window, &screen.core.reader_prefs(), screen.core.reading_mode(), on_mode);
}

/// The initial load. Clears a failure card, shows the spinner, waits for the source manager, then loads.
fn start(screen: &Rc<Screen>) {
    if let Some(card) = screen.stack.child_by_name(PAGE_CARD) {
        screen.stack.remove(&card);
    }
    if screen.top_slot.first_child().is_none() {
        rebuild_top_bar(screen);
    }
    screen.stack.set_visible_child_name(PAGE_LOADING);
    refresh(screen);
    let screen = Rc::clone(screen);
    glib::spawn_future_local(async move {
        let ready = screen.ready.borrow().clone();
        let result = if wait_ready(ready).await {
            // one frame, so the spinner is on screen before the load takes the thread
            glib::timeout_future(std::time::Duration::ZERO).await;
            screen.core.initial_load()
        } else {
            Err(LoadError::new(strings::unknown_error()))
        };
        match result {
            Ok(()) => {
                screen.stack.set_visible_child_name(PAGE_VIEWER);
                *screen.events.borrow_mut() = screen.core.take_events();
                install_viewer(&screen);
                watch_events(&screen);
                sync_top_bar(&screen);
                refresh(&screen);
            }
            // the page list is still being fetched: the spinner stays and the load is tried again shortly
            Err(error) if source_pages::is_pending(&error.message) => {
                let weak = Rc::downgrade(&screen);
                glib::timeout_add_local_once(PENDING_RETRY, move || {
                    if let Some(screen) = weak.upgrade() {
                        start(&screen);
                    }
                });
            }
            Err(error) => show_card(&screen, &error),
        }
    });
}

/// Builds the viewer for the manga's reading mode and attaches it (T-44a-070). `attach_viewer` destroys the previous
/// viewer first, so a mode switch never has two viewers alive. The viewer opens on the chapter's requested page by
/// itself, and from then on reports back through the chrome.
fn install_viewer(screen: &Rc<Screen>) {
    let kind = adapter::kind_of(screen.core.reading_mode());
    let chrome = ReaderChrome { screen: Rc::downgrade(screen) };
    let ctx = ViewerContext {
        emit: Emitter::new(Rc::new({
            let chrome = chrome.clone();
            move |event| chrome.route(event)
        })),
        settings: screen.core.viewer_settings(),
        prefs: Some(screen.core.reader_prefs()),
        downloads: screen.core.downloads(),
        menu_open: Rc::clone(&screen.menu_open),
    };
    // the new-user flag was stored and cleared on read; the setting is cut, so the hard-coded value applies on each open
    let new_user = diet::READER_OVERLAY_NEW_USER;
    let wanted = ctx.settings.overlay_on_start || new_user;
    screen.nav_overlay.set_navigation(nav_overlay::zones_for(kind, &ctx.settings), wanted);
    screen.viewer_kind.set(Some(kind));
    chrome.attach_viewer(Box::new(EngineViewer::new(make_viewer(kind, ctx), screen.viewer_box.clone())));
}

/// Hands the current chapters to the attached viewer again, after a chapter changed or finished loading.
fn reload_chapters(screen: &Screen) {
    let Some(chapters) = screen.core.viewer_chapters() else { return };
    // a viewer that is being attached holds the slot; its first chapters are the ones it is given
    if let Ok(slot) = screen.viewer_slot.try_borrow() {
        if let Some(viewer) = slot.as_ref() {
            viewer.set_chapters(&chapters);
            move_references(&screen.viewer_refs, Some(chapters));
        }
    }
}

/// Moves the viewer's chapter references to `next` (T-43a-002). The new set is referenced before the old one is released,
/// so a chapter in both keeps its pages. A chapter that leaves the window is released, and its loader is recycled.
fn move_references(held: &RefCell<Option<ViewerChapters>>, next: Option<ViewerChapters>) {
    let old = std::mem::replace(&mut *held.borrow_mut(), next.clone());
    match next {
        Some(new) => ViewerChapters::replace(old.as_ref(), &new),
        None => {
            if let Some(old) = old {
                old.release_all();
            }
        }
    }
}

/// `ReloadViewerChapters`: a new viewer when the reading mode changed (the old one is destroyed first), otherwise the
/// same viewer with the new chapters.
fn reload_viewer(screen: &Rc<Screen>) {
    if screen.viewer_kind.get() == Some(adapter::kind_of(screen.core.reading_mode())) {
        reload_chapters(screen);
    } else {
        install_viewer(screen);
    }
}

/// Carries out what the core's session has sent since the last call. Events arrive in order and none is dropped.
fn drain_events(screen: &Rc<Screen>) {
    let events: Vec<ReaderEvent> = screen.events.borrow().as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
    for event in events {
        match event {
            ReaderEvent::ReloadViewerChapters => reload_viewer(screen),
            // the image actions belong to the page sheet (T-043a-045)
            ReaderEvent::PageChanged
            | ReaderEvent::SetCoverResult(_)
            | ReaderEvent::SavedImage(_)
            | ReaderEvent::ShareImage(_)
            | ReaderEvent::CopyImage(_) => {}
        }
    }
}

/// Follows the core's events while the reader is open: a preload that finished, or a mode change that came from
/// somewhere other than the reader's own buttons.
fn watch_events(screen: &Rc<Screen>) {
    let weak = Rc::downgrade(screen);
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || match weak.upgrade() {
        Some(screen) => {
            drain_events(&screen);
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
}

fn show_card(screen: &Rc<Screen>, error: &LoadError) {
    let (retry_screen, close) = (Rc::clone(screen), Rc::clone(&screen.on_close));
    let retry = move || {
        if let Some(gate) = &retry_screen.core.extras.retry_gate {
            *retry_screen.ready.borrow_mut() = gate().ready_watch();
        }
        start(&retry_screen);
    };
    let card = build_error_card(error, retry, move || close());
    screen.stack.add_named(&card, Some(PAGE_CARD));
    screen.stack.set_visible_child_name(PAGE_CARD);
}

/// The slider's page index goes to the attached viewer's `move_to_page`. With no viewer attached (the load has not
/// finished) there is nothing to move.
fn move_viewer(screen: &Screen, index: usize) {
    keep_menu_for_a_moment(screen);
    if let Some(viewer) = screen.viewer_slot.borrow().as_ref() {
        screen.core.move_viewer_to(viewer.as_ref(), index);
    }
}

/// Page moves the slider and the chapter buttons cause (the pager settling, the strip scrolling) must not hide the bar.
/// ponytail: a short time window shared by both viewers; a per-viewer "slider moving" flag if it ever swallows a real swipe.
fn keep_menu_for_a_moment(screen: &Screen) {
    screen.keep_menu_until.set(Some(std::time::Instant::now() + std::time::Duration::from_millis(1200)));
}

/// Rebuilds the top bar when the chapter shown is not the current one.
fn sync_top_bar(screen: &Rc<Screen>) {
    if screen.core.current_chapter_id() != Some(screen.shown_chapter.get()) {
        rebuild_top_bar(screen);
    }
}

/// Builds the top bar for the current chapter (T-043a-040): back, the manga and chapter title, the bookmark toggle
/// and the overflow menu. Back returns to the page below; the title opens the manga's details.
fn rebuild_top_bar(screen: &Rc<Screen>) {
    let info = screen.core.top_bar();
    if info.is_some() {
        screen.shown_chapter.set(screen.core.current_chapter_id().unwrap_or(-1));
    }
    while let Some(child) = screen.top_slot.first_child() {
        screen.top_slot.remove(&child);
    }
    let back = Rc::clone(&screen.on_close);
    let Some(info) = info else {
        // no chapter yet (the page list is loading, or the load failed): the bar still holds Back and the window buttons
        *screen.position.borrow_mut() = None;
        screen.top_slot.append(&build_bare_top_bar(move || back()));
        return;
    };
    let bookmark = Rc::downgrade(screen);
    let actions = TopBarActions {
        on_back: Box::new(move || back()),
        on_bookmark: Box::new(move || {
            let Some(screen) = bookmark.upgrade() else { return };
            if let Some(Err(_)) = screen.core.toggle_bookmark() {
                screen.toaster.show(strings::unknown_error(), ToastLength::Short);
            }
            // the bar is rebuilt after the click has finished, so its icon and label follow the new state
            glib::idle_add_local_once(move || rebuild_top_bar(&screen));
        }),
    };
    let (bar, title) = build_top_bar(&info, actions);
    *screen.position.borrow_mut() = Some(title);
    screen.top_slot.append(&bar);
    update_position(screen);
}

/// Shows "Page n of total" under the title in the top bar; nothing on a transition card or before the pages are known.
fn update_position(screen: &Screen) {
    if let Some(title) = screen.position.borrow().as_ref() {
        let (page, total) = screen.core.page_counts();
        title.set_subtitle(page_header::page_position(page, total).as_deref());
    }
}

/// The handlers of the top bar menu's actions. Each holds the screen weakly, as the actions live on its root.
fn menu_handlers(me: &Weak<Screen>) -> MenuHandlers {
    fn run(me: &Weak<Screen>, f: impl Fn(&Rc<Screen>) + 'static) -> Rc<dyn Fn()> {
        let me = me.clone();
        Rc::new(move || {
            if let Some(screen) = me.upgrade() {
                f(&screen);
            }
        })
    }
    let mode_me = me.clone();
    MenuHandlers {
        // a chosen mode applies to this manga at once, like the settings window's chips
        on_mode: Rc::new(move |mode| {
            let Some(s) = mode_me.upgrade() else { return };
            s.core.apply_mode(mode);
            if !s.core.show_reading_mode() {
                s.toaster.show(chrome::mode_label(mode), ToastLength::Short);
            }
            drain_events(&s);
            refresh(&s);
        }),
        on_crop: run(me, |s| toggle_crop(s)),
        on_settings: run(me, open_settings),
        on_open_browser: run(me, open_in_browser),
        on_share: run(me, share_chapter),
        on_set_cover: run(me, |screen| match screen.core.visible_page() {
            Some(page) => set_cover(screen, &page),
            None => {
                screen.toaster.show(strings::unknown_error(), ToastLength::Short);
            }
        }),
    }
}

/// The chapter's web address, or an error toast when it has none.
fn chapter_web_url(screen: &Screen) -> Option<String> {
    let url = screen.core.chapter_url().and_then(|u| chrome::web_url(&u));
    if url.is_none() {
        screen.toaster.show(strings::unknown_error(), ToastLength::Short);
    }
    url
}

/// Opens the chapter in the web view or the browser.
fn open_in_browser(screen: &Rc<Screen>) {
    let Some(url) = chapter_web_url(screen) else { return };
    crate::ui::open_link(&url, window_of(screen).as_ref());
}

/// Shares (copies) the chapter's web address.
fn share_chapter(screen: &Rc<Screen>) {
    let Some(url) = chapter_web_url(screen) else { return };
    if let Some(root) = screen.root.upgrade() {
        root.clipboard().set_text(&url);
    }
}

/// A press of a chapter button or of N or P. At the ends, or while a load runs, it does nothing.
fn press(screen: &Rc<Screen>, step: Step) {
    let (previous, next) = screen.core.nav_enabled();
    let enabled = match step {
        Step::Previous => previous,
        Step::Next => next,
    };
    if !enabled || screen.stepping.replace(true) {
        return;
    }
    keep_menu_for_a_moment(screen);
    refresh(screen);
    run_step(screen, step);
}

/// Loads the adjacent chapter on the next idle turn. A page list still being fetched keeps the navigator's spinner on
/// and is asked for again shortly (the buttons stay disabled meanwhile); any other outcome ends the step.
fn run_step(screen: &Rc<Screen>, step: Step) {
    let screen = Rc::clone(screen);
    glib::idle_add_local_once(move || {
        let outcome = screen.core.step(step);
        if matches!(&outcome, AdjacentOutcome::Failed(message) if source_pages::is_pending(message)) {
            let weak = Rc::downgrade(&screen);
            glib::timeout_add_local_once(PENDING_RETRY, move || {
                if let Some(screen) = weak.upgrade() {
                    run_step(&screen, step);
                }
            });
            return;
        }
        screen.stepping.set(false);
        if matches!(outcome, AdjacentOutcome::Moved(_)) {
            // the viewer shows the new chapter from its first page
            reload_chapters(&screen);
            move_viewer(&screen, 0);
        }
        drain_events(&screen);
        sync_top_bar(&screen);
        refresh(&screen);
        if let AdjacentOutcome::Failed(message) = outcome {
            screen.toaster.show(failure_toast(&message), ToastLength::Short);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::cache::{CACHE_LIMIT_BYTES, DiskCache};
    use crate::reader::loader::PageEntry;
    use crate::reader::loader::http::{HttpLoader, HttpPageSource};
    use kuro_data::item::ItemRepo;
    use kuro_platform::kind::Kind;
    use kuro_prefs::store::PrefStore;
    use kuro_prefs::reader::ReadingMode;
    use crate::reader::delete_queue::Pending;

    struct Pages;

    impl HttpPageSource for Pages {
        fn page_list(&self, _chapter_url: &str) -> Result<Vec<PageEntry>, String> {
            Ok(vec![PageEntry { url: "p0".into(), image_url: None }, PageEntry { url: "p1".into(), image_url: None }])
        }
        fn image_url(&self, page_url: &str) -> Result<String, String> {
            Ok(format!("https://img/{page_url}"))
        }
        fn image(&self, _image_url: &str) -> Result<Vec<u8>, String> {
            Ok(b"img".to_vec())
        }
    }

    #[derive(Default)]
    struct NoStore;

    impl ReaderStore for NoStore {
        fn save_progress(&self, _: i64, _: i64, _: bool) {}
        fn set_bookmark(&self, _: i64, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn mark_read(&self, _: i64) {}
        fn upsert_history(&self, _: i64, _: i64, _: i64) {}
        fn set_viewer_flags(&self, _: i64, _: i64, _: i64) {}
        fn flush(&self) {}
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Arc<Db>,
        manga_id: i64,
        chapter_ids: Vec<i64>,
    }

    fn fixture(chapters: usize) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = kuro_db::schema_unified::db_path(dir.path());
        let writer = kuro_db::schema_unified::open_file(&path).unwrap();
        let db = Arc::new(Db::new(&path, writer).unwrap());
        let manga_id = EntryRepo::new(Arc::clone(&db)).insert(&Entry::create(Kind::Manga)).unwrap();
        let new: Vec<Item> = (0..chapters)
            .map(|i| {
                let mut c = Item::create(Kind::Manga);
                c.entry_id = manga_id;
                c.url = format!("/c/{i}");
                c.number = (i + 1) as f64;
                c.source_order = (chapters - i) as i64;
                c
            })
            .collect();
        let chapter_ids = ItemRepo::new(Arc::clone(&db)).add_all(&new).unwrap().iter().map(|c| c.id).collect();
        Fixture { _dir: dir, db, manga_id, chapter_ids }
    }

    fn http_build(tag: &'static str) -> BuildLoader {
        Rc::new(move |chapter| {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-screen-tests").join(tag);
            let cache = Arc::new(DiskCache::open(root, CACHE_LIMIT_BYTES).unwrap());
            Ok(Loader::Http(HttpLoader::new(Arc::new(Pages), 1, chapter.chapter.url.clone(), cache)))
        })
    }

    fn core(f: &Fixture, requested: usize, build: BuildLoader) -> ReaderCore {
        ReaderCore::new(ReaderDeps {
            db: Arc::clone(&f.db),
            manga_id: f.manga_id,
            chapter_id: f.chapter_ids[requested],
            prefs: ReaderPrefs::new(PrefStore::in_memory()),
            store: Rc::new(NoStore),
            build,
            is_downloaded: Rc::new(|_| false),
            downloaded_only: false,
            incognito: false,
            prefs_store: PrefStore::in_memory(),
        })
    }

    fn core_with(f: &Fixture, chapter_id: i64, prefs_store: PrefStore, extras: ReaderExtras, build: BuildLoader) -> ReaderCore {
        ReaderCore::with_extras(
            ReaderDeps {
                db: Arc::clone(&f.db),
                manga_id: f.manga_id,
                chapter_id,
                prefs: ReaderPrefs::new(prefs_store.clone()),
                store: Rc::new(NoStore),
                build,
                is_downloaded: Rc::new(|_| false),
                downloaded_only: false,
                incognito: false,
                prefs_store,
            },
            extras,
        )
    }

    fn data_extras(dir: &std::path::Path, sink: Option<DeleteSink>) -> ReaderExtras {
        ReaderExtras { data_dir: Some(dir.to_path_buf()), delete_sink: sink, ..ReaderExtras::default() }
    }

    fn record(f: &Fixture, chapter: usize, page_index: i64, age_ms: i64) -> restore::SavedSession {
        restore::SavedSession {
            manga_id: f.manga_id,
            chapter_id: f.chapter_ids[chapter],
            page_index,
            saved_at: history_rules::now_ms() - age_ms,
        }
    }

    fn current_requested_page(core: &ReaderCore) -> i64 {
        core.session.borrow().as_ref().unwrap().current_chapter().unwrap().requested_page.get()
    }

    #[test]
    fn a_reader_opened_without_a_chapter_restores_the_saved_chapter_and_page() {
        let f = fixture(3);
        let dir = tempfile::tempdir().unwrap();
        restore::save(&restore::path_in(dir.path()), &record(&f, 1, 1, 60_000)).unwrap();
        let core = core_with(&f, 0, PrefStore::in_memory(), data_extras(dir.path(), None), http_build("restore"));
        core.initial_load().unwrap();
        assert_eq!(core.current_index(), Some(1));
        assert_eq!(current_requested_page(&core), 1);
    }

    #[test]
    fn an_explicit_chapter_wins_and_the_setting_off_or_an_old_record_restores_nothing() {
        let f = fixture(3);
        let dir = tempfile::tempdir().unwrap();
        restore::save(&restore::path_in(dir.path()), &record(&f, 1, 1, 60_000)).unwrap();
        let explicit = core_with(&f, f.chapter_ids[2], PrefStore::in_memory(), data_extras(dir.path(), None), http_build("restore-explicit"));
        explicit.initial_load().unwrap();
        assert_eq!(explicit.current_index(), Some(2), "the chapter the library asked for wins");

        restore::save(&restore::path_in(dir.path()), &record(&f, 1, 1, restore::WINDOW_MS + 1_000)).unwrap();
        let stale = core_with(&f, 0, PrefStore::in_memory(), data_extras(dir.path(), None), http_build("restore-old"));
        assert!(stale.initial_load().is_err(), "a record older than 24 hours is not used");
    }

    #[test]
    fn a_restored_page_beyond_the_chapter_is_kept_inside_it() {
        let f = fixture(1);
        let dir = tempfile::tempdir().unwrap();
        restore::save(&restore::path_in(dir.path()), &record(&f, 0, 40, 1_000)).unwrap();
        let core = core_with(&f, 0, PrefStore::in_memory(), data_extras(dir.path(), None), http_build("restore-clamp"));
        core.initial_load().unwrap();
        assert_eq!(current_requested_page(&core), 1, "the chapter has two pages");
    }

    #[test]
    fn closing_hands_the_delete_queue_to_the_sink_once_and_without_a_sink_the_rule_is_off() {
        let f = fixture(3);
        let dir = tempfile::tempdir().unwrap();
        let store = PrefStore::in_memory();
        DownloadPrefs::new(&store).remove_after_read_slots.set(0).unwrap();
        let handed: Rc<RefCell<Vec<Pending>>> = Rc::default();
        let flushes = Rc::new(Cell::new(0));
        let sink = DeleteSink {
            defer: {
                let handed = Rc::clone(&handed);
                Rc::new(move |pending| handed.borrow_mut().push(pending))
            },
            flush: {
                let flushes = Rc::clone(&flushes);
                Rc::new(move || flushes.set(flushes.get() + 1))
            },
        };
        let core = core_with(&f, f.chapter_ids[0], store.clone(), data_extras(dir.path(), Some(sink)), http_build("delete"));
        core.initial_load().unwrap();
        let finish = |core: &ReaderCore| {
            let chapter = Rc::clone(core.session.borrow().as_ref().unwrap().current_chapter().unwrap());
            let last = Rc::clone(chapter.pages().unwrap().last().unwrap());
            last.set_chapter(&chapter);
            core.page_selected(&last, &mut |_| {}, &mut || {});
        };
        finish(&core);
        core.flush_deletes();
        core.flush_deletes();
        assert_eq!(*handed.borrow(), vec![Pending { manga_id: f.manga_id, chapter_id: f.chapter_ids[0] }]);
        assert_eq!(flushes.get(), 1, "closing flushes once");

        let quiet = core_with(&f, f.chapter_ids[0], store, data_extras(dir.path(), None), http_build("delete-off"));
        quiet.initial_load().unwrap();
        finish(&quiet);
        quiet.flush_deletes();
        assert_eq!((handed.borrow().len(), flushes.get()), (1, 1), "without a sink nothing is handed over");
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap()
    }

    #[test]
    fn opening_waits_for_the_source_manager_then_proceeds() {
        runtime().block_on(async {
            let (registry, ready) = watch::channel(false);
            let waiting = tokio::spawn(wait_ready(ready.clone()));
            tokio::task::yield_now().await;
            assert!(!waiting.is_finished(), "the open must not go on while the gate is closed");
            registry.send_replace(true);
            assert!(waiting.await.unwrap());
            assert!(wait_ready(ready).await, "an open gate does not wait");
        });
    }

    #[test]
    fn a_source_manager_that_goes_away_unready_ends_the_wait() {
        runtime().block_on(async {
            let (registry, ready) = watch::channel(false);
            drop(registry);
            assert!(!wait_ready(ready).await);
        });
    }

    #[test]
    fn the_initial_load_opens_the_requested_chapter() {
        let f = fixture(3);
        let core = core(&f, 1, http_build("open"));
        assert_eq!(core.nav_enabled(), (false, false), "nothing to navigate before the load");
        core.initial_load().unwrap();
        assert!(core.take_events().is_some());
        assert_eq!(core.nav_enabled(), (true, true));
    }

    #[test]
    fn a_missing_chapter_or_manga_gives_the_card_message() {
        let f = fixture(1);
        let mut other = core(&f, 0, http_build("missing"));
        other.deps.chapter_id = 9999;
        assert_eq!(other.initial_load().unwrap_err().message, "Requested chapter of id 9999 not found in chapter list");
        other.deps.manga_id = 9999;
        assert_eq!(other.initial_load().unwrap_err().message, "Unknown err");
    }

    #[test]
    fn retry_repeats_the_whole_initial_load_and_can_succeed() {
        let f = fixture(2);
        let fail = Rc::new(Cell::new(true));
        let flag = Rc::clone(&fail);
        let inner = http_build("retry");
        let core = core(&f, 0, Rc::new(move |chapter| if flag.get() { Err(LoadError::new("offline")) } else { inner(chapter) }));
        let error = core.initial_load().unwrap_err();
        assert_eq!(super::super::chrome::error_card_message(&error), "Failed to load pages: offline");
        assert_eq!(core.nav_enabled(), (false, false), "a failed load leaves no session");
        fail.set(false);
        core.initial_load().unwrap();
        assert_eq!(core.nav_enabled(), (false, true));
    }

    #[test]
    fn next_and_previous_are_disabled_at_the_ends_and_the_buttons_follow_the_move() {
        let f = fixture(3);
        let core = core(&f, 0, http_build("nav"));
        core.initial_load().unwrap();
        assert_eq!(core.nav_enabled(), (false, true));
        assert_eq!(core.step(Step::Previous), AdjacentOutcome::NoNeighbour);
        assert_eq!(core.step(Step::Next), AdjacentOutcome::Moved(1));
        assert_eq!(core.nav_enabled(), (true, true));
        assert_eq!(core.step(Step::Next), AdjacentOutcome::Moved(2));
        assert_eq!(core.nav_enabled(), (true, false));
        assert_eq!(core.current_index(), Some(2));
    }

    #[test]
    fn a_failed_adjacent_load_reports_its_message_and_keeps_the_chapter() {
        let f = fixture(2);
        let fail = Rc::new(Cell::new(false));
        let flag = Rc::clone(&fail);
        let inner = http_build("adjacent-fail");
        let core = core(&f, 0, Rc::new(move |chapter| if flag.get() { Err(LoadError::new("offline")) } else { inner(chapter) }));
        core.initial_load().unwrap();
        fail.set(true);
        assert_eq!(core.step(Step::Next), AdjacentOutcome::Failed("offline".into()));
        assert_eq!(core.current_index(), Some(0));
        assert_eq!(failure_toast("offline"), "offline");
        assert_eq!(failure_toast("  "), "Unknown error");
    }

    #[test]
    fn the_crop_toggle_writes_the_manga_setting_and_reports_on_then_off() {
        let f = fixture(1);
        let core = core(&f, 0, http_build("crop"));
        core.initial_load().unwrap();
        // the default mode preference is left to right, a pager mode
        assert_eq!(core.mode(), ReadingMode::LeftToRight);
        assert!(!core.crop_on());
        let on = core.toggle_crop();
        assert_eq!(on, CropView { value: true, icon: CROP_ICON_ON, toast: "On".to_string() });
        assert!(core.reader_prefs().crop_borders().get());
        assert!(core.crop_on());
        assert!(!core.deps.prefs.crop_borders.get(), "the default of the Settings page stays off");
        let off = core.toggle_crop();
        assert_eq!(off, CropView { value: false, icon: CROP_ICON_OFF, toast: "Off".to_string() });
        assert!(!core.crop_on());
    }

    #[test]
    fn a_webtoon_manga_flips_the_same_crop_switch() {
        let f = fixture(1);
        EntryRepo::new(Arc::clone(&f.db)).set_viewer_bits(f.manga_id, i64::from(ReadingMode::Webtoon.flag_value()), 0x7).unwrap();
        let core = core(&f, 0, http_build("crop-webtoon"));
        core.initial_load().unwrap();
        assert_eq!(core.toggle_crop().toast, "On");
        assert!(core.reader_prefs().crop_borders().get(), "the long strip follows the one crop switch");
        assert!(core.crop_on());
    }

    #[derive(Default)]
    struct MoveLog(RefCell<Vec<u32>>);

    impl Viewer for MoveLog {
        fn set_chapters(&self, _: &ViewerChapters) {}
        fn move_to_page(&self, page: &Rc<ReaderPage>) {
            self.0.borrow_mut().push(page.index);
        }
        fn destroy(&self) {}
        fn handle_key(&self, _: gtk::gdk::Key, _: gtk::gdk::ModifierType) -> bool {
            false
        }
        fn handle_motion(&self, _: f64, _: f64) -> bool {
            false
        }
    }

    #[test]
    fn the_slider_asks_the_viewer_for_the_page_at_its_index() {
        let f = fixture(1);
        let core = core(&f, 0, http_build("slider"));
        let viewer = MoveLog::default();
        assert!(!core.move_viewer_to(&viewer, 0), "nothing is loaded before the initial load");
        core.initial_load().unwrap();
        assert!(core.move_viewer_to(&viewer, 1));
        assert!(!core.move_viewer_to(&viewer, 99), "an index past the last page moves nothing");
        assert_eq!(viewer.0.borrow().as_slice(), [1]);
    }

    #[test]
    fn a_chapter_leaving_the_viewer_is_released_and_one_that_stays_is_kept() {
        let held = RefCell::new(None);
        let chapter = |name: &str| {
            let mut c = Item::create(Kind::Manga);
            c.name = name.into();
            ReaderChapter::new(c)
        };
        let (a, b, c) = (chapter("a"), chapter("b"), chapter("c"));
        move_references(&held, Some(ViewerChapters { curr: Rc::clone(&a), prev: None, next: Some(Rc::clone(&b)) }));
        assert_eq!((a.references(), b.references()), (1, 1));
        move_references(&held, Some(ViewerChapters { curr: Rc::clone(&b), prev: Some(Rc::clone(&a)), next: Some(Rc::clone(&c)) }));
        assert_eq!((a.references(), b.references(), c.references()), (1, 1, 1), "a and b stay, c arrives");
        move_references(&held, None);
        assert_eq!((a.references(), b.references(), c.references()), (0, 0, 0), "the viewer is gone");
    }

    #[test]
    fn the_top_bar_shows_the_manga_the_chapter_the_bookmark_and_the_source_kind() {
        let f = fixture(2);
        EntryRepo::new(Arc::clone(&f.db))
            .update(&EntryUpdate { title: Some("My Manga".into()), ..EntryUpdate::new(f.manga_id) })
            .unwrap();
        let core = core(&f, 1, http_build("top-bar"));
        assert!(core.top_bar().is_none(), "no bar before the initial load");
        core.initial_load().unwrap();
        let bar = core.top_bar().unwrap();
        assert_eq!((bar.manga_title.as_str(), bar.bookmarked, bar.is_http_source), ("My Manga", false, true));
        assert_eq!(core.current_chapter_id(), Some(f.chapter_ids[1]));
        assert_eq!(core.toggle_bookmark(), Some(Ok(true)));
        assert!(core.top_bar().unwrap().bookmarked);
        assert_eq!(core.step(Step::Previous), AdjacentOutcome::Moved(0));
        assert_eq!(core.current_chapter_id(), Some(f.chapter_ids[0]), "the bar follows the chapter");
        assert!(!core.top_bar().unwrap().bookmarked, "the bookmark belongs to the chapter, not the reader");
    }

    /// Network pages with real image bytes, so the viewer can show them.
    struct PngPages;

    impl HttpPageSource for PngPages {
        fn page_list(&self, _chapter_url: &str) -> Result<Vec<PageEntry>, String> {
            Ok(vec![PageEntry { url: "p0".into(), image_url: None }, PageEntry { url: "p1".into(), image_url: None }])
        }
        fn image_url(&self, page_url: &str) -> Result<String, String> {
            Ok(format!("https://img/{page_url}.png"))
        }
        fn image(&self, _image_url: &str) -> Result<Vec<u8>, String> {
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(20, 30, image::Rgba([50, 60, 70, 255])))
                .write_to(&mut out, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            Ok(out.into_inner())
        }
    }

    fn page_views(widget: &gtk::Widget, out: &mut Vec<crate::reader::page_view::PageView>) {
        if let Some(view) = widget.downcast_ref::<crate::reader::page_view::PageView>() {
            out.push(view.clone());
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            page_views(&c, out);
            child = c.next_sibling();
        }
    }

    fn pump_until(done: impl Fn() -> bool) -> bool {
        let ctx = glib::MainContext::default();
        for _ in 0..1000 {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    /// T-44a-070 end to end: the reader attaches the viewer of the manga's mode once the chapter has loaded, the network
    /// pages reach it through the loader, and a mode switch destroys the old viewer before the new one is attached.
    /// The top bar is there while the chapter is still loading, so the window can be dragged and closed from it.
    #[test]
    fn the_reader_has_its_header_bar_before_the_chapter_loads() {
        crate::testing::with_display(|| {
            let f = fixture(2);
            let deps = ReaderDeps {
                db: Arc::clone(&f.db),
                manga_id: f.manga_id,
                chapter_id: f.chapter_ids[0],
                prefs: ReaderPrefs::new(PrefStore::in_memory()),
                store: Rc::new(NoStore),
                build: http_build("header"),
                is_downloaded: Rc::new(|_| false),
                downloaded_only: false,
                incognito: false,
                prefs_store: PrefStore::in_memory(),
            };
            let (toaster, _feed) = kuro_platform::toast::toast_channel();
            let screen = open(deps, &crate::reader::launch::open_gate(), toaster, || {});
            let root: gtk::Widget = screen.root.clone().upcast();
            let bars = crate::screens::alone::descendants(&root);
            assert!(bars.iter().any(|w| w.is::<gtk::HeaderBar>()), "the reader shows its header bar while the chapter loads");
        });
    }

    struct SixPngPages;

    impl HttpPageSource for SixPngPages {
        fn page_list(&self, _chapter_url: &str) -> Result<Vec<PageEntry>, String> {
            Ok((0..6).map(|i| PageEntry { url: format!("p{i}"), image_url: None }).collect())
        }
        fn image_url(&self, page_url: &str) -> Result<String, String> {
            PngPages.image_url(page_url)
        }
        fn image(&self, image_url: &str) -> Result<Vec<u8>, String> {
            PngPages.image(image_url)
        }
    }

    /// A mode switch keeps the page on screen: the new viewer opens where the old one was, not at the chapter's start.
    #[test]
    fn a_mode_switch_keeps_the_page_on_screen() {
        crate::testing::with_display(|| {
            let f = fixture(2);
            let build: BuildLoader = Rc::new(|chapter| {
                let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-screen-tests/mode-keeps-page");
                let cache = Arc::new(DiskCache::open(root, CACHE_LIMIT_BYTES).unwrap());
                Ok(Loader::Http(HttpLoader::new(Arc::new(SixPngPages), 1, chapter.chapter.url.clone(), cache)))
            });
            let deps = ReaderDeps {
                db: Arc::clone(&f.db),
                manga_id: f.manga_id,
                chapter_id: f.chapter_ids[0],
                prefs: ReaderPrefs::new(PrefStore::in_memory()),
                store: Rc::new(NoStore),
                build,
                is_downloaded: Rc::new(|_| false),
                downloaded_only: false,
                incognito: false,
                prefs_store: PrefStore::in_memory(),
            };
            let (toaster, _feed) = kuro_platform::toast::toast_channel();
            let screen = open(deps, &crate::reader::launch::open_gate(), toaster, || {});
            let window = gtk::Window::new();
            window.set_default_size(500, 600);
            window.set_child(Some(&screen.root));
            window.present();
            assert!(pump_until(|| screen.core.page_counts() == (1, 6)), "{:?}", screen.core.page_counts());
            let inner = screen.chrome.screen.upgrade().unwrap();
            move_viewer(&inner, 3);
            assert!(pump_until(|| screen.core.page_counts().0 == 4), "the slider moved to page 4: {:?}", screen.core.page_counts());
            let settle = || {
                let (ctx, end) = (glib::MainContext::default(), std::time::Instant::now() + std::time::Duration::from_millis(600));
                while std::time::Instant::now() < end {
                    while ctx.iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            for mode in [ReadingMode::Vertical, ReadingMode::RightToLeft, ReadingMode::LeftToRight, ReadingMode::Webtoon, ReadingMode::ContinuousVertical, ReadingMode::Vertical] {
                screen.core.apply_mode(mode);
                drain_events(&inner);
                settle();
                assert_eq!(screen.core.page_counts().0, 4, "still on page 4 after switching to {mode:?}");
            }
            window.destroy();
        });
    }

    #[test]
    fn the_reader_attaches_its_viewer_shows_network_pages_and_swaps_the_viewer_on_a_mode_switch() {
        crate::testing::with_display(|| {
            let f = fixture(2);
            let build: BuildLoader = Rc::new(|chapter| {
                let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-screen-tests/viewer");
                let _ = std::fs::remove_dir_all(&root);
                let cache = Arc::new(DiskCache::open(root, CACHE_LIMIT_BYTES).unwrap());
                Ok(Loader::Http(HttpLoader::new(Arc::new(PngPages), 1, chapter.chapter.url.clone(), cache)))
            });
            let deps = ReaderDeps {
                db: Arc::clone(&f.db),
                manga_id: f.manga_id,
                chapter_id: f.chapter_ids[0],
                prefs: ReaderPrefs::new(PrefStore::in_memory()),
                store: Rc::new(NoStore),
                build,
                is_downloaded: Rc::new(|_| false),
                downloaded_only: false,
                incognito: false,
                prefs_store: PrefStore::in_memory(),
            };
            let (toaster, _feed) = kuro_platform::toast::toast_channel();
            let screen = open(deps, &crate::reader::launch::open_gate(), toaster, || {});
            let window = gtk::Window::new();
            window.set_default_size(500, 600);
            window.set_child(Some(&screen.root));
            window.present();

            assert!(pump_until(|| screen.viewer.first_child().is_some()), "the viewer is attached once the chapter has loaded");
            assert!(screen.viewer.first_child().unwrap().is::<gtk::ScrolledWindow>(), "the default mode reads as a pager");
            assert!(
                pump_until(|| {
                    let mut views = Vec::new();
                    page_views(screen.viewer.upcast_ref(), &mut views);
                    views.iter().any(|v| v.state() == crate::reader::page_decode::LoadState::Ready)
                }),
                "a network page is fetched by the loader and shown by the viewer"
            );

            screen.core.apply_mode(ReadingMode::Webtoon);
            assert!(pump_until(|| screen.viewer.first_child().is_some_and(|w| w.is::<crate::reader::viewers::webtoon::frame::ZoomFrame>())));
            assert_eq!(screen.viewer.observe_children().n_items(), 1, "the old viewer is gone, not stacked under the new one");
            screen.core.apply_mode(ReadingMode::LeftToRight);
            assert!(pump_until(|| screen.viewer.first_child().is_some_and(|w| w.is::<gtk::ScrolledWindow>())));
            assert_eq!(screen.viewer.observe_children().n_items(), 1);
            window.destroy();
        });
    }

    /// Hiding and showing the bottom bar must not grow the window: the page area's minimum height stays small.
    #[test]
    fn the_bars_come_back_at_the_top_and_the_bottom_edge_only() {
        assert!(at_bar_edge(2.0, 800.0) && at_bar_edge(796.0, 800.0));
        assert!(!at_bar_edge(400.0, 800.0));
        assert!(!at_bar_edge(400.0, 0.0), "no size yet: only the top edge counts");
    }

    #[test]
    fn toggling_the_menu_does_not_grow_the_window() {
        crate::testing::with_display(|| {
            let f = fixture(2);
            let deps = ReaderDeps {
                db: Arc::clone(&f.db),
                manga_id: f.manga_id,
                chapter_id: f.chapter_ids[0],
                prefs: ReaderPrefs::new(PrefStore::in_memory()),
                store: Rc::new(NoStore),
                build: http_build("menu-toggle"),
                is_downloaded: Rc::new(|_| false),
                downloaded_only: false,
                incognito: false,
                prefs_store: PrefStore::in_memory(),
            };
            let (toaster, _feed) = kuro_platform::toast::toast_channel();
            let screen = open(deps, &crate::reader::launch::open_gate(), toaster, || {});
            let window = gtk::Window::new();
            window.set_default_size(900, 600);
            window.set_child(Some(&screen.root));
            window.present();
            assert!(pump_until(|| screen.viewer.first_child().is_some() && window.height() > 0));
            let settle = || {
                let (ctx, end) = (glib::MainContext::default(), std::time::Instant::now() + std::time::Duration::from_millis(300));
                while std::time::Instant::now() < end {
                    while ctx.iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            for mode in [ReadingMode::LeftToRight, ReadingMode::RightToLeft, ReadingMode::Vertical, ReadingMode::Webtoon] {
                screen.core.apply_mode(mode);
                settle();
                for toggle in 0..4 {
                    screen.chrome.toggle_menu();
                    settle();
                    assert_eq!(window.height(), 600, "the window keeps its height ({mode:?}, toggle {toggle})");
                    let min = screen.viewer.measure(gtk::Orientation::Vertical, -1).0;
                    assert!(min <= 100, "the page area asks for {min}px at least ({mode:?})");
                }
            }
            window.destroy();
        });
    }
}
