//! The static builders that replace the mockup rows of `entry.rs` (T-40a-034): `sources`, `migrate_sources`,
//! `browse_source`, `global_search` and `migrate_entries`. Each takes a snapshot of its model and the
//! callbacks of its navigation, and draws no row that the snapshot does not hold: an empty snapshot is an empty page.
//! The screen registry calls these in place of the sample-row builders.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use kuro_platform::kind::Kind;

use gtk::prelude::*;
use kuro_domain::source::browse::BrowseQuery;
use kuro_domain::source::model::Source;
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_source::filter_list::FilterList;
use tokio::sync::watch;

use super::entries::FavoriteStore;
use super::favorite_flow::{FavoriteFlow, FavoriteHooks};
use super::filter_sheet::FilterSheetView;
use super::filters::SourceFilters;
use super::grid::list_height;
use super::header::OverflowItem;
use super::header_bar::{HeaderDeps, SourceHeader, build as build_header, stub_header};
#[cfg(test)]
use super::icons::resolve;
use super::listing::ListItem;
use super::nav::{BackAction, action_url, back_action};
use super::listing_view::{ListingDeps, ListingView};
use super::names::subtitle_display_name;
#[cfg(test)]
use super::names::localized_display_name;
use super::primitives::{ToastSpec, stub_source_label};
use super::route::search_listing;
use crate::widgets::view_options::{Item as MenuItem, Pick, Section, ViewOptions};
#[cfg(test)]
use super::sources::LOCAL_SOURCE_ID;
use super::sources_page::{SourcesNav, SourcesPage};
use super::view::{CoverLoader, list_row};
use crate::widgets::context_menu::{self, Item};
#[cfg(test)]
use super::view::source_row;

/// `sources`: the installed-sources tab of one kind.
pub fn sources(nav: SourcesNav, ready: watch::Receiver<bool>) -> SourcesPage {
    SourcesPage::new(nav, ready)
}

// ---------- migrate_sources ----------

#[cfg(test)]
/// The migration source picker: the sorted sources with their favourite counts, and the sort the list is in.
#[derive(Clone, Debug)]
pub struct MigrateSourcesSnapshot {
    /// Already sorted for display (`migration::sort_migration_sources`).
    pub sources: Vec<(Source, i64)>,
    /// What the page says when no source has a favourite: the manga picker says `information_empty_library`, the anime
    /// picker `texts::MIGRATE_SOURCES_EMPTY` (T-41a-036).
    pub empty_text: String,
    /// Set when the favourite counts could not be read: the page shows an error row with Retry instead of the list
    /// (E-MS-02).
    pub error: Option<String>,
}

#[cfg(test)]
pub struct MigrateSourcesNav {
    /// A row: the favourites of that source.
    pub open: Rc<dyn Fn(i64)>,
    /// The row menu's Copy: copies the id.
    pub copy_id: Rc<dyn Fn(i64)>,
    /// The header's help action: the source migration guide.
    pub open_help: Rc<dyn Fn()>,
    /// The error row's Retry: reads the counts again.
    pub retry: Rc<dyn Fn()>,
}

#[cfg(test)]
pub fn migrate_sources(snapshot: &MigrateSourcesSnapshot, nav: MigrateSourcesNav) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // the prompt and the help button head the list column, as a section heading would
    let sort = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    sort.set_margin_start(16);
    sort.set_margin_end(16);
    sort.set_margin_top(12);
    sort.append(&gtk::Label::builder().label(tr(MsgKey::migration_selection_prompt)).hexpand(true).xalign(0.0).wrap(true).css_classes(["dim-label"]).build());
    let help = gtk::Button::from_icon_name("help-browser-symbolic");
    help.set_tooltip_text(Some(&tr(MsgKey::label_help)));
    let open_help = Rc::clone(&nav.open_help);
    help.connect_clicked(move |_| open_help());
    sort.append(&help);
    root.append(&sort);

    if let Some(message) = &snapshot.error {
        root.append(&super::look::error_state(message, Some(Rc::clone(&nav.retry))));
        return root;
    }
    if snapshot.sources.is_empty() {
        root.append(&super::look::empty_state("view-list-symbolic", &snapshot.empty_text, &[]).0);
        return root;
    }
    let column = crate::ui::column();
    let list = crate::ui::boxed_list();
    for (source, count) in &snapshot.sources {
        let language = (!source.lang.is_empty()).then(|| localized_display_name(&source.lang));
        let badge = super::look::badge(&migrate_count(*count), false);
        let icon = resolve(source.is_stub, false, source.id == LOCAL_SOURCE_ID);
        let row = source_row(&source.name, language.as_deref(), icon, None, source.is_stub, Some(badge.upcast_ref()));
        let (open, copy, id) = (Rc::clone(&nav.open), Rc::clone(&nav.copy_id), source.id);
        let open_item = Rc::clone(&open);
        list.append(&list_row(&row, move || open(id), move || {
            let (open, copy) = (Rc::clone(&open_item), Rc::clone(&copy));
            vec![Item::new(super::texts::OPEN, 0, move || open(id)), Item::new(tr(MsgKey::action_copy_to_clipboard), 1, move || copy(id))]
        }));
    }
    column.append(&list);
    root.append(&crate::ui::scrolled(&column));
    root
}

#[cfg(test)]
/// The badge of a migrate source row, in words: how many library entries come from it.
pub fn migrate_count(count: impl std::fmt::Display) -> String {
    format!("{count} in library")
}

// ---------- browse_source ----------

/// The library side of a browse page: the store its covers read and write, and what the flows report back.
pub struct FavoritesDeps {
    pub store: Arc<dyn FavoriteStore>,
    pub toast: Rc<dyn Fn(ToastSpec)>,
    /// The duplicate dialog's Show manga.
    pub open_entry: Rc<dyn Fn(i64)>,
    /// The duplicate dialog's Migrate existing entry: the duplicate (old) and the browsed entry (new).
    pub migrate: Rc<dyn Fn(i64, i64)>,
    pub now_ms: Rc<dyn Fn() -> i64>,
    /// The category chooser's Edit categories: the Categories page on this kind's section.
    pub edit_categories: Rc<dyn Fn()>,
    /// The page's kind: the duplicate dialog's wording.
    pub kind: Kind,
}

pub struct BrowseSourceDeps {
    pub supports_latest: bool,
    pub configurable: bool,
    pub prefs: Rc<super::prefs::BrowsePrefs>,
    /// The source's id, name, language and stub flag are read from here. `menu` and `favorite_urls` are filled
    /// in by `browse_source` from `favorites`.
    pub listing: ListingDeps,
    /// `source_defaults`, and the sheet that edits a working copy of them.
    pub filters: Rc<SourceFilters>,
    pub favorites: Option<FavoritesDeps>,
    /// The hide-in-library preference of the source's kind, read when the page is made (`BrowsePrefs::hide_in_library`
    /// for manga): favourites are then left out of the listing (T-40b-034).
    pub hide_in_library: bool,
    /// What the page opens on: Popular, or a search (a browse page opened with a query, B§4.6).
    pub initial_query: BrowseQuery,
    pub on_back: Rc<dyn Fn()>,
    pub on_overflow: Rc<dyn Fn(OverflowItem)>,
    /// Opens a url in the browser (the local source's help guide, a cover's Open in browser).
    pub open_url: Rc<dyn Fn(&str)>,
    /// The source's site: a cover whose url is a path opens in the browser on it.
    pub home_url: Option<String>,
}

/// The right-click menu of a browsed cover: Open, Add to library or Remove from library (the favourite flow, which asks
/// before a remove and about a duplicate), and Open in browser when the entry has a web address.
pub fn cover_menu(item: &ListItem, open: Rc<dyn Fn(&ListItem)>, flow: Option<Rc<FavoriteFlow>>, web: Option<String>, open_url: Rc<dyn Fn(&str)>) -> Vec<Item> {
    let entry = item.clone();
    let mut items = vec![Item::new(super::texts::OPEN, 0, move || open(&entry))];
    if let Some(flow) = flow {
        let label = tr(if item.favorite { MsgKey::remove_from_library } else { MsgKey::add_to_library });
        let entry = item.clone();
        items.push(Item::new(label, 1, move || flow.long_press(entry.clone())));
    }
    if let Some(url) = web {
        items.push(Item::new(tr(MsgKey::action_open_in_browser), 2, move || open_url(&url)));
    }
    items
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// One source's listing with its header, search and Popular, Latest and Filter chips.
pub struct BrowseSourcePage {
    pub root: gtk::Box,
    pub header: SourceHeader,
    pub listing: ListingView,
    pub search: gtk::SearchEntry,
    pub popular: gtk::ToggleButton,
    pub latest: Option<gtk::ToggleButton>,
    /// The Filter menu button; its popover is the filter sheet.
    pub filter: Option<gtk::MenuButton>,
    /// Keeps the favourite flow alive with the page.
    pub flow: Option<Rc<FavoriteFlow>>,
}

/// What `browse_source` builds: the listing page, or for a source that is not installed the missing-source page.
pub enum SourcePage {
    Browse(BrowseSourcePage),
    /// Title, back and the message; no search, no chips, no retry (T-40b-040).
    Stub(gtk::Box),
}

impl SourcePage {
    pub fn widget(&self) -> &gtk::Box {
        match self {
            SourcePage::Browse(page) => &page.root,
            SourcePage::Stub(root) => root,
        }
    }
}

/// The page of a source that is not installed: its name as the title, a way back and `Source not installed: <name
/// (LANG)>`, or the bare id when the name or the language is blank (T-41a-008).
fn stub_page(listing: &ListingDeps, on_back: Rc<dyn Fn()>) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&stub_header(&listing.source_name, on_back));
    let label = stub_source_label(&listing.source_name, &listing.source_lang, listing.source_id);
    root.append(&super::look::empty_state("dialog-warning-symbolic", &trf(MsgKey::source_not_installed, &[("arg1", Arg::Str(&label))]), &[]).0);
    root
}

pub fn browse_source(deps: BrowseSourceDeps) -> SourcePage {
    let BrowseSourceDeps { supports_latest, configurable, prefs, mut listing, filters, favorites, hide_in_library, initial_query, on_back, on_overflow, open_url, home_url } = deps;
    if listing.stub {
        return SourcePage::Stub(stub_page(&listing, on_back));
    }
    let (source_name, is_local, source_id) = (listing.source_name.clone(), listing.is_local, listing.source_id);

    // the help actions open the guide; the rest go to the caller
    let on_action = Rc::clone(&listing.on_action);
    let help_url = Rc::clone(&open_url);
    listing.on_action = Rc::new(move |action| match action_url(action) {
        Some(url) => help_url(url),
        None => on_action(action),
    });
    // the cover menu and the live favourite flags come from the library store; the flow is built once the listing exists
    let flow_slot: Rc<RefCell<Option<Rc<FavoriteFlow>>>> = Rc::default();
    let (on_menu, open_cover, browser) = (Rc::clone(&flow_slot), Rc::clone(&listing.open_entry), Rc::clone(&open_url));
    listing.menu = Rc::new(move |item: &ListItem| {
        let flow = on_menu.borrow().clone();
        let web = super::nav::entry_web_url(home_url.as_deref(), &item.url);
        cover_menu(item, Rc::clone(&open_cover), flow, web, Rc::clone(&browser))
    });
    listing.favorite_urls = favorites.as_ref().map(|f| f.store.favorite_urls(source_id));
    // a page opened on a search starts from the source's defaults, like a search made from Popular
    let start_filters = if matches!(initial_query, BrowseQuery::Search(_)) { filters.for_search(false) } else { FilterList::default() };
    let listing_view = ListingView::with_filters(listing, initial_query.clone(), start_filters);
    listing_view.set_hide_in_library(hide_in_library);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr(MsgKey::action_search_hint)));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let (search_bar, search_toggle) = crate::ui::search_row(&search, &root);

    let header = build_header(HeaderDeps {
        source_name,
        prefs,
        is_local,
        configurable,
        // Back with an active search and text in the box clears the text first and stays (B-EC-10)
        on_back: {
            let (search, toggle) = (search.clone(), search_toggle.clone());
            Rc::new(move || match back_action(toggle.is_active(), search.text().as_str()) {
                BackAction::ClearQuery => search.set_text(""),
                BackAction::Leave => on_back(),
            })
        },
        search: search_toggle,
        on_display_mode: {
            let listing = listing_view.clone();
            Rc::new(move |mode| listing.set_display_mode(mode))
        },
        on_refresh: {
            let listing = listing_view.clone();
            Rc::new(move || listing.restart_with(listing.query(), listing.filters()))
        },
        on_overflow: {
            let open_url = Rc::clone(&open_url);
            Rc::new(move |item| match (item, is_local) {
                (OverflowItem::Help, true) => {
                    if let Some(url) = super::nav::help_url() {
                        open_url(url);
                    }
                }
                _ => on_overflow(item),
            })
        },
    });

    if let BrowseQuery::Search(text) = &initial_query {
        search.set_text(text);
        header.search.set_active(true);
    }

    // a search is always a new listing, and a blank one is sent as the empty string (B-EC-08, B-EC-09). It starts from
    // the source's defaults when Popular or Latest was showing, and keeps the sheet's values when a search was running.
    let view = listing_view.clone();
    let seed = Rc::clone(&filters);
    search.connect_activate(move |entry| {
        let running = matches!(view.query(), BrowseQuery::Search(_));
        view.restart_with(search_listing(entry.text().as_str()), seed.for_search(running));
    });

    // the shared top row of a page (`ui::pill_row`), as the library's categories and the download queue's actions
    let chips = crate::ui::pill_row();
    chips.set_margin_start(16);
    chips.set_margin_end(16);
    let mut labels = vec![tr(MsgKey::popular)];
    if supports_latest {
        labels.push(tr(MsgKey::latest));
    }
    // a page that opens on a search has neither pressed (`usize::MAX` matches no index)
    let active = match initial_query {
        BrowseQuery::Popular => 0,
        BrowseQuery::Latest if supports_latest => 1,
        _ => usize::MAX,
    };
    let mut pills = crate::ui::choice_pills(&chips, &labels, active).into_iter();
    let popular = pills.next().expect("the Popular pill");
    let mut latest_chip = None;
    if let Some(latest) = pills.next() {
        let (view, seed) = (listing_view.clone(), Rc::clone(&filters));
        latest.connect_toggled(move |button| {
            if button.is_active() {
                view.restart_with(BrowseQuery::Latest, seed.for_search(false));
            }
        });
        latest_chip = Some(latest);
    }
    let (view, seed) = (listing_view.clone(), Rc::clone(&filters));
    popular.connect_toggled(move |button| {
        if button.is_active() {
            view.restart_with(BrowseQuery::Popular, seed.for_search(false));
        }
    });
    let mut filter_chip = None;
    if filters.has_filters() {
        // the filter sheet is the popover of this button (no separate window)
        let chip = crate::ui::quiet_menu(gtk::MenuButton::builder().label(tr(MsgKey::action_filter)).build());
        let (target, entry, closer) = (listing_view.clone(), search.clone(), chip.downgrade());
        let sheet = FilterSheetView::new(
            Rc::clone(&filters),
            // Apply searches with the sheet's values and the text in the search box
            move |list| target.restart_with(search_listing(entry.text().as_str()), list),
            move || {
                if let Some(chip) = closer.upgrade() {
                    chip.popdown();
                }
            },
        );
        chip.set_popover(Some(&sheet.popover()));
        chips.append(&chip);
        filter_chip = Some(chip);
    }

    root.append(&header.bar);
    root.append(&search_bar);
    // Escape clears a selection first (issue #7), as in the Library; Ctrl+A selects every loaded cover, Ctrl+I flips them
    {
        let view = listing_view.downgrade();
        let esc = gtk::EventControllerKey::new();
        esc.set_propagation_phase(gtk::PropagationPhase::Capture);
        esc.connect_key_pressed(move |_, key, _, _| match view.upgrade().filter(|view| key == gtk::gdk::Key::Escape && view.has_selection()) {
            Some(view) => {
                view.clear_selection();
                gtk::glib::Propagation::Stop
            }
            None => gtk::glib::Propagation::Proceed,
        });
        root.add_controller(esc);
        let (view, entry) = (listing_view.downgrade(), search.clone());
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(view) = view.upgrade() else { return gtk::glib::Propagation::Proceed };
            let ctrl = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let typing = entry.has_focus() || entry.focus_child().is_some();
            match crate::library::keys::shortcut(key, ctrl, view.has_selection(), typing) {
                Some(crate::library::keys::Shortcut::SelectAll) if !typing => view.select_all(),
                Some(crate::library::keys::Shortcut::Invert) if !typing => view.invert_selection(),
                _ => return gtk::glib::Propagation::Proceed,
            }
            gtk::glib::Propagation::Stop
        });
        root.add_controller(keys);
    }
    // Escape closes the open search first and then the window's handler leaves; Back clears its text first (B-EC-10)
    {
        let (bar, close) = (search_bar.clone(), search_bar.clone());
        super::keys::attach_escape_step(&root, move || bar.is_search_mode(), move || close.set_search_mode(false));
    }
    // the shared selection header takes the place of the chips while covers are selected (`selection_bar`)
    let count = gtk::Label::new(None);
    let sel_header = {
        let (cancel, all) = (listing_view.downgrade(), listing_view.downgrade());
        crate::ui::selection_header(
            &count,
            move || {
                if let Some(view) = cancel.upgrade() {
                    view.clear_selection();
                }
            },
            move || {
                if let Some(view) = all.upgrade() {
                    view.select_all();
                }
            },
        )
    };
    sel_header.set_visible(false);
    root.append(&sel_header);
    root.append(&chips);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    listing_view.widget().set_vexpand(true);
    root.append(listing_view.widget());

    // the library flows: a long press asks, a flag flips at once and a failed write puts it back
    let flow = favorites.map(|FavoritesDeps { store, toast, open_entry, migrate, now_ms, edit_categories, kind }| {
        let (flag_view, toast_view) = (listing_view.downgrade(), listing_view.downgrade());
        let hooks = FavoriteHooks {
            set_flag: Rc::new(move |id, flag| {
                if let Some(view) = flag_view.upgrade() {
                    view.set_favorite(id, flag);
                }
            }),
            toast: Rc::new(move |spec| {
                if toast_view.upgrade().is_some() {
                    toast(spec);
                }
            }),
            open_entry,
            migrate,
            now_ms,
            edit_categories,
        };
        FavoriteFlow::new_for(store, &root, hooks, kind)
    });
    *flow_slot.borrow_mut() = flow.clone();
    root.append(&selection_bar(&listing_view, flow.clone(), SelectionRow { count, header: sel_header, chips: chips.clone() }));

    // list mode fills the window below the top bar (B§2.8)
    let (view, bar, last) = (listing_view.downgrade(), header.bar.clone(), Cell::new(-1));
    root.add_tick_callback(move |root, _| {
        let px = list_height(root.height(), bar.height());
        if px != last.replace(px) {
            match view.upgrade() {
                Some(view) => view.set_min_content_height(px),
                None => return gtk::glib::ControlFlow::Break,
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    SourcePage::Browse(BrowseSourcePage { root, header, listing: listing_view, search, popular, latest: latest_chip, filter: filter_chip, flow })
}

/// The widgets the selection swaps: the count label of the shared selection header, the header, and the chip row it
/// replaces.
struct SelectionRow {
    count: gtk::Label,
    header: gtk::Box,
    chips: gtk::Box,
}

/// The bulk actions of a browse page's selection (issue #7): Add to library for the selected covers that are not in the
/// library (the favourite flow's bulk add: the default category, or the chooser once, then a duplicate check per title),
/// and Set categories for those that are. Each clears the selection. The bar shows while something is selected.
fn selection_bar(listing: &ListingView, flow: Option<Rc<FavoriteFlow>>, row: SelectionRow) -> gtk::Revealer {
    let add = gtk::Button::with_label(&tr(MsgKey::add_to_library));
    let categories = gtk::Button::with_label(&tr(MsgKey::action_move_category));
    let bar = crate::ui::action_bar(&[add.clone(), categories.clone()]);
    if let Some(flow) = &flow {
        let (view, flow_add) = (listing.downgrade(), Rc::clone(flow));
        add.connect_clicked(move |_| {
            let Some(view) = view.upgrade() else { return };
            let picked = view.selected();
            view.clear_selection();
            flow_add.add_many(picked);
        });
        let (view, flow_set) = (listing.downgrade(), Rc::clone(flow));
        categories.connect_clicked(move |_| {
            let Some(view) = view.upgrade() else { return };
            let picked = view.selected();
            view.clear_selection();
            flow_set.set_categories_many(picked);
        });
    }
    let (view, shown) = (listing.downgrade(), bar.clone());
    let can_act = flow.is_some();
    listing.connect_selection(move || {
        let Some(view) = view.upgrade() else { return };
        let picked = view.selected();
        let open = !picked.is_empty();
        row.count.set_label(&trf(MsgKey::library_selected_count, &[("count", Arg::Str(&picked.len().to_string()))]));
        row.header.set_visible(open);
        row.chips.set_visible(!open);
        add.set_visible(can_act && picked.iter().any(|item| !item.favorite));
        categories.set_visible(can_act && picked.iter().any(|item| item.favorite));
        shown.set_reveal_child(open);
    });
    bar
}

// ---------- global_search ----------

/// What one source's search shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchState {
    Loading,
    Error(String),
    Results(Vec<ListItem>),
    /// The saved results of the last same search (`results_cache`), shown while the source is asked again.
    Refreshing(Vec<ListItem>),
}

impl SearchState {
    /// The source has not answered this search yet.
    pub fn waiting(&self) -> bool {
        matches!(self, SearchState::Loading | SearchState::Refreshing(_))
    }

    /// The covers the row shows, saved or answered.
    pub fn items(&self) -> Option<&[ListItem]> {
        match self {
            SearchState::Results(items) | SearchState::Refreshing(items) => Some(items),
            SearchState::Loading | SearchState::Error(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchRow {
    pub source: Source,
    pub state: SearchState,
    /// The source the search started from (a migrate search): its header carries a "\u{25B6} " prefix.
    pub origin: bool,
}

/// Global search: the query, how many sources have answered of how many, and one row per source in the order the model
/// chose.
#[derive(Clone, Debug)]
pub struct GlobalSearchSnapshot {
    pub query: String,
    /// The sources that have answered, with an error counting as an answer.
    pub answered: usize,
    /// The sources asked.
    pub total: usize,
    pub rows: Vec<SearchRow>,
}

/// Which sources a search covers. Every enabled source: the pinned-only filter is cut with the pins (the language filter
/// is the only way to leave sources out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceFilter {
    #[default]
    All,
}

/// The chip row of the search toolbar: the source filter, and the Has results toggle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchChips {
    pub filter: SourceFilter,
    pub only_has_results: bool,
}

/// A card of a result row is 96 px wide (T-40a-011).
pub const SEARCH_CARD_WIDTH_PX: i32 = 96;

/// The progress bar shows while at least one source has answered and not all of them have (E-40-57..59).
pub fn progress_visible(answered: usize, total: usize) -> bool {
    answered >= 1 && answered < total
}

/// The rows the chips leave: Has results keeps rows whose search found something. A chip change filters what is already
/// here and never runs the search again (D-UI-10).
pub fn visible_rows(rows: &[SearchRow], chips: SearchChips) -> Vec<&SearchRow> {
    rows.iter().filter(|row| !chips.only_has_results || row.state.items().is_some_and(|items| !items.is_empty())).collect()
}

/// The title of a source header: the name, with a prefix on the origin source.
pub fn row_title(row: &SearchRow) -> String {
    if row.origin { format!("\u{25B6} {}", row.source.name) } else { row.source.name.clone() }
}

pub struct GlobalSearchNav {
    /// Enter in the search entry.
    pub search: Rc<dyn Fn(&str)>,
    /// The arrow of a source's header, with the text in the search box now (submitted or not): its browse listing with
    /// that text (GS-EC-15).
    pub open_source: Rc<dyn Fn(i64, &str)>,
    /// A card.
    pub open_entry: Rc<dyn Fn(i64, &ListItem)>,
    /// The right-click menu of a card of the source with this id.
    pub menu: Rc<dyn Fn(i64, &ListItem) -> Vec<Item>>,
    pub load_cover: CoverLoader,
    /// The chips as the page starts with them (the stored filter and toggle).
    pub chips: SearchChips,
    /// A chip changed: the page already filters its rows; this is for storing the choice. It runs no search.
    pub on_chips: Rc<dyn Fn(SearchChips)>,
}

/// What the page redraws from.
struct SearchView {
    column: gtk::Box,
    progress: gtk::ProgressBar,
    nav: GlobalSearchNav,
    chips: Cell<SearchChips>,
    last: RefCell<Option<GlobalSearchSnapshot>>,
    /// The text in the search box, as typed.
    typed: Rc<RefCell<String>>,
}

/// The global search page. It shows a spinner until the source registry is initialised, then the rows of the snapshot;
/// `update` replaces the rows and the progress as results arrive, and leaves the entry alone.
pub struct GlobalSearchPage {
    root: gtk::Widget,
    entry: gtk::SearchEntry,
    view: Rc<SearchView>,
    /// The header actions of the page: the shared search toggle and the search options button.
    actions: Vec<gtk::Widget>,
}

pub fn global_search(nav: GlobalSearchNav, ready: watch::Receiver<bool>) -> GlobalSearchPage {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(&tr(MsgKey::action_global_search_hint)));
    entry.set_hexpand(true);
    // the one search bar of the app (`ui::search_row`); the toggle goes among the header actions, and a global search page
    // opens with the bar showing
    let (bar, toggle) = crate::ui::search_row(&entry, &body);
    bar.set_search_mode(true);
    body.append(&bar);
    // Escape clears the typed query first; with none, it goes on and leaves the page
    {
        let (text, clear) = (entry.clone(), entry.clone());
        super::keys::attach_escape_step(&body, move || !text.text().is_empty(), move || clear.set_text(""));
    }
    let progress = gtk::ProgressBar::new();
    progress.set_visible(false);
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).valign(gtk::Align::Start).build();
    column.set_margin_start(16);
    column.set_margin_end(16);
    let view = Rc::new(SearchView { column, progress, chips: Cell::new(nav.chips), nav, last: RefCell::new(None), typed: Rc::default() });
    let search = Rc::clone(&view);
    entry.connect_activate(move |entry| (search.nav.search)(entry.text().as_str()));
    let typing = Rc::clone(&view);
    entry.connect_changed(move |entry| *typing.typed.borrow_mut() = entry.text().to_string());
    let options = search_options(&view);
    body.append(&view.progress);
    body.append(&gtk::ScrolledWindow::builder().child(&view.column).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
    // the registry's readiness gates the whole page: the existing source gate shows the spinner, then crossfades in
    let root = crate::source_gate::when_sources_ready(&kuro_core::sources::SourceGate::new(ready), &body);
    GlobalSearchPage { root, entry, view, actions: vec![toggle.upcast(), options.widget.clone().upcast()] }
}

/// The tooltip of the search options button, and the name of its one check.
pub fn search_options_label() -> String {
    tr(MsgKey::browse_search_options)
}
const HAS_RESULTS: &str = "has_results";

/// The search options (B§10.1) on the shared view-options button next to the search box: Show only "Has results". The
/// source filter has one value (All) since the pinned-only filter was cut, so it has no item.
fn search_options(view: &Rc<SearchView>) -> ViewOptions {
    let menu = ViewOptions::new(&search_options_label(), &[Section::filter(tr(MsgKey::ui_show_only), vec![MenuItem::toggle(HAS_RESULTS, tr(MsgKey::has_results))])]);
    menu.set_toggle(HAS_RESULTS, view.chips.get().only_has_results);
    let toggled = Rc::clone(view);
    menu.connect(move |pick| {
        if let Pick::Toggle(HAS_RESULTS, on) = pick {
            toggled.set_chips(SearchChips { only_has_results: on, ..toggled.chips.get() });
        }
    });
    menu
}

impl SearchView {
    fn set_chips(&self, chips: SearchChips) {
        if self.chips.replace(chips) == chips {
            return;
        }
        (self.nav.on_chips)(chips);
        self.redraw();
    }

    fn redraw(&self) {
        while let Some(child) = self.column.first_child() {
            self.column.remove(&child);
        }
        let last = self.last.borrow();
        let Some(snapshot) = last.as_ref() else { return };
        self.progress.set_fraction(if snapshot.total == 0 { 0.0 } else { (snapshot.answered as f64 / snapshot.total as f64).clamp(0.0, 1.0) });
        self.progress.set_visible(progress_visible(snapshot.answered, snapshot.total));
        for row in visible_rows(&snapshot.rows, self.chips.get()) {
            self.column.append(&search_row(row, &self.nav, Rc::clone(&self.typed)));
        }
    }
}

impl GlobalSearchPage {
    pub fn widget(&self) -> &gtk::Widget {
        &self.root
    }

    /// The widgets the screen puts in its header: the search toggle, then the search options.
    pub fn actions(&self) -> Vec<gtk::Widget> {
        self.actions.clone()
    }

    #[cfg(test)]
    /// The progress bar's fraction while it shows, `None` while it is hidden.
    pub fn progress(&self) -> Option<f64> {
        self.view.progress.is_visible().then(|| self.view.progress.fraction())
    }

    #[cfg(test)]
    pub fn update(&self, snapshot: &GlobalSearchSnapshot) {
        draw(&self.entry, &self.view, snapshot);
    }

    /// The hint of the empty search box. The anime page says "Search a source by title" (T-41a-027).
    pub fn set_hint(&self, hint: &str) {
        self.entry.set_placeholder_text(Some(hint));
    }

    /// `update` as a closure that holds the page's contents but not the page widget itself, so a model that owns it does not
    /// keep the page alive.
    pub fn updater(&self) -> impl Fn(&GlobalSearchSnapshot) + 'static {
        let (entry, view) = (self.entry.clone(), Rc::clone(&self.view));
        move |snapshot| draw(&entry, &view, snapshot)
    }
}

fn draw(entry: &gtk::SearchEntry, view: &SearchView, snapshot: &GlobalSearchSnapshot) {
    if entry.text().as_str() != snapshot.query && !entry.has_focus() {
        entry.set_text(&snapshot.query);
    }
    *view.last.borrow_mut() = Some(snapshot.clone());
    view.redraw();
}

fn search_row(row: &SearchRow, nav: &GlobalSearchNav, typed: Rc<RefCell<String>>) -> gtk::Box {
    let section = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).vexpand(false).valign(gtk::Align::Start).build();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
    titles.set_hexpand(true);
    titles.append(&gtk::Label::builder().label(row_title(row)).xalign(0.0).css_classes(["heading"]).build());
    if !row.source.lang.is_empty() {
        titles.append(&gtk::Label::builder().label(subtitle_display_name(&row.source.lang)).xalign(0.0).css_classes(["dim-label"]).build());
    }
    header.append(&titles);
    let go = gtk::Button::from_icon_name("go-next-symbolic");
    go.set_tooltip_text(Some(&row.source.name));
    let (open, id) = (Rc::clone(&nav.open_source), row.source.id);
    // the text is read when the arrow is pressed, not when the row was drawn
    go.connect_clicked(move |_| open(id, &typed.borrow().clone()));
    header.append(&go);
    section.append(&header);
    match &row.state {
        SearchState::Loading => {
            let spinner = crate::ui::loading(crate::ui::LoadSize::Inline);
            spinner.set_halign(gtk::Align::Center);
            spinner.set_margin_top(16);
            spinner.set_margin_bottom(16);
            section.append(&spinner);
        }
        SearchState::Error(message) => {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.append(&gtk::Image::from_icon_name("dialog-error-symbolic"));
            line.append(&gtk::Label::builder().label(message).xalign(0.0).wrap(true).css_classes(["error"]).build());
            section.append(&line);
        }
        SearchState::Results(items) if items.is_empty() => {
            section.append(&gtk::Label::builder().label(tr(MsgKey::no_results_found)).xalign(0.0).css_classes(["dim-label"]).build());
        }
        SearchState::Results(items) | SearchState::Refreshing(items) => {
            let covers = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).valign(gtk::Align::Start).vexpand(false).build();
            for item in items {
                let cell = super::view::grid_cell_lines(
                    Some(item),
                    kuro_prefs::library::LibraryDisplayMode::ComfortableGrid,
                    &nav.load_cover,
                    &super::view::always(),
                    SEARCH_CARD_WIDTH_PX,
                    3,
                );
                cell.set_vexpand(false);
                let button = gtk::Button::builder().child(&cell).css_classes(["flat"]).valign(gtk::Align::Start).vexpand(false).build();
                let (open, entry, id) = (Rc::clone(&nav.open_entry), item.clone(), row.source.id);
                button.connect_clicked(move |_| open(id, &entry));
                let (menu, entry) = (Rc::clone(&nav.menu), item.clone());
                context_menu::attach(&button, move |_, _| menu(id, &entry));
                covers.append(&button);
            }
            // the row is as tall as its cards; the page scrolls through the sources
            section.append(&gtk::ScrolledWindow::builder().child(&covers).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).vexpand(false).build());
        }
    }
    section
}

// ---------- migrate_entries ----------

/// One favourite of the source being migrated from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateEntry {
    pub id: i64,
    pub title: String,
    pub cover: Option<String>,
}

pub struct MigrateEntriesNav {
    /// The row: the migrate search for this entry.
    pub open_search: Rc<dyn Fn(i64)>,
    /// The cover: the entry itself.
    pub open_entry: Rc<dyn Fn(i64)>,
    pub load_cover: CoverLoader,
}

/// The favourites of one source, sorted by case-insensitive title. A load failure shows the error and an empty list.
pub fn migrate_entries(entries: &[MigrateEntry], error: Option<&str>, nav: MigrateEntriesNav) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(message) = error {
        root.append(&gtk::Label::builder().label(message).margin_top(8).css_classes(["error"]).build());
    }
    let mut sorted: Vec<&MigrateEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.title.to_lowercase());
    let column = crate::ui::column();
    let list = crate::ui::boxed_list();
    for entry in sorted {
        let item = ListItem { id: entry.id, title: entry.title.clone(), cover: entry.cover.clone(), ..ListItem::default() };
        let row = super::view::list_cell(Some(&item), &nav.load_cover, &super::view::always());
        let (open_search, id) = (Rc::clone(&nav.open_search), entry.id);
        // a click on the row opens the migrate search; a click on the cover opens the entry and claims the click
        let cell = row.clone();
        let row = list_row(&cell, move || open_search(id), Vec::new);
        if let Some(cover) = cell.first_child() {
            let open_entry = Rc::clone(&nav.open_entry);
            let click = gtk::GestureClick::new();
            click.connect_released(move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                open_entry(id);
            });
            cover.add_controller(click);
        }
        list.append(&row);
    }
    // an empty boxed list would still draw its frame
    list.set_visible(list.first_child().is_some());
    column.append(&list);
    root.append(&crate::ui::scrolled(&column));
    root
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;

    fn labels(root: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(label) = root.downcast_ref::<gtk::Label>() {
            out.push(label.label().to_string());
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            labels(&widget, out);
            child = widget.next_sibling();
        }
    }

    fn all_labels(root: &impl IsA<gtk::Widget>) -> Vec<String> {
        let mut out = Vec::new();
        labels(root.upcast_ref(), &mut out);
        out
    }

    fn nav_log() -> Rc<RefCell<Vec<String>>> {
        Rc::default()
    }

    fn no_sample_rows_in_an_empty_snapshot() {
        let log = nav_log();
        let (a, b) = (Rc::clone(&log), Rc::clone(&log));
        let migrate = migrate_sources(
            &MigrateSourcesSnapshot { sources: vec![], empty_text: tr(MsgKey::information_empty_library), error: None },
            MigrateSourcesNav {
                open: Rc::new(move |id| a.borrow_mut().push(format!("open {id}"))),
                copy_id: Rc::new(move |id| b.borrow_mut().push(format!("copy {id}"))),
                open_help: Rc::new(|| {}),
                retry: Rc::new(|| {}),
            },
        );
        let text = all_labels(&migrate);
        assert!(text.contains(&tr(MsgKey::information_empty_library)), "an empty list says so");
        assert!(!text.iter().any(|t| t.contains("Source A") || t.contains("Extension A")), "{text:?}");

        let (_tx, ready) = watch::channel(true);
        let search = global_search(
            search_nav(SearchChips::default()).0,
            ready,
        );
        search.update(&GlobalSearchSnapshot { query: String::new(), answered: 0, total: 0, rows: vec![] });
        assert!(!all_labels(search.widget()).iter().any(|t| t.starts_with("Source")), "no invented sources");

        let entries = migrate_entries(
            &[],
            None,
            MigrateEntriesNav { open_search: Rc::new(|_| {}), open_entry: Rc::new(|_| {}), load_cover: Rc::new(|_, _| {}) },
        );
        assert!(!all_labels(&entries).iter().any(|t| t.starts_with("Title")), "no invented entries");
    }

    fn migrate_sources_lists_the_snapshot_with_counts_and_marks_stubs() {
        let log = nav_log();
        let (a, b) = (Rc::clone(&log), Rc::clone(&log));
        let mut stub = Source::new(2, "Gone", "en");
        stub.is_stub = true;
        let snapshot = MigrateSourcesSnapshot {
            sources: vec![(Source::new(1, "Alpha", "en"), 12), (stub, 3), (Source::new(3, "Nolang", ""), 1)],
            empty_text: String::new(),
            error: None,
        };
        let page = migrate_sources(
            &snapshot,
            MigrateSourcesNav {
                open: Rc::new(move |id| a.borrow_mut().push(format!("open {id}"))),
                copy_id: Rc::new(move |id| b.borrow_mut().push(format!("copy {id}"))),
                open_help: Rc::new(|| {}),
                retry: Rc::new(|| {}),
            },
        );
        let text = all_labels(&page);
        for expected in ["Alpha", "12 in library", "Gone", "3 in library", "Nolang", "1 in library", &tr(MsgKey::not_installed)] {
            assert!(text.iter().any(|t| t == expected), "{expected} in {text:?}");
        }
        assert_eq!(text.iter().filter(|t| **t == tr(MsgKey::not_installed)).count(), 1, "only the stub is marked");
    }

    fn global_search_shows_each_state_of_a_source() {
        let source = |id, name: &str| Source::new(id, name, "en");
        let item = ListItem { title: "Berserk".into(), ..ListItem::default() };
        let (ready_tx, ready) = watch::channel(false);
        let page = global_search(search_nav(SearchChips { filter: SourceFilter::All, only_has_results: false }).0, ready);
        assert_eq!(
            page.widget().downcast_ref::<gtk::Stack>().and_then(|s| s.visible_child_name()).as_deref(),
            Some("spinner"),
            "a spinner until the registry is initialised"
        );
        ready_tx.send_replace(true);
        let context = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while page.widget().downcast_ref::<gtk::Stack>().and_then(|s| s.visible_child_name()).as_deref() != Some("content")
            && start.elapsed().as_secs() < 2
        {
            while context.iteration(false) {}
        }
        page.update(&GlobalSearchSnapshot {
            query: "ber".into(),
            answered: 2,
            total: 4,
            rows: vec![
                SearchRow { source: source(1, "Alpha"), state: SearchState::Results(vec![item]), origin: false },
                SearchRow { source: source(2, "Beta"), state: SearchState::Results(vec![]), origin: false },
                SearchRow { source: source(3, "Gamma"), state: SearchState::Error("offline".into()), origin: false },
                SearchRow { source: source(4, "Delta"), state: SearchState::Loading, origin: false },
            ],
        });
        let text = all_labels(page.widget());
        for expected in ["Alpha", "Berserk", "Beta", "Gamma", "offline", "Delta", &tr(MsgKey::no_results_found)] {
            assert!(text.iter().any(|t| t == expected), "{expected} in {text:?}");
        }
    }

    /// Everything the search page called back, as text.
    type SearchLog = Rc<RefCell<Vec<String>>>;

    fn search_nav(chips: SearchChips) -> (GlobalSearchNav, SearchLog) {
        let log: SearchLog = Rc::default();
        let (searched, opened, menus, chipped) = (Rc::clone(&log), Rc::clone(&log), Rc::clone(&log), Rc::clone(&log));
        let nav = GlobalSearchNav {
            search: Rc::new(move |q| searched.borrow_mut().push(format!("search {q}"))),
            open_source: Rc::new(|_, _| {}),
            open_entry: Rc::new(move |id, item| opened.borrow_mut().push(format!("open {id} {}", item.title))),
            menu: Rc::new(move |id, item: &ListItem| {
                menus.borrow_mut().push(format!("menu {id} {}", item.title));
                Vec::new()
            }),
            load_cover: Rc::new(|_, _| {}),
            chips,
            on_chips: Rc::new(move |c| chipped.borrow_mut().push(format!("chips {c:?}"))),
        };
        (nav, log)
    }

    fn descendants<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Vec<T> {
        fn walk<T: IsA<gtk::Widget>>(widget: &gtk::Widget, out: &mut Vec<T>) {
            if let Some(found) = widget.downcast_ref::<T>() {
                out.push(found.clone());
            }
            let mut child = widget.first_child();
            while let Some(w) = child {
                walk(&w, out);
                child = w.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(widget.upcast_ref(), &mut out);
        out
    }

    fn global_search_has_chips_a_progress_bar_that_hides_at_the_ends_and_96_px_cards() {
        let (nav, log) = search_nav(SearchChips::default());
        let page = global_search(nav, watch::channel(true).1);
        let item = ListItem { title: "Berserk".into(), ..ListItem::default() };
        let rows = vec![
            SearchRow { source: Source::new(1, "Alpha", "en"), state: SearchState::Results(vec![item]), origin: false },
            SearchRow { source: Source::new(2, "Beta", "en"), state: SearchState::Results(vec![]), origin: false },
            SearchRow { source: Source::new(3, "Gamma", "en"), state: SearchState::Loading, origin: true },
        ];
        let snapshot = |answered| GlobalSearchSnapshot { query: "ber".into(), answered, total: 3, rows: rows.clone() };

        let context = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while page.widget().downcast_ref::<gtk::Stack>().and_then(|s| s.visible_child_name()).as_deref() != Some("content")
            && start.elapsed().as_secs() < 2
        {
            while context.iteration(false) {}
        }
        page.update(&snapshot(0));
        assert!(!page.view.progress.is_visible(), "no source has answered yet");
        page.update(&snapshot(1));
        assert!(page.view.progress.is_visible(), "one of three");
        page.update(&snapshot(3));
        assert!(!page.view.progress.is_visible(), "all answered");

        let text = all_labels(page.widget());
        for expected in ["Alpha", "Beta", &format!("\u{25B6} Gamma")] {
            assert!(text.iter().any(|t| t == expected), "{expected} in {text:?}: every source is searched and the origin is listed");
        }

        // the search options: one menu button with Show only "Has results", and no chip row
        let toggles: Vec<gtk::ToggleButton> = descendants(page.widget());
        assert!(toggles.iter().all(|c| c.label().is_none()), "no chips: {:?}", toggles.iter().map(|c| c.label()).collect::<Vec<_>>());
        // the options button is a header action of the page, next to the shared search toggle
        let menus: Vec<gtk::MenuButton> = page.actions().iter().flat_map(|w| descendants::<gtk::MenuButton>(w)).collect();
        let menu = menus.iter().find(|b| b.tooltip_text().as_deref() == Some(search_options_label().as_str())).expect("the search options");
        let labels = crate::widgets::view_options::menu_labels(&menu.menu_model().expect("the menu"));
        assert_eq!(labels, ["Show only".to_owned(), tr(MsgKey::has_results)]);
        menu.activate_action("viewopt.has_results", None).expect("the Has results check");
        let text = all_labels(page.widget());
        assert!(text.iter().any(|t| t == "Alpha") && !text.iter().any(|t| t == "Beta" || t == "\u{25B6} Gamma"), "Has results keeps only rows with results: {text:?}");
        assert!(log.borrow().iter().all(|entry| entry.starts_with("chips ")), "a chip change never runs the search: {:?}", log.borrow());
        assert_eq!(log.borrow().len(), 1);

        // cards are 96 px wide and a right click asks for the card's menu
        let card = descendants::<gtk::Button>(page.widget()).into_iter().find(|b| b.has_css_class("flat") && b.child().is_some_and(|c| c.is::<gtk::Box>())).expect("card");
        assert!(descendants::<gtk::Widget>(&card).iter().any(|w| w.width_request() == SEARCH_CARD_WIDTH_PX), "the cover is 96 px wide");
        let controllers = card.observe_controllers();
        let right = (0..controllers.n_items())
            .filter_map(|i| controllers.item(i))
            .filter_map(|c| c.downcast::<gtk::GestureClick>().ok())
            .find(|c| c.button() == gtk::gdk::BUTTON_SECONDARY)
            .expect("right click");
        right.emit_by_name::<()>("pressed", &[&1_i32, &0.0_f64, &0.0_f64]);
        assert_eq!(log.borrow().last().map(String::as_str), Some("menu 1 Berserk"));
        card.emit_clicked();
        assert_eq!(log.borrow().last().map(String::as_str), Some("open 1 Berserk"));
    }

    fn migrate_entries_sorts_by_title_without_case_and_shows_a_load_error() {
        let entries = [
            MigrateEntry { id: 1, title: "zebra".into(), cover: None },
            MigrateEntry { id: 2, title: "Apple".into(), cover: None },
            MigrateEntry { id: 3, title: "banana".into(), cover: None },
        ];
        let page = migrate_entries(
            &entries,
            Some("could not load"),
            MigrateEntriesNav { open_search: Rc::new(|_| {}), open_entry: Rc::new(|_| {}), load_cover: Rc::new(|_, _| {}) },
        );
        let text = all_labels(&page);
        // each cover shows the title's initials until its picture loads
        assert_eq!(text, ["could not load", "A", "Apple", "B", "banana", "Z", "zebra"]);
    }

    // ----- browse_source -----

    use std::sync::Mutex;

    use kuro_prefs::store::PrefStore;
    use kuro_source::filter::Filter;
    use kuro_source::filter_list::FilterList;

    use crate::browse::listing::{ListedPage, LoadFuture, PageLoader};
    use crate::browse::primitives::EmptyAction;

    /// Serves the scripted results and records each request.
    struct Script {
        results: Mutex<Vec<Result<ListedPage, String>>>,
        requests: Mutex<Vec<(BrowseQuery, FilterList, u32)>>,
    }

    impl PageLoader for Script {
        fn load(&self, query: BrowseQuery, filters: FilterList, page: u32) -> LoadFuture {
            self.requests.lock().expect("lock").push((query, filters, page));
            let next = {
                let mut results = self.results.lock().expect("lock");
                if results.is_empty() { Ok(ListedPage { items: vec![ListItem { title: "x".into(), ..ListItem::default() }], has_next: false }) } else { results.remove(0) }
            };
            Box::pin(async move { next })
        }
    }

    #[allow(dead_code)] // kept for the shape of the type; not built or read yet
    struct Rig {
        page: SourcePage,
        script: Arc<Script>,
        prefs: Rc<super::super::prefs::BrowsePrefs>,
        log: Rc<RefCell<Vec<String>>>,
        _runtime: tokio::runtime::EnterGuard<'static>,
    }

    fn defaults() -> FilterList {
        FilterList(vec![Filter::check_box("Completed")])
    }

    fn source_rig(stub: bool, filters: FilterList, results: Vec<Result<ListedPage, String>>) -> Rig {
        source_rig_hiding(stub, filters, results, false)
    }

    fn source_rig_hiding(stub: bool, filters: FilterList, results: Vec<Result<ListedPage, String>>, hide: bool) -> Rig {
        let rt: &'static tokio::runtime::Runtime =
            Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().expect("runtime")));
        let entered = rt.enter();
        let script = Arc::new(Script { results: Mutex::new(results), requests: Mutex::default() });
        let prefs = Rc::new(super::super::prefs::BrowsePrefs::new(PrefStore::in_memory(), Kind::Manga));
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let (back, action, overflow, url) = (Rc::clone(&log), Rc::clone(&log), Rc::clone(&log), Rc::clone(&log));
        let (_tx, ready) = watch::channel(true);
        std::mem::forget(_tx);
        let page = browse_source(BrowseSourceDeps {
            supports_latest: true,
            configurable: false,
            prefs: Rc::clone(&prefs),
            listing: ListingDeps {
                loader: Arc::clone(&script) as Arc<dyn PageLoader>,
                prefs: Rc::clone(&prefs),
                ready,
                stub,
                is_local: false,
                source_id: 42,
                source_name: "Gone".into(),
                source_lang: "en".into(),
                load_cover: Rc::new(|_, _| {}),
                open_entry: Rc::new(|_| {}),
                menu: Rc::new(|_: &ListItem| Vec::new()),
                favorite_urls: None,
                on_action: Rc::new(move |a| action.borrow_mut().push(format!("action {a:?}"))),
            },
            filters: Rc::new(SourceFilters::new(filters)),
            favorites: None,
            hide_in_library: hide,
            initial_query: BrowseQuery::Popular,
            on_back: Rc::new(move || back.borrow_mut().push("back".into())),
            on_overflow: Rc::new(move |i| overflow.borrow_mut().push(format!("overflow {i:?}"))),
            open_url: Rc::new(move |u| url.borrow_mut().push(format!("url {u}"))),
            home_url: None,
        });
        Rig { page, script, prefs, log, _runtime: entered }
    }

    fn pump_until(mut done: impl FnMut() -> bool) -> bool {
        let context = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            while context.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        done()
    }

    fn all_buttons(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Button> {
        fn go(w: &gtk::Widget, out: &mut Vec<gtk::Button>) {
            if let Some(b) = w.downcast_ref::<gtk::Button>() {
                out.push(b.clone());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                go(&c, out);
                child = c.next_sibling();
            }
        }
        let mut out = Vec::new();
        go(root.upcast_ref(), &mut out);
        out
    }

    fn a_stub_source_shows_only_its_title_back_and_the_missing_message() {
        let rig = source_rig(true, defaults(), vec![]);
        let SourcePage::Stub(root) = &rig.page else { panic!("a stub source gets the missing-source page") };
        let text = all_labels(root);
        assert!(text.contains(&"Gone".to_owned()), "the title is the stub's name: {text:?}");
        let expected = trf(MsgKey::source_not_installed, &[("arg1", Arg::Str("Gone (EN)"))]);
        assert!(text.contains(&expected), "{text:?}");
        let mut searches = Vec::new();
        fn find(w: &gtk::Widget, out: &mut Vec<gtk::SearchEntry>) {
            if let Some(e) = w.downcast_ref::<gtk::SearchEntry>() {
                out.push(e.clone());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                find(&c, out);
                child = c.next_sibling();
            }
        }
        find(root.upcast_ref(), &mut searches);
        assert!(searches.is_empty(), "no search bar");
        let buttons = all_buttons(root);
        assert_eq!(buttons.len(), 1, "back is the only control: no chips, no filter, no retry");
        buttons[0].emit_clicked();
        assert_eq!(*rig.log.borrow(), ["back"]);
        assert!(rig.script.requests.lock().expect("lock").is_empty(), "and never loads");
        let label = stub_source_label("", "en", 42);
        assert_eq!(label, "42", "a blank name falls back to the id");
    }

    fn back_clears_the_search_text_first_and_then_leaves() {
        let rig = source_rig(false, FilterList::default(), vec![]);
        let SourcePage::Browse(page) = &rig.page else { panic!("browse page") };
        let back = all_buttons(&page.header.bar).into_iter().find(|b| b.icon_name().as_deref() == Some("go-previous-symbolic")).expect("back");
        back.emit_clicked();
        assert_eq!(*rig.log.borrow(), ["back"], "no search: Back leaves");
        rig.log.borrow_mut().clear();
        page.header.search.set_active(true);
        page.search.set_text("one piece");
        back.emit_clicked();
        assert!(rig.log.borrow().is_empty(), "an active search with text clears the text and stays");
        assert_eq!(page.search.text().as_str(), "");
        back.emit_clicked();
        assert_eq!(*rig.log.borrow(), ["back"], "the second Back leaves");
    }

    fn popular_latest_and_a_search_from_them_start_from_fresh_defaults() {
        let rig = source_rig(false, defaults(), vec![]);
        let SourcePage::Browse(page) = &rig.page else { panic!("browse page") };
        assert!(pump_until(|| !rig.script.requests.lock().expect("lock").is_empty()));
        // a search from Popular starts from the defaults
        page.search.set_text("berserk");
        page.search.emit_by_name::<()>("activate", &[]);
        assert!(pump_until(|| rig.script.requests.lock().expect("lock").len() == 2));
        let (query, filters, _) = rig.script.requests.lock().expect("lock")[1].clone();
        assert_eq!((query, filters), (BrowseQuery::Search("berserk".into()), defaults()));
        page.latest.as_ref().expect("latest").set_active(true);
        assert!(pump_until(|| rig.script.requests.lock().expect("lock").len() == 3));
        let (query, filters, _) = rig.script.requests.lock().expect("lock")[2].clone();
        assert_eq!((query, filters), (BrowseQuery::Latest, defaults()), "Latest takes a fresh clone of the defaults");
    }

    fn the_apply_button_of_the_sheet_searches_with_the_sheet_values() {
        let rig = source_rig(false, defaults(), vec![]);
        let SourcePage::Browse(page) = &rig.page else { panic!("browse page") };
        assert!(pump_until(|| !rig.script.requests.lock().expect("lock").is_empty()));
        page.search.set_text("naruto");
        let sheet = page.filter.as_ref().expect("filter chip").popover().expect("the filter sheet is the button's popover");
        let buttons = all_buttons(&sheet);
        let check = {
            fn go(w: &gtk::Widget) -> Option<gtk::CheckButton> {
                if let Some(c) = w.downcast_ref::<gtk::CheckButton>() {
                    return Some(c.clone());
                }
                let mut child = w.first_child();
                while let Some(c) = child {
                    if let Some(found) = go(&c) {
                        return Some(found);
                    }
                    child = c.next_sibling();
                }
                None
            }
            go(sheet.child().expect("child").upcast_ref()).expect("check box")
        };
        check.set_active(true);
        let apply = buttons.iter().find(|b| b.label().as_deref() == Some(tr(MsgKey::action_apply).as_str())).expect("Apply button");
        apply.emit_clicked();
        assert!(pump_until(|| rig.script.requests.lock().expect("lock").len() == 2));
        let (query, filters, _) = rig.script.requests.lock().expect("lock")[1].clone();
        assert_eq!(query, BrowseQuery::Search("naruto".into()), "the text in the search box goes with it");
        assert!(matches!(&filters.0[0], Filter::CheckBox { state: true, .. }), "and the sheet's values");
    }

    fn the_error_state_has_only_retry_and_retry_asks_again() {
        let rig = source_rig(false, FilterList::default(), vec![Err("offline".into())]);
        let SourcePage::Browse(page) = &rig.page else { panic!("browse page") };
        let error_buttons = || all_buttons(page.listing.widget()).into_iter().filter(|b| b.tooltip_text().is_some()).collect::<Vec<_>>();
        assert!(pump_until(|| !error_buttons().is_empty()));
        let buttons = error_buttons();
        assert_eq!(buttons.len(), 1, "retry only: no help page (T-14a-047), no web view button");
        assert_eq!(buttons[0].tooltip_text().as_deref(), Some(EmptyAction::Retry.label().as_str()));
        buttons[0].emit_clicked();
        assert!(pump_until(|| rig.script.requests.lock().expect("lock").len() == 2), "retry asks the first page again");
        assert!(rig.log.borrow().is_empty(), "retry is handled inside the listing");
    }

    /// Issue #7: selected covers swap the chips for the selection header and show the action bar; Escape clears them.
    fn selecting_covers_swaps_the_chips_for_the_selection_header_and_escape_clears_it() {
        let found = ListedPage {
            items: (1..=3).map(|id| ListItem { id, title: format!("t{id}"), url: format!("/{id}"), ..ListItem::default() }).collect(),
            has_next: false,
        };
        let rig = source_rig(false, FilterList::default(), vec![Ok(found)]);
        let SourcePage::Browse(page) = &rig.page else { panic!("browse page") };
        let chips = page.popular.parent().expect("the chip row");
        assert!(pump_until(|| {
            page.listing.select_all();
            page.listing.has_selection()
        }));
        let count = trf(MsgKey::library_selected_count, &[("count", Arg::Str("3"))]);
        assert!(all_labels(&page.root).contains(&count), "the header counts the selection");
        assert!(!chips.is_visible(), "the selection header takes the chips' place");
        let bar = descendants::<gtk::Revealer>(&page.root).into_iter().find(|r| r.reveals_child()).expect("the action bar shows");
        assert!(all_buttons(&bar).iter().all(|b| !b.is_visible()), "a page without the library store offers no bulk action");
        let none = gtk::gdk::ModifierType::empty();
        let all = page.root.observe_controllers();
        let keys: Vec<gtk::EventControllerKey> = (0..all.n_items()).filter_map(|i| all.item(i)).filter_map(|c| c.downcast::<gtk::EventControllerKey>().ok()).collect();
        assert!(keys.iter().any(|k| k.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Escape, &0_u32, &none])), "Escape is taken");
        assert!(!page.listing.has_selection(), "Escape clears the selection");
        assert!(chips.is_visible() && !bar.reveals_child());
    }

    fn escape_closes_an_open_search_first_and_otherwise_goes_on() {
        let (_tx, ready) = watch::channel(true);
        let page = global_search(search_nav(SearchChips::default()).0, ready);
        let entry = descendants::<gtk::SearchEntry>(page.widget()).into_iter().next().expect("entry");
        entry.set_text("one piece");
        let controller = |widget: &gtk::Widget| {
            let all = widget.observe_controllers();
            (0..all.n_items()).filter_map(|i| all.item(i)).filter_map(|c| c.downcast::<gtk::EventControllerKey>().ok()).count()
        };
        // the entry sits in the shared search bar at the top of the page body; the body carries the Escape step and the bar's
        // key capture
        let body = entry.ancestor(gtk::SearchBar::static_type()).and_then(|bar| bar.parent()).expect("body");
        assert_eq!(controller(&body), 2, "the page carries the Escape step and the search bar's key capture");
        let all = body.observe_controllers();
        let keys = (0..all.n_items()).filter_map(|i| all.item(i)).find_map(|c| c.downcast::<gtk::EventControllerKey>().ok()).expect("controller");
        let none = gtk::gdk::ModifierType::empty();
        assert!(keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Escape, &0_u32, &none]), "the first Escape closes the search");
        assert_eq!(entry.text().as_str(), "", "the typed text is gone");
        assert!(!keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Escape, &0_u32, &none]), "the second one goes on, to leave the page");
    }

    pub(crate) fn smoke() {
        escape_closes_an_open_search_first_and_otherwise_goes_on();
        selecting_covers_swaps_the_chips_for_the_selection_header_and_escape_clears_it();
        a_stub_source_shows_only_its_title_back_and_the_missing_message();
        back_clears_the_search_text_first_and_then_leaves();
        popular_latest_and_a_search_from_them_start_from_fresh_defaults();
        the_apply_button_of_the_sheet_searches_with_the_sheet_values();
        the_error_state_has_only_retry_and_retry_asks_again();
        no_sample_rows_in_an_empty_snapshot();
        migrate_sources_lists_the_snapshot_with_counts_and_marks_stubs();
        global_search_shows_each_state_of_a_source();
        global_search_has_chips_a_progress_bar_that_hides_at_the_ends_and_96_px_cards();
        migrate_entries_sorts_by_title_without_case_and_shows_a_load_error();
    }
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    fn row(id: i64, origin: bool, state: SearchState) -> SearchRow {
        let source = Source::new(id, &format!("S{id}"), "en");
        SearchRow { source, state, origin }
    }

    fn found() -> SearchState {
        SearchState::Results(vec![ListItem::default()])
    }

    fn ids(rows: &[&SearchRow]) -> Vec<i64> {
        rows.iter().map(|r| r.source.id).collect()
    }

    #[test]
    fn a_cover_menu_opens_the_entry_and_offers_the_browser_only_with_a_web_address() {
        let opened: Rc<RefCell<Vec<String>>> = Rc::default();
        let (seen, urls) = (Rc::clone(&opened), Rc::clone(&opened));
        let open: Rc<dyn Fn(&ListItem)> = Rc::new(move |item: &ListItem| seen.borrow_mut().push(item.title.clone()));
        let open_url: Rc<dyn Fn(&str)> = Rc::new(move |url: &str| urls.borrow_mut().push(url.to_owned()));
        let item = ListItem { title: "Berserk".into(), ..ListItem::default() };
        let plain = cover_menu(&item, Rc::clone(&open), None, None, Rc::clone(&open_url));
        assert_eq!(plain.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), [super::super::texts::OPEN], "no library store, no web address");
        let menu = cover_menu(&item, open, None, Some("https://a.test/b".into()), open_url);
        assert_eq!(menu.len(), 2);
        assert_eq!(menu[1].label, tr(MsgKey::action_open_in_browser));
        (menu[0].run)();
        (menu[1].run)();
        assert_eq!(*opened.borrow(), ["Berserk", "https://a.test/b"]);
    }

    #[test]
    fn the_progress_bar_shows_only_from_one_answer_until_all_have_answered() {
        assert!(!progress_visible(0, 5), "nobody has answered");
        assert!(progress_visible(1, 5));
        assert!(progress_visible(4, 5));
        assert!(!progress_visible(5, 5), "everybody has");
        assert!(!progress_visible(0, 0));
    }

    #[test]
    fn the_chips_filter_the_rows_they_are_given() {
        let rows = vec![
            row(1, false, found()),
            row(2, false, found()),
            row(3, true, SearchState::Results(vec![])),
            row(4, false, SearchState::Loading),
        ];
        let all = SearchChips { filter: SourceFilter::All, only_has_results: false };
        assert_eq!(ids(&visible_rows(&rows, all)), [1, 2, 3, 4]);
        let results = SearchChips { filter: SourceFilter::All, only_has_results: true };
        assert_eq!(ids(&visible_rows(&rows, results)), [1, 2], "an empty result or a loading row has none");
        assert_eq!(SearchChips::default().filter, SourceFilter::All, "every source is searched by default");
    }

    #[test]
    fn the_origin_source_carries_the_arrow_prefix_and_a_card_is_96_px() {
        assert_eq!(row_title(&row(1, true, found())), "\u{25B6} S1");
        assert_eq!(row_title(&row(2, false, found())), "S2");
        assert_eq!(SEARCH_CARD_WIDTH_PX, 96);
    }
}
