//! The Updates screen of one kind (T-42a-030, T-42a-034, T-42a-036): the feed under date headers, the selection gestures
//! (the row menu's Select, tap while selecting, Ctrl and Shift click, Escape) and the bulk bar (bookmark, fillermark, read
//! state, download, delete, play). Each row shows its download state and offers the indicator's actions. In selection
//! mode the cover tap is off and the bar shows; there is no search field.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use std::collections::HashMap;
use std::time::Duration;

use chrono::{Local, Months};
use gtk::gdk::{self, ModifierType};
use gtk::glib;
use adw::prelude::*;
use gtk::{Box as GBox, Button, CheckButton, GestureClick, Image, Label, ListBox, Orientation as O};
use kuro_domain::dates::ListRow;
use kuro_platform::kind::Kind;
use kuro_domain::updates::repo::UpdatesRepo;
use kuro_i18n::{MsgKey, tr};

use crate::data;
use crate::misc::diet_defaults as diet;
use crate::updates::bottom_bar::{self, BarAction, BarCommand, Selected};
use crate::updates::bulk::{self, BulkAction};
use crate::updates::coalesce::watch_coalesced;
use crate::updates::download_actions::{self, DownloadAction, EntryFacts, UnitTarget};
use crate::updates::download_exec;
use crate::updates::download_host;
use crate::updates::download_state::{Change, DiskCache, DownloadState};
use crate::browse::look;
use crate::updates::feed::{self, FeedRow, Gesture, Press};
use crate::updates::format;
use crate::updates::history::COALESCE_MS;
use crate::updates::{copy, open_route, qualities};
use crate::updates::selection::{Selection, is_selection_mode};
use crate::updates::state_page::PageStates;
use crate::updates::updates_dialog::confirm_bulk_delete;
use crate::updates::updates_page::{self, BackAction, UpdatesLoad, back_action, cover_click_enabled};
use crate::widgets::context_menu::{self, Item};
use crate::widgets::download_button::{DownloadButton, Request};

/// The icon of the empty feed.
const EMPTY_ICON: &str = "software-update-available-symbolic";

/// How long the screen's toasts stay (the Updates failure and the bar's errors).
const TOAST_MS: u64 = 4_000;

/// The gesture of a click: Ctrl and Shift change what a tap means.
fn click_gesture(state: ModifierType) -> Gesture {
    if state.contains(ModifierType::CONTROL_MASK) {
        Gesture::CtrlClick
    } else if state.contains(ModifierType::SHIFT_MASK) {
        Gesture::ShiftClick
    } else {
        Gesture::Tap
    }
}

/// Runs `run` when Enter or Space is pressed on the focused `row`. The row is not activatable (its mouse clicks are the
/// gestures'), so the list's own activation never fires and nothing runs twice.
fn on_key_activate(row: &gtk::ListBoxRow, run: impl Fn() + 'static) {
    let key = gtk::EventControllerKey::new();
    key.connect_key_pressed(move |_, keyval, _, _| {
        if matches!(keyval, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
            run();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    row.add_controller(key);
}

/// The rows whose entry title holds `query`, ignoring case; every row for an empty query.
fn matching(rows: &[FeedRow], query: &str) -> Vec<FeedRow> {
    let query = query.trim().to_lowercase();
    rows.iter().filter(|r| query.is_empty() || r.row.entry_title.to_lowercase().contains(&query)).cloned().collect()
}

pub struct UpdatesScreen {
    kind: Kind,
    root: GBox,
    /// The selection row under the header: Cancel, the count, Select all and Select inverse.
    sel_header: GBox,
    sel_label: Label,
    /// The bottom action bar of the selection and the box its buttons go in.
    bar_revealer: gtk::Revealer,
    bar: GBox,
    col: GBox,
    pages: PageStates,
    load: RefCell<UpdatesLoad<FeedRow>>,
    selection: RefCell<Selection>,
    watch: RefCell<Option<glib::JoinHandle<()>>>,
    /// The watches on the download hub (unit events and cache or queue changes), alive as long as the screen.
    download_watch: RefCell<Vec<glib::JoinHandle<()>>>,
    /// Disk checks per unit, dropped on each download-cache signal (T-42b-003).
    disk: RefCell<DiskCache>,
    /// Row position by unit id, rebuilt with each new list, so a download event finds its row (T-42b-040).
    index: RefCell<HashMap<i64, usize>>,
    /// The anime "use external downloader" preference, read once at construction (T-42b-004).
    external_downloader: bool,
    refresh_pending: std::cell::Cell<bool>,
    invalidate_pending: std::cell::Cell<bool>,
    /// The search entry of the page header (`mount::updates`), like History's: it filters the feed by entry title.
    search: gtk::SearchEntry,
    query: RefCell<String>,
}

impl UpdatesScreen {
    /// Builds the screen and starts its query.
    pub fn new(kind: Kind) -> Rc<Self> {
        let screen = Rc::new_cyclic(|weak: &Weak<Self>| {
            let col = crate::ui::column();
            let list = crate::ui::scrolled(&col);
            let retry = weak.clone();
            let pages = PageStates::new(&list, move || {
                if let Some(screen) = retry.upgrade() {
                    screen.load.borrow_mut().retry();
                    screen.start();
                }
            });
            // the selection looks as on the Library page: the shared selection header under the page header, the
            // actions in the shared bar that slides up at the bottom
            let run = |run: fn(&Rc<Self>)| {
                let weak = weak.clone();
                move || {
                    if let Some(screen) = weak.upgrade() {
                        run(&screen);
                    }
                }
            };
            let sel_label = Label::new(None);
            let sel_header = crate::ui::selection_header(&sel_label, run(Self::clear_selection), run(Self::select_all));
            let invert = Button::with_label(&tr(MsgKey::action_select_inverse));
            let invert_run = run(Self::select_inverse);
            invert.connect_clicked(move |_| invert_run());
            sel_header.append(&invert);
            sel_header.set_visible(false);
            // the shared selection bar: more actions than the window is wide scroll sideways
            let bar_revealer = crate::ui::action_bar(&[]);
            let bar = crate::ui::action_bar_row(&bar_revealer);
            let body = GBox::new(O::Vertical, 0);
            body.append(&sel_header);
            body.append(pages.widget());
            body.append(&bar_revealer);
            Self {
                kind,
                root: body,
                sel_header,
                sel_label,
                bar_revealer,
                bar,
                col,
                pages,
                load: RefCell::new(UpdatesLoad::Loading),
                selection: RefCell::default(),
                watch: RefCell::default(),
                download_watch: RefCell::default(),
                disk: RefCell::default(),
                index: RefCell::default(),
                search: gtk::SearchEntry::builder().placeholder_text(tr(MsgKey::action_search_hint)).hexpand(true).build(),
                query: RefCell::default(),
                external_downloader: diet::DOWNLOAD_USE_EXTERNAL,
                refresh_pending: std::cell::Cell::new(false),
                invalidate_pending: std::cell::Cell::new(false),
            }
        });
        screen.pages.set_empty_look(EMPTY_ICON, None);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&screen);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(screen) = weak.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = state.contains(ModifierType::CONTROL_MASK);
            if key == gtk::gdk::Key::Escape && screen.escape() {
                glib::Propagation::Stop
            } else if ctrl && key.to_lower() == gtk::gdk::Key::a {
                screen.select_all();
                glib::Propagation::Stop
            } else if ctrl && key.to_lower() == gtk::gdk::Key::i {
                screen.select_inverse();
                glib::Propagation::Stop
            } else if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) && screen.enter() {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        screen.root.add_controller(keys);
        let weak = Rc::downgrade(&screen);
        screen.search.connect_search_changed(move |entry| {
            if let Some(screen) = weak.upgrade() {
                let text = entry.text().to_string();
                if *screen.query.borrow() != text {
                    *screen.query.borrow_mut() = text;
                    screen.render();
                }
            }
        });
        screen.start();
        screen.watch_downloads();
        screen.watch_last_updated();
        screen
    }

    /// Redraws the "Library last updated" line every 30 s while the screen is shown and not selecting (T-42a-026).
    fn watch_last_updated(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(30), move || {
            let Some(screen) = weak.upgrade() else { return glib::ControlFlow::Break };
            if screen.root.is_mapped() && !screen.selection_mode() {
                screen.render();
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Shows `text` on the shared toast layer for the usual toast time.
    pub fn show_toast(&self, text: &str) {
        crate::toast::notify_for(text, Duration::from_millis(TOAST_MS));
    }

    /// The search entry of this kind, for the page header.
    pub fn search(&self) -> &gtk::SearchEntry {
        &self.search
    }

    pub fn widget(&self) -> &GBox {
        &self.root
    }

    /// The one action of the empty feed: Update library, wired by the page header (`mount::updates`).
    pub fn set_empty_action(&self, action: &Button) {
        self.pages.set_empty_look(EMPTY_ICON, Some(action));
    }

    fn db(&self) -> std::sync::Arc<kuro_db::handle::Db> {
        data::app().db.clone()
    }

    fn selection_mode(&self) -> bool {
        matches!(&*self.load.borrow(), UpdatesLoad::Loaded(rows) if is_selection_mode(rows))
    }

    /// Escape in selection mode clears the selection and is consumed; otherwise it navigates (not consumed).
    fn escape(self: &Rc<Self>) -> bool {
        match back_action(self.selection_mode()) {
            BackAction::ClearSelection => {
                self.clear_selection();
                true
            }
            BackAction::Navigate => false,
        }
    }

    /// Enter on the focused row does what a tap on it does: opens the unit, or toggles it while selecting (T-42b-038).
    /// `false` when the focus is not on a row itself (a button inside it keeps its own Enter), so the key goes on.
    fn enter(self: &Rc<Self>) -> bool {
        let focus = self.root.root().and_then(|root| gtk::prelude::RootExt::focus(&root));
        let Some(id) = focus.and_then(|w| feed::unit_of_row_name(&w.widget_name())) else { return false };
        self.press_later(Gesture::Tap, id);
        true
    }

    fn clear_selection(self: &Rc<Self>) {
        if let UpdatesLoad::Loaded(rows) = &mut *self.load.borrow_mut() {
            self.selection.borrow_mut().toggle_all(rows, false);
        }
        self.render();
    }

    fn select_all(self: &Rc<Self>) {
        if let UpdatesLoad::Loaded(rows) = &mut *self.load.borrow_mut() {
            self.selection.borrow_mut().toggle_all(rows, true);
        }
        self.render();
    }

    fn select_inverse(self: &Rc<Self>) {
        if let UpdatesLoad::Loaded(rows) = &mut *self.load.borrow_mut() {
            self.selection.borrow_mut().invert(rows);
        }
        self.render();
    }

    fn start(self: &Rc<Self>) {
        if let Some(previous) = self.watch.borrow_mut().take() {
            previous.abort();
        }
        let months = u32::try_from(kuro_domain::UPDATES_WINDOW_MONTHS).unwrap_or(3);
        let since = Local::now().checked_sub_months(Months::new(months)).map_or(0, |t| t.timestamp_millis());
        let sub = UpdatesRepo::new(self.db()).watch_recent(self.kind, since);
        let weak = Rc::downgrade(self);
        let handle = watch_coalesced(sub, COALESCE_MS, move |published| {
            let Some(screen) = weak.upgrade() else { return };
            let toast = screen.publish(published.map(|rows| (*rows).clone()));
            screen.render();
            if let Some(message) = toast {
                screen.show_toast(&message);
            }
        });
        *self.watch.borrow_mut() = Some(handle);
        self.render();
    }

    /// Applies one query result. A failure keeps a feed on screen and returns the toast text (T-42b-003).
    fn publish(&self, result: Result<Vec<kuro_domain::updates::model::UpdateRow>, String>) -> Option<String> {
        let mut load = self.load.borrow_mut();
        match result {
            Ok(rows) => {
                let previous = match &*load {
                    UpdatesLoad::Loaded(previous) => previous.clone(),
                    _ => Vec::new(),
                };
                let mut next = feed::replace_rows(rows, &previous, &mut self.selection.borrow_mut());
                self.rederive(&mut next, false);
                *self.index.borrow_mut() = feed::index_rows(&next);
                *load = UpdatesLoad::Loaded(next);
                None
            }
            Err(message) => load.apply_failure(&message),
        }
    }

    fn render(self: &Rc<Self>) {
        let (page, rows) = {
            let load = self.load.borrow();
            let rows = match &*load {
                UpdatesLoad::Loaded(rows) => rows.clone(),
                _ => Vec::new(),
            };
            (load.page(), rows)
        };
        let mode = is_selection_mode(&rows);
        self.fill_bar(&rows, mode);
        self.fill(&rows, mode);
        self.pages.show(&page);
    }

    fn fill(self: &Rc<Self>, all: &[FeedRow], mode: bool) {
        while let Some(child) = self.col.first_child() {
            child.unparent();
        }
        self.col.append(&last_updated_label());
        let query = self.query.borrow().clone();
        let shown = matching(all, &query);
        let rows = &shown[..];
        if rows.is_empty() && !all.is_empty() {
            self.col.append(&crate::ui::dim(&tr(MsgKey::no_results_found)));
        }
        let items = feed::layout(
            rows,
            &Local,
            Local::now().date_naive(),
            diet::LIBRARY_RELATIVE_TIME,
            diet::LIBRARY_DATE_FORMAT,
            kuro_i18n::current_locale().tag,
        );
        // each list with the rows the feed has selected: libadwaita draws their selected fill
        let mut lists: Vec<(ListBox, Vec<gtk::ListBoxRow>)> = Vec::new();
        for item in items {
            match item {
                ListRow::Header(text) => {
                    self.col.append(&crate::ui::heading(&text));
                    let list = crate::ui::boxed_list();
                    self.col.append(&list);
                    lists.push((list, Vec::new()));
                }
                ListRow::Item(i) => {
                    if lists.is_empty() {
                        let list = crate::ui::boxed_list();
                        self.col.append(&list);
                        lists.push((list, Vec::new()));
                    }
                    let Some((list, picked)) = lists.last_mut() else { continue };
                    let row = self.row(&rows[i], mode);
                    list.append(&row);
                    if rows[i].selected {
                        picked.push(row);
                    }
                }
            }
        }
        for (list, picked) in lists {
            crate::widgets::selection_mirror::mirror_list_box(&list, picked);
        }
        if let Some(footer) = updates_page::limit_footer(all.len()) {
            self.col.append(&crate::ui::dim(&footer));
        }
    }

    /// One feed row (`look::list_row`): the selection check, the unread dot and the cover lead, the title sits over a line
    /// of unit name and progress, and the bookmark, fillermark and download indicator end the row. A read or seen row
    /// dims to 0.38.
    fn row(self: &Rc<Self>, feed_row: &FeedRow, mode: bool) -> gtk::ListBoxRow {
        let row = &feed_row.row;
        let lead = GBox::new(O::Horizontal, 12);
        let check = CheckButton::builder().active(feed_row.selected).visible(mode).can_target(false).valign(gtk::Align::Center).build();
        lead.append(&check);
        let dot = Image::from_icon_name("media-record-symbolic");
        dot.set_pixel_size(8);
        dot.add_css_class("accent");
        dot.set_valign(gtk::Align::Center);
        dot.set_visible(!row.done);
        lead.append(&dot);

        let art = look::cover_art(self.kind, row.entry_id, row.source_id, row.thumbnail_url.clone(), true, row.cover_last_modified);
        let cover = look::row_cover(&row.entry_title, Some(&art), look::SMALL.width, look::SMALL.height);
        let cover_tap = GestureClick::new();
        let (weak, entry_id) = (Rc::downgrade(self), row.entry_id);
        cover_tap.connect_pressed(move |gesture, _, _, _| {
            let Some(screen) = weak.upgrade() else { return };
            if cover_click_enabled(screen.selection_mode()) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                crate::screens::open_entry(screen.kind, entry_id);
            }
        });
        cover.add_controller(cover_tap);
        lead.append(&cover);

        // the bookmark and filler marks sit before the download indicator
        let end = GBox::new(O::Horizontal, 4);
        if row.bookmark {
            let star = Image::from_icon_name("starred-symbolic");
            star.set_pixel_size(14);
            star.add_css_class("accent");
            star.set_valign(gtk::Align::Center);
            end.append(&star);
        }
        if row.fillermark && self.kind.is_anime() {
            // the stock theme has no tertiary colour; the warning colour keeps it apart from the bookmark
            let tag = Image::from_icon_name("tag-symbolic");
            tag.set_pixel_size(14);
            tag.add_css_class("warning");
            tag.set_valign(gtk::Align::Center);
            end.append(&tag);
        }
        end.append(&self.download_button(feed_row, mode));
        let detail = match updates_page::progress_text(self.kind, row.done, row.progress, row.total_seconds) {
            Some(progress) => format!("{} · {progress}", row.item_name),
            None => row.item_name.clone(),
        };
        let list_row = look::list_row(Some(lead.upcast()), &row.entry_title, Some(&detail), Some(end.upcast()));
        list_row.set_title_lines(1);
        list_row.set_subtitle_lines(1);
        list_row.set_widget_name(&feed::row_name(row.item_id));
        list_row.set_opacity(updates_page::row_alpha(row.done));
        self.attach_gestures(list_row.upcast_ref(), row.item_id);
        list_row.upcast()
    }

    /// Tap, Ctrl+click and Shift+click on a row, and its right-click menu (a long press on a touch screen), whose Select
    /// starts the selection as the long press did.
    fn attach_gestures(self: &Rc<Self>, row: &gtk::ListBoxRow, id: i64) {
        let click = GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_released(move |gesture, _, _, _| {
            if let Some(screen) = weak.upgrade() {
                screen.press_later(click_gesture(gesture.current_event_state()), id);
            }
        });
        row.add_controller(click);
        let weak = Rc::downgrade(self);
        context_menu::attach(row, move |_, _| weak.upgrade().map_or_else(Vec::new, |screen| screen.row_menu(id)));
        // Enter and Space on the focused row act as a tap
        let weak = Rc::downgrade(self);
        on_key_activate(row, move || {
            if let Some(screen) = weak.upgrade() {
                screen.press_later(Gesture::Tap, id);
            }
        });
    }

    /// The right-click menu of the row of unit `id`: Open, Mark as read or unread (watched or unwatched), Download while
    /// it is not downloaded or queued, and Select. The row's facts are copied out before the items are made.
    fn row_menu(self: &Rc<Self>, id: i64) -> Vec<Item> {
        let facts = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => rows.iter().find(|r| r.row.item_id == id).map(|r| (r.row.done, r.download.state)),
            _ => None,
        };
        let Some((done, state)) = facts else { return Vec::new() };
        let weak = Rc::downgrade(self);
        let act = move |run: Box<dyn Fn(&Rc<Self>)>| {
            let weak = weak.clone();
            move || {
                if let Some(screen) = weak.upgrade() {
                    run(&screen);
                }
            }
        };
        let mark = if done { BarAction::MarkUnread } else { BarAction::MarkRead };
        let mut items = vec![
            Item::new(crate::browse::texts::OPEN, 0, act(Box::new(move |screen: &Rc<Self>| screen.open_unit(id, false)))),
            Item::new(tr(bottom_bar::label(mark, self.kind)), 1, act(Box::new(move |screen: &Rc<Self>| screen.mark_one(id, !done)))),
        ];
        if matches!(state, DownloadState::NotDownloaded | DownloadState::Error) {
            items.push(Item::new(tr(MsgKey::action_download), 1, act(Box::new(move |screen: &Rc<Self>| screen.row_download(DownloadAction::Start, id)))));
        }
        items.push(Item::new(tr(MsgKey::action_select), 2, act(Box::new(move |screen: &Rc<Self>| screen.press(Gesture::LongPress, id)))));
        items
    }

    /// Marks one unit read (or seen) or not, as the bar's action does for a selection; marking read removes its download
    /// when the preference says so (E-U33).
    fn mark_one(self: &Rc<Self>, id: i64, read: bool) {
        let target = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => rows.iter().find(|r| r.row.item_id == id).map(|r| UnitTarget { entry_id: r.row.entry_id, unit_id: id, state: r.download.state }),
            _ => None,
        };
        let Some(target) = target else { return };
        match bulk::apply(&self.db(), self.kind, &[id], BulkAction::MarkRead(read)) {
            Ok(()) if read => {
                self.tell_trackers(&[id]);
                self.remove_downloads_of_read(vec![target]);
            }
            Ok(()) => {}
            Err(message) => self.show_toast(&message),
        }
    }

    /// Handles the press after the gesture callback returns, because a change rebuilds the row that owns the gesture.
    fn press_later(self: &Rc<Self>, gesture: Gesture, id: i64) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(screen) = weak.upgrade() {
                screen.press(gesture, id);
            }
        });
    }

    fn press(self: &Rc<Self>, gesture: Gesture, id: i64) {
        let outcome = match &mut *self.load.borrow_mut() {
            UpdatesLoad::Loaded(rows) => feed::press(gesture, &mut self.selection.borrow_mut(), rows, id),
            _ => Press::Nothing,
        };
        match outcome {
            Press::Open => self.open_unit(id, false),
            Press::Changed => self.render(),
            Press::Nothing => {}
        }
    }

    /// Opens the unit. Anime follows the "always use external player" preference inverted by `alt` (E-U35): the row
    /// tap passes `false`, the bar's play actions `true`. Manga opens in the reader.
    fn open_unit(&self, id: i64, alt: bool) {
        let target = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => rows.iter().find(|r| r.row.item_id == id).map(|r| r.row.entry_id),
            _ => None,
        };
        if let Some(entry_id) = target {
            let always_external = self.kind.is_anime() && crate::misc::diet_defaults::PLAYER_ALWAYS_EXTERNAL;
            open_route::open(self.kind, entry_id, id, open_route::route(self.kind, always_external, alt));
        }
    }

    /// One batch write of the selected rows whose flag differs from the target, then the selection rule of the action.
    /// With "remove after marked as read" on, the downloads of the rows the write marked read are removed (E-U33).
    fn bulk(self: &Rc<Self>, action: BulkAction) {
        let (ids, newly_read) = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => {
                let ids = bulk::targets(&feed::selected_flags(rows), action);
                let newly_read: Vec<UnitTarget> = rows
                    .iter()
                    .filter(|r| ids.contains(&r.row.item_id))
                    .map(|r| UnitTarget { entry_id: r.row.entry_id, unit_id: r.row.item_id, state: r.download.state })
                    .collect();
                (ids, newly_read)
            }
            _ => return,
        };
        let result = bulk::apply(&self.db(), self.kind, &ids, action);
        if result.is_ok() && action == BulkAction::MarkRead(true) {
            self.tell_trackers(&ids);
            self.remove_downloads_of_read(newly_read);
        }
        let message = match &mut *self.load.borrow_mut() {
            UpdatesLoad::Loaded(rows) => bulk::after_bulk(action, result, &mut self.selection.borrow_mut(), rows),
            _ => None,
        };
        self.render();
        if let Some(message) = message {
            self.show_toast(&message);
        }
    }

    /// Tells the trackers about the units just marked read, the same send as the title page and the Library (the numbers are
    /// read from the stored rows).
    fn tell_trackers(&self, ids: &[i64]) {
        let items = kuro_data::item::ItemRepo::new(self.db());
        let marked: Vec<_> = ids.iter().filter_map(|id| items.by_id(*id).ok().flatten()).collect();
        crate::entry_core::tracking::progress::marked_items(self.kind, &marked);
    }

    /// Removes the downloads of the units just marked read, grouped per entry, when the preference is on (E-U33).
    fn remove_downloads_of_read(&self, newly_read: Vec<UnitTarget>) {
        let enabled = diet::DOWNLOAD_REMOVE_AFTER_MARKED_READ;
        if !enabled || newly_read.is_empty() {
            return;
        }
        let (backend, kind) = (download_host::current(), self.kind);
        download_host::spawn_detached(move || {
            download_exec::remove_read_downloads(&*backend, kind, true, &newly_read);
        });
    }

    /// Runs a download action over `targets` detached from the screen (T-42a-031), so leaving the screen does not
    /// cancel it. The backend is taken here because the shell's index is only reachable on the GTK thread.
    fn run_download(&self, action: DownloadAction, targets: Vec<UnitTarget>) {
        if action == DownloadAction::ShowQualities {
            return self.show_qualities(&targets);
        }
        let sources: HashMap<i64, i64> = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => rows.iter().map(|r| (r.row.entry_id, r.row.source_id)).collect(),
            _ => return,
        };
        let (backend, kind) = (download_host::current(), self.kind);
        // every row of the feed is a library entry
        let facts = move |backend: &dyn download_host::DownloadBackend, entry: i64| {
            sources.get(&entry).map(|source| EntryFacts { in_library: true, source_installed: backend.source_installed(kind, *source) })
        };
        if action == DownloadAction::Delete {
            // the source lookups are in-memory reads, so the skipped entries are known here to warn and toast (T-42b-010)
            let skipped = download_actions::delete_skipped(&targets, |entry| facts(&*backend, entry));
            if !skipped.is_empty() {
                tracing::warn!(?skipped, "delete skipped entries whose source is not installed");
                self.show_toast(&copy::delete_source_missing());
            }
        }
        download_host::spawn_detached(move || {
            download_exec::dispatch(&*backend, kind, action, &targets, |entry| facts(&*backend, entry));
        });
    }

    /// SHOW_QUALITIES: opens the episode options sheet when exactly one row is the target; any other count leaves
    /// everything as it is (E-U49).
    fn show_qualities(&self, targets: &[UnitTarget]) {
        let request = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => qualities::request_of(targets, rows, self.external_downloader),
            _ => None,
        };
        if let Some(request) = request {
            qualities::open(request);
        }
    }

    /// A bar download action over the selection; the selection clears afterwards (as `toggleAllSelection(false)`).
    fn bulk_download(self: &Rc<Self>, action: DownloadAction) {
        let targets = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => feed::selected_targets(rows),
            _ => return,
        };
        self.run_download(action, targets);
        self.clear_selection();
    }

    /// An indicator action on one row.
    fn row_download(self: &Rc<Self>, action: DownloadAction, id: i64) {
        let target = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => rows
                .iter()
                .find(|r| r.row.item_id == id)
                .map(|r| UnitTarget { entry_id: r.row.entry_id, unit_id: r.row.item_id, state: r.download.state }),
            _ => None,
        };
        if let Some(target) = target {
            self.run_download(action, vec![target]);
        }
    }

    /// The bar's play actions: the single selected unit opens in the player that is not the preferred one (E-U35).
    fn play_selected(self: &Rc<Self>, alt: bool) {
        let id = match &*self.load.borrow() {
            UpdatesLoad::Loaded(rows) => match rows.iter().filter(|r| r.selected).collect::<Vec<_>>().as_slice() {
                [only] => Some(only.row.item_id),
                _ => None,
            },
            _ => None,
        };
        let Some(id) = id else { return };
        self.open_unit(id, alt);
        self.clear_selection();
    }

    /// Re-derives the download state of `rows` from the backend's queue and the cached disk check.
    fn rederive(&self, rows: &mut [FeedRow], invalidate: bool) {
        let (backend, kind) = (download_host::current(), self.kind);
        let mut cache = self.disk.borrow_mut();
        if invalidate {
            cache.invalidate();
        }
        feed::rederive(rows, |id| backend.queue_entry(kind, id), &mut cache, |key| backend.on_disk(kind, key));
    }

    /// Redraws after the quiet period of a burst of download events. `invalidate` also drops the disk checks.
    fn refresh_later(self: &Rc<Self>, invalidate: bool) {
        if invalidate {
            self.invalidate_pending.set(true);
        }
        if self.refresh_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(COALESCE_MS), move || {
            let Some(screen) = weak.upgrade() else { return };
            screen.refresh_pending.set(false);
            if screen.invalidate_pending.replace(false) {
                if let UpdatesLoad::Loaded(rows) = &mut *screen.load.borrow_mut() {
                    screen.rederive(rows, true);
                }
            }
            screen.render();
        });
    }

    /// Follows the downloads: a change of a queued unit (`downloads::live`) patches the row it names, and a cache or
    /// queue change on the hub re-derives every row (T-42b-002, T-42b-003). The feed query itself is not re-run for these.
    fn watch_downloads(self: &Rc<Self>) {
        let mut changes = download_host::hub().cache.subscribe();
        let weak = Rc::downgrade(self);
        let on_changes = glib::spawn_future_local(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match changes.recv().await {
                    Ok(()) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
                match weak.upgrade() {
                    Some(screen) => screen.refresh_later(true),
                    None => return,
                }
            }
        });
        *self.download_watch.borrow_mut() = vec![on_changes];
        let (weak, kind) = (Rc::downgrade(self), self.kind);
        crate::downloads::live::follow(&self.root, move |live| {
            if live.kind == kind
                && let Some(screen) = weak.upgrade()
            {
                screen.patch_unit(live.id, Change::Status(live.state));
                screen.patch_unit(live.id, Change::Progress(i64::from(live.percent)));
            }
        });
    }

    /// One download event: patches the matching row, then redraws after the quiet period.
    fn patch_unit(self: &Rc<Self>, unit_id: i64, change: Change) {
        let found = match &mut *self.load.borrow_mut() {
            UpdatesLoad::Loaded(rows) => feed::apply_download(rows, &self.index.borrow(), unit_id, change),
            _ => false,
        };
        if found {
            self.refresh_later(false);
        }
    }

    /// The download button of a row (`widgets::download_button`): a click starts, retries, cancels, or offers Delete. It
    /// is off while selecting, as the cover tap is.
    fn download_button(self: &Rc<Self>, feed_row: &FeedRow, mode: bool) -> gtk::Button {
        let (weak, id) = (Rc::downgrade(self), feed_row.row.item_id);
        let button = DownloadButton::new(move |request| {
            let action = match request {
                Request::Start => DownloadAction::Start,
                Request::Cancel => DownloadAction::Cancel,
                Request::Delete => DownloadAction::Delete,
            };
            if let Some(screen) = weak.upgrade() {
                screen.row_download(action, id);
            }
        });
        button.set(feed_row.download.state, feed_row.download.progress.clamp(0, 100) as u8);
        button.widget.set_sensitive(cover_click_enabled(mode));
        button.widget
    }
}

impl UpdatesScreen {
    /// Rebuilds the selection row and the bulk bar from the selection (T-42a-036): the "N selected" count with select
    /// all, select inverse and cancel, and the actions the predicates allow (up to the ten of `bottom_bar`).
    fn fill_bar(self: &Rc<Self>, rows: &[FeedRow], mode: bool) {
        while let Some(child) = self.bar.first_child() {
            self.bar.remove(&child);
        }
        self.sel_header.set_visible(mode);
        self.bar_revealer.set_reveal_child(mode);
        if !mode {
            return;
        }
        let selected: Vec<Selected> = rows
            .iter()
            .filter(|r| r.selected)
            .map(|r| Selected {
                read: r.row.done,
                bookmark: r.row.bookmark,
                fillermark: r.row.fillermark,
                progress: r.row.progress,
                state: r.download.state,
            })
            .collect();
        self.sel_label.set_label(&bottom_bar::selected_text(selected.len()));
        let always_external = crate::misc::diet_defaults::PLAYER_ALWAYS_EXTERNAL;
        for action in bottom_bar::visible_actions(self.kind, &selected, always_external) {
            self.bar.append(&self.bar_button(action, self.kind));
        }
    }

    fn bar_button(self: &Rc<Self>, action: BarAction, kind: Kind) -> Button {
        let content = GBox::new(O::Horizontal, 6);
        content.append(&Image::from_icon_name(bottom_bar::icon(action)));
        content.append(&Label::new(Some(&tr(bottom_bar::label(action, kind)))));
        let button = Button::new();
        button.set_child(Some(&content));
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            let Some(screen) = weak.upgrade() else { return };
            match bottom_bar::command(action) {
                BarCommand::Bulk(bulk_action) => screen.bulk(bulk_action),
                BarCommand::Download(DownloadAction::Delete) => {
                    // the bar's delete always asks; OK runs the delete plan (T-42a-032)
                    let confirm = Rc::downgrade(&screen);
                    confirm_bulk_delete(screen.kind, move || {
                        if let Some(screen) = confirm.upgrade() {
                            screen.bulk_download(DownloadAction::Delete);
                        }
                    });
                }
                BarCommand::Download(download) => screen.bulk_download(download),
                BarCommand::Play { alt } => screen.play_selected(alt),
            }
        });
        button
    }
}

/// The "Library last updated: ..." line at the top of the feed (T-42b-016), the context line of the Library page.
fn last_updated_label() -> Label {
    let last = data::app().library.last_updated_timestamp.get();
    let text = format::last_updated_line(last, Local::now().timestamp_millis());
    Label::builder().label(text).xalign(0.0).css_classes(["kb-library-context"]).build()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn titled(title: &str) -> FeedRow {
        use kuro_domain::updates::model::UpdateRow;
        FeedRow {
            row: UpdateRow {
                kind: kuro_platform::kind::Kind::Manga,
                entry_id: 1,
                entry_title: title.into(),
                item_id: 1,
                item_name: "Chapter 1".into(),
                scanlator: None,
                done: false,
                bookmark: false,
                fillermark: false,
                progress: 0,
                total_seconds: 0,
                source_id: 1,
                date_fetch: 0,
                date_upload: 0,
                thumbnail_url: None,
                cover_last_modified: 0,
            },
            selected: false,
            download: crate::updates::feed::NOT_DOWNLOADED,
        }
    }

    #[test]
    fn the_search_keeps_the_rows_whose_title_holds_the_text_ignoring_case() {
        let rows = [titled("One Piece"), titled("Naruto"), titled("Piece of Cake")];
        let names = |q: &str| matching(&rows, q).into_iter().map(|r| r.row.entry_title).collect::<Vec<_>>();
        assert_eq!(names(""), ["One Piece", "Naruto", "Piece of Cake"], "no text: every row");
        assert_eq!(names("  PIECE "), ["One Piece", "Piece of Cake"]);
        assert!(names("bleach").is_empty());
    }

    #[test]
    fn enter_and_space_on_an_updates_row_run_its_tap_once_each_and_other_keys_do_not() {
        crate::testing::with_display(|| {
            let row = gtk::ListBoxRow::builder().activatable(false).build();
            let taps = Rc::new(Cell::new(0));
            let seen = Rc::clone(&taps);
            on_key_activate(&row, move || seen.set(seen.get() + 1));
            let controllers = row.observe_controllers();
            let key = (0..controllers.n_items()).filter_map(|i| controllers.item(i)).find_map(|c| c.downcast::<gtk::EventControllerKey>().ok()).expect("the row's key controller");
            let press = |keyval: gdk::Key| key.emit_by_name::<bool>("key-pressed", &[&keyval, &0u32, &ModifierType::empty()]);
            assert!(press(gdk::Key::Return));
            assert!(press(gdk::Key::space));
            assert!(!press(gdk::Key::a), "other keys pass through");
            assert_eq!(taps.get(), 2);
        });
    }
}
