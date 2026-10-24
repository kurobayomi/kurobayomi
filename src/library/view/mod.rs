//! The library view: category tabs, a context line, the cover grid of the active page, the selection row and bar,
//! and the empty states. Its search entry, refresh button and view options sit in the page header (`controls`, placed by
//! `screens::libraries`). Pipeline work runs on the worker (`worker`); this file applies its replies.
//! Borrows of the shared state are never held across a widget call, because widget calls can re-enter the handlers.

mod card;
mod options;
mod tabs;
mod welcome;

use std::cell::{Cell as FlagCell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{Orientation as O, gdk, gio, glib};
use kuro_data::entry::EntryRepo;
use kuro_db::handle::Db;
use kuro_db::codec::FetchType;
use kuro_domain::category::service::CategoryService;
use kuro_platform::kind::Kind;
use kuro_domain::library::active::clamp_page;
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::page::{LibraryItem, toolbar_title};
use kuro_i18n::{MsgKey, tr, tr_plural};
use kuro_domain::library::search::{SEARCH_DEBOUNCE, empty_page, shows_global_search};
use kuro_images::art::ArtRef;
use kuro_prefs::library::LibraryPrefs;

use crate::library::actions;
use crate::misc::diet_defaults as diet;
use crate::library::cells::{self, Cell};
use crate::library::category_dialog::CategoryDialog;
use crate::library::keys;
use crate::library::layout;
use crate::library::load;
use crate::library::query::{Built, Facts};
use crate::library::select::Selection;
use crate::library::worker::{Gate, Job, Reply, Worker};
use crate::widgets::context_menu::{self, Item};

/// What the view reads from the app: storage, preferences and the lookups that live outside storage.
pub struct Deps {
    pub kind: Kind,
    pub db: Arc<Db>,
    pub prefs: Rc<LibraryPrefs>,
    pub categories: Rc<CategoryService>,
    pub facts: Facts,
    pub downloaded_only: Rc<dyn Fn() -> bool>,
    pub logged_in: Rc<dyn Fn() -> BTreeSet<i64>>,
    /// `pref_always_use_external_player`, read when the continue button is clicked.
    pub always_external: Rc<dyn Fn() -> bool>,
    /// Whether the chapter or episode with this id is on disk; the entry's downloaded filter reads it.
    pub item_downloaded: Rc<dyn Fn(i64) -> bool>,
}

/// What the continue button asks the app to open: `item` is the chapter or episode id of `entry`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resume {
    pub entry: i64,
    pub item: i64,
    /// Play it in the external player (anime only; the reader ignores it).
    pub external: bool,
}

/// Navigation out of the library: open an entry, resume it, search globally, edit the categories.
pub struct Nav {
    pub open: Rc<dyn Fn(i64)>,
    pub resume: Rc<dyn Fn(Resume)>,
    /// Queues the downloads of the selected entries: how many, from the Download menu.
    pub download: Rc<dyn Fn(Vec<i64>, actions::DownloadAmount)>,
    /// Starts removing the selected entries from the library.
    pub delete: Rc<dyn Fn(Vec<i64>)>,
    pub global_search: Rc<dyn Fn(&str)>,
    pub edit_categories: Rc<dyn Fn()>,
    /// Starts an update pass of one category (`Some(id)`); `false` when a pass of this kind is already running.
    pub update: Rc<dyn Fn(Option<i64>) -> bool>,
    /// Whether a pass of this kind is running now; the refresh button shows it.
    pub update_running: Rc<dyn Fn() -> bool>,
}

/// Starts an update of the active category and says what happened: the refresh button and F5 (T-37a-035).
/// The whole-library pass behind the refresh button and the menu: no category, with the toast of its scope (T-37a-035).
fn update_whole_library(inner: &Rc<RefCell<Inner>>) {
    let update = inner.borrow().nav.update.clone();
    let started = update(None);
    crate::toast::notify_for(&actions::update_toast(actions::UpdateScope::Library, !started), crate::library::TOAST_TIME);
    sync_refresh_button(inner);
}

fn refresh_active(inner: &Rc<RefCell<Inner>>) {
    let (update, category) = {
        let st = inner.borrow();
        let page = st.built.as_ref().and_then(|b| b.pages.get(active_index(&st)));
        (st.nav.update.clone(), page.map(|p| p.category.id))
    };
    let started = update(category);
    crate::toast::notify_for(&actions::update_toast(actions::UpdateScope::Category, !started), crate::library::TOAST_TIME);
    sync_refresh_button(inner);
}

/// The shared state of one library view. Lives in an `Rc<RefCell<_>>` that handlers reach through a `Weak`.
struct Inner {
    deps: Deps,
    nav: Nav,
    worker: Worker,
    gate: Gate,
    built: Option<Built>,
    /// The active tab by index, used when `active` is not on the current pages.
    page: usize,
    /// The active tab by category id; this is what the app remembers across launches.
    active: Option<i64>,
    query: Option<String>,
    search_turn: u64,
    selection: Selection,
    /// What the grid shows now: the entry id of each cell (`None` for the global search row) and the ids that were
    /// selected. A selection change then rebinds only the cards whose selected state changed, not every card.
    shown_ids: Vec<Option<i64>>,
    shown_selected: std::collections::HashSet<i64>,
    /// The view options menu button of the page header; its dot shows while a filter is on (T-37a-056).
    options: options::Options,
    /// The logged-in trackers of the last load, so a render never asks the secret store again.
    logged_in: BTreeSet<i64>,
    last_resume: HashMap<i64, Instant>,
    store: gio::ListStore,
    /// The context line: the category name and counts, shown only when the header switch does not already say it.
    title: gtk::Label,
    /// "Use -term to exclude", shown while the search text starts with `-` (E-04).
    hint: gtk::Label,
    /// The selection row (count, select all, invert, cancel); the header's search and refresh hide while it shows (U-1.1).
    sel_header: gtk::Box,
    download_button: gtk::MenuButton,
    search: gtk::SearchEntry,
    tabs: tabs::TabStrip,
    scroller: gtk::ScrolledWindow,
    grid: gtk::GridView,
    /// List mode shows this over the same store instead of the grid (T-37a-021).
    list: gtk::ListView,
    /// Starts an update of the active category; busy while a pass of this kind runs (T-26a-102).
    refresh_button: gtk::Button,
    empty: gtk::Box,
    empty_label: gtk::Label,
    global_button: gtk::Button,
    /// The action of an empty library that has a repository: open Browse (T-37a-053).
    empty_action: gtk::Button,
    /// The welcome view of an empty library with no repository yet (docs/design/app-flow.md, first run).
    welcome: welcome::Welcome,
    bar: gtk::Revealer,
    bar_label: gtk::Label,
}

/// Handle to the library view. The widget is `widget()`; `refresh()` reruns the pipeline after a setting or data
/// change.
#[derive(Clone)]
pub struct LibraryView {
    inner: Rc<RefCell<Inner>>,
    root: gtk::Box,
}

impl LibraryView {
    pub fn new(deps: Deps, nav: Nav) -> std::io::Result<Self> {
        let (replies_tx, replies_rx) = async_channel::unbounded::<Reply>();
        let worker = Worker::spawn(replies_tx)?;
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();

        let title = gtk::Label::new(None);
        title.add_css_class("kb-library-context");
        title.set_xalign(0.0);
        title.set_margin_start(layout::SPACE_L);
        title.set_margin_top(layout::SPACE_S);
        let hint = gtk::Label::new(Some(&layout::exclude_hint()));
        hint.add_css_class("dim-label");
        hint.set_xalign(0.0);
        hint.set_margin_start(layout::SPACE_L);
        hint.set_visible(false);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&tr(MsgKey::action_search_hint)));
        // the page header sizes it (`ui::page` clamps a search entry to `ui::SEARCH_WIDTH`)
        search.set_hexpand(true);
        let refresh_button = crate::ui::refresh_button(&tr(MsgKey::action_update_library));
        let options = options::Options::new(deps.kind);

        let tabs = tabs::TabStrip::new();
        let grid = gtk::GridView::new(
            Some(gtk::NoSelection::new(Some(store.clone()))),
            Some(gtk::SignalListItemFactory::new()),
        );
        grid.set_vexpand(true);
        grid.set_single_click_activate(false);
        grid.add_css_class("kb-library-grid");
        let list = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(store.clone()))),
            Some(gtk::SignalListItemFactory::new()),
        );
        list.set_vexpand(true);
        list.set_single_click_activate(false);
        // the shared page scroller: it scrolls down only, so a wide grid never shows a sideways scrollbar
        let scroller = crate::ui::scrolled(&grid);

        let global_button = gtk::Button::new();
        global_button.set_visible(false);
        let empty_action = gtk::Button::with_label(&tr(MsgKey::browse));
        empty_action.add_css_class("suggested-action");
        empty_action.set_visible(false);
        // the shared empty state; the message and the two actions are set by `render`
        let (empty, empty_label) = crate::browse::look::empty_state(
            match deps.kind {
                Kind::Anime => "video-x-generic-symbolic",
                Kind::Manga => "accessories-dictionary-symbolic",
            },
            "",
            &[global_button.clone(), empty_action.clone()],
        );
        let welcome = welcome::Welcome::new();

        let bar_label = gtk::Label::new(None);
        // the shared selection header goes in here by `wire_bar`, which has the handlers' weak reference
        let sel_header = gtk::Box::new(O::Vertical, 0);
        sel_header.set_visible(false);
        let download_button = crate::ui::quiet_menu(gtk::MenuButton::builder().label(tr(MsgKey::action_download)).build());
        // the buttons are packed into the bar's `ActionBar` by `wire_bar`
        let bar = crate::ui::action_bar(&[]);

        let root = gtk::Box::new(O::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.append(&sel_header);
        root.append(&tabs.scroller);
        root.append(&title);
        root.append(&hint);
        root.append(&scroller);
        root.append(&empty);
        root.append(welcome.widget());
        root.append(&bar);

        // the library opens on the tab it was left on (T-37a-017); an id that is no longer a page falls back to the first
        let remembered = remembered_category(&deps.prefs, deps.kind);
        let inner = Rc::new(RefCell::new(Inner {
            deps,
            nav,
            worker,
            gate: Gate::default(),
            built: None,
            page: 0,
            active: Some(remembered),
            query: None,
            search_turn: 0,
            selection: Selection::default(),
            shown_ids: Vec::new(),
            shown_selected: std::collections::HashSet::new(),
            options,
            logged_in: BTreeSet::new(),
            last_resume: HashMap::new(),
            store,
            title,
            hint,
            sel_header,
            download_button,
            search,
            tabs,
            scroller,
            grid,
            list,
            refresh_button,
            empty,
            empty_label,
            global_button,
            empty_action,
            welcome,
            bar,
            bar_label,
        }));
        let view = Self { inner: inner.clone(), root };
        // a resize across a column boundary or the 900 px breakpoint changes the count without a data change
        let (weak, seen) = (Rc::downgrade(&inner), FlagCell::new((-1, -1)));
        let scroller = inner.borrow().scroller.clone();
        scroller.add_tick_callback(move |scroller, _| {
            let size = (scroller.width(), scroller.height());
            let before = seen.replace(size);
            if before != size {
                if let Some(inner) = weak.upgrade() {
                    if before.0 != size.0 {
                        apply_columns(&inner);
                    }
                    // List rows divide the page height, so a new height or list size rebuilds them
                    if is_list_mode(&inner.borrow()) {
                        render(&inner);
                    }
                }
            }
            glib::ControlFlow::Continue
        });
        // a new cover size redraws the cards at their new width and recounts the columns (the first call is the seed: the
        // first render draws them)
        let (weak, seeded) = (Rc::downgrade(&inner), FlagCell::new(false));
        let (grid, cover_size) = (inner.borrow().grid.clone(), inner.borrow().deps.prefs.cover_size.clone());
        crate::bind::bind(&grid, &cover_size, move |_, _| {
            if !seeded.replace(true) {
                return;
            }
            if let Some(inner) = weak.upgrade() {
                apply_columns(&inner);
                render(&inner);
            }
        });
        view.wire(&replies_rx);
        view.refresh();
        Ok(view)
    }

    /// The widget to place in the page.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The search entry, the refresh button and the view options button, for the page header.
    pub fn controls(&self) -> (gtk::SearchEntry, gtk::Button, gtk::Widget) {
        let st = self.inner.borrow();
        (st.search.clone(), st.refresh_button.clone(), st.options.widget.clone().upcast())
    }

    /// Reruns the pipeline with the current preferences and query.
    pub fn refresh(&self) {
        submit(&self.inner);
    }

    /// Sets the search text from outside (B-1.3): the entry shows it and the library filters at once, without the
    /// typing debounce. An empty text clears the search.
    pub fn set_query(&self, text: &str) {
        let search = {
            let mut st = self.inner.borrow_mut();
            st.search_turn += 1;
            st.query = (!text.is_empty()).then(|| text.to_owned());
            st.search.clone()
        };
        search.set_text(text);
        submit(&self.inner);
    }

    /// The header menu's "Update category": a pass over the category on show.
    pub fn update_category(&self) {
        refresh_active(&self.inner);
    }

    /// How many categories the library shows tabs for.
    pub fn category_count(&self) -> usize {
        self.inner.borrow().built.as_ref().map_or(0, |built| built.pages.len())
    }

    /// How many cells the grid shows. Test-only.
    #[cfg(test)]
    pub(crate) fn cover_count(&self) -> usize {
        self.inner.borrow().store.n_items() as usize
    }

    fn wire(&self, replies: &async_channel::Receiver<Reply>) {
        let weak = Rc::downgrade(&self.inner);

        // the grid and the list share one binder; List mode shows the list (T-37a-021)
        let (grid_factory, list_factory) = {
            let st = self.inner.borrow();
            (st.grid.factory(), st.list.factory())
        };
        for factory in [grid_factory, list_factory].into_iter().flatten() {
            let Ok(factory) = factory.downcast::<gtk::SignalListItemFactory>() else { continue };
            let weak_bind = weak.clone();
            factory.connect_bind(move |_, obj| {
                if let Some(item) = obj.downcast_ref::<gtk::ListItem>() {
                    bind_item(&weak_bind, item);
                }
            });
        }
        self.wire_refresh(&weak);
        let options = self.inner.borrow().options.clone();
        options.wire(&weak);

        let weak_search = weak.clone();
        let search = self.inner.borrow().search.clone();
        search.connect_search_changed(move |entry| {
            let Some(inner) = weak_search.upgrade() else { return };
            let text = entry.text().to_string();
            let turn = {
                let mut st = inner.borrow_mut();
                st.hint.set_visible(layout::shows_exclude_hint(&text));
                st.search_turn += 1;
                st.search_turn
            };
            let weak_later = weak_search.clone();
            glib::timeout_add_local_once(SEARCH_DEBOUNCE, move || {
                let Some(inner) = weak_later.upgrade() else { return };
                if inner.borrow().search_turn != turn {
                    return;
                }
                let query = (!text.is_empty()).then_some(text);
                if inner.borrow().query == query {
                    return;
                }
                inner.borrow_mut().query = query;
                submit(&inner);
            });
        });

        let key = gtk::EventControllerKey::new();
        let weak_key = weak.clone();
        key.connect_key_pressed(move |_, keyval, _, modifiers| {
            let Some(inner) = weak_key.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            let (has_selection, typing) = {
                let st = inner.borrow();
                (!st.selection.is_empty(), st.search.has_focus() || st.search.focus_child().is_some())
            };
            let Some(shortcut) = keys::shortcut(keyval, ctrl, has_selection, typing) else { return glib::Propagation::Proceed };
            match shortcut {
                keys::Shortcut::SelectAll => select_with(&inner, |sel, page| sel.select_all(page)),
                keys::Shortcut::Invert => select_with(&inner, |sel, page| sel.invert(page)),
                keys::Shortcut::DeleteSelection => selected_action(&inner, |nav| nav.delete.clone()),
                keys::Shortcut::RefreshCategory => refresh_active(&inner),
                keys::Shortcut::Tab(step) => {
                    let target = {
                        let st = inner.borrow();
                        let count = st.built.as_ref().map_or(0, |b| b.pages.len());
                        (active_index(&st) as i64 + step).clamp(0, (count as i64 - 1).max(0)) as usize
                    };
                    select_tab(&inner, target);
                }
            }
            glib::Propagation::Stop
        });
        self.root.add_controller(key);
        // Escape runs in the capture phase so it clears the selection before the search entry sees it
        let esc = gtk::EventControllerKey::new();
        esc.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_esc = weak.clone();
        esc.connect_key_pressed(move |_, keyval, _, _| {
            if keyval != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            let Some(inner) = weak_esc.upgrade() else { return glib::Propagation::Proceed };
            if escape(&inner) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.root.add_controller(esc);
        // the search entry sits in the page header, outside the root, so Escape typed in it arrives as stop-search
        let weak_stop = weak.clone();
        search.connect_stop_search(move |_| {
            if let Some(inner) = weak_stop.upgrade() {
                escape(&inner);
            }
        });

        self.wire_bar(&weak);
        self.watch_prefs();
        self.watch_rows();

        let weak_reply = weak.clone();
        let replies = replies.clone();
        glib::spawn_future_local(async move {
            while let Ok(reply) = replies.recv().await {
                let Some(inner) = weak_reply.upgrade() else { break };
                apply_reply(&inner, reply);
            }
        });
    }

    /// Reloads the library when a preference that shapes it changes (T-37a-004). The store reports changes on a
    /// channel, which this polls on the main context; the task ends with the view.
    fn watch_prefs(&self) {
        let (changes, kind) = {
            let st = self.inner.borrow();
            (st.deps.prefs.store().subscribe(), st.deps.kind)
        };
        let weak = Rc::downgrade(&self.inner);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(inner) = weak.upgrade() else { return glib::ControlFlow::Break };
            let mut relevant = false;
            while let Ok(changed) = changes.try_recv() {
                relevant |= changed.key.as_deref().is_none_or(|key| load::affects_library(kind, key));
            }
            if relevant {
                submit(&inner);
            }
            glib::ControlFlow::Continue
        });
    }

    /// Reloads the library when its rows change in the database: an entry added or removed on its details page, a
    /// refresh, an episode watched (upstream collects the library flow). The live query runs on the tokio runtime the
    /// GTK thread has entered; without one (some tests) only the explicit reloads run.
    fn watch_rows(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let (db, kind) = {
            let st = self.inner.borrow();
            (st.deps.db.clone(), st.deps.kind)
        };
        let (changed_tx, changed_rx) = async_channel::unbounded::<()>();
        // the query pauses while the view is off screen (a progress save in the player touches its tables) and runs
        // once when it is shown again, which reloads only when the rows changed meanwhile
        let (shown_tx, shown_rx) = tokio::sync::watch::channel(true);
        // ponytail: the live query reads the library and the worker reads it again; hand its rows to the worker if
        // the double read ever shows on a large library
        async fn forward<T>(
            mut sub: kuro_db::subscribe::Subscription<T>,
            mut shown: tokio::sync::watch::Receiver<bool>,
            changed: async_channel::Sender<()>,
        ) {
            loop {
                tokio::select! {
                    got = sub.changed() => if got.is_none() || changed.send(()).await.is_err() { break },
                    seen = shown.changed() => match seen {
                        Ok(()) if *shown.borrow_and_update() => sub.resume(),
                        Ok(()) => sub.pause(),
                        // the view is gone
                        Err(_) => break,
                    },
                }
            }
        }
        runtime.spawn(forward(EntryRepo::new(db).subscribe_library(kind), shown_rx, changed_tx));
        let shown_tx = Rc::new(shown_tx);
        let shown_map = shown_tx.clone();
        self.root.connect_map(move |_| {
            shown_map.send_replace(true);
        });
        self.root.connect_unmap(move |_| {
            shown_tx.send_replace(false);
        });
        let weak = Rc::downgrade(&self.inner);
        glib::spawn_future_local(async move {
            while changed_rx.recv().await.is_ok() {
                let Some(inner) = weak.upgrade() else { break };
                submit(&inner);
            }
        });
    }

    /// The refresh button (T-26a-102): starts a pass of the whole library, says so in a toast, and stays busy
    /// while a pass of this kind runs. The F5 shortcut keeps updating the active category only.
    fn wire_refresh(&self, weak: &Weak<RefCell<Inner>>) {
        let button = self.inner.borrow().refresh_button.clone();
        sync_refresh_button(&self.inner);
        let weak_click = weak.clone();
        button.connect_clicked(move |_| {
            let Some(inner) = weak_click.upgrade() else { return };
            update_whole_library(&inner);
        });
        // a pass that ends, or one started elsewhere, changes the button without a click
        let weak_poll = weak.clone();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(inner) = weak_poll.upgrade() else { return glib::ControlFlow::Break };
            sync_refresh_button(&inner);
            glib::ControlFlow::Continue
        });
    }

    /// The selection header (count, select all, invert, cancel) and the action bar (change categories, mark,
    /// download, delete).
    fn wire_bar(&self, weak: &Weak<RefCell<Inner>>) {
        let kind = self.inner.borrow().deps.kind;
        let (read_label, unread_label) = match kind {
            Kind::Manga => (tr(MsgKey::action_mark_as_read), tr(MsgKey::action_mark_as_unread)),
            Kind::Anime => (tr(MsgKey::action_mark_as_seen), tr(MsgKey::action_mark_as_unseen)),
        };
        type Action = Box<dyn Fn(&Rc<RefCell<Inner>>)>;
        let connect = |button: &gtk::Button, action: Action| {
            let weak = weak.clone();
            button.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    action(&inner);
                }
            });
        };
        let (sel_header, download_button) = {
            let st = self.inner.borrow();
            (st.sel_header.clone(), st.download_button.clone())
        };
        let on_cancel = {
            let weak = weak.clone();
            move || {
                if let Some(inner) = weak.upgrade() {
                    select_with(&inner, |sel, _| sel.clear());
                }
            }
        };
        let on_select_all = {
            let weak = weak.clone();
            move || {
                if let Some(inner) = weak.upgrade() {
                    select_with(&inner, |sel, page| sel.select_all(page));
                }
            }
        };
        let header = crate::ui::selection_header(&self.inner.borrow().bar_label, on_cancel, on_select_all);
        let invert = gtk::Button::with_label(&tr(MsgKey::action_select_inverse));
        connect(&invert, Box::new(|inner: &Rc<RefCell<Inner>>| select_with(inner, |sel, page| sel.invert(page))));
        header.append(&invert);
        sel_header.append(&header);
        let bar_box = crate::ui::action_bar_row(&self.inner.borrow().bar);
        {
            let buttons: Vec<(String, Action)> = vec![
                (tr(MsgKey::action_move_category), Box::new(|inner: &Rc<RefCell<Inner>>| {
                    let ids = inner.borrow().selection.ids().to_vec();
                    open_categories(inner, ids, true);
                })),
                (read_label, Box::new(|inner: &Rc<RefCell<Inner>>| mark_selected(inner, true))),
                (unread_label, Box::new(|inner: &Rc<RefCell<Inner>>| mark_selected(inner, false))),
                (tr(MsgKey::action_delete), Box::new(|inner: &Rc<RefCell<Inner>>| selected_action(inner, |nav| nav.delete.clone()))),
            ];
            for (label, action) in buttons {
                let button = gtk::Button::with_label(&label);
                connect(&button, action);
                bar_box.append(&button);
            }
            // the Download menu: the next 1, 5, 10 or 25 items, or every unread one (T-37a-028), shared with the entry pages
            let weak_menu = weak.clone();
            let popover = crate::entry_core::download_menu::popover(kind, move |amount| {
                if let Some(inner) = weak_menu.upgrade() {
                    selected_download(&inner, amount);
                }
            });
            download_button.set_popover(Some(&popover));
            bar_box.append(&download_button);
            crate::ui::no_sticky_focus(&bar_box);
        }
        let weak_global = weak.clone();
        let global = self.inner.borrow().global_button.clone();
        global.connect_clicked(move |_| {
            let Some(inner) = weak_global.upgrade() else { return };
            let query = inner.borrow().query.clone();
            if let Some(query) = query {
                let open = inner.borrow().nav.global_search.clone();
                open(&query);
            }
        });
        // shown only once a repository exists (`render`); without one the welcome view leads the user
        let action = self.inner.borrow().empty_action.clone();
        action.connect_clicked(|_| crate::ui::open("browse"));
    }
}

/// Whether any manga or anime source repository is set up. Decides what the empty library offers.
fn repositories_exist() -> bool {
    if !crate::data::is_installed() {
        return false;
    }
    let sources = kuro_prefs::source::SourcePrefs::new(crate::data::app().base.store().clone());
    !sources.manga_extension_repos.get().is_empty() || !sources.anime_extension_repos.get().is_empty()
}

/// Turns the refresh arrow (and keeps the button off) while a pass of this kind runs.
fn sync_refresh_button(inner: &Rc<RefCell<Inner>>) {
    let (button, running) = {
        let st = inner.borrow();
        (st.refresh_button.clone(), st.nav.update_running.clone())
    };
    crate::ui::set_refreshing(&button, running());
}

/// Sends the current settings to the worker as a new generation.
fn submit(inner: &Rc<RefCell<Inner>>) {
    let mut st = inner.borrow_mut();
    let generation = st.gate.issue();
    let downloaded_only = (st.deps.downloaded_only)();
    let logged_in = (st.deps.logged_in)();
    let request = load::request(&st.deps.prefs, st.deps.kind, downloaded_only, &logged_in, st.query.clone());
    st.logged_in = logged_in;
    let job = Job { generation, db: st.deps.db.clone(), request, facts: st.deps.facts.clone() };
    if !st.worker.submit(job) {
        tracing::error!("the library worker has stopped");
    }
}

/// The toast when the library cannot be read; the error itself goes to the log.
fn load_failed() -> String {
    tr(MsgKey::library_load_failed)
}

fn apply_reply(inner: &Rc<RefCell<Inner>>, reply: Reply) {
    {
        let mut st = inner.borrow_mut();
        if !st.gate.is_current(reply.generation) {
            return;
        }
        match reply.result {
            Ok(built) => {
                // ids that left the library drop out of the selection; the counts follow the new rows (T-37a-059)
                st.selection.refresh(&built.library);
                st.built = Some(built);
            }
            Err(e) => {
                tracing::error!(error = %e, "loading the library failed");
                drop(st);
                crate::toast::notify_for(&load_failed(), crate::library::TOAST_TIME);
                return;
            }
        }
    }
    render(inner);
}

/// Index of the active tab: the remembered category when it is still shown, else the clamped stored index.
/// The line under the header: the category name with its count, or only the count when the name would repeat the
/// kind the header switch already shows ("Never a second Anime").
fn context_line(title: &str, count: Option<usize>, default_title: &str) -> String {
    match (title == default_title, count) {
        (true, Some(n)) => tr_plural(MsgKey::anime_num_entries, n as i64, &[]),
        (true, None) => String::new(),
        (false, Some(n)) => format!("{title} ({n})"),
        (false, None) => title.to_string(),
    }
}

/// Escape: clears the selection first, then the search. Returns whether it cleared anything.
fn escape(inner: &Rc<RefCell<Inner>>) -> bool {
    let (has_selection, search) = {
        let st = inner.borrow();
        (!st.selection.is_empty(), st.search.clone())
    };
    match keys::escape(has_selection, !search.text().is_empty()) {
        keys::Escape::ClearSelection => select_with(inner, |sel, _| sel.clear()),
        keys::Escape::ClearSearch => search.set_text(""),
        keys::Escape::Ignore => return false,
    }
    true
}

/// The category the library showed last (`last_used_*_category`, written by `select_tab`).
fn remembered_category(prefs: &LibraryPrefs, kind: Kind) -> i64 {
    i64::from(prefs.last_used_category(kind).get())
}

fn active_index(st: &Inner) -> usize {
    let pages = st.built.as_ref().map_or(&[][..], |b| b.pages.as_slice());
    if let Some(id) = st.active {
        if let Some(index) = pages.iter().position(|p| p.category.id == id) {
            return index;
        }
    }
    clamp_page(st.page as i64, pages.len())
}

/// Redraws the toolbar, tabs, grid, empty state and selection bar from the current state.
fn render(inner: &Rc<RefCell<Inner>>) {
    render_with(inner, false);
}

/// `render`; with `selection_only` the grid keeps its cards and only those whose selected state changed are rebound, so
/// starting or changing a selection does not reload every cover.
fn render_with(inner: &Rc<RefCell<Inner>>, selection_only: bool) {
    let weak = Rc::downgrade(inner);
    let view = {
        let st = inner.borrow();
        let pages = st.built.as_ref().map_or(&[][..], |b| b.pages.as_slice());
        let index = active_index(&st);
        let search_active = st.query.is_some();
        let tabs_on = layout::tabs_visible(diet::LIBRARY_CATEGORY_TABS, search_active);
        let show_tabs = tabs_on && pages.len() > 1;
        let default_label = tr(MsgKey::label_default);
        let default_title = match st.deps.kind {
            Kind::Anime => tr(MsgKey::label_anime_library),
            Kind::Manga => tr(MsgKey::label_manga_library),
        };
        let show_counts = diet::LIBRARY_ITEM_COUNT || search_active;
        let labels: Vec<String> = if show_tabs {
            pages
                .iter()
                .map(|p| layout::tab_label(p.category.display_name(&default_label), show_counts.then_some(p.items.len())))
                .collect()
        } else {
            Vec::new()
        };
        let title = match pages.get(index) {
            Some(page) => {
                let distinct = st.built.as_ref().map_or(0, |b| b.distinct_count);
                let (title, count) = toolbar_title(
                    tabs_on,
                    diet::LIBRARY_ITEM_COUNT,
                    &default_title,
                    page.category.display_name(&default_label),
                    page.items.len(),
                    distinct,
                );
                context_line(&title, count, &default_title)
            }
            None => String::new(),
        };
        let items: Vec<LibraryItem> = pages.get(index).map_or_else(Vec::new, |p| p.items.clone());
        let grid_cells = cells::cells(&items, st.query.as_deref());
        let global_label = cells::global_label(st.query.as_deref().unwrap_or_default());
        let (empty_text, show_global, empty_action) = if items.is_empty() && st.built.is_some() {
            let filters = st.built.as_ref().is_some_and(|b| b.has_active_filters);
            let library_empty = st.built.as_ref().is_none_or(|b| b.is_library_empty());
            let page = empty_page(search_active, filters, library_empty);
            let action = actions::empty_library_action(page, repositories_exist());
            let text = match action {
                Some(actions::EmptyLibraryAction::Browse) => tr(MsgKey::library_empty_search_hint),
                _ => page.text().to_owned(),
            };
            (text, shows_global_search(search_active, true), action)
        } else {
            (String::new(), false, None)
        };
        let hidden = st.selection.hidden_count(&items);
        let selected = st.selection.len();
        let bar_text = match (selected, hidden) {
            (0, _) => String::new(),
            (n, 0) => kuro_i18n::trf(kuro_i18n::MsgKey::library_selected_count, &[("count", kuro_i18n::Arg::Str(&n.to_string()))]),
            (n, h) => kuro_i18n::trf(kuro_i18n::MsgKey::library_selected_hidden, &[("count", kuro_i18n::Arg::Str(&n.to_string())), ("hidden", kuro_i18n::Arg::Str(&h.to_string()))]),
        };
        Rendered {
            title,
            labels,
            index,
            items,
            grid_cells,
            empty_text,
            show_global,
            empty_action,
            global_label,
            bar_text,
            bar_open: !st.selection.is_empty(),
            download_visible: actions::download_visible(st.selection.rows()),
        }
    };

    let (title_label, tabs, store, scroller, empty, empty_label, global, bar, bar_label, header, sel_header, download) = {
        let st = inner.borrow();
        (
            st.title.clone(),
            st.tabs.clone(),
            st.store.clone(),
            st.scroller.clone(),
            st.empty.clone(),
            st.empty_label.clone(),
            st.global_button.clone(),
            st.bar.clone(),
            st.bar_label.clone(),
            [st.search.clone().upcast::<gtk::Widget>(), st.refresh_button.clone().upcast(), st.options.widget.clone().upcast()],
            st.sel_header.clone(),
            st.download_button.clone(),
        )
    };
    title_label.set_label(&view.title);
    title_label.set_visible(!view.title.is_empty());
    let on_select: Rc<dyn Fn(usize)> = Rc::new(move |index| {
        if let Some(inner) = weak.upgrade() {
            select_tab(&inner, index);
        }
    });
    tabs.set(&view.labels, view.index, on_select);

    apply_columns(inner);
    show_mode_view(inner);
    let ids: Vec<Option<i64>> = view.grid_cells.iter().map(|cell| if let Cell::Item(item) = cell { Some(item.row.id) } else { None }).collect();
    let selected: std::collections::HashSet<i64> = inner.borrow().selection.ids().iter().copied().collect();
    let (same_cards, before) = {
        let st = inner.borrow();
        (selection_only && st.shown_ids == ids && store.n_items() as usize == ids.len(), st.shown_selected.clone())
    };
    if same_cards {
        for (position, id) in ids.iter().enumerate() {
            if id.is_some_and(|id| selected.contains(&id) != before.contains(&id)) {
                // a new object at the position, so the grid rebinds this card (the same object would keep its widget)
                store.splice(position as u32, 1, &[glib::BoxedAnyObject::new(view.grid_cells[position].clone())]);
            }
        }
    } else {
        let objects: Vec<glib::BoxedAnyObject> = view.grid_cells.iter().cloned().map(glib::BoxedAnyObject::new).collect();
        store.splice(0, store.n_items(), objects.as_slice());
    }
    {
        let mut st = inner.borrow_mut();
        st.shown_ids = ids;
        st.shown_selected = selected;
    }
    let has_items = !view.items.is_empty();
    scroller.set_visible(has_items);
    let welcome_shown = !has_items && view.empty_action == Some(actions::EmptyLibraryAction::AddRepository);
    empty.set_visible(!has_items && !welcome_shown);
    empty_label.set_label(&view.empty_text);
    global.set_visible(view.show_global);
    global.set_label(&view.global_label);
    let (action, welcome) = {
        let st = inner.borrow();
        (st.empty_action.clone(), st.welcome.clone())
    };
    action.set_visible(view.empty_action == Some(actions::EmptyLibraryAction::Browse));
    action.set_label(&tr(MsgKey::browse));
    welcome.widget().set_visible(welcome_shown);
    bar_label.set_label(&view.bar_text);
    bar.set_reveal_child(view.bar_open);
    header.iter().for_each(|w| w.set_visible(!view.bar_open));
    sel_header.set_visible(view.bar_open);
    download.set_visible(view.download_visible);
    options::Options::sync(inner);
}

/// Puts the list or the grid in the scroller for the display mode.
fn show_mode_view(inner: &Rc<RefCell<Inner>>) {
    let (scroller, list, grid, list_mode) = {
        let st = inner.borrow();
        (st.scroller.clone(), st.list.clone(), st.grid.clone(), is_list_mode(&st))
    };
    let wanted: gtk::Widget = if list_mode { list.upcast() } else { grid.upcast() };
    if scroller.child().as_ref() != Some(&wanted) {
        scroller.set_child(Some(&wanted));
    }
}

fn is_list_mode(st: &Inner) -> bool {
    st.deps.prefs.display_mode.get() == kuro_prefs::library::LibraryDisplayMode::List
}

/// The row height of List mode for the current page height and list size (T-37a-021).
fn list_row_height(st: &Inner) -> f64 {
    let size = layout::list_size(window_width(&st.scroller), diet::LIBRARY_COLUMNS, diet::LIBRARY_COLUMNS);
    layout::list_row_height(f64::from(st.scroller.height()), size)
}

/// Width of the window that holds `widget`, or of the widget itself before it has a window.
fn window_width(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.root().map_or_else(|| widget.width(), |root| root.width())
}

/// Sets the grid's column count from the width of the grid's area and the stored cover size (T-08a-043): as many columns
/// of that size as fit, at least two.
fn apply_columns(inner: &Rc<RefCell<Inner>>) {
    let (grid, columns) = {
        let st = inner.borrow();
        // the grid's own area, not the window (which includes the sidebar): a count taken from the window asked for
        // more width than the page has, which grew the window without end once the page stopped scrolling sideways
        let width = match st.scroller.width() {
            0 => window_width(&st.scroller),
            own => own,
        };
        (st.grid.clone(), width)
    };
    let cover_size = inner.borrow().deps.prefs.cover_size.get();
    crate::widgets::cover_grid::apply_columns(&grid, columns, cover_size);
}

/// Everything `render` draws, read in one borrow and applied without it.
struct Rendered {
    title: String,
    labels: Vec<String>,
    index: usize,
    items: Vec<LibraryItem>,
    grid_cells: Vec<Cell>,
    empty_text: String,
    show_global: bool,
    empty_action: Option<actions::EmptyLibraryAction>,
    global_label: String,
    bar_text: String,
    bar_open: bool,
    download_visible: bool,
}

/// Makes `index` the active tab and remembers its category.
fn select_tab(inner: &Rc<RefCell<Inner>>, index: usize) {
    let (id, prefs, kind) = {
        let mut st = inner.borrow_mut();
        st.page = index;
        let id = st.built.as_ref().and_then(|b| b.pages.get(index)).map(|p| p.category.id);
        st.active = id;
        (id, st.deps.prefs.clone(), st.deps.kind)
    };
    if let Some(id) = id {
        if let Err(e) = prefs.last_used_category(kind).set(id as i32) {
            tracing::warn!(error = %e, "remembering the library tab failed");
        }
    }
    render(inner);
}

/// Runs a selection change against the active page, then redraws. The page slice is read under the same borrow.
fn select_with(inner: &Rc<RefCell<Inner>>, change: impl FnOnce(&mut Selection, &[LibraryItem])) {
    {
        let mut guard = inner.borrow_mut();
        let st = &mut *guard;
        let index = active_index(st);
        let page: &[LibraryItem] =
            st.built.as_ref().and_then(|b| b.pages.get(index)).map_or(&[], |p| p.items.as_slice());
        change(&mut st.selection, page);
    }
    render_with(inner, true);
}

/// Queues the downloads of the selected entries with one amount of the Download menu, then clears the selection.
fn selected_download(inner: &Rc<RefCell<Inner>>, amount: actions::DownloadAmount) {
    let (ids, run) = {
        let st = inner.borrow();
        (st.selection.ids().to_vec(), st.nav.download.clone())
    };
    run(ids, amount);
    inner.borrow_mut().selection.clear();
    render(inner);
}

/// Hands the selected entry ids to a navigation hook (delete), then clears the selection.
fn selected_action(inner: &Rc<RefCell<Inner>>, hook: impl Fn(&Nav) -> Rc<dyn Fn(Vec<i64>)>) {
    let (ids, run) = {
        let st = inner.borrow();
        (st.selection.ids().to_vec(), hook(&st.nav))
    };
    run(ids);
    inner.borrow_mut().selection.clear();
    render(inner);
}

/// Marks every item of the entries `ids` read (or unread), reports a partial failure and reloads.
fn mark(inner: &Rc<RefCell<Inner>>, ids: &[i64], read: bool) {
    let db = inner.borrow().deps.db.clone();
    let ids = ids.to_vec();
    let weak = Rc::downgrade(inner);
    glib::spawn_future_local(async move {
        let backend = crate::updates::download_host::current();
        let ok = actions::mark_many(&db, backend, &ids, read, actions::remove_after_marked_read()).await;
        if let Some(text) = actions::partial_failure_toast(ok, ids.len()) {
            crate::toast::notify_for(&text, crate::library::TOAST_TIME);
        }
        if let Some(inner) = weak.upgrade() {
            submit(&inner);
        }
    });
}

/// Marks every selected entry and clears the selection once the writes are done (E-35).
fn mark_selected(inner: &Rc<RefCell<Inner>>, read: bool) {
    let ids = inner.borrow().selection.ids().to_vec();
    mark(inner, &ids, read);
    inner.borrow_mut().selection.clear();
    render(inner);
}

/// The categories dialog for `entries`. `clear_selection`: the entries are the selection, which ends on OK.
fn open_categories(inner: &Rc<RefCell<Inner>>, entries: Vec<i64>, clear_selection: bool) {
    let (svc, edit) = {
        let st = inner.borrow();
        (st.deps.categories.clone(), st.nav.edit_categories.clone())
    };
    let categories = match svc.repo().visible() {
        Ok(categories) => categories,
        Err(e) => {
            tracing::error!(error = %e, "reading the categories failed");
            return;
        }
    };
    let memberships: Vec<Vec<i64>> = entries
        .iter()
        .map(|id| match svc.repo().by_entry(*id, true) {
            Ok(list) => list.iter().map(|c| c.id).collect(),
            Err(e) => {
                tracing::error!(error = %e, entry = id, "reading the entry categories failed");
                Vec::new()
            }
        })
        .collect();
    let dialog = CategoryDialog::new(entries.clone(), &categories, &memberships);
    let weak = Rc::downgrade(inner);
    let on_confirm = move |include: Vec<i64>, exclude: Vec<i64>| {
        let Some(inner) = weak.upgrade() else { return };
        if clear_selection {
            inner.borrow_mut().selection.clear();
            render(&inner);
        }
        let svc = svc.clone();
        let entries = entries.clone();
        let weak = Rc::downgrade(&inner);
        glib::spawn_future_local(async move {
            match actions::set_membership(&svc, &entries, &include, &exclude).await {
                Ok(0) => {}
                Ok(failed) => tracing::error!(failed, "some entries kept their categories"),
                Err(e) => tracing::error!(error = %e, "saving categories failed"),
            }
            if let Some(inner) = weak.upgrade() {
                submit(&inner);
            }
        });
    };
    crate::category_picker::open(
        dialog,
        move |d| {
            let (include, exclude) = d.confirm();
            on_confirm(include, exclude);
        },
        move || edit(),
        || {},
    );
}

/// Builds one cell of the grid: the global search row, or the card of an entry. On a card, a primary click opens,
/// toggles or selects a range (`keys::press`); a right click (a long press on a touch screen) opens its menu.
fn bind_item(weak: &Weak<RefCell<Inner>>, list_item: &gtk::ListItem) {
    let Some(inner) = weak.upgrade() else { return };
    let Some(boxed) = list_item.item().and_then(|o| o.downcast::<glib::BoxedAnyObject>().ok()) else { return };
    let cell: Cell = boxed.borrow::<Cell>().clone();
    let item = match cell {
        Cell::Global(query) => {
            let button = gtk::Button::with_label(&cells::global_label(&query));
            button.add_css_class("library-global-search");
            let weak_global = weak.clone();
            button.connect_clicked(move |_| {
                let Some(inner) = weak_global.upgrade() else { return };
                let open = inner.borrow().nav.global_search.clone();
                open(&query);
            });
            list_item.set_child(Some(&button));
            return;
        }
        Cell::Item(item) => item,
    };
    let (mode, selected, show_continue_pref) = {
        let st = inner.borrow();
        (
            st.deps.prefs.display_mode.get(),
            st.selection.contains(item.row.id),
            diet::LIBRARY_CONTINUE_BUTTON,
        )
    };
    let show_continue = actions::continue_visible(show_continue_pref, &item.row);
    let art = cover_art(&inner.borrow(), &item);
    let (kind, db) = {
        let st = inner.borrow();
        (st.deps.kind, st.deps.db.clone())
    };
    // a local anime without a cover gets one from its folder or a video frame, once; the cards then reload
    if kind == Kind::Anime && item.row.is_local() && art.as_ref().is_none_or(|a| a.url.is_none()) {
        let weak_cover = weak.clone();
        crate::library::frame_cover::ensure(db, item.row.id, move || {
            if let Some(inner) = weak_cover.upgrade() {
                submit(&inner);
            }
        });
    }
    let extras = card::Extras {
        art: art.as_ref(),
        kind,
        season_parent: is_season_parent(&inner.borrow(), &item),
        width: kuro_domain::library::columns::column_width(inner.borrow().deps.prefs.cover_size.get()),
    };
    let card = if mode == kuro_prefs::library::LibraryDisplayMode::List {
        let row_height = list_row_height(&inner.borrow());
        card::build_list(&item, selected, show_continue, row_height, &extras)
    } else {
        card::build(&item, mode, selected, show_continue, &extras)
    };
    list_item.set_child(Some(&card.root));

    // the action runs on release; a long press (mouse or touch) selects instead, as on the episode and chapter rows, and the
    // release that ends it does not also open the entry
    let long_pressed = Rc::new(std::cell::Cell::new(false));
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let row: LibraryRow = item.row.clone();
    click.connect_pressed({
        let long_pressed = Rc::clone(&long_pressed);
        move |_, _, _, _| long_pressed.set(false)
    });
    let (weak_click, click_row, click_long) = (weak.clone(), row.clone(), Rc::clone(&long_pressed));
    click.connect_released(move |gesture, _, _, _| {
        if click_long.get() {
            return;
        }
        let Some(inner) = weak_click.upgrade() else { return };
        let modifiers = gesture.current_event_state();
        let selecting = !inner.borrow().selection.is_empty();
        let action = keys::press(
            modifiers.contains(gdk::ModifierType::SHIFT_MASK),
            modifiers.contains(gdk::ModifierType::CONTROL_MASK),
            selecting,
        );
        match action {
            keys::Press::Range => select_with(&inner, |sel, page| sel.toggle_range(&click_row, page)),
            keys::Press::Toggle => select_with(&inner, |sel, _| sel.toggle(&click_row)),
            keys::Press::Open => {
                let open = inner.borrow().nav.open.clone();
                open(click_row.id);
            }
        }
    });
    card.root.add_controller(click);
    let long = gtk::GestureLongPress::new();
    let (weak_long, long_row) = (weak.clone(), row.clone());
    long.connect_pressed(move |gesture, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        long_pressed.set(true);
        // on idle, so no borrow of the view is held while the selection redraws the grid
        let (weak, row) = (weak_long.clone(), long_row.clone());
        glib::idle_add_local_once(move || {
            if let Some(inner) = weak.upgrade() {
                select_with(&inner, |sel, _| sel.toggle(&row));
            }
        });
    });
    card.root.add_controller(long);
    // the right click (and the Menu key) open the cover's menu; the long press selects
    let weak_menu = weak.clone();
    context_menu::attach_without_long_press(&card.root, move |_, _| cover_menu(&weak_menu, &row));

    if let Some(button) = card.continue_button {
        let weak_resume = weak.clone();
        let resume_row = item.row.clone();
        let id = resume_row.id;
        button.connect_clicked(move |_| {
            let Some(inner) = weak_resume.upgrade() else { return };
            let allowed = {
                let mut st = inner.borrow_mut();
                let now = Instant::now();
                let allowed = actions::continue_click_allowed(st.last_resume.get(&id).copied(), now);
                if allowed {
                    st.last_resume.insert(id, now);
                }
                allowed
            };
            if allowed {
                resume_entry(&inner, &resume_row);
            }
        });
    }
}

/// Whether the item is an anime season parent: its unread badge counts seasons (T-37a-048).
fn is_season_parent(st: &Inner, item: &LibraryItem) -> bool {
    EntryRepo::new(st.deps.db.clone()).by_id(item.row.id).is_ok_and(|e| e.fetch_type() == FetchType::Seasons)
}

/// The stored cover of an entry, local or not; the card decides what it can draw (`layout::cover_kind`).
/// ponytail: one row read per bind; carry the thumbnail in `LibraryRow` if large libraries scroll slowly.
fn cover_art(st: &Inner, item: &LibraryItem) -> Option<ArtRef> {
    let db = st.deps.db.clone();
    let e = EntryRepo::new(db).by_id(item.row.id).ok()?;
    Some(crate::browse::look::cover_art(e.kind, item.row.id, e.source, e.thumbnail_url.filter(|u| !u.is_empty()), e.favorite, e.cover_last_modified))
}

/// The right-click menu of one cover (`actions::cover_menu`). Each item acts on this entry only and leaves any
/// selection as it is; the items run on idle after the menu closed, so no borrow of the view is held.
fn cover_menu(weak: &Weak<RefCell<Inner>>, row: &LibraryRow) -> Vec<Item> {
    let Some(inner) = weak.upgrade() else { return Vec::new() };
    let kind = inner.borrow().deps.kind;
    let id = row.id;
    actions::cover_menu(kind, row.is_local())
        .into_iter()
        .map(|(action, label, section)| {
            let weak = weak.clone();
            Item::new(label, section, move || {
                let Some(inner) = weak.upgrade() else { return };
                match action {
                    actions::CoverAction::Open => {
                        let open = inner.borrow().nav.open.clone();
                        open(id);
                    }
                    actions::CoverAction::MarkAllViewed => mark(&inner, &[id], true),
                    actions::CoverAction::DownloadUnviewed => {
                        let download = inner.borrow().nav.download.clone();
                        download(vec![id], actions::DownloadAmount::AllUnread);
                    }
                    actions::CoverAction::ChangeCategories => open_categories(&inner, vec![id], false),
                    // the remove dialog is the confirmation
                    actions::CoverAction::Remove => {
                        let delete = inner.borrow().nav.delete.clone();
                        delete(vec![id]);
                    }
                }
            })
        })
        .collect()
}

/// The continue button: opens the entry's next item, in the external player when the preference is on, or says
/// that nothing is left (E-25).
fn resume_entry(inner: &Rc<RefCell<Inner>>, row: &LibraryRow) {
    let (db, downloaded_only, item_downloaded, always_external, resume) = {
        let st = inner.borrow();
        (
            st.deps.db.clone(),
            (st.deps.downloaded_only)(),
            st.deps.item_downloaded.clone(),
            st.deps.always_external.clone(),
            st.nav.resume.clone(),
        )
    };
    match actions::next_item(&db, row, downloaded_only, |item| item_downloaded(item)) {
        Ok(Some(item)) => resume(Resume { entry: row.id, item, external: always_external() }),
        Ok(None) => crate::toast::notify_for(&actions::no_next_item(), crate::library::TOAST_TIME),
        Err(e) => {
            tracing::error!(error = %e, entry = row.id, "finding the next item failed");
            crate::toast::notify_for(&actions::no_next_item(), crate::library::TOAST_TIME);
        }
    }
}

/// A manga library view over a temporary database holding one favorite, "Berserk", for tests.
#[cfg(test)]
pub(crate) fn test_view(nav: Nav) -> (tempfile::TempDir, LibraryView) {
    test_view_of(Kind::Manga, nav)
}

/// A library view of `kind` over a temporary database holding one favorite, "Berserk", for tests.
#[cfg(test)]
fn test_view_of(kind: Kind, nav: Nav) -> (tempfile::TempDir, LibraryView) {
    use crate::library::query::SourceInfo;
    use kuro_data::entry::Entry;
    use kuro_prefs::store::PrefStore;
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("a.db");
    let db = Arc::new(Db::new(&path, kuro_db::schema_unified::open_file(&path).expect("open the db")).expect("db handle"));
    EntryRepo::new(db.clone())
        .insert(&Entry { favorite: true, source: 7, url: "u".into(), title: "Berserk".into(), ..Entry::create(kind) })
        .expect("insert");
    let store = PrefStore::in_memory();
    let prefs = Rc::new(LibraryPrefs::new(store.clone()));
    let categories = Rc::new(CategoryService::new(db.clone(), kind, Arc::new(LibraryPrefs::new(store))));
    let deps = Deps {
        kind,
        db,
        prefs,
        categories,
        facts: Facts {
            downloads: Arc::new(|_| 0),
            source: Arc::new(|_| SourceInfo { name: "Local".into(), language: "en".into() }),
        },
        downloaded_only: Rc::new(|| false),
        logged_in: Rc::new(BTreeSet::new),
        always_external: Rc::new(|| false),
        item_downloaded: Rc::new(|_| false),
    };
    let view = LibraryView::new(deps, nav).expect("view");
    (dir, view)
}

/// Navigation that does nothing, for tests.
#[cfg(test)]
pub(crate) fn test_nav() -> Nav {
    Nav {
        open: Rc::new(|_| {}),
        resume: Rc::new(|_| {}),
        download: Rc::new(|_, _| {}),
        delete: Rc::new(|_| {}),
        global_search: Rc::new(|_| {}),
        edit_categories: Rc::new(|| {}),
        update: Rc::new(|_| true),
        update_running: Rc::new(|| false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Pumps the default main context until `done` holds or two seconds pass.
    fn pump_until(done: impl Fn() -> bool) -> bool {
        let ctx = glib::MainContext::default();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    #[test]
    fn t_37a_017_the_library_reopens_on_the_category_it_was_left_on() {
        let prefs = LibraryPrefs::new(kuro_prefs::store::PrefStore::in_memory());
        assert_eq!(remembered_category(&prefs, Kind::Manga), 0, "the Default category before any tab was picked");
        prefs.last_used_manga_category.set(7).expect("write");
        prefs.last_used_anime_category.set(9).expect("write");
        assert_eq!(remembered_category(&prefs, Kind::Manga), 7);
        assert_eq!(remembered_category(&prefs, Kind::Anime), 9);
    }

    #[test]
    fn the_view_loads_the_worker_reply_and_renders_selection_and_search() {
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let (_dir, view) = test_view(test_nav());
            let inner = view.inner.clone();
            assert!(pump_until(|| inner.borrow().built.is_some()), "the worker reply arrives");
            assert_eq!(inner.borrow().store.n_items(), 1, "one cover for the one entry");
            assert!(!inner.borrow().title.is_visible(), "B-7.3: the default title is the header switch's word, so the context line hides");

            let row = inner.borrow().built.as_ref().expect("built").pages[0].items[0].row.clone();
            select_with(&inner, |sel, _| sel.toggle(&row));
            assert!(inner.borrow().bar.reveals_child(), "the selection bar opens");
            assert_eq!(inner.borrow().bar_label.label().as_str(), "1 selected");
            assert!(inner.borrow().sel_header.is_visible() && !inner.borrow().search.is_visible() && !inner.borrow().refresh_button.is_visible(), "the selection row replaces the header controls");
            assert!(inner.borrow().download_button.is_visible(), "no local entry is selected");
            select_with(&inner, |sel, _| sel.clear());
            assert!(!inner.borrow().bar.reveals_child());
            assert!(inner.borrow().search.is_visible() && inner.borrow().refresh_button.is_visible() && !inner.borrow().sel_header.is_visible());

            // B-1.3: a query from outside fills the box, and B-6.4: the global row is the first cell while it matches
            view.set_query("Berserk");
            assert_eq!(inner.borrow().search.text().as_str(), "Berserk");
            assert!(pump_until(|| inner.borrow().store.n_items() == 2), "the global row plus the match");
            let first = inner.borrow().store.item(0).and_downcast::<glib::BoxedAnyObject>().expect("first cell");
            assert!(matches!(&*first.borrow::<Cell>(), Cell::Global(q) if q == "Berserk"));

            view.set_query("zzz");
            let search_done = pump_until(|| inner.borrow().built.as_ref().is_some_and(|b| b.distinct_count == 0));
            assert!(search_done, "a search with no match empties the page");
            assert_eq!(inner.borrow().store.n_items(), 0);
            assert_eq!(inner.borrow().empty_label.label().as_str(), "No results found");

            // Escape typed in the header's search entry arrives as stop-search and clears the query
            view.controls().0.emit_by_name::<()>("stop-search", &[]);

            // the view options write at once and the dot follows the filter
            let options = inner.borrow().options.clone();
            options.group().activate_action("unseen", None);
            assert_eq!(inner.borrow().deps.prefs.filter_unread.get(), kuro_prefs::tristate::TriState::ENABLED_IS);
            assert!(pump_until(|| options.dot_visible()), "the dot shows while a Show only filter is on");
            options.group().activate_action("unseen", None);
            assert_eq!(inner.borrow().deps.prefs.filter_unread.get(), kuro_prefs::tristate::TriState::DISABLED);
            assert!(pump_until(|| !options.dot_visible()), "and hides once it is off");
            options.group().activate_action("mode", Some(&"list".to_variant()));
            assert_eq!(inner.borrow().deps.prefs.display_mode.get(), kuro_prefs::library::LibraryDisplayMode::List);
            assert!(pump_until(|| is_list_mode(&inner.borrow()) && inner.borrow().scroller.child() == Some(inner.borrow().list.clone().upcast::<gtk::Widget>())), "List mode shows the list");
            assert_eq!(inner.borrow().search.text().as_str(), "");
            assert!(pump_until(|| inner.borrow().query.is_none()), "clearing the box clears the query");
        });
    }

    /// An entry added from its details page (a write the library did not make) shows up without a manual refresh.
    #[test]
    fn an_entry_added_elsewhere_appears_without_a_refresh() {
        use kuro_data::entry::{Entry, EntryUpdate};
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let rt = kuro_core::runtime::build().expect("runtime");
            let _enter = rt.enter();
            let (_dir, view) = test_view(test_nav());
            let inner = view.inner.clone();
            assert!(pump_until(|| inner.borrow().store.n_items() == 1), "the one favorite");
            let repo = EntryRepo::new(inner.borrow().deps.db.clone());
            let browsed = repo.insert(&Entry { source: 7, url: "b".into(), title: "Vagabond".into(), ..Entry::create(Kind::Manga) }).expect("insert");
            repo.update(&EntryUpdate { favorite: Some(true), ..EntryUpdate::new(browsed) }).expect("favorite");
            assert!(pump_until(|| inner.borrow().store.n_items() == 2), "the added entry appears");
        });
    }

    /// The reported bug was on anime: an anime added on its details page shows up in the Library on its own. While
    /// the Library is off screen its live query pauses, and showing it again brings the rows up to date.
    #[test]
    fn an_anime_added_elsewhere_appears_and_the_hidden_library_waits_until_shown() {
        use kuro_data::entry::{Entry, EntryUpdate};
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let rt = kuro_core::runtime::build().expect("runtime");
            let _enter = rt.enter();
            let (_dir, view) = test_view_of(Kind::Anime, test_nav());
            let window = gtk::Window::new();
            window.set_child(Some(&view.widget()));
            window.present();
            let inner = view.inner.clone();
            assert!(pump_until(|| view.root.is_mapped() && inner.borrow().store.n_items() == 1), "the one favorite");
            let repo = EntryRepo::new(inner.borrow().deps.db.clone());
            let add = |url: &str| {
                let id = repo.insert(&Entry { source: 7, url: url.into(), title: url.into(), ..Entry::create(Kind::Anime) }).expect("insert");
                repo.update(&EntryUpdate { favorite: Some(true), ..EntryUpdate::new(id) }).expect("favorite");
            };
            add("Bungou Stray Dogs");
            assert!(pump_until(|| inner.borrow().store.n_items() == 2), "the added anime appears");

            window.set_visible(false);
            assert!(pump_until(|| !view.root.is_mapped()), "the library is off screen");
            add("Vagabond");
            assert!(!pump_until(|| inner.borrow().store.n_items() == 3), "a hidden library does not reload");
            window.present();
            assert!(pump_until(|| inner.borrow().store.n_items() == 3), "showing it again brings the new anime");
            window.destroy();
        });
    }

    #[test]
    fn the_context_line_never_repeats_the_kind_word() {
        assert_eq!(context_line("Manga", Some(12), "Manga"), "12 entries");
        assert_eq!(context_line("Manga", None, "Manga"), "");
        assert_eq!(context_line("Default", Some(4), "Manga"), "Default (4)");
        assert_eq!(context_line("Default", None, "Manga"), "Default");
    }
}
