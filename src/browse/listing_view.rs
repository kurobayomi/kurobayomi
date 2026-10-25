//! The content area of one source listing: the widget that draws `ContentPlan` and pages the grid or list through a
//! `gio::ListStore` (T-40a-003, T-40a-004, T-40b-003, T-40b-004). The rules are in `listing` and `content`; this file
//! only maps their output to widgets and runs the loaders on the runtime.
//!
//! Documented deviation from E-BR-09 (a row subscribes to its library row only while it is bound, and unbinding cancels the
//! subscription): there are no per-row subscriptions. One watcher follows the urls of the source's library entries
//! (`favorite_urls`) and `apply_favorite_urls` pushes the flag into every loaded item of the store, which rebinds the rows
//! that show them. The effect the edge case asks for is the same: a bound row always shows the library's flag, a row that
//! is off screen holds nothing that outlives it (it is a `ListStore` item, not a live subscription), and the watcher is
//! one per page and ends with it. Appending a page extends the store in place (`extend_from_slice`), so the rows already
//! on screen are not rebuilt. The 2,000-item scroll check is manual (`p2-edge-cases-and-tests.md` section 7.3).

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use kuro_domain::source::browse::BrowseQuery;
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_prefs::library::LibraryDisplayMode;
use kuro_source::filter_list::FilterList;
use tokio::sync::watch;

use super::content::ContentPlan;
use super::glue::{consume_events, follow_changes, follow_ready, keep_alive, own_tasks};
use super::grid::{CONTAINER_MARGIN_PX, CoverBinds, last_visible_index};
use super::listing::{ListItem, ListedPage, Listing, PageLoader};
use super::prefs::BrowsePrefs;
use super::primitives::{EmptyAction, ToastAction, ToastDuration, ToastSpec, stub_source_label};
use super::tasks::{PageTasks, Tagged};
use super::view::{CoverGuard, CoverLoader, empty_state, grid_cell, list_cell};
use crate::widgets::context_menu::{self, Item};

/// What the listing needs from the app.
pub struct ListingDeps {
    pub loader: Arc<dyn PageLoader>,
    pub prefs: Rc<BrowsePrefs>,
    /// `SourceRegistry::subscribe_initialized()`.
    pub ready: watch::Receiver<bool>,
    /// The source is not installed.
    pub stub: bool,
    pub is_local: bool,
    /// The source's id, name and language; a stub page names the source from them (`stub_source_label`).
    pub source_id: i64,
    pub source_name: String,
    pub source_lang: String,
    pub load_cover: CoverLoader,
    /// A cover was activated.
    pub open_entry: Rc<dyn Fn(&ListItem)>,
    /// The right-click menu of a cover.
    pub menu: Rc<dyn Fn(&ListItem) -> Vec<Item>>,
    /// The urls of this source's library entries, published again on every change: covers observe their favourite flag
    /// live (T-40b-033).
    pub favorite_urls: Option<watch::Receiver<BTreeSet<String>>>,
    /// An error-state button other than retry was pressed.
    pub on_action: Rc<dyn Fn(EmptyAction)>,
}

/// What a loader reports. The page tag is the pager's generation of the request.
struct Loaded {
    ask_generation: u64,
    /// The page number asked; the first page of a listing is saved for the next visit.
    page: u32,
    result: Result<ListedPage, String>,
}

struct Inner {
    deps: ListingDeps,
    listing: RefCell<Listing>,
    tasks: Rc<RefCell<PageTasks<Loaded>>>,
    ready: Cell<bool>,
    display: Cell<LibraryDisplayMode>,
    toast_dismissed: Cell<bool>,
    /// Whether a toast was raised (the toast itself lives in the shared layer).
    toast_shown: Cell<bool>,
    stack: gtk::Stack,
    error_slot: gtk::Box,
    /// The loading row under the items while the next page loads (the paging spinner).
    footer: gtk::Box,
    store: gtk::gio::ListStore,
    grid: gtk::GridView,
    list: gtk::ListView,
    scroller: gtk::ScrolledWindow,
}

/// The listing widget. Cloning shares the same listing.
#[derive(Clone)]
pub struct ListingView {
    inner: Rc<Inner>,
    root: gtk::Overlay,
}

/// A listing that does not keep itself alive, for callbacks that the listing's own deps reach.
#[derive(Clone)]
pub struct WeakListingView {
    inner: Weak<Inner>,
    root: glib::WeakRef<gtk::Overlay>,
}

impl WeakListingView {
    pub fn upgrade(&self) -> Option<ListingView> {
        Some(ListingView { inner: self.inner.upgrade()?, root: self.root.upgrade()? })
    }
}

impl ListingView {
    #[cfg(test)]
    pub fn new(deps: ListingDeps, query: BrowseQuery) -> Self {
        Self::with_filters(deps, query, FilterList::default())
    }

    /// A listing that starts with `query` and the filters its search runs with (a browse page opened on a search).
    pub fn with_filters(deps: ListingDeps, query: BrowseQuery, filters: FilterList) -> Self {
        let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::NoSelection::new(Some(store.clone()));
        let grid = gtk::GridView::new(Some(selection.clone()), None::<gtk::ListItemFactory>);
        grid.set_single_click_activate(true);
        for set in [gtk::GridView::set_margin_start, gtk::GridView::set_margin_end, gtk::GridView::set_margin_top, gtk::GridView::set_margin_bottom] {
            set(&grid, CONTAINER_MARGIN_PX);
        }
        let list = gtk::ListView::new(Some(selection), None::<gtk::ListItemFactory>);
        list.set_single_click_activate(true);

        // the grid or the list is the scroller's own child (never inside a Stack): a scrollable that is wrapped in a viewport
        // gets its whole natural height, builds every row at once, and GTK 4.14 then spreads rows over that height
        let scroller = gtk::ScrolledWindow::builder().child(&grid).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        let error_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // a source the user turned off is not missing: it says so and offers to turn it on, then opens its listing again
        let stub = if crate::source_switch::is_off(deps.source_id) {
            let turn_on = gtk::Button::with_label(&tr(MsgKey::source_switch_turn_on));
            turn_on.add_css_class("suggested-action");
            let source_id = deps.source_id;
            turn_on.connect_clicked(move |button| {
                button.set_sensitive(false);
                let button = button.downgrade();
                crate::source_switch::turn_on_then(crate::source_switch::key_of(source_id).unwrap_or_default(), move |worked| {
                    if let Some(button) = button.upgrade() {
                        button.set_sensitive(!worked);
                    }
                    if worked {
                        reopen_listing(source_id);
                    }
                });
            });
            super::look::empty_state("dialog-information-symbolic", &tr(MsgKey::source_switch_off), std::slice::from_ref(&turn_on)).0
        } else {
            super::look::empty_state(
                "dialog-warning-symbolic",
                &trf(MsgKey::source_not_installed, &[("arg1", Arg::Str(&stub_source_label(&deps.source_name, &deps.source_lang, deps.source_id)))]),
                &[],
            )
            .0
        };
        let empty = super::look::empty_state("view-list-symbolic", &tr(MsgKey::empty_screen), &[]).0;
        let stack = gtk::Stack::new();
        stack.add_named(&crate::ui::loading(crate::ui::LoadSize::Page), Some("spinner"));
        stack.add_named(&stub, Some("stub"));
        stack.add_named(&error_slot, Some("error"));
        stack.add_named(&empty, Some("empty"));
        // the items, and under them a loading row while a later page loads
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let more = crate::ui::loading(crate::ui::LoadSize::Inline);
        more.set_halign(gtk::Align::Center);
        more.set_margin_top(16);
        more.set_margin_bottom(16);
        footer.append(&more);
        footer.set_visible(false);
        let items_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        items_page.append(&scroller);
        items_page.append(&footer);
        stack.add_named(&items_page, Some("items"));

        let root = gtk::Overlay::new();
        root.set_child(Some(&stack));

        let display = deps.prefs.sources.display_mode.get();
        let ready = *deps.ready.borrow();
        let inner = Rc::new(Inner {
            ready: Cell::new(ready),
            display: Cell::new(display),
            listing: RefCell::new({
                let mut listing = Listing::new(query.clone());
                listing.restart_with(query, filters);
                listing
            }),
            tasks: Rc::new(RefCell::new(PageTasks::new())),
            toast_dismissed: Cell::new(false),
            toast_shown: Cell::new(false),
            deps,
            stack,
            error_slot,
            footer,
            store,
            grid,
            list,
            scroller,
        });
        let view = Self { inner, root };
        view.wire();
        view
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.root
    }

    pub fn downgrade(&self) -> WeakListingView {
        WeakListingView { inner: Rc::downgrade(&self.inner), root: self.root.downgrade() }
    }

    #[cfg(test)]
    /// Starts a new listing with the filters it had: a Popular or Latest switch or a refresh (B-EC-08). Loaders of
    /// the old one are aborted and their late pages dropped.
    pub fn restart(&self, query: BrowseQuery) {
        let filters = self.inner.listing.borrow().filters().clone();
        self.inner.restart(query, filters);
    }

    /// `search(query, filters)` (B§4.6.1): a new listing that searches with its own copy of `filters`.
    pub fn restart_with(&self, query: BrowseQuery, filters: FilterList) {
        self.inner.restart(query, filters);
    }

    /// `browse_hide_in_library_items`, read by the page when it was made (B§1.1, T-40b-034): favourites of the pages that
    /// arrive after this call are not kept.
    pub fn set_hide_in_library(&self, hide: bool) {
        self.inner.listing.borrow_mut().set_hide_in_library(hide);
    }

    #[cfg(test)]
    /// How many covers the listing holds.
    pub fn loaded_count(&self) -> u32 {
        self.inner.store.n_items()
    }

    /// A look was picked in the view options (and already stored).
    pub fn set_display_mode(&self, mode: LibraryDisplayMode) {
        self.inner.display.set(mode);
        self.inner.apply_display();
        self.inner.render();
    }

    pub fn query(&self) -> BrowseQuery {
        self.inner.listing.borrow().query().clone()
    }

    /// The listing's filters, as its running search holds them.
    pub fn filters(&self) -> FilterList {
        self.inner.listing.borrow().filters().clone()
    }

    /// Shows the favourite flag of the entry with library id `id` (an optimistic flip or its undo).
    pub fn set_favorite(&self, id: i64, favorite: bool) {
        self.inner.set_favorite_where(|item| item.id == id, favorite);
    }

    /// Makes the list fill the window below the top bar (B§2.8): the content never gets shorter than this.
    pub fn set_min_content_height(&self, px: i32) {
        self.inner.scroller.set_min_content_height(if self.inner.display.get() == LibraryDisplayMode::List { px } else { 0 });
    }

    fn wire(&self) {
        let inner = &self.inner;
        // a destroyed page aborts its loaders; the closure also keeps the state alive for as long as the widget lives
        own_tasks(&self.root, &inner.tasks);
        keep_alive(&self.root, inner);

        let weak = Rc::downgrade(inner);
        consume_events(&inner.tasks, move |loaded| {
            if let Some(inner) = weak.upgrade() {
                inner.arrived(loaded);
            }
        });
        let weak = Rc::downgrade(inner);
        follow_ready(inner.deps.ready.clone(), move |ready| {
            let Some(inner) = weak.upgrade() else { return false };
            inner.ready.set(ready);
            inner.render();
            inner.pump();
            true
        });
        if let Some(urls) = inner.deps.favorite_urls.clone() {
            let weak = Rc::downgrade(inner);
            follow_changes(urls, move |urls| match weak.upgrade() {
                Some(inner) => {
                    inner.apply_favorite_urls(&urls);
                    true
                }
                None => false,
            });
        }

        let weak = Rc::downgrade(inner);
        let scrolled = move |_: &gtk::Adjustment| {
            if let Some(inner) = weak.upgrade() {
                inner.pump_if_near_end();
            }
        };
        let adjustment = inner.scroller.vadjustment();
        adjustment.connect_value_changed(scrolled.clone());
        adjustment.connect_upper_notify(scrolled);
        // the grid's columns follow the size of the window
        for adjustment in [inner.scroller.hadjustment(), inner.scroller.vadjustment()] {
            let weak = Rc::downgrade(inner);
            adjustment.connect_page_size_notify(move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.apply_columns();
                }
            });
        }

        let weak = Rc::downgrade(inner);
        let open = move |position: u32| {
            let Some(inner) = weak.upgrade() else { return };
            if let Some(item) = inner.item_at(position) {
                (inner.deps.open_entry)(&item);
            }
        };
        inner.grid.connect_activate({
            let open = open.clone();
            move |_, position| open(position)
        });
        inner.list.connect_activate(move |_, position| open(position));

        // the cells are drawn at the stored cover size, now and again whenever the setting changes
        let weak = Rc::downgrade(inner);
        crate::bind::bind(&inner.grid, &inner.deps.prefs.cover_size, move |_, _| {
            if let Some(inner) = weak.upgrade() {
                inner.apply_display();
                inner.apply_columns();
            }
        });
        inner.prefill();
        inner.render();
        inner.pump();
    }
}

impl Inner {
    fn restart(self: &Rc<Self>, query: BrowseQuery, filters: FilterList) {
        self.tasks.borrow_mut().restart();
        self.listing.borrow_mut().restart_with(query, filters);
        self.store.remove_all();
        self.toast_dismissed.set(false);
        self.prefill();
        self.render();
        self.pump();
    }

    /// The saved results of the last visit, shown while the site is asked again (`results_cache`). A stub or a local
    /// source has none.
    fn prefill(&self) {
        if self.deps.stub || self.deps.is_local {
            return;
        }
        let saved = {
            let listing = self.listing.borrow();
            super::results_cache::read(self.deps.source_id, listing.query(), listing.filters())
        };
        let Some(saved) = saved else { return };
        self.listing.borrow_mut().prefill(saved);
        let slots: Vec<glib::BoxedAnyObject> = self.listing.borrow().slots().iter().map(|slot| glib::BoxedAnyObject::new(slot.clone())).collect();
        self.store.extend_from_slice(&slots);
    }

    /// The loaded item at a store position.
    fn item_at(&self, position: u32) -> Option<ListItem> {
        let object = self.store.item(position).and_downcast::<glib::BoxedAnyObject>()?;
        let item = object.borrow::<Option<ListItem>>().clone();
        item
    }

    /// Sets the favourite flag of every loaded item that `matches`, in the listing and in the store.
    fn set_favorite_where(&self, matches: impl Fn(&ListItem) -> bool, favorite: bool) {
        for index in 0..self.store.n_items() {
            let Some(item) = self.item_at(index) else { continue };
            if item.favorite == favorite || !matches(&item) {
                continue;
            }
            if self.listing.borrow_mut().set_favorite(index as usize, favorite) {
                let updated = self.listing.borrow().slots()[index as usize].clone();
                self.store.splice(index, 1, &[glib::BoxedAnyObject::new(updated)]);
            }
        }
    }

    /// Every loaded item shows the flag the library holds for its url.
    fn apply_favorite_urls(&self, urls: &BTreeSet<String>) {
        for favorite in [true, false] {
            self.set_favorite_where(|item| urls.contains(&item.url) == favorite, favorite);
        }
    }

    /// Asks for the next page when none is in flight. Loaders run on the runtime and report through the channel.
    fn pump(self: &Rc<Self>) {
        if !self.ready.get() || self.deps.stub {
            return;
        }
        let Some(ask) = self.listing.borrow_mut().ask() else { return };
        let (query, filters) = {
            let listing = self.listing.borrow();
            (listing.query().clone(), listing.filters().clone())
        };
        let load = self.deps.loader.load(query, filters, ask.page);
        let name = self.deps.source_name.clone();
        let (generation, tx) = self.tasks.borrow().sender();
        self.tasks.borrow_mut().spawn(async move {
            // whatever the loader does (fail, hang, panic) the spinner ends with a sentence and the Retry button
            let result = super::errors::settle(super::errors::BACKSTOP, &name, load).await;
            let _ = tx.send(Tagged { generation, event: Loaded { ask_generation: ask.generation, page: ask.page, result } }).await;
        });
        self.render();
    }

    fn pump_if_near_end(self: &Rc<Self>) {
        let adjustment = self.scroller.vadjustment();
        let cells = self.store.n_items() as usize;
        let last = last_visible_index(adjustment.value(), adjustment.page_size(), adjustment.upper(), cells);
        if self.listing.borrow().wants_more(last) {
            self.pump();
        }
    }

    fn arrived(self: &Rc<Self>, loaded: Loaded) {
        match loaded.result {
            Ok(page) => {
                let saved = (loaded.page == 1 && !self.deps.is_local).then(|| page.items.clone());
                let replacing = self.listing.borrow().is_provisional();
                let first_new = if replacing { 0 } else { self.listing.borrow().slots().len() };
                let delivered = self.listing.borrow_mut().deliver(loaded.ask_generation, page);
                // the site's first page replaces the saved results in place
                let replaced = replacing && !self.listing.borrow().is_provisional();
                if replaced {
                    self.store.remove_all();
                }
                if let (Some(items), Some(_)) = (saved, delivered) {
                    let listing = self.listing.borrow();
                    super::results_cache::write(self.deps.source_id, listing.query(), listing.filters(), &items);
                }
                match delivered {
                    // an empty page was recorded as an error
                    Some(0) if !replaced => self.toast_dismissed.set(false),
                    Some(_) => {
                        let listing = self.listing.borrow();
                        let added: Vec<glib::BoxedAnyObject> =
                            listing.slots()[first_new..].iter().map(|slot| glib::BoxedAnyObject::new(slot.clone())).collect();
                        drop(listing);
                        self.store.extend_from_slice(&added);
                    }
                    None => {}
                }
            }
            Err(message) => {
                if self.listing.borrow_mut().fail(loaded.ask_generation, message) {
                    self.toast_dismissed.set(false);
                }
            }
        }
        self.render();
        self.pump_if_near_end();
    }

    /// The columns of the grid follow the width of the grid's own area, counted by the shared cover grid exactly as the
    /// Library counts its own (`widgets::cover_grid`).
    fn apply_columns(self: &Rc<Self>) {
        let viewport = self.scroller.hadjustment().page_size() as i32;
        if viewport > 0 {
            crate::widgets::cover_grid::apply_columns(&self.grid, viewport, self.deps.prefs.cover_size.get());
        }
    }

    /// Gives the grid and the list a factory for the current display mode and shows the right one.
    fn apply_display(self: &Rc<Self>) {
        let mode = self.display.get();
        let load = Rc::clone(&self.deps.load_cover);
        let menu = Rc::clone(&self.deps.menu);
        let cover_px = crate::widgets::cover_grid::cover_width(self.deps.prefs.cover_size.get());
        // one ticket per recycled cell: a cover that arrives after its cell was rebound is dropped (edge R10)
        let binds: Rc<RefCell<CoverBinds>> = Rc::default();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else { return };
            let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            // the cell takes focus so the Menu key reaches its menu
            holder.set_focusable(true);
            let (weak, menu) = (item.downgrade(), Rc::clone(&menu));
            context_menu::attach(&holder, move |_, _| {
                let entry = weak.upgrade().and_then(|row| row.item()).and_downcast::<glib::BoxedAnyObject>();
                // the item is copied out, so no borrow of the row is held while the menu is built or run
                let item = entry.and_then(|entry| entry.borrow::<Option<ListItem>>().clone());
                item.map_or_else(Vec::new, |item| menu(&item))
            });
            item.set_child(Some(&holder));
        });
        factory.connect_bind({
            let binds = Rc::clone(&binds);
            move |_, object| {
                let Some(row) = object.downcast_ref::<gtk::ListItem>() else { return };
                let Some(boxed) = row.item().and_downcast::<glib::BoxedAnyObject>() else { return };
                let Some(holder) = row.child().and_downcast::<gtk::Box>() else { return };
                let key = glib::object::ObjectType::as_ptr(row) as usize;
                let ticket = binds.borrow_mut().bind(key);
                let guard: CoverGuard = {
                    let binds = Rc::clone(&binds);
                    Rc::new(move || binds.borrow().accepts(key, ticket))
                };
                let slot = boxed.borrow::<Option<ListItem>>();
                let cell = if mode == LibraryDisplayMode::List {
                    list_cell(slot.as_ref(), &load, &guard)
                } else {
                    grid_cell(slot.as_ref(), mode, &load, &guard, cover_px)
                };
                while let Some(old) = holder.first_child() {
                    holder.remove(&old);
                }
                holder.append(&cell);
            }
        });
        factory.connect_teardown(move |_, object| {
            binds.borrow_mut().release(glib::object::ObjectType::as_ptr(object) as usize);
        });
        if mode == LibraryDisplayMode::List {
            self.list.set_factory(Some(&factory));
            self.scroller.set_child(Some(&self.list));
        } else {
            self.grid.set_factory(Some(&factory));
            self.scroller.set_child(Some(&self.grid));
        }
    }

    fn render(self: &Rc<Self>) {
        let plan = self.listing.borrow().plan(self.ready.get(), self.deps.stub, self.deps.is_local, self.display.get());
        self.footer.set_visible(matches!(plan, ContentPlan::Items { .. }) && self.listing.borrow().loading_more());
        match plan {
            ContentPlan::Spinner => self.stack.set_visible_child_name("spinner"),
            ContentPlan::Stub => self.stack.set_visible_child_name("stub"),
            ContentPlan::Empty => self.stack.set_visible_child_name("empty"),
            ContentPlan::Error { message, actions } => {
                self.clear(&self.error_slot);
                let weak = Rc::downgrade(self);
                let on_action = Rc::clone(&self.deps.on_action);
                self.error_slot.append(&empty_state(&message, &actions, move |action| {
                    match (action, weak.upgrade()) {
                        (EmptyAction::Retry, Some(inner)) => inner.retry(),
                        _ => on_action(action),
                    }
                }));
                self.stack.set_visible_child_name("error");
            }
            ContentPlan::Items { toast, .. } => {
                self.stack.set_visible_child_name("items");
                if let Some(spec) = toast.filter(|_| !self.toast_dismissed.get()) {
                    self.show_toast(&spec);
                }
            }
        }
    }

    /// The toast of a failed later page: the shared layer's toast, with Retry when the spec carries it.
    fn show_toast(self: &Rc<Self>, spec: &ToastSpec) {
        self.toast_shown.set(true);
        let duration = match spec.duration {
            ToastDuration::Seconds(s) => Some(std::time::Duration::from_secs(u64::from(s))),
            ToastDuration::Indefinite => None,
        };
        if spec.action == Some(ToastAction::Retry) {
            let weak = Rc::downgrade(self);
            crate::toast::notify_action(&spec.message, &tr(MsgKey::action_retry), duration, move || {
                if let Some(inner) = weak.upgrade() {
                    inner.retry();
                }
            });
        } else {
            crate::toast::notify_for(&spec.message, duration.unwrap_or(crate::toast::ACTION_DURATION));
        }
        if let Some(duration) = duration {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(duration, move || {
                if let Some(inner) = weak.upgrade() {
                    inner.toast_dismissed.set(true);
                    inner.render();
                }
            });
        }
    }

    fn retry(self: &Rc<Self>) {
        self.toast_dismissed.set(false);
        self.listing.borrow_mut().clear_errors();
        self.pump();
    }

    fn clear(&self, slot: &gtk::Box) {
        while let Some(child) = slot.first_child() {
            slot.remove(&child);
        }
    }
}

/// Closes the listing of a source that was just turned on and opens it again, now that the source is loaded.
fn reopen_listing(source_id: i64) {
    let Some(kind) = [kuro_platform::kind::Kind::Manga, kuro_platform::kind::Kind::Anime].into_iter().find(|kind| super::directory::current(*kind).catalogue(source_id).is_some()) else { return };
    crate::ui::pop();
    super::pending::open(super::route::BrowseRoute::BrowseSource { kind, source_id, query: kuro_domain::source::browse::BrowseQuery::Popular, migrating: false });
}

#[cfg(test)]
pub(crate) mod tests {
    use kuro_platform::kind::Kind;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::*;
    use kuro_prefs::store::PrefStore;

    /// Serves pages from a script: each call takes the next entry.
    struct Script {
        pages: Mutex<Vec<Result<ListedPage, String>>>,
        calls: Mutex<Vec<(BrowseQuery, u32)>>,
        filters: Mutex<Vec<FilterList>>,
    }

    impl PageLoader for Script {
        fn load(&self, query: BrowseQuery, filters: FilterList, page: u32) -> super::super::listing::LoadFuture {
            self.calls.lock().expect("lock").push((query, page));
            self.filters.lock().expect("lock").push(filters);
            let next = {
                let mut pages = self.pages.lock().expect("lock");
                if pages.is_empty() { Ok(ListedPage { items: vec![], has_next: false }) } else { pages.remove(0) }
            };
            Box::pin(async move { next })
        }
    }

    fn items(titles: &[&str], has_next: bool) -> Result<ListedPage, String> {
        Ok(ListedPage {
            items: titles.iter().map(|t| ListItem { title: (*t).into(), url: format!("/{t}"), ..ListItem::default() }).collect(),
            has_next,
        })
    }

    struct Rig {
        view: ListingView,
        script: Arc<Script>,
        ready: watch::Sender<bool>,
        opened: Rc<RefCell<Vec<String>>>,
        actions: Rc<RefCell<Vec<EmptyAction>>>,
        /// The runtime stays entered while the rig lives, as `main` keeps it for the app. Leaked: it is a test.
        _entered: tokio::runtime::EnterGuard<'static>,
    }

    fn rig(pages: Vec<Result<ListedPage, String>>, ready: bool, stub: bool, is_local: bool) -> Rig {
        let rt: &'static tokio::runtime::Runtime =
            Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().expect("runtime")));
        let entered = rt.enter();
        let script = Arc::new(Script { pages: Mutex::new(pages), calls: Mutex::default(), filters: Mutex::default() });
        let (ready_tx, ready_rx) = watch::channel(ready);
        let opened: Rc<RefCell<Vec<String>>> = Rc::default();
        let actions: Rc<RefCell<Vec<EmptyAction>>> = Rc::default();
        let (open_log, action_log) = (Rc::clone(&opened), Rc::clone(&actions));
        let deps = ListingDeps {
            loader: Arc::clone(&script) as Arc<dyn PageLoader>,
            prefs: Rc::new(BrowsePrefs::new(PrefStore::in_memory(), Kind::Manga)),
            ready: ready_rx,
            stub,
            is_local,
            source_id: 9,
            source_name: "Gone Source".into(),
            source_lang: "en".into(),
            load_cover: Rc::new(|_, _| {}),
            open_entry: Rc::new(move |item| open_log.borrow_mut().push(item.title.clone())),
            menu: Rc::new(|_: &ListItem| Vec::new()),
            favorite_urls: None,
            on_action: Rc::new(move |action| action_log.borrow_mut().push(action)),
        };
        let view = ListingView::new(deps, BrowseQuery::Popular);
        Rig { view, script, ready: ready_tx, opened, actions, _entered: entered }
    }

    fn pump(mut done: impl FnMut() -> bool) -> bool {
        let context = glib::MainContext::default();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            while context.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    fn shown(rig: &Rig) -> String {
        rig.view.inner.stack.visible_child_name().map(|n| n.to_string()).unwrap_or_default()
    }

    fn the_spinner_shows_until_the_registry_is_ready_then_the_items_appear_without_reopening() {
        let rig = rig(vec![items(&["a", "b", "c"], false)], false, false, false);
        assert_eq!(shown(&rig), "spinner");
        assert!(rig.script.calls.lock().expect("lock").is_empty(), "nothing loads before the registry is ready");
        rig.ready.send_replace(true);
        assert!(pump(|| shown(&rig) == "items"), "initialisation finishing updates the open page");
        assert_eq!(rig.view.inner.store.n_items(), 3);
        assert_eq!(rig.script.calls.lock().expect("lock")[0], (BrowseQuery::Popular, 1));
    }

    fn a_search_is_a_new_listing_and_the_old_pages_are_dropped() {
        let rig = rig(vec![items(&["a"], false), items(&["found"], false)], true, false, false);
        assert!(pump(|| rig.view.inner.store.n_items() == 1));
        rig.view.restart(BrowseQuery::Search(String::new()));
        assert!(pump(|| rig.script.calls.lock().expect("lock").len() == 2));
        assert_eq!(rig.script.calls.lock().expect("lock")[1], (BrowseQuery::Search(String::new()), 1), "a blank search is still sent");
        assert!(pump(|| rig.view.inner.store.n_items() == 1 && rig.view.query() == BrowseQuery::Search(String::new())));
    }

    fn an_error_without_items_shows_its_actions_and_retry_loads_again() {
        let rig = rig(vec![Err("offline".into()), items(&["a"], false)], true, false, false);
        assert!(pump(|| shown(&rig) == "error"));
        let mut found = Vec::new();
        fn buttons(widget: &gtk::Widget, out: &mut Vec<gtk::Button>) {
            if let Some(b) = widget.downcast_ref::<gtk::Button>() {
                out.push(b.clone());
            }
            let mut child = widget.first_child();
            while let Some(w) = child {
                buttons(&w, out);
                child = w.next_sibling();
            }
        }
        buttons(rig.view.inner.error_slot.upcast_ref(), &mut found);
        assert_eq!(found.len(), 1, "retry is the only button; there is no help page to open");
        found[0].emit_clicked();
        assert!(rig.actions.borrow().is_empty(), "retry is handled inside the view");
        assert!(pump(|| shown(&rig) == "items"), "retry asks the failed page again");
    }

    fn a_local_source_error_offers_retry_and_is_never_empty() {
        let rig = rig(vec![Err("no folder".into())], true, false, true);
        assert!(pump(|| shown(&rig) == "error"));
        fn count(widget: &gtk::Widget) -> usize {
            let mut total = usize::from(widget.is::<gtk::Button>());
            let mut child = widget.first_child();
            while let Some(w) = child {
                total += count(&w);
                child = w.next_sibling();
            }
            total
        }
        let count = count(rig.view.inner.error_slot.upcast_ref());
        assert_eq!(count, 1);
    }

    fn a_later_page_failure_keeps_the_items_and_shows_a_toast_that_stays_up() {
        let rig = rig(vec![items(&["a"], true), Err("page 2 failed".into())], true, false, false);
        assert!(pump(|| rig.view.inner.toast_shown.get()));
        assert_eq!(shown(&rig), "items");
        assert_eq!(rig.view.inner.store.n_items(), 1);
        let _ = pump(|| false);
        assert!(rig.view.inner.toast_shown.get(), "no auto-hide for an error toast");
    }

    fn texts(root: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(label) = root.downcast_ref::<gtk::Label>() {
            out.push(label.label().to_string());
        }
        let mut child = root.first_child();
        while let Some(w) = child {
            texts(&w, out);
            child = w.next_sibling();
        }
    }

    fn an_empty_first_page_is_the_error_state_and_an_empty_later_page_is_a_toast() {
        let rig = rig(vec![items(&[], false)], true, false, false);
        assert!(pump(|| shown(&rig) == "error"), "an empty page 1 is an error, not the plain empty screen");
        let mut found = Vec::new();
        texts(rig.view.inner.error_slot.upcast_ref(), &mut found);
        assert!(found.contains(&tr(MsgKey::no_results_found)), "{found:?}");

        let rig = rig_second(vec![items(&["a"], true), items(&[], true)]);
        assert!(pump(|| rig.view.inner.toast_shown.get()), "an empty later page raises the retry toast");
        assert_eq!(shown(&rig), "items");
        assert_eq!(rig.view.inner.store.n_items(), 1);
        let calls = rig.script.calls.lock().expect("lock").len();
        let _ = pump(|| false);
        assert_eq!(rig.script.calls.lock().expect("lock").len(), calls, "the pager does not keep asking");
    }

    fn rig_second(pages: Vec<Result<ListedPage, String>>) -> Rig {
        rig(pages, true, false, false)
    }

    fn a_search_hands_its_own_filters_to_the_loader() {
        use kuro_source::filter::Filter;
        let rig = rig(vec![items(&["a"], false), items(&["b"], false)], true, false, false);
        assert!(pump(|| rig.view.inner.store.n_items() == 1));
        let filters = FilterList(vec![Filter::check_box("Flag")]);
        rig.view.restart_with(BrowseQuery::Search("x".into()), filters.clone());
        assert!(pump(|| rig.script.calls.lock().expect("lock").len() == 2));
        assert_eq!(rig.script.filters.lock().expect("lock")[1], filters);
        assert_eq!(rig.view.filters(), filters);
        rig.view.restart(BrowseQuery::Latest);
        assert_eq!(rig.view.filters(), filters, "a Popular or Latest switch through restart keeps the filter list");
    }

    fn covers_observe_the_favourite_flag_of_their_entry() {
        let rig = rig(vec![items(&["a", "b"], false)], true, false, false);
        assert!(pump(|| rig.view.inner.store.n_items() == 2));
        let flag = |i: u32| rig.view.inner.item_at(i).map(|item| item.favorite);
        assert_eq!((flag(0), flag(1)), (Some(false), Some(false)));
        rig.view.inner.apply_favorite_urls(&["/b".to_owned()].into());
        assert_eq!((flag(0), flag(1)), (Some(false), Some(true)), "the library's url set decides");
        rig.view.inner.apply_favorite_urls(&BTreeSet::new());
        assert_eq!(flag(1), Some(false), "removing from the library clears the flag");
        let id = {
            let mut listing = rig.view.inner.listing.borrow_mut();
            assert!(listing.set_favorite(0, false));
            7
        };
        rig.view.inner.set_favorite_where(|item| item.title == "a", true);
        assert_eq!(flag(0), Some(true));
        rig.view.set_favorite(id, false);
        assert_eq!(flag(0), Some(true), "no entry has id 7, nothing changes");
    }

    fn a_stub_shows_its_label_and_never_loads() {
        let rig = rig(vec![items(&["a"], false)], true, true, false);
        assert_eq!(shown(&rig), "stub");
        let mut found = Vec::new();
        texts(rig.view.inner.stack.upcast_ref(), &mut found);
        let expected = trf(MsgKey::source_not_installed, &[("arg1", Arg::Str("Gone Source (EN)"))]);
        assert!(found.contains(&expected), "the stub names the source from name and language: {found:?}");
        let _ = pump(|| false);
        assert!(rig.script.calls.lock().expect("lock").is_empty());
    }

    fn the_display_mode_picks_the_grid_or_the_list_and_activation_opens_the_entry() {
        let rig = rig(vec![items(&["a", "b"], false)], true, false, false);
        assert!(pump(|| rig.view.inner.store.n_items() == 2));
        assert_eq!(rig.view.inner.scroller.child().as_ref(), Some(rig.view.inner.grid.upcast_ref()), "compact grid is the default");
        rig.view.set_display_mode(LibraryDisplayMode::List);
        assert_eq!(rig.view.inner.scroller.child().as_ref(), Some(rig.view.inner.list.upcast_ref()));
        rig.view.set_display_mode(LibraryDisplayMode::CoverOnlyGrid);
        assert_eq!(rig.view.inner.scroller.child().as_ref(), Some(rig.view.inner.grid.upcast_ref()));
        rig.view.inner.grid.emit_by_name::<()>("activate", &[&1_u32]);
        assert_eq!(*rig.opened.borrow(), ["b"]);
    }

    fn dropping_the_view_aborts_its_loader() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().expect("runtime");
        let _guard = rt.enter();
        struct Hang(Arc<std::sync::atomic::AtomicUsize>);
        struct Live(Arc<std::sync::atomic::AtomicUsize>);
        impl Drop for Live {
            fn drop(&mut self) {
                self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        impl PageLoader for Hang {
            fn load(&self, _query: BrowseQuery, _filters: FilterList, _page: u32) -> super::super::listing::LoadFuture {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let guard = Live(Arc::clone(&self.0));
                Box::pin(async move {
                    let _guard = guard;
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    Err(String::new())
                })
            }
        }
        let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (_tx, ready) = watch::channel(true);
        let view = ListingView::new(
            ListingDeps {
                loader: Arc::new(Hang(Arc::clone(&live))),
                prefs: Rc::new(BrowsePrefs::new(PrefStore::in_memory(), Kind::Manga)),
                ready,
                stub: false,
                is_local: false,
                source_id: 1,
                source_name: String::new(),
                source_lang: String::new(),
                load_cover: Rc::new(|_, _| {}),
                open_entry: Rc::new(|_| {}),
                menu: Rc::new(|_: &ListItem| Vec::new()),
                favorite_urls: None,
                on_action: Rc::new(|_| {}),
            },
            BrowseQuery::Popular,
        );
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(view);
        assert!(pump(|| live.load(std::sync::atomic::Ordering::SeqCst) == 0), "popping the page aborts its loader");
    }

    pub(crate) fn smoke() {
        the_spinner_shows_until_the_registry_is_ready_then_the_items_appear_without_reopening();
        a_search_is_a_new_listing_and_the_old_pages_are_dropped();
        an_error_without_items_shows_its_actions_and_retry_loads_again();
        a_local_source_error_offers_retry_and_is_never_empty();
        a_later_page_failure_keeps_the_items_and_shows_a_toast_that_stays_up();
        an_empty_first_page_is_the_error_state_and_an_empty_later_page_is_a_toast();
        a_search_hands_its_own_filters_to_the_loader();
        covers_observe_the_favourite_flag_of_their_entry();
        a_stub_shows_its_label_and_never_loads();
        the_display_mode_picks_the_grid_or_the_list_and_activation_opens_the_entry();
        dropping_the_view_aborts_its_loader();
    }
}

