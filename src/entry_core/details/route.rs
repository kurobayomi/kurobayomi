//! The anime details route: a thin GTK page over `EntryModel` (state) and `EntryRunner` (the store). It shows a
//! loading state until the first read arrives, the page when the model is `Success`, and a "not found" state with a
//! Back button when the row is missing.
//!
//! Layout: at or above `layout::TWO_PANE_MIN_WIDTH` the info box, description and actions sit in a left pane and the
//! item header and episode list in the right one; below it everything is one column and the info block scrolls with
//! the list (it is the first row of the list view, so 3000 episodes still cost 30 widgets). The airing row is pinned
//! above both layouts, full width. The FAB floats over the list and collapses to its icon on a forward scroll.
//! `ui::screen("anime_entry_details")` resolves here through `build`, and `open(id)` pushes the page of one anime.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Label, Orientation as O, glib};
use kuro_data::anime_relation::AnimeRelationGroup;
use kuro_data::entry::SeasonEntry;
use kuro_db::handle::Db;
use kuro_db::codec::FetchType;
use kuro_domain::entries::defaults::{set_item_settings_default, set_season_settings_default};
use kuro_domain::entries::flags::EntryFlags;
use kuro_domain::entries::season_display::SeasonDisplayMode;
use kuro_platform::kind::Kind;
use kuro_downloads::DownloadState;
use kuro_i18n::{MsgKey, tr};
use kuro_prefs::store::PrefStore;

use crate::entry_core::constants::{FALLBACK_WIDTH, LIST_MAX_WIDTH, left_pane_width};

/// The width the page is laid out for before it has one: the window's, so a narrow window never gets the two-pane layout
/// for a frame (its minimum width would push the window wider); the app's default size before there is a window.
fn first_width() -> i32 {
    crate::ui::window_width().unwrap_or(FALLBACK_WIDTH)
}

use super::actions::{OpenPlan, continue_plan, tap_plan};
use super::airing::{Airing, format_airing, visible as airing_visible};
use super::bottom::{BottomAction, MenuContext, Selected, bottom_actions};
use super::download_backend::EntryDownloads;
use super::dispatch::{bottom_cmds, menu_cmds};
use super::details_error::{DetailsError, required};
use crate::entry_core::download_menu::{DownloadAmount, menu_visible};
use super::intro_dialog;
use crate::entry_core::overflow_menu::{self, Entry as OverflowEntry, Item as OverflowItem};
use super::viewer;
use super::dialog_state::{EntryDialog, DialogSlot};
use crate::duplicate_dialog::{self, DuplicateChoice};
use super::season_grid::{Layout, SeasonGrid};
use super::season_pipeline::process_seasons;
use super::season_tile::{SeasonFacts, SeasonItem, continue_episode, season_facts};
use super::set_default::{self, Scope};
use crate::entry_core::selection::RowClick;
use super::info::header_model;
use super::layout::{bottom_bar_width, two_pane};
use super::model::{EntryModel, EntryLoadContext};
use super::refresh::refresh_snack;
use super::remote::{LiveSources, RemoteAnimeBackend};
use super::runner::{self, EntryBackend, EntryCmd, EntryEvent, EntryRunner};
use super::store::EntryStore;
use super::related_view;
use crate::data;
use crate::entry_core::dialog::CategoryDialog;
use crate::entry_core::handoff::{self, MigrateRequest};
use crate::entry_core::header::SearchRequest;
use crate::entry_core::action_row::interval_click_opens_dialog;
use crate::entry_core::action_row_view::{ActionRow, RowState};
use crate::entry_core::fetch_interval_dialog;
use crate::entry_core::header_view::Handlers;
use crate::entry_core::message::{Notice, UserMessage, present};
use crate::entry_core::info_block::InfoBlock;
use crate::entry_core::info_pane::TwoPane;
use crate::entry_core::list_model::RowAction;
use crate::entry_core::flags::scanlator_filter_active;
use crate::entry_core::selection::Command;
use crate::entry_core::list_options::{self, Spec};
use crate::entry_core::scanlator_sheet;
use crate::entry_core::list_view::{self, Callbacks, ItemList, Press};
use crate::entry_core::text::{Expansion, description};
use crate::entry_core::page_shell::{self, detach};
use crate::ui;
use crate::widgets::download_button::Request;
use crate::widgets::view_options::{Pick, ViewOptions};

/// The skip-intro length a manual skip starts from (the Default intro length preference is cut, so this is fixed).
const DEFAULT_INTRO_LENGTH: i32 = 85;


thread_local! {
    static TARGET: Cell<Option<i64>> = const { Cell::new(None) };
}

/// Pushes the details page of one anime.
pub fn open(entry_id: i64) {
    open_kind(Kind::Anime, entry_id);
}

/// Pushes the details page of one entry of either kind.
pub fn open_kind(kind: Kind, entry_id: i64) {
    ui::push(&page_for(kind, entry_id));
}

/// The page for the `anime_entry_details` screen id. Without a target there is no row to show, so it is the page of a
/// row that does not exist: the "not found" state with a Back button.
pub fn build() -> GBox {
    page_for(Kind::Anime, TARGET.with(Cell::take).unwrap_or(NO_ENTRY))
}

/// The page for the `manga_entry_details` screen id: no row to show, so the "not found" page of a manga.
pub fn build_manga() -> GBox {
    page_for(Kind::Manga, NO_ENTRY)
}

/// An id no row has (ids start at 1): `open(NO_ENTRY)` shows the "not found" page, for a caller that has no row to name.
pub const NO_ENTRY: i64 = 0;

fn page_for(kind: Kind, entry_id: i64) -> GBox {
    crate::entry_core::tracking::session::refresh_from_host();
    let app = data::app();
    let db = app.db.clone();
    let prefs = app.base.store().clone();
    let show_airing = crate::misc::diet_defaults::ENTRY_SHOW_AIRING_TIME;
    let ctx = EntryLoadContext {
        from_source: false,
        downloaded_only: app.base.downloaded_only.get(),
        ..Default::default()
    };
    let handle = tokio::runtime::Handle::current();
    // the live sources of this build; an id it does not know reports that the source is not installed
    let backend: Arc<dyn EntryBackend> = match kind {
        Kind::Anime => Arc::new(
            RemoteAnimeBackend::new(Arc::clone(&db), &prefs, Arc::new(LiveSources::new(crate::screens::anime_sources())), handle.clone()).with_downloads(EntryDownloads::new()),
        ),
        Kind::Manga => Arc::new(crate::entry_core::backend::SourceBackend::live(Arc::clone(&db), &prefs)),
    };
    let runner = Rc::new(runner::spawn(&handle, EntryStore::new(Arc::clone(&db), kind), prefs.clone(), Arc::clone(&backend), ctx));
    runner.send(EntryCmd::Load { entry_id });
    assemble(runner, show_airing, Env { prefs, backend, db, kind })
}

/// What the page needs from the app besides its runner: the preference store, the backend the runner uses (for the
/// player choices that happen on the GTK thread) and the database.
struct Env {
    prefs: PrefStore,
    backend: Arc<dyn EntryBackend>,
    db: Arc<Db>,
    kind: Kind,
}

/// Everything the widgets and handlers share. Handlers hold `Weak<View>`; the event pump holds the only strong
/// reference, so the page frees itself when the pump ends.
struct View {
    model: RefCell<Option<EntryModel>>,
    /// The first read arrived (`Loaded`), with or without a row.
    loaded: Cell<bool>,
    runner: Rc<EntryRunner>,
    env: Env,
    show_airing: bool,
    refresh_button: Button,
    /// "Download all", in the head of the list (not a menu): queues every unseen or unread item.
    download_all: Button,
    /// The page actions in the top bar; each hidden while the entry on show does not offer it, and all of them in the
    /// phone layout, where the ⋮ menu below stands in for them.
    overflow_buttons: Vec<(OverflowItem, Button)>,
    /// The ⋮ menu of the phone layout; hidden in a wide window or while it has nothing to offer.
    overflow_button: Option<gtk::MenuButton>,
    /// The window is in the phone layout (narrower than `shell::layout::WIDE_MIN_WIDTH_PX`).
    actions_narrow: Cell<bool>,
    bottom: gtk::Revealer,
    bottom_row: GBox,
    stack: gtk::Stack,
    title: Option<Label>,
    header_bar: Option<gtk::HeaderBar>,
    toolbar: Vec<gtk::Widget>,
    list: Rc<ItemList>,
    scroller: gtk::ScrolledWindow,
    /// The info block (header, action row, description, related rows): the left pane, or the first row of the list.
    info: InfoBlock,
    /// The two-pane body: the left pane with the info block, and the list.
    panes: TwoPane,
    /// The item header, the refresh spinner and the empty line: the head of the list.
    list_head: GBox,
    count: Label,
    missing: Label,
    spinner: gtk::Spinner,
    empty: GBox,
    empty_label: Label,
    airing_row: GBox,
    airing_title: Label,
    airing_time: Label,
    fab: Button,
    fab_label: Label,
    scroll: Cell<(f64, bool)>,
    bar: gtk::Revealer,
    bar_count: Label,
    /// The list options menu of the top bar, the two lists it can describe, and which one it shows (`Some(true)`: the
    /// season grid of a `Seasons` entry).
    options: ViewOptions,
    episode_spec: Spec,
    season_spec: Spec,
    options_seasons: Cell<Option<bool>>,
    /// The one dialog slot (T-39b-030) and the window that shows it. `dialog_gen` names the request on screen, so the
    /// close of a window that a newer request replaced is ignored.
    dialogs: RefCell<DialogSlot>,
    dialog_window: RefCell<Option<gtk::Window>>,
    dialog_gen: Cell<u64>,
    seasons: SeasonGrid,
    /// What the season grid shows: its cells, columns, whether the size is fixed, and the width bucket.
    seasons_shown: RefCell<Option<(Vec<SeasonItem>, Layout)>>,
    two_pane: Cell<Option<bool>>,
    width: Cell<i32>,
    expansion: Rc<Cell<Expansion>>,
    /// The related groups the runner published, the box they are drawn in (under the description) and what it shows.
    related: RefCell<Vec<AnimeRelationGroup>>,
    related_host: GBox,
    related_shown: RefCell<Option<(Vec<AnimeRelationGroup>, kuro_prefs::library::LibraryDisplayMode)>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The preference of the player the taps use (`pref_always_use_external_player`).
fn always_external(prefs: &PrefStore) -> bool {
    prefs.pref::<bool>("pref_always_use_external_player", false).get()
}

/// The page actions in the order the top bar shows them (user, 2026-10-25): Edit categories first, Change intro length
/// last, next to the list options.
const ACTIONS: [OverflowItem; 6] = [
    OverflowItem::EditCategories,
    OverflowItem::Migrate,
    OverflowItem::Share,
    OverflowItem::OpenInBrowser,
    OverflowItem::Settings,
    OverflowItem::ChangeIntro,
];

/// The page actions as header buttons (user, 2026-10-25), in [`ACTIONS`] order. Each shows only while
/// `overflow_menu::items` offers it for the entry on show, and only in a wide window (`View::sync_actions`).
/// Whether the main window is in the phone layout now (a page built before its first frame lays out for it).
fn narrow_window() -> bool {
    ui::window_width().is_some_and(|w| w < crate::shell::layout::WIDE_MIN_WIDTH_PX)
}

fn overflow_buttons(late: &Rc<RefCell<Weak<View>>>) -> Vec<(OverflowItem, Button)> {
    ACTIONS
        .into_iter()
        .map(|item| {
            let button = crate::ui::icon_button(overflow_menu::icon(item), &overflow_menu::label(item));
            button.set_visible(false);
            let run = Rc::clone(late);
            button.connect_clicked(move |_| {
                if let Some(view) = run.borrow().upgrade() {
                    view.run_overflow(item);
                }
            });
            (item, button)
        })
        .collect()
}

/// The same page actions as a ⋮ menu, for the phone layout, where the buttons do not fit (user, 2026-10-25). Its entries
/// read the entry on show each time the menu opens, so the items follow it.
fn overflow_sections(late: &Rc<RefCell<Weak<View>>>) -> Vec<Vec<crate::ui::MenuEntry>> {
    let entry = |item: OverflowItem| {
        let (run, shown) = (Rc::clone(late), Rc::clone(late));
        crate::ui::MenuEntry::new(overflow_menu::label(item), move || {
            if let Some(view) = run.borrow().upgrade() {
                view.run_overflow(item);
            }
        })
        .when(move || shown.borrow().upgrade().is_some_and(|view| view.overflow_items().contains(&item)))
    };
    vec![ACTIONS.into_iter().map(entry).collect()]
}

fn assemble(runner: Rc<EntryRunner>, show_airing: bool, env: Env) -> GBox {
    let kind = env.kind;
    let stack = page_shell::state_stack();

    // the list head: the item header row, the refresh spinner and the empty line
    let page_shell::ItemHeaderRow { row: header_row, count, missing, download_all } = page_shell::item_header_row();
    // the refresh spinner of the list head: inline size, spinning only while a refresh runs
    let spinner = ui::loading(ui::LoadSize::Inline);
    spinner.set_spinning(false);
    spinner.set_halign(Align::Center);
    let weak_view: Rc<RefCell<Weak<View>>> = Rc::default();
    // nothing listed: the list says so, with Retry (a refresh that finds nothing raises no notification)
    let (empty, empty_label) = list_view::empty_list(&tr(crate::entry_core::wording::none_found(kind)), {
        let weak = Rc::clone(&weak_view);
        move || {
            if let Some(view) = weak.borrow().upgrade() {
                view.refresh();
            }
        }
    });
    // a `Seasons` entry lists its seasons in a grid under the head; the grid is hidden for an episode list. A tile's
    // continue button plays the next unseen episode of that season.
    let seasons = SeasonGrid::new(
        Rc::new(open),
        Some(Rc::new({
            let weak = Rc::clone(&weak_view);
            move |season_id: i64| {
                if let Some(view) = weak.borrow().upgrade() {
                    view.continue_season(season_id);
                }
            }
        })),
    );
    let list_head = GBox::new(O::Vertical, 4);
    list_head.append(&header_row);
    list_head.append(&spinner);
    list_head.append(&empty);
    list_head.append(&seasons.widget);

    // the action row (T-39a-026): library, interval (the dialog of T-38a-028), tracking and Browser. Browser shows
    // only for an HTTP source (`RowState::webview`) and opens the title's page in the system browser.
    let action_row = ActionRow::new(
        Rc::new({
            let weak = Rc::clone(&weak_view);
            move || {
                if let Some(view) = weak.borrow().upgrade() {
                    view.open_interval();
                }
            }
        }),
        Some(Rc::new({
            let weak = Rc::clone(&weak_view);
            move || {
                if let Some(view) = weak.borrow().upgrade() {
                    view.tracking_pressed();
                }
            }
        })),
        Some(Rc::new({
            let weak = Rc::clone(&weak_view);
            move || {
                if let Some(view) = weak.borrow().upgrade() {
                    view.browser_pressed();
                }
            }
        })),
    );
    let library = action_row.library.clone();
    let list = ItemList::new(kind, Callbacks {
        on_row: {
            let weak = Rc::clone(&weak_view);
            Rc::new(move |id: i64, press: Press| {
                let view = weak.borrow().upgrade();
                if let Some(view) = view {
                    view.row_pressed(id, false, press.alt, press.mods);
                }
            })
        },
        on_menu: {
            let weak = Rc::clone(&weak_view);
            Rc::new(move |id: i64, action: RowAction| {
                let view = weak.borrow().upgrade();
                match (view, action) {
                    (Some(view), RowAction::Select) => view.row_pressed(id, true, false, crate::entry_core::selection::Mods::default()),
                    (Some(view), RowAction::Reload) => {
                        crate::cache_limit::forget_item(kind, id);
                        view.row_pressed(id, false, false, crate::entry_core::selection::Mods::default());
                    }
                    (Some(view), action) => view.menu_pressed(id, action),
                    (None, _) => {}
                }
            })
        },
        on_download: {
            let weak = Rc::clone(&weak_view);
            Rc::new(move |id: i64, request: Request| {
                if let Some(view) = weak.borrow().upgrade() {
                    view.download_pressed(id, request);
                }
            })
        },
        on_summary: {
            let weak = Rc::clone(&weak_view);
            Rc::new(move |id: i64| {
                if let Some(view) = weak.borrow().upgrade() {
                    view.summary_pressed(id);
                }
            })
        },
    });
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list.widget)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    // a drag beside the rows selects every row it touches (issue #7)
    let drag = crate::widgets::drag_select::DragSelect::new(&scroller);
    drag.attach(&list.widget, list.drag_hooks());
    let panes = TwoPane::new(drag.widget().upcast_ref());
    let info = InfoBlock::new(action_row);
    // the related rows come with the description (`ExpandableAnimeDescription`); there are none until the runner has some
    let related_host = GBox::new(O::Vertical, 0);
    related_host.set_visible(false);
    info.root.append(&related_host);

    // the airing row spans both panes
    let airing_title = Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build();
    let airing_time = Label::builder().xalign(1.0).wrap(true).css_classes(["dim-label"]).build();
    let airing_row = GBox::new(O::Horizontal, 12);
    airing_row.set_margin_start(16);
    airing_row.set_margin_end(16);
    airing_row.set_margin_top(8);
    airing_row.set_margin_bottom(8);
    airing_row.set_visible(false);
    airing_row.append(&airing_title);
    airing_row.append(&airing_time);

    let (fab, fab_label) = page_shell::fab_button();
    fab.set_halign(Align::End);
    fab.set_valign(Align::End);
    fab.set_margin_end(16);
    fab.set_margin_bottom(16);
    fab.set_visible(false);

    let column = GBox::new(O::Vertical, 0);
    column.append(&airing_row);
    column.append(&panes.root);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&column));
    overlay.add_overlay(&fab);
    overlay.set_vexpand(true);

    // the selection header, the one of Library and Updates: Cancel, the count, Select all. The view does not exist yet, so the
    // buttons reach it through `late`, set once it is built. Ctrl+I still inverts the selection.
    let late: Rc<RefCell<Weak<View>>> = Rc::default();
    let bar_count = Label::new(None);
    let selecting = |edit: fn(&mut EntryModel, &[i64], &[i64])| {
        let late = Rc::clone(&late);
        move || {
            if let Some(view) = late.borrow().upgrade() {
                view.edit_selection(edit);
            }
        }
    };
    let bar_row = crate::ui::selection_header(&bar_count, selecting(|m, _, _| m.selection.clear()), selecting(|m, _, full| m.selection.set_all(full, true)));
    let bar = gtk::Revealer::builder().child(&bar_row).transition_type(gtk::RevealerTransitionType::SlideDown).build();

    // the bottom action bar of the selection (behaviour 1.4): its rows follow the selected episodes, it is hidden
    // when nothing is selected, and it is as wide as the window narrow and half of it wide
    let bottom_row = GBox::new(O::Horizontal, 6);
    bottom_row.add_css_class("toolbar");
    bottom_row.set_halign(Align::Center);
    // the row scrolls sideways when its labels do not fit, so a narrow window is never pushed wider than itself
    let bottom_scroll = gtk::ScrolledWindow::builder().child(&bottom_row).vscrollbar_policy(gtk::PolicyType::Never).hscrollbar_policy(gtk::PolicyType::Automatic).propagate_natural_height(true).build();
    let bottom = gtk::Revealer::builder().child(&bottom_scroll).transition_type(gtk::RevealerTransitionType::SlideUp).build();

    let ready = GBox::new(O::Vertical, 0);
    ready.append(&bar);
    ready.append(&overlay);
    ready.append(&bottom);
    stack.add_named(&ready, Some("ready"));

    let (episode_spec, season_spec) = (list_options::item_spec(kind), list_options::season_spec());
    let options = ViewOptions::new(&list_options::tooltip(), &episode_spec.sections());
    let refresh_button = crate::ui::refresh_button(&tr(MsgKey::action_webview_refresh));
    // the header row every page shares (`ui::HeaderActions`); the page actions are buttons showing the items
    // `overflow_menu::items` gives for this entry and its source (T-39a-075), and Download all is in the head of the list
    let overflow_buttons = overflow_buttons(&late);
    let toolbar = crate::ui::HeaderActions {
        refresh: Some(refresh_button.clone().upcast()),
        extra: overflow_buttons.iter().map(|(_, button)| button.clone().upcast()).collect(),
        options: Some(options.widget.clone().upcast()),
        menu: overflow_sections(&late),
        ..Default::default()
    }
    .widgets();
    let overflow_button = toolbar.iter().find_map(|w| w.downcast_ref::<gtk::MenuButton>().filter(|b| b.has_css_class(ui::MORE_MENU)).cloned());
    let page = ui::page("", &stack, toolbar.clone(), false);
    ui::tag(&page, if kind.is_anime() { "anime_entry_details" } else { "manga_entry_details" });
    let (header, title, back) = page_shell::header_with_back(&page);

    let view = Rc::new(View {
        model: RefCell::new(None),
        loaded: Cell::new(false),
        runner: Rc::clone(&runner),
        env,
        show_airing,
        refresh_button: refresh_button.clone(),
        download_all: download_all.clone(),
        overflow_buttons,
        overflow_button,
        actions_narrow: Cell::new(narrow_window()),
        bottom,
        bottom_row,
        stack,
        title,
        header_bar: header.clone(),
        toolbar,
        list: Rc::clone(&list),
        scroller: scroller.clone(),
        info,
        panes,
        list_head,
        count,
        missing,
        spinner,
        empty,
        empty_label,
        airing_row,
        airing_title,
        airing_time,
        fab: fab.clone(),
        fab_label,
        scroll: Cell::new((0.0, false)),
        bar,
        bar_count,
        options: options.clone(),
        episode_spec,
        season_spec,
        options_seasons: Cell::new(None),
        dialogs: RefCell::default(),
        dialog_window: RefCell::new(None),
        dialog_gen: Cell::new(0),
        seasons,
        seasons_shown: RefCell::new(None),
        two_pane: Cell::new(None),
        width: Cell::new(first_width()),
        expansion: Rc::new(Cell::new(Expansion::new(false, false))),
        related: RefCell::new(Vec::new()),
        related_host,
        related_shown: RefCell::new(None),
    });
    let weak = Rc::downgrade(&view);
    *weak_view.borrow_mut() = weak.clone();
    view.apply_width(first_width());
    // auto-track binds in the background after an add: the Tracking button reads the count again when it lands
    crate::updates::track_binding::watch_binds({
        let weak = weak.clone();
        move |bound_id| {
            let Some(view) = weak.upgrade() else { return false };
            let loaded = view.model.borrow().as_ref().map(|m| (m.entry.id, m.entry.source));
            if let Some((id, source_id)) = loaded.filter(|(id, _)| *id == bound_id) {
                view.runner.send(EntryCmd::ReadTracking { entry_id: id, source_id });
            }
            true
        }
    });

    let on_view = |f: fn(&Rc<View>)| {
        let weak = weak.clone();
        move |_: &Button| {
            if let Some(view) = weak.upgrade() {
                f(&view);
            }
        }
    };
    options.connect({
        let weak = weak.clone();
        move |pick| {
            if let Some(view) = weak.upgrade() {
                view.list_option(pick);
            }
        }
    });
    refresh_button.connect_clicked(on_view(View::refresh));
    library.connect(
        {
            let weak = weak.clone();
            move || {
                if let Some(view) = weak.upgrade() {
                    view.toggle_library();
                }
            }
        },
        {
            let weak = weak.clone();
            move || {
                if let Some(view) = weak.upgrade() {
                    view.open_categories();
                }
            }
        },
    );
    download_all.connect_clicked(on_view(|v| v.run_download(DownloadAmount::AllUnread)));
    *late.borrow_mut() = weak.clone();
    // the page actions follow the window between buttons and the ⋮ menu (`View::sync_actions`)
    page.add_tick_callback({
        let weak = weak.clone();
        move |page, _| {
            let width = page.root().map(|root| root.width()).filter(|w| *w > 0);
            if let (Some(view), Some(width)) = (weak.upgrade(), width) {
                let narrow = width < crate::shell::layout::WIDE_MIN_WIDTH_PX;
                if view.actions_narrow.replace(narrow) != narrow {
                    view.sync_actions();
                }
            }
            glib::ControlFlow::Continue
        }
    });
    fab.connect_clicked(on_view(View::open_next));
    back.connect_clicked(on_view(|v| {
        if !v.back_pressed() {
            ui::pop();
        }
    }));
    scroller.vadjustment().connect_value_changed({
        let weak = weak.clone();
        move |_| {
            if let Some(view) = weak.upgrade() {
                view.update_fab();
            }
        }
    });
    page.add_controller(key_controller(&weak));
    // the layout follows the page width; the tick callback goes away with the widget
    page.add_tick_callback({
        let weak = weak.clone();
        move |page, _| {
            if let Some(view) = weak.upgrade()
                && page.width() > 0
            {
                view.apply_width(page.width());
            }
            glib::ControlFlow::Continue
        }
    });
    // the countdown re-derives from the absolute target each second and stops with the page
    glib::timeout_add_local(Duration::from_secs(1), {
        let weak = weak.clone();
        move || match weak.upgrade() {
            Some(view) => {
                view.tick_airing();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        }
    });

    // the queue state of the episodes follows the download queue live
    crate::downloads::live::follow(&page, {
        let weak = weak.clone();
        move |live| {
            if live.kind == kind
                && let Some(view) = weak.upgrade()
            {
                view.live_changed(live.id, live.state, live.percent);
            }
        }
    });

    // the pump owns the only strong reference to the view
    let events = runner.events();
    glib::spawn_future_local(async move {
        while let Ok(first) = events.recv().await {
            view.on_event(first);
            while let Ok(next) = events.try_recv() {
                view.on_event(next);
            }
            view.render();
        }
    });
    // a page that leaves the stack stops its runner: subscriptions are dropped, a fetch is cancelled, writes finish
    page.connect_destroy(move |_| runner.send(EntryCmd::Shutdown));
    page
}

/// Escape clears the selection first and then goes back; Space flips the focused row; Ctrl+A and Ctrl+I select all and invert.
fn key_controller(weak: &Weak<View>) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    let weak = weak.clone();
    keys.connect_key_pressed(move |_, key, _, state| {
        let Some(view) = weak.upgrade() else { return glib::Propagation::Proceed };
        let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let selecting = view.model.borrow().as_ref().is_some_and(|m| m.selection.is_active());
        match key {
            gtk::gdk::Key::Escape => {
                if !view.back_pressed() {
                    ui::pop();
                }
                glib::Propagation::Stop
            }
            // F5 refreshes like the header button (T-39a-038); a selection disables it
            gtk::gdk::Key::F5 if !selecting => {
                view.refresh();
                glib::Propagation::Stop
            }
            // Space flips the focused row (Shift extends a range), as the long press and the tap do
            gtk::gdk::Key::space if view.list.focused_id().is_some() => {
                let id = view.list.focused_id().unwrap_or_default();
                let command = if state.contains(gtk::gdk::ModifierType::SHIFT_MASK) { Command::LongPress } else { Command::Tap };
                view.edit_selection(|m, view, _| m.selection.toggle_row(view, id, command));
                view.list.focus_row(id);
                glib::Propagation::Stop
            }
            gtk::gdk::Key::a | gtk::gdk::Key::A if ctrl => {
                view.edit_selection(|m, _, full| m.selection.set_all(full, true));
                glib::Propagation::Stop
            }
            gtk::gdk::Key::i | gtk::gdk::Key::I if ctrl => {
                view.edit_selection(|m, _, full| m.selection.invert(full));
                glib::Propagation::Stop
            }
            _ if super::super::shortcut_help_view::is_help_key(key) => {
                super::super::shortcut_help_view::open();
                glib::Propagation::Stop
            }
            _ if view.header_key_pressed(key, state) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        }
    });
    keys
}

impl View {
    /// F (the list options menu), R (refresh), Ctrl+M (migrate, a favorite only) and W (the browser, an HTTP source only), the
    /// header keys of `shortcuts::header_key` (T-38a-046). `true` when the key was one of them and ran.
    fn header_key_pressed(self: &Rc<Self>, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        use crate::entry_core::selection::Mods;
        use crate::entry_core::shortcuts::{HeaderKey, header_key};
        if state.intersects(gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SUPER_MASK) {
            return false;
        }
        let Some(letter) = key.to_lower().to_unicode() else { return false };
        let (favorite, http_source, selecting) = {
            let model = self.model.borrow();
            let Some(m) = model.as_ref().filter(|m| m.accepts_dispatch()) else { return false };
            (m.entry.favorite, self.env.backend.is_http_source(m.entry.source), m.selection.is_active())
        };
        let mods = Mods { shift: state.contains(gtk::gdk::ModifierType::SHIFT_MASK), ctrl: state.contains(gtk::gdk::ModifierType::CONTROL_MASK) };
        match header_key(letter, mods, http_source) {
            Some(HeaderKey::Filter) => self.options.popup(),
            // a selection disables Refresh, as it does the header button
            Some(HeaderKey::Refresh) if !selecting => self.refresh(),
            Some(HeaderKey::Migrate) if favorite => self.migrate(),
            Some(HeaderKey::Browser) => self.browser_pressed(),
            _ => return false,
        }
        true
    }

    fn on_event(self: &Rc<Self>, event: EntryEvent) {
        match event {
            EntryEvent::Loaded(loaded) => {
                let first = !self.loaded.replace(true);
                if let Some(m) = &loaded {
                    let large = self.two_pane.get().unwrap_or(false);
                    self.expansion.set(Expansion::new(large, m.from_source));
                    // progress made in another app reaches the trackers first: pull it once per open of a library entry
                    if first && m.entry.favorite {
                        crate::entry_core::tracking::actions::pull_progress(self.env.kind, m.entry.id);
                    }
                }
                *self.model.borrow_mut() = loaded.map(|b| *b);
            }
            EntryEvent::Snapshot { entry, episodes } => {
                if let Some(m) = self.model.borrow_mut().as_mut() {
                    m.apply_snapshot(*entry, episodes);
                }
            }
            EntryEvent::RefreshEnded(result) => {
                // a failed refresh says why in a notification (behaviour 19)
                // a source the user turned off is not an error: the notice offers to turn it on, then refreshes
                let missing = matches!(result, Err(crate::entry_core::errors::DetailError::SourceNotInstalled));
                let entry = self.model.borrow().as_ref().map(|m| (m.entry.id, m.entry.source));
                let off = match entry.filter(|_| missing) {
                    Some((entry_id, source)) => {
                        let runner = Rc::downgrade(&self.runner);
                        crate::source_switch::offer_turn_on(source, move || {
                            if let Some(runner) = runner.upgrade() {
                                runner.send(EntryCmd::Refresh { entry_id });
                            }
                        })
                    }
                    None => false,
                };
                if let Some(message) = result.as_ref().err().filter(|_| !off).and_then(refresh_snack) {
                    // Retry runs the refresh again; an uninstalled source has nothing to retry
                    let entry_id = self.model.borrow().as_ref().map(|m| m.entry.id);
                    match entry_id.filter(|_| !matches!(result, Err(crate::entry_core::errors::DetailError::SourceNotInstalled))) {
                        Some(entry_id) => {
                            self.show_notice(Notice::of(&crate::entry_core::message::with_retry(message), Some(EntryCmd::Refresh { entry_id })));
                        }
                        None => self.present(message, None),
                    }
                }
                if let Some(m) = self.model.borrow_mut().as_mut() {
                    m.finish_refresh(result);
                }
            }
            EntryEvent::Downloads(on_disk) => {
                if let Some(m) = self.model.borrow_mut().as_mut() {
                    m.downloaded = on_disk;
                }
            }
            EntryEvent::Categories { all, linked } => self.show_category_dialog(&all, &linked),
            EntryEvent::AddCategories { all, linked } => self.show_add_category_dialog(&all, &linked),
            EntryEvent::Added => {}
            EntryEvent::Removed(outcome) => {
                // downloads exist: ask whether they go too. Dismissing the prompt keeps them.
                let entry_id = self.model.borrow().as_ref().map(|m| m.entry.id);
                if let (true, Some(entry_id)) = (outcome.ask_delete_downloads, entry_id) {
                    self.present(UserMessage::DeleteDownloadsForAnime, Some(EntryCmd::DeleteDownloads { entry_id }));
                }
            }
            EntryEvent::Failed(message) => self.present(UserMessage::Plain(message), None),
            EntryEvent::Related(groups) => *self.related.borrow_mut() = groups,
            EntryEvent::Scanlators(sets) => {
                if let Some(m) = self.model.borrow_mut().as_mut() {
                    m.scanlators = sets;
                }
            }
            EntryEvent::SeasonFacts(extras) => {
                if let Some(m) = self.model.borrow_mut().as_mut() {
                    m.set_season_extras(extras);
                }
            }
            EntryEvent::DefaultsApplied { scope, updated } => self.present(set_default::applied_message(self.env.kind, scope, updated), None),
            EntryEvent::Duplicate { existing } => self.show_duplicate(&existing),
            EntryEvent::Tracking(count) => {
                if let Some(m) = self.model.borrow_mut().as_mut() {
                    m.tracking_count = count;
                }
            }
        }
    }

    /// Switches between the two layouts. It moves the info block between the left pane and the first row of the list.
    fn apply_width(self: &Rc<Self>, width: i32) {
        let resized = self.width.replace(width) != width;
        let two = two_pane(width);
        // the left pane is a share of the page, so it and the header's cover in it grow with the window
        let pane_resized = two && self.panes.set_width(left_pane_width(width));
        // beside the pane the list starts at the pane's edge instead of floating centred in a wide right side (user,
        // 2026-10-25): the room past its widest goes to the right
        let spare = if two { (width - self.panes.width() - 1 - LIST_MAX_WIDTH).max(0) } else { 0 };
        self.list.widget.set_margin_end(spare);
        if self.two_pane.get() == Some(two) {
            // the layout is the same, but the automatic columns of the season grid follow the width (T-39a-034), and so does
            // the header's cover in either layout
            if resized && let Some(m) = self.model.borrow().as_ref() {
                self.show_seasons(m, m.entry.fetch_type() == FetchType::Seasons);
                if !two || pane_resized {
                    self.render_info(m);
                }
            }
            return;
        }
        self.two_pane.set(Some(two));
        self.panes.set_large(two);
        detach(&self.info.root);
        if two {
            self.panes.left.append(&self.info.root);
            self.list.set_leading(vec![self.list_head.clone().upcast()]);
        } else {
            self.list.set_leading(vec![self.info.root.clone().upcast(), self.list_head.clone().upcast()]);
        }
        self.update_fade();
        self.render();
    }

    fn render(self: &Rc<Self>) {
        let model = self.model.borrow();
        let name = match (self.loaded.get(), model.as_ref()) {
            (false, _) => "loading",
            (true, None) => "missing",
            (true, Some(_)) => "ready",
        };
        self.stack.set_visible_child_name(name);
        for w in &self.toolbar {
            w.set_sensitive(model.is_some());
        }
        let Some(m) = model.as_ref() else { return };
        if let Some(title) = &self.title {
            title.set_label(&m.entry.title);
        }
        let lines = self.list_rows(m);
        let seasons_entry = m.entry.fetch_type() == FetchType::Seasons;
        let none_shown = if seasons_entry { m.seasons.is_empty() } else { lines.is_empty() };
        self.list.sync(lines);
        self.show_seasons(m, seasons_entry);
        let header = m.item_header();
        self.count.set_label(&header.count);
        self.missing.set_label(header.missing.as_deref().unwrap_or(""));
        self.missing.set_visible(header.missing.is_some());
        // the list options follow the list on screen: the episode list, or the season grid of a `Seasons` entry
        if self.options_seasons.replace(Some(seasons_entry)) != Some(seasons_entry) {
            self.options.set_sections(&self.spec_of(seasons_entry).sections());
        }
        let word = if seasons_entry { m.entry.season_flags() } else { m.entry.item_flags };
        self.spec_of(seasons_entry).sync(&self.options, word, m.downloaded_only);
        self.options.set_extra_filter(scanlator_filter_active(&m.scanlators.available, &m.scanlators.excluded));
        // a selection disables the menu, as it did the old header button
        self.options.widget.set_sensitive(header.enabled);
        self.spinner.set_spinning(m.flow.refreshing);
        self.spinner.set_visible(m.flow.refreshing);
        // one refresh runs at a time; its arrow turns meanwhile
        ui::set_refreshing(&self.refresh_button, m.flow.refreshing);
        self.refresh_button.set_sensitive(!m.flow.refreshing && !m.selection.is_active());
        self.empty.set_visible(none_shown && !m.flow.refreshing);
        self.empty_label.set_label(&tr(if seasons_entry { MsgKey::no_seasons_error } else { crate::entry_core::wording::none_found(self.env.kind) }));
        // the anime and manga statuses share the SAnime/SManga completed code (2)
        self.info.action_row.apply(&RowState::of_entry(
            m.entry.favorite,
            m.entry.next_update,
            m.entry.status,
            m.entry.fetch_interval,
            m.tracking_count as i64,
            self.env.backend.is_http_source(m.entry.source),
        ));
        // a local or stub source has no downloads to queue (`isLocalOrStub`, T-39b-059)
        let local_or_stub = m.is_local() || self.env.backend.source_info(m.entry.source).stub;
        self.download_all.set_visible(menu_visible(local_or_stub, self.env.backend.has_download_handler()));
        self.sync_actions();
        // the button stays while nothing is selected, and is disabled when no episode is unseen (T-EPS-NEXT-1)
        let fab = m.fab();
        self.fab.set_visible(!m.selection.is_active());
        self.fab.set_sensitive(m.next_unseen().is_some());
        self.fab_label.set_label(&tr(fab.label()));
        self.update_fab();
        self.bar.set_reveal_child(m.selection.is_active());
        let selected = m.selection.count_in(&m.processed_ids());
        self.bar_count.set_label(&format!("{selected} {}", tr(MsgKey::selected).to_lowercase()));
        self.render_info(m);
        self.render_related();
        self.render_bottom(m);
        drop(model);
        self.tick_airing();
    }

    /// The bottom action bar: the rows `bottom_actions` gives for the selected episodes, shown only while something
    /// is selected, as wide as the window narrow and half of it wide.
    fn render_bottom(self: &Rc<Self>, m: &EntryModel) {
        let selected: Vec<Selected> = m
            .episodes
            .iter()
            .filter(|e| m.selection.contains(e.id))
            .map(|e| Selected { bookmark: e.bookmark, fillermark: e.fillermark, seen: e.done, last_second_seen: e.progress, downloaded: m.downloaded.contains(&e.id) })
            .collect();
        // Download and Delete show only when the app can queue downloads (`onDownloadEpisode != null`)
        let ctx = MenuContext { kind: self.env.kind, has_download_handler: self.env.backend.has_download_handler(), always_external: always_external(&self.env.prefs) };
        let rows = bottom_actions(&selected, ctx);
        while let Some(child) = self.bottom_row.first_child() {
            self.bottom_row.remove(&child);
        }
        for action in &rows {
            let button = Button::builder().css_classes(["flat"]).build();
            let content = GBox::new(O::Horizontal, 6);
            content.append(&gtk::Image::from_icon_name(action.icon()));
            content.append(&Label::new(Some(&tr(action.label(self.env.kind)))));
            button.set_child(Some(&content));
            let weak = Rc::downgrade(self);
            let action = *action;
            button.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.bottom_pressed(action);
                }
            });
            self.bottom_row.append(&button);
        }
        match bottom_bar_width(self.width.get()) {
            Some(width) => {
                self.bottom_row.set_width_request(width);
                self.bottom_row.set_halign(Align::Center);
            }
            None => {
                self.bottom_row.set_width_request(-1);
                self.bottom_row.set_halign(Align::Fill);
            }
        }
        self.bottom.set_reveal_child(!rows.is_empty());
    }

    /// A press on a row of the bottom bar: the writes use the selected episodes that differ, and the selection is
    /// cleared after the call (behaviour 11.2).
    fn bottom_pressed(self: &Rc<Self>, action: BottomAction) {
        let remove = self.remove_after_seen();
        let dispatch = {
            let mut model = self.model.borrow_mut();
            let Some(m) = model.as_mut().filter(|m| m.accepts_dispatch()) else { return };
            let dispatch = bottom_cmds(m, action.effect(), remove);
            m.selection.clear();
            dispatch
        };
        for cmd in dispatch.cmds {
            self.track_marked(&cmd);
            self.runner.send(cmd);
        }
        if let Some((id, external)) = dispatch.open {
            self.open_episode(id, OpenPlan { external });
        }
        self.render();
    }

    /// `pref_remove_after_marked_as_read`: a mark seen deletes the downloads of the episodes it changed (T-39b-013).
    fn remove_after_seen(&self) -> bool {
        crate::misc::diet_defaults::DOWNLOAD_REMOVE_AFTER_MARKED_READ
    }

    /// A context menu item on a row, written for that one episode. It works while selecting too (E-5).
    fn menu_pressed(self: &Rc<Self>, id: i64, action: RowAction) {
        let cmds = {
            let model = self.model.borrow();
            let Some(m) = model.as_ref().filter(|m| m.accepts_dispatch()) else { return };
            menu_cmds(m, id, action, self.env.backend.has_download_handler(), self.remove_after_seen())
        };
        for cmd in cmds {
            self.track_marked(&cmd);
            self.runner.send(cmd);
        }
    }

    /// A mark seen goes to the trackers too, with the numbers of the episodes it changed.
    fn track_marked(&self, cmd: &EntryCmd) {
        let EntryCmd::SetSeen { ids, seen: true } = cmd else { return };
        let model = self.model.borrow();
        let Some(m) = model.as_ref() else { return };
        let numbers: Vec<f64> = m.episodes.iter().filter(|e| ids.contains(&e.id)).map(|e| e.number).collect();
        crate::entry_core::tracking::progress::marked(self.env.kind, m.entry.id, true, &numbers, crate::data::app().base.incognito_mode.get());
    }

    /// Shows a message on the app's notification layer, replacing the one on screen. `action` is what its button runs.
    fn present(&self, message: UserMessage, action: Option<EntryCmd>) {
        self.show_notice(Notice::of(&present(message), action));
    }

    /// Shows `notice`; its button sends its command to this page's runner while the page is alive (E-UI-1).
    fn show_notice(&self, notice: Notice<EntryCmd>) {
        let runner = Rc::downgrade(&self.runner);
        notice.show(move |cmd| {
            if let Some(runner) = runner.upgrade() {
                runner.send(cmd);
            }
        });
    }

    /// The refresh button: a manual refresh of everything. One runs at a time; a second request is not sent.
    fn refresh(self: &Rc<Self>) {
        let entry_id = {
            let mut model = self.model.borrow_mut();
            let Some(m) = model.as_mut().filter(|m| m.accepts_dispatch() && !m.flow.refreshing) else { return };
            m.flow.begin_refresh();
            m.entry.id
        };
        self.runner.send(EntryCmd::Refresh { entry_id });
        self.render();
    }

    /// Puts `dialog` in the slot: the one on screen closes first, so the last request wins. Returns the name of the
    /// request, for `adopt`.
    fn claim_slot(&self, dialog: EntryDialog) -> u64 {
        let id = self.dialog_gen.get() + 1;
        self.dialog_gen.set(id);
        self.dialogs.borrow_mut().open(dialog);
        let old = self.dialog_window.borrow_mut().take();
        if let Some(old) = old {
            old.close();
        }
        id
    }

    /// Keeps the window of request `id`. Its close, however it happens, dismisses the slot unless the window said
    /// otherwise (`dismiss_on_close` false: the category dialog reports its own OK and dismiss).
    fn adopt(self: &Rc<Self>, id: u64, window: &gtk::Window, dismiss_on_close: bool) {
        *self.dialog_window.borrow_mut() = Some(window.clone());
        if dismiss_on_close {
            let weak = Rc::downgrade(self);
            window.connect_close_request(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.dismiss_dialog(id);
                }
                glib::Propagation::Proceed
            });
        }
    }

    /// `autoOpenTrack`: a tracker is logged in and "open the track menu on adding" is on.
    fn auto_open_track(&self) -> bool {
        crate::entry_core::tracking::session::refresh_from_host();
        let pref_on = crate::misc::diet_defaults::ENTRY_TRACK_ON_ADDING;
        crate::entry_core::tracking::summary::open_track_sheet_on_add(pref_on, self.env.backend.has_logged_in_tracker())
    }

    /// The dismiss of request `id` (`onDismissRequest`). When the add flow chose a category and auto-open is on, the
    /// track sheet opens now.
    fn dismiss_dialog(&self, id: u64) {
        if self.dialog_gen.get() != id {
            return;
        }
        self.dialog_window.borrow_mut().take();
        let auto = self.auto_open_track();
        if self.dialogs.borrow_mut().dismiss(auto) {
            self.open_tracking();
        }
    }

    /// The interval button (T-39a-026): the dialog of a favorite. OK and Reset store the wheel through the runner.
    fn open_interval(self: &Rc<Self>) {
        let (entry_id, interval, expected, favorite) = {
            let model = self.model.borrow();
            let Some(m) = model.as_ref().filter(|m| m.accepts_dispatch()) else { return };
            let expected = (m.entry.status != kuro_domain::entries::flags::STATUS_COMPLETED).then_some(m.entry.next_update);
            (m.entry.id, m.entry.fetch_interval, expected, m.entry.favorite)
        };
        if !interval_click_opens_dialog(favorite) {
            return;
        }
        let runner = self.runner.clone();
        let id = self.claim_slot(EntryDialog::SetAnimeFetchInterval { entry_id });
        let window = fetch_interval_dialog::open(interval, expected, move |wheel| {
            runner.send(EntryCmd::SetFetchInterval { entry_id, wheel });
        });
        self.adopt(id, &window, true);
    }

    /// The library button: adds the entry, or removes it from the library.
    fn toggle_library(self: &Rc<Self>) {
        // a favorite leaves the library; an add checks for a same-named library entry first (T-39a-047)
        let target = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).map(|m| m.entry.id);
        if let Some(entry_id) = target {
            self.runner.send(EntryCmd::ToggleLibrary { entry_id, check_duplicates: true });
        }
    }

    /// The duplicate window of an add that found a same-named library entry. A row closes it first and then acts: Show
    /// opens the entry that is there, Migrate starts the migrate flow from it, Add anyway adds without the check.
    fn show_duplicate(self: &Rc<Self>, existing: &[kuro_data::entry::Entry]) {
        let (Some(duplicate), Some(entry_id)) = (existing.first().cloned(), self.model.borrow().as_ref().map(|m| m.entry.id)) else { return };
        let runner = Rc::clone(&self.runner);
        let id = self.claim_slot(EntryDialog::DuplicateAnime { entry_id, duplicate_id: duplicate.id });
        let window = duplicate_dialog::open(
            self.env.kind,
            move |choice| match choice {
                DuplicateChoice::Show => open(duplicate.id),
                DuplicateChoice::Migrate => handoff::open_migrate(MigrateRequest { entry_id: duplicate.id, kind: Kind::Anime, title: duplicate.title.clone() }),
                DuplicateChoice::AddAnyway => runner.send(EntryCmd::ToggleLibrary { entry_id, check_duplicates: false }),
            },
            || {},
        );
        self.adopt(id, &window, true);
    }

    /// The tracking page of this anime. It explains the AniList login itself when AniList is not logged in.
    fn open_tracking(&self) {
        crate::entry_core::tracking::session::refresh_from_host();
        let loaded = self.model.borrow().as_ref().map(|m| (m.entry.id, m.entry.source, m.entry.title.clone()));
        if let Some((id, source_id, title)) = loaded {
            // a bind, an edit or a stop on the tracking page reads the count again, so the button follows at once
            let runner = Rc::downgrade(&self.runner);
            let changed = move || {
                if let Some(runner) = runner.upgrade() {
                    runner.send(EntryCmd::ReadTracking { entry_id: id, source_id });
                }
            };
            crate::entry_core::tracking::page::open_sheet(crate::entry_core::tracking::page::Entry { kind: self.env.kind, id, title }, changed);
        }
    }

    /// The tracking button.
    fn tracking_pressed(&self) {
        self.claim_slot(EntryDialog::TrackSheet);
        self.open_tracking();
    }

    /// The Browser button and W (an HTTP source only): the entry's page in the system browser, as Open in browser.
    fn browser_pressed(&self) {
        let url = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).and_then(|m| self.env.backend.web_url(m.entry.source, &m.entry.url));
        if let Some(url) = url {
            overflow_menu::open_in_browser(&url);
        }
    }

    /// A long press on the library button of a favorite: reads the categories, and the answer opens the editor.
    fn open_categories(&self) {
        let entry_id = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).map(|m| m.entry.id);
        if let Some(entry_id) = entry_id {
            self.runner.send(EntryCmd::LoadCategories { entry_id });
        }
    }

    /// The category editor over the answer of `LoadCategories`. OK writes the checked ids; any other way out writes
    /// nothing.
    fn show_category_dialog(self: &Rc<Self>, all: &[(i64, String)], linked: &[i64]) {
        let Some(entry_id) = self.model.borrow().as_ref().map(|m| m.entry.id) else { return };
        let runner = Rc::clone(&self.runner);
        let id = self.claim_slot(EntryDialog::ChangeCategory { entry_id });
        let (ok, dismissed) = (Rc::downgrade(self), Rc::downgrade(self));
        let window = crate::category_picker::open(
            CategoryDialog::new(entry_id, false, all, linked),
            move |m| {
                runner.send(EntryCmd::SetCategories { entry_id, category_ids: m.checked_ids() });
                if let Some(view) = ok.upgrade() {
                    view.confirm_category(id);
                }
            },
            || crate::ui::open("categories"),
            move || {
                if let Some(view) = dismissed.upgrade() {
                    view.dismiss_dialog(id);
                }
            },
        );
        self.adopt(id, &window, false);
    }

    /// OK in the category dialog: the slot empties without the dismiss, so the track sheet does not follow (5.3).
    fn confirm_category(&self, id: u64) {
        if self.dialog_gen.get() == id {
            self.dialog_window.borrow_mut().take();
            self.dialogs.borrow_mut().confirm_category();
        }
    }

    /// The category dialog of an add with no default category (T-39b-026). OK adds the entry in the ticked categories;
    /// a dismiss adds nothing, since favorite is written only with the answer.
    fn show_add_category_dialog(self: &Rc<Self>, all: &[(i64, String)], linked: &[i64]) {
        let Some(entry_id) = self.model.borrow().as_ref().map(|m| m.entry.id) else { return };
        let runner = Rc::clone(&self.runner);
        // the choose branch sets the pending flag: a dismiss may open the track sheet after it
        let id = self.claim_slot(EntryDialog::ChangeCategory { entry_id });
        self.dialogs.borrow_mut().open_change_category_from_add(entry_id);
        let (ok, dismissed) = (Rc::downgrade(self), Rc::downgrade(self));
        let window = crate::category_picker::open(
            CategoryDialog::new(entry_id, true, all, linked),
            move |m| {
                runner.send(EntryCmd::AddWithCategories { entry_id, category_ids: m.checked_ids() });
                if let Some(view) = ok.upgrade() {
                    view.confirm_category(id);
                }
            },
            || crate::ui::open("categories"),
            move || {
                if let Some(view) = dismissed.upgrade() {
                    view.dismiss_dialog(id);
                }
            },
        );
        self.adopt(id, &window, false);
    }

    /// The width the season grid has: the page, or the right pane of the two-pane layout.
    fn grid_width(&self) -> i32 {
        let width = self.width.get();
        if self.two_pane.get() == Some(true) { (width - self.panes.width() - 1).max(0) } else { width }
    }

    /// The season grid of a `Seasons` entry. Its minimum column count is the entry's size setting (`Layout::columns_for`); it
    /// is redrawn only when the cells, the columns or the width changed.
    fn show_seasons(&self, m: &EntryModel, seasons_entry: bool) {
        if !seasons_entry {
            self.seasons.widget.set_visible(false);
            *self.seasons_shown.borrow_mut() = None;
            return;
        }
        let size = m.entry.season().grid_size();
        let mode = m.entry.season().grid_mode();
        let width = self.grid_width();
        // the facts of a season are what the runner read for it; the filters, the sort and the overlays are the pipeline's
        let seasons: Vec<(SeasonEntry, SeasonFacts)> = m
            .seasons
            .iter()
            .map(|s| {
                let local = s.entry.source == kuro_source::local::LOCAL_SOURCE_ID;
                (s.clone(), season_facts(s, m.season_extras.get(&s.entry.id), local))
            })
            .collect();
        let items: Vec<SeasonItem> = process_seasons(&m.entry, &seasons, m.downloaded_only);
        let layout = Layout {
            mode,
            number_mode: m.entry.season().display_mode() == kuro_db::flags::season::DISPLAY_MODE_NUMBER,
            // the page can continue watching: a tile shows the button when its season has an unseen episode
            has_continue: true,
            columns: Layout::columns_for(width, i64::from(size), mode),
            size: i64::from(size),
            fixed: size > 0 || mode == SeasonDisplayMode::List,
            width,
            height: self.scroller.height(),
        };
        let key = (items, layout);
        if self.seasons_shown.borrow().as_ref() == Some(&key) {
            return;
        }
        self.seasons.set(&key.0, &key.1);
        *self.seasons_shown.borrow_mut() = Some(key);
    }

    /// The page actions offered for the entry on show (`overflow_menu::items`); none before the entry loads.
    fn overflow_items(&self) -> Vec<OverflowItem> {
        let model = self.model.borrow();
        let Some(m) = model.as_ref().filter(|m| m.accepts_dispatch()) else { return Vec::new() };
        let source = m.entry.source;
        overflow_menu::items(OverflowEntry {
            favorite: m.entry.favorite,
            // a manga has no intro: its fetch type reads as episodes too
            has_intro_length: self.env.kind.is_anime() && m.entry.fetch_type() == FetchType::Episodes,
            http_source: self.env.backend.is_http_source(source),
            configurable_source: self.env.backend.is_configurable_source(source),
        })
    }

    /// Shows the page actions the entry on show offers: as buttons in a wide window, in the ⋮ menu in the phone layout.
    fn sync_actions(&self) {
        let (offered, narrow) = (self.overflow_items(), self.actions_narrow.get());
        for (item, button) in &self.overflow_buttons {
            button.set_visible(!narrow && offered.contains(item));
        }
        if let Some(menu) = &self.overflow_button {
            menu.set_visible(narrow && !offered.is_empty());
        }
    }

    fn run_overflow(self: &Rc<Self>, item: OverflowItem) {
        match item {
            OverflowItem::ChangeIntro => self.open_intro_length(),
            OverflowItem::EditCategories => self.open_categories(),
            OverflowItem::Migrate => self.migrate(),
            OverflowItem::Share => self.share(),
            OverflowItem::OpenInBrowser => self.open_in_browser(),
            OverflowItem::Settings => self.open_source_preferences(),
        }
    }

    /// Change intro length (T-39a-075): the wheel of the entry's skip-intro length. OK stores it through the runner.
    fn open_intro_length(self: &Rc<Self>) {
        let (entry_id, stored, disabled) = {
            let model = self.model.borrow();
            let Some(m) = model.as_ref().filter(|m| m.accepts_dispatch()) else { return };
            (m.entry.id, m.entry.viewer().skip_intro_length(), m.entry.viewer().skip_intro_disable())
        };
        let default = DEFAULT_INTRO_LENGTH;
        let runner = self.runner.clone();
        let id = self.claim_slot(EntryDialog::ChangeAnimeSkipIntro);
        let window = intro_dialog::open(viewer::dialog_start(stored, disabled, default), move |length| {
            runner.send(EntryCmd::SetSkipIntroLength { entry_id, length });
        });
        self.adopt(id, &window, true);
    }

    /// Settings (T-39a-057): the preferences screen of this entry's source, which the request names.
    fn open_source_preferences(self: &Rc<Self>) {
        let source_id = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).map(|m| m.entry.source);
        if let Some(source_id) = source_id {
            handoff::open_source_preferences(handoff::SourcePrefsRequest { source_id, kind: self.env.kind });
        }
    }

    /// Migrate (T-39a-075): starts the migrate flow from this entry, as the duplicate choice does.
    fn migrate(self: &Rc<Self>) {
        let request = self
            .model
            .borrow()
            .as_ref()
            .filter(|m| m.accepts_dispatch())
            .map(|m| MigrateRequest { entry_id: m.entry.id, kind: self.env.kind, title: m.entry.title.clone() });
        if let Some(request) = request {
            handoff::open_migrate(request);
        }
    }

    /// Share (T-39a-075): copies the entry's address to the clipboard and says so (`overflow_menu::share`).
    fn share(self: &Rc<Self>) {
        let url = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).and_then(|m| self.env.backend.web_url(m.entry.source, &m.entry.url));
        if let Some(text) = url.and_then(|url| overflow_menu::share(&url)) {
            self.present(UserMessage::Plain(text), None);
        }
    }

    /// Open in browser (T-39b-051): the entry's address in the desktop's browser. Offered for an HTTP source only.
    fn open_in_browser(self: &Rc<Self>) {
        let url = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).and_then(|m| self.env.backend.web_url(m.entry.source, &m.entry.url));
        if let Some(url) = url {
            overflow_menu::open_in_browser(&url);
        }
    }

    /// Download all (`download_menu::targets`): queues every unseen or unread item of the displayed list.
    fn run_download(&self, amount: DownloadAmount) {
        let (entry_id, ids) = {
            let model = self.model.borrow();
            let Some(m) = model.as_ref().filter(|m| m.accepts_dispatch()) else { return };
            (m.entry.id, m.download_targets(amount))
        };
        if !ids.is_empty() {
            self.runner.send(EntryCmd::Download { entry_id, ids, start_now: false });
        }
    }

    /// Draws the info block: the header in the layout and column of the page, then the description with its genre chips.
    fn render_info(self: &Rc<Self>, m: &EntryModel) {
        let two = self.two_pane.get().unwrap_or(false);
        let model = header_model(&m.entry, &m.source);
        let column = if two { self.panes.width() } else { self.width.get() };
        let handlers = self.handlers();
        self.info.set_header(&model, two, column, &handlers);
        self.info.set_description(&description(m.entry.description.as_deref()), &model.genres, &self.expansion, &handlers);
    }

    fn handlers(self: &Rc<Self>) -> Handlers {
        let weak = Rc::downgrade(self);
        let kind = self.env.kind;
        Handlers {
            // the query goes to the opened screen through `handoff::take_search`
            on_search: Rc::new({
                move |request: SearchRequest| {
                    let http_source = weak.upgrade().is_some_and(|view| {
                        let source = view.model.borrow().as_ref().map(|m| m.entry.source);
                        source.is_some_and(|source| view.env.backend.is_http_source(source))
                    });
                    handoff::open_search(request, kind, http_source);
                }
            }),
            on_copy: Rc::new(|text: String| {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
            }),
        }
    }

    /// The FAB label shows at the top, after a backward scroll and at the end of the list (`shouldExpandFAB`).
    fn update_fab(&self) {
        self.fab_label.set_visible(page_shell::fab_label_shown(&self.scroll, &self.scroller.vadjustment()));
        self.update_fade();
    }

    /// The scroll position fades the title and the header bar background in the narrow layout (T-39a-036).
    fn update_fade(&self) {
        let narrow = !self.two_pane.get().unwrap_or(false);
        let value = self.scroller.vadjustment().value();
        page_shell::fade_header(self.title.as_ref(), self.header_bar.as_ref(), narrow, value, self.info.root.height(), false);
    }

    /// One tick of the airing row: the remaining time comes from the absolute target.
    fn tick_airing(&self) {
        let model = self.model.borrow();
        let Some(m) = model.as_ref() else { return };
        let airing = Airing::of(&m.entry);
        let remaining = airing.map_or(0, |a| a.remaining(now_ms()));
        let show = airing_visible(remaining, self.show_airing, m.entry.status);
        self.airing_row.set_visible(show);
        if let (true, Some(airing)) = (show, airing) {
            self.airing_title.set_label(&airing.title());
            self.airing_time.set_label(&format_airing(remaining));
        }
    }

    fn row_pressed(self: &Rc<Self>, id: i64, long: bool, alt: bool, mods: crate::entry_core::selection::Mods) {
        let open = {
            let mut model = self.model.borrow_mut();
            let Some(m) = model.as_mut().filter(|m| m.accepts_dispatch()) else { return };
            let view = m.processed_ids();
            match m.selection.press(&view, id, mods, long) {
                RowClick::Open(id) => Some(id),
                RowClick::Toggled => None,
            }
        };
        if let Some(id) = open {
            // a tap opens in the player the setting says, and Alt picks the other one (17.4)
            let plan = tap_plan(always_external(&self.env.prefs), alt);
            self.open_episode(id, plan);
        }
        self.render();
    }

    /// The rows of the list. The download button shows only when the app can queue this entry's episodes: not for a
    /// local or stub source, and not without a download handler (`isLocalOrStub`, T-39b-059).
    fn list_rows(&self, m: &EntryModel) -> Vec<crate::entry_core::list_model::RowItem> {
        let mut rows = m.lines();
        let can_download = !m.is_local() && !self.env.backend.source_info(m.entry.source).stub && self.env.backend.has_download_handler();
        if !can_download {
            for row in &mut rows {
                row.indicator = false;
            }
        }
        rows
    }

    /// A change of one episode in the download queue: only the list is drawn again.
    fn live_changed(&self, id: i64, state: DownloadState, percent: u8) {
        let rows = {
            let mut model = self.model.borrow_mut();
            let Some(m) = model.as_mut() else { return };
            m.apply_live(id, state, percent);
            self.list_rows(m)
        };
        self.list.sync(rows);
    }

    /// A press on an episode's download button: Start queues it (an error is queued again), Cancel takes it out of the
    /// queue at once, and Delete (confirmed in the button's popover) removes its files.
    fn download_pressed(&self, id: i64, request: Request) {
        let Some(entry_id) = self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).map(|m| m.entry.id) else { return };
        match request {
            Request::Start => self.runner.send(EntryCmd::Download { entry_id, ids: vec![id], start_now: false }),
            Request::Cancel => {
                crate::downloads::live::cancel(self.env.kind, id);
                self.live_changed(id, DownloadState::NotDownloaded, 0);
            }
            Request::Delete => self.runner.send(EntryCmd::DeleteEpisodeDownloads { entry_id, ids: vec![id] }),
        }
    }

    /// A tap on a summary toggles it, unless an episode is selected (T-39a-012). The row is drawn again with its new limit.
    fn summary_pressed(self: &Rc<Self>, id: i64) {
        let changed = {
            let mut model = self.model.borrow_mut();
            let Some(m) = model.as_mut() else { return };
            let selecting = m.selection.is_active();
            m.summaries.click(id, selecting)
        };
        if changed {
            self.render();
        }
    }

    /// Opens an episode as the plan says. A missing episode is a typed error shown in the snackbar, not a panic.
    fn open_episode(&self, id: i64, plan: OpenPlan) {
        let found = {
            let model = self.model.borrow();
            required(model.as_ref().and_then(|m| m.episodes.iter().find(|e| e.id == id).map(|e| (m.entry.id, m.entry.title.clone(), e.name.clone()))), || {
                DetailsError::EpisodeNotFound(id)
            })
        };
        match found {
            Ok((entry_id, _, _)) if self.env.kind == Kind::Manga => self.open_chapter(entry_id, id),
            Ok((entry_id, title, name)) => self.play(entry_id, id, &title, &name, plan),
            Err(error) => self.show_error(error),
        }
    }

    /// A chapter opens the reader, by ids so two folders with one title keep apart and a browsed entry needs no library row.
    fn open_chapter(&self, entry_id: i64, chapter_id: i64) {
        // repeated clicks, or clicks queued while the window was busy, open the reader once
        if !ui::first_within("reader", 1500) {
            return;
        }
        let local = self.model.borrow().as_ref().is_some_and(|m| m.entry.source == kuro_local::LOCAL_SOURCE_ID);
        ui::push(&if local {
            crate::local_flow::reader::open_local_chapter(entry_id, chapter_id)
        } else {
            crate::reader::launch::open_chapter(entry_id, chapter_id)
        });
    }

    /// Plays in the external player, or in the internal one when the external one cannot start.
    fn play(&self, entry_id: i64, episode_id: i64, title: &str, name: &str, plan: OpenPlan) {
        // repeated clicks, or clicks queued while the window was busy, open the player once
        if !crate::ui::first_within("player", 1500) {
            return;
        }
        // a local episode plays in the internal player at its saved position, whatever the plan says
        if crate::local_flow::open::open_local_episode(episode_id) {
            return;
        }
        Self::open_player(&self.env.backend, plan, entry_id, episode_id, title, name);
    }

    /// Hands the episode to the external player the plan names, or to the internal one.
    fn open_player(backend: &Arc<dyn EntryBackend>, plan: OpenPlan, entry_id: i64, episode_id: i64, title: &str, name: &str) {
        if plan.external && backend.play_external(entry_id, episode_id) {
            return;
        }
        // an episode of an online source resolves its video and plays in the internal player
        if crate::player_view::open_source_episode(&crate::screens::anime_sources(), episode_id) {
            return;
        }
        crate::screens::open_media(Kind::Anime, title, name);
    }

    /// Logs a typed error and shows its text in the snackbar.
    fn show_error(&self, error: DetailsError) {
        self.present(UserMessage::Plain(error.record().user_message()), None);
    }

    /// The continue button of a season tile: plays that season's next unseen episode, as the FAB does for an entry.
    fn continue_season(&self, season_id: i64) {
        let store = EntryStore::new(Arc::clone(&self.env.db), self.env.kind);
        let played = (|| -> Result<(), DetailsError> {
            let season = required(store.entry(season_id)?, || DetailsError::AnimeNotFound(season_id))?;
            let episodes = store.episodes(season_id)?;
            let episode = required(continue_episode(&episodes), || DetailsError::EpisodeNotFound(season_id))?;
            let plan = continue_plan(always_external(&self.env.prefs));
            self.play(season_id, episode.id, &season.title, &episode.name, plan);
            Ok(())
        })();
        if let Err(error) = played {
            self.show_error(error);
        }
    }

    /// Draws the related groups under the description when they or the display mode changed.
    fn render_related(&self) {
        let mode = kuro_prefs::library::LibraryDisplayMode::CompactGrid;
        let key = (self.related.borrow().clone(), mode);
        if self.related_shown.borrow().as_ref() == Some(&key) {
            return;
        }
        while let Some(child) = self.related_host.first_child() {
            self.related_host.remove(&child);
        }
        if let Some(section) = related_view::section(&key.0, mode, Rc::new(open)) {
            self.related_host.append(&section);
        }
        // an empty box would still take the info block's spacing
        self.related_host.set_visible(self.related_host.first_child().is_some());
        *self.related_shown.borrow_mut() = Some(key);
    }

    /// The FAB: opens the first unseen episode, with the setting as it is and no inversion (17.2).
    fn open_next(self: &Rc<Self>) {
        let next = self.model.borrow().as_ref().and_then(EntryModel::next_unseen);
        if let Some(id) = next {
            let plan = continue_plan(always_external(&self.env.prefs));
            self.open_episode(id, plan);
        }
    }

    /// Edits the selection, then redraws. The closure gets the processed ids (the view, for toggles) and every
    /// episode id (for select all and invert, which act on the full list).
    fn edit_selection(self: &Rc<Self>, edit: impl FnOnce(&mut EntryModel, &[i64], &[i64])) {
        {
            let mut model = self.model.borrow_mut();
            let Some(m) = model.as_mut().filter(|m| m.accepts_dispatch()) else { return };
            let view = m.processed_ids();
            let full: Vec<i64> = m.episodes.iter().map(|e| e.id).collect();
            edit(m, &view, &full);
        }
        self.render();
    }

    /// Back or Escape: the selection is cleared first and the key is consumed. `false` means navigate up.
    fn back_pressed(self: &Rc<Self>) -> bool {
        let consumed = self.model.borrow_mut().as_mut().is_some_and(|m| m.selection.back());
        if consumed {
            self.render();
        }
        consumed
    }

    fn spec_of(&self, seasons: bool) -> &Spec {
        if seasons { &self.season_spec } else { &self.episode_spec }
    }

    /// A pick in the list options menu: a masked write of the episode or season flag word, or "Use as default for all
    /// titles" (asked first). The menu shows the stored word again on the next render.
    fn list_option(self: &Rc<Self>, pick: Pick) {
        if pick == Pick::Action(list_options::SCANLATORS) {
            return self.open_scanlator_sheet();
        }
        if pick == Pick::Action(list_options::DEFAULT) {
            let weak = Rc::downgrade(self);
            list_options::confirm_default(move || {
                if let Some(view) = weak.upgrade() {
                    view.use_as_default();
                }
            });
            return;
        }
        let Some((entry_id, seasons)) =
            self.model.borrow().as_ref().filter(|m| m.accepts_dispatch()).map(|m| (m.entry.id, m.entry.fetch_type() == FetchType::Seasons))
        else {
            return;
        };
        let Some((flag, mask)) = self.spec_of(seasons).write(&pick) else { return };
        // an entry outside the library keeps its flags for the session: the model takes the change as well as the row
        if let Some(m) = self.model.borrow_mut().as_mut() {
            if seasons {
                m.note_season_write(flag, mask);
            } else {
                m.note_flag_write(flag, mask);
            }
        }
        self.runner.send(if seasons { EntryCmd::SetSeasonFlagBits { entry_id, flag, mask } } else { EntryCmd::SetFlagBits { entry_id, flag, mask } });
        self.render();
    }

    /// The exclusion sheet of the Scanlator filter row. OK writes the set it hands back.
    fn open_scanlator_sheet(self: &Rc<Self>) {
        let Some((entry_id, sets)) = self.model.borrow().as_ref().map(|m| (m.entry.id, m.scanlators.clone())) else { return };
        let weak = Rc::downgrade(self);
        scanlator_sheet::open(&sets.available, &sets.excluded, move |excluded| {
            if let Some(view) = weak.upgrade() {
                view.runner.send(EntryCmd::SetExcludedScanlators { entry_id, excluded });
            }
        });
    }

    /// "Use as default for all titles": stores the flag word of the list on screen as the default of its scope, and the
    /// runner applies that default to every favorite. Only that scope is written: an episode save never touches the
    /// season flags of the library, and the reverse. The message waits for the run, to say how many entries took the
    /// settings (E-DEF-3, `EntryEvent::DefaultsApplied`).
    fn use_as_default(self: &Rc<Self>) {
        let Some((seasons, word)) = self.model.borrow().as_ref().map(|m| {
            let seasons = m.entry.fetch_type() == FetchType::Seasons;
            (seasons, if seasons { m.entry.season_flags() } else { m.entry.item_flags })
        }) else {
            return;
        };
        let scope = if seasons {
            set_season_settings_default(&self.env.prefs, word);
            Scope::Season
        } else {
            set_item_settings_default(&self.env.prefs, self.env.kind, word);
            Scope::Episode
        };
        self.runner.send(EntryCmd::ApplyDefaultsToLibrary { scope });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub;
    impl runner::EntryBackend for Stub {
        fn source_info(&self, _: i64) -> crate::entry_core::source_info::SourceInfo {
            crate::entry_core::source_info::SourceInfo { name: "Stub".into(), stub: true }
        }
        fn fetch(&self, _: i64, _: super::super::entry_state::RefreshPlan, _: crate::entry_core::refresh_rules::Trigger) -> runner::EntryFetch {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    fn env_of(store: &EntryStore) -> Env {
        Env { prefs: PrefStore::in_memory(), backend: Arc::new(Stub), db: store.db(), kind: Kind::Anime }
    }

    use crate::entry_core::page_shell::test_util::{descendants, pump};

    /// A backend whose every fetch fails with `error`; it counts the fetches.
    struct Failing(crate::entry_core::errors::DetailError, std::sync::atomic::AtomicUsize);
    impl runner::EntryBackend for Failing {
        fn source_info(&self, _: i64) -> crate::entry_core::source_info::SourceInfo {
            crate::entry_core::source_info::SourceInfo { name: "Src".into(), stub: false }
        }
        fn fetch(&self, _: i64, _: super::super::entry_state::RefreshPlan, _: crate::entry_core::refresh_rules::Trigger) -> runner::EntryFetch {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let error = self.0.clone();
            Box::pin(async move { Err(error) })
        }
    }

    /// A failed refresh (HTTP 503) shows its notification with Retry, and Retry refreshes again; an uninstalled source
    /// shows its notification with no Retry. The notification is the app's one layer (`toast::attach`).
    #[test]
    fn a_failed_refresh_offers_retry_and_an_uninstalled_source_does_not() {
        use crate::entry_core::errors::DetailError;
        use std::sync::atomic::Ordering;
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let rt = kuro_core::runtime::build().expect("runtime");
            let (_dir, store, id) = super::super::store::fixture::with_episodes(true, true, 3, 1);
            for (error, retry) in [(DetailError::Http(503), true), (DetailError::SourceNotInstalled, false)] {
                let backend = Arc::new(Failing(error.clone(), Default::default()));
                let runner = Rc::new(runner::spawn(rt.handle(), store.clone(), PrefStore::in_memory(), backend.clone(), EntryLoadContext::default()));
                let host = gtk::Overlay::new();
                crate::toast::attach(&host);
                let page = assemble(Rc::clone(&runner), true, Env { backend: backend.clone(), ..env_of(&store) });
                runner.send(EntryCmd::Load { entry_id: id });
                let stack = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::Stack>().ok()).find(|s| s.child_by_name("ready").is_some()).expect("the page has a stack");
                assert!(pump(&ctx, || stack.visible_child_name().as_deref() == Some("ready")), "the page becomes ready");
                assert!(
                    !descendants(page.upcast_ref()).iter().any(|w| w.has_css_class("app-notification")),
                    "the page has no notification bar of its own"
                );
                runner.send(EntryCmd::Refresh { entry_id: id });
                let snack = descendants(host.upcast_ref())
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::Revealer>().ok())
                    .find(|r| r.child().is_some_and(|c| c.has_css_class("app-notification")))
                    .expect("the notification layer");
                assert!(pump(&ctx, || snack.reveals_child()), "{error:?}: the failed refresh shows a notification");
                let action = snack.child().and_then(|bar| bar.first_child()).and_then(|label| label.next_sibling()).and_downcast::<Button>().expect("the action button");
                assert_eq!(action.is_visible(), retry, "{error:?}: Retry only when there is something to retry");
                if retry {
                    assert_eq!(action.label().as_deref(), Some(tr(MsgKey::action_retry).as_str()));
                    let before = backend.1.load(Ordering::SeqCst);
                    action.emit_clicked();
                    assert!(pump(&ctx, || backend.1.load(Ordering::SeqCst) > before), "Retry sends the refresh to the runner");
                }
                runner.send(EntryCmd::Shutdown);
            }
        });
    }

    /// A refresh that finds no episodes raises no notification: the list's empty state says so, with a Retry button that
    /// refreshes again.
    #[test]
    fn no_episodes_is_the_empty_state_with_retry_not_a_notification() {
        use crate::entry_core::errors::DetailError;
        use std::sync::atomic::Ordering;
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let rt = kuro_core::runtime::build().expect("runtime");
            let (_dir, store, id) = super::super::store::fixture::with_episodes(true, true, 0, 0);
            let backend = Arc::new(Failing(DetailError::NoEpisodes, Default::default()));
            let runner = Rc::new(runner::spawn(rt.handle(), store.clone(), PrefStore::in_memory(), backend.clone(), EntryLoadContext::default()));
            let host = gtk::Overlay::new();
            crate::toast::attach(&host);
            let page = assemble(Rc::clone(&runner), true, Env { backend: backend.clone(), ..env_of(&store) });
            runner.send(EntryCmd::Load { entry_id: id });
            let stack = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::Stack>().ok()).find(|s| s.child_by_name("ready").is_some()).expect("the page has a stack");
            assert!(pump(&ctx, || stack.visible_child_name().as_deref() == Some("ready")), "the page becomes ready");
            let before = backend.1.load(Ordering::SeqCst);
            runner.send(EntryCmd::Refresh { entry_id: id });
            assert!(pump(&ctx, || backend.1.load(Ordering::SeqCst) > before), "the refresh ran and found nothing");
            let until = std::time::Instant::now() + Duration::from_millis(300);
            while std::time::Instant::now() < until {
                ctx.iteration(false);
            }
            let layer = descendants(host.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::Revealer>().ok()).find(|r| r.child().is_some_and(|c| c.has_css_class("app-notification"))).expect("the notification layer");
            assert!(!layer.reveals_child(), "nothing pops up for an empty list");
            // the list head is a row of the list view, so it exists once the page has a size
            let window = gtk::Window::builder().default_width(900).default_height(700).child(&page).build();
            window.present();
            let find_retry = || {
                descendants(page.upcast_ref())
                    .into_iter()
                    .filter_map(|w| w.downcast::<Button>().ok())
                    .find(|b| b.label().as_deref() == Some(tr(MsgKey::action_retry).as_str()))
            };
            assert!(pump(&ctx, || find_retry().is_some()), "the empty state has Retry");
            let retry = find_retry().expect("Retry");
            assert!(retry.parent().is_some_and(|state| state.is_visible()), "and it shows, since the list is empty");
            let before = backend.1.load(Ordering::SeqCst);
            retry.emit_clicked();
            assert!(pump(&ctx, || backend.1.load(Ordering::SeqCst) > before), "Retry refreshes again");
            runner.send(EntryCmd::Shutdown);
            window.set_child(None::<&gtk::Widget>);
            window.close();
        });
    }

    /// Add to library on an entry opened from Browse (not a favorite, never refreshed): the click writes favorite=1 in
    /// the default category and the button turns to "In library".
    #[test]
    fn add_to_library_from_the_details_page_writes_the_row_and_flips_the_button() {
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let rt = kuro_core::runtime::build().expect("runtime");
            let (_dir, store, id) = super::super::store::fixture::with_episodes(false, false, 2, 0);
            let category = kuro_testkit::insert_category(&store.db(), Kind::Anime, "Watching");
            let prefs = PrefStore::in_memory();
            kuro_prefs::library::LibraryPrefs::new(prefs.clone()).default_anime_category.set(category as i32).expect("default category");
            let runner = Rc::new(runner::spawn(rt.handle(), store.clone(), prefs, Arc::new(Stub), EntryLoadContext::default()));
            let page = assemble(Rc::clone(&runner), true, env_of(&store));
            runner.send(EntryCmd::Load { entry_id: id });
            let label = |text: &str| descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).find(|l| l.label() == text);
            assert!(pump(&ctx, || label("Frieren").is_some()), "the page loaded the entry");
            let button = label("Add to library").and_then(|l| l.ancestor(Button::static_type())).and_downcast::<Button>().expect("the library button");
            button.emit_clicked();
            assert!(pump(&ctx, || store.entry(id).unwrap().is_some_and(|a| a.favorite)), "the click writes favorite=1");
            assert_eq!(store.category_ids(id).unwrap(), vec![category], "the default category is linked");
            assert!(pump(&ctx, || label("In library").is_some()), "the button turns to In library");
            runner.send(EntryCmd::Shutdown);
        });
    }

    /// One display test (GTK is bound to the thread that starts it): Loading, then the page with its rows and a
    /// stub warning, the single column at 360 px with no horizontal minimum above 360, the two panes from 900 px with
    /// the info block moved to the left, and Missing with a Back button.
    #[test]
    fn anime_route_smoke() {
        use kuro_db::flags::item as ep;
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let rt = kuro_core::runtime::build().expect("runtime");
            let (_dir, store, id) = super::super::store::fixture::with_episodes(true, true, 3, 1);
            let spawn = |entry_id| {
                let runner = Rc::new(runner::spawn(rt.handle(), store.clone(), kuro_prefs::store::PrefStore::in_memory(), Arc::new(Stub), EntryLoadContext::default()));
                let page = assemble(Rc::clone(&runner), true, env_of(&store));
                runner.send(EntryCmd::Load { entry_id });
                (page, runner)
            };
            let stack_of = |page: &GBox| descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::Stack>().ok()).find(|s| s.child_by_name("ready").is_some()).expect("the page has a stack");

            let (page, _runner) = spawn(id);
            let stack = stack_of(&page);
            assert_eq!(stack.visible_child_name().as_deref(), Some("loading"));
            assert!(pump(&ctx, || stack.visible_child_name().as_deref() == Some("ready")), "the page becomes ready");
            let all = descendants(page.upcast_ref());
            let list = all.iter().find_map(|w| w.clone().downcast::<gtk::ListView>().ok()).expect("the list view");
            // at 360 px the page is one column: the info block and the list head are the first two rows of the list
            let window = gtk::Window::builder().default_width(360).default_height(700).child(&page).build();
            window.present();
            assert!(pump(&ctx, || list.model().is_some_and(|m| m.n_items() == 5)), "two leading rows and three episodes");
            for _ in 0..20 {
                while ctx.iteration(false) {}
                std::thread::sleep(Duration::from_millis(10));
            }
            let buttons: Vec<Button> = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Button>().ok()).collect();
            let labels: Vec<String> = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).map(|l| l.label().to_string()).collect();
            assert!(labels.iter().any(|t| t == "In library"), "the library button of a favorite");
            assert!(labels.iter().any(|t| t == "Tracking"), "the tracking button has its callback");
            let webview = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).find(|l| l.label() == "Browser").and_then(|l| l.ancestor(Button::static_type()));
            assert!(webview.is_some_and(|b| !b.is_visible()), "Browser shows for an HTTP source only");
            let download = buttons.iter().find(|b| b.label().as_deref() == Some("Download all")).expect("the Download all button exists");
            assert!(!download.is_visible(), "no download handler, no Download all");
            assert!(buttons.iter().any(|b| b.tooltip_text().as_deref() == Some("Refresh") && b.is_sensitive()), "the refresh button");
            for menu in descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::MenuButton>().ok()) {
                if let Some(model) = menu.menu_model() {
                    assert!(!crate::widgets::view_options::menu_labels(&model).iter().any(|l| l == "Refresh"), "Refresh is the header button, not a menu item");
                }
            }
            let fab = buttons.iter().find(|b| b.has_css_class("pill")).expect("the FAB");
            assert!(fab.is_visible() && fab.is_sensitive(), "an unseen episode exists");
            let (min, _, _, _) = page.measure(gtk::Orientation::Horizontal, -1);
            assert!(min <= 360, "no horizontal minimum above 360 px, got {min}");
            let left = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::ScrolledWindow>().ok()).find(|w| w.width_request() == crate::entry_core::constants::LEFT_PANE_WIDTH).expect("the left pane");
            assert!(!left.is_visible(), "no left pane in one column");
            // one list options menu in the top bar, with the episode options; the count row is no button
            let options: Vec<gtk::MenuButton> = descendants(page.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::MenuButton>().ok())
                .filter(|b| b.tooltip_text().as_deref() == Some(&list_options::tooltip()))
                .collect();
            assert_eq!(options.len(), 1, "one list options button");
            let labels = crate::widgets::view_options::menu_labels(&options[0].menu_model().expect("the menu"));
            for want in ["Show only", "Downloaded", "Unwatched", "Bookmarked", "Filler", "Sort by", "Source order", "Number", "Upload date", "Reverse order", "Display", "Episode number", "Use as default for all titles"] {
                assert!(labels.iter().any(|t| t == want), "the menu has {want}: {labels:?}");
            }
            assert!(!labels.iter().any(|t| t == "Items per row"), "an episode list shows no season options");
            let count_row = descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).find(|l| l.label() == "3 episodes");
            assert!(count_row.is_none_or(|l| l.ancestor(Button::static_type()).is_none()), "no second sort button above the list");
            // Unwatched writes the flag and leaves the two unwatched episodes; turning it off brings the third back
            let rows = &store;
            let unseen = |on: bool| move || rows.entry(id).unwrap().is_some_and(|a| (a.item_flags & ep::UNDONE_MASK == ep::SHOW_UNDONE) == on);
            options[0].activate_action("viewopt.unseen", None).expect("the Unwatched check");
            assert!(pump(&ctx, unseen(true)), "the check writes the flag");
            assert!(pump(&ctx, || list.model().is_some_and(|m| m.n_items() == 4)), "two leading rows and two unwatched episodes, got {:?}", list.model().map(|m| m.n_items()));
            options[0].activate_action("viewopt.unseen", None).expect("the Unwatched check");
            assert!(pump(&ctx, unseen(false)), "off clears the group");
            assert!(pump(&ctx, || list.model().is_some_and(|m| m.n_items() == 5)), "every episode again");
            // from 900 px the info block moves to the left pane and the list keeps only its head
            window.set_default_size(1000, 700);
            assert!(pump(&ctx, || list.model().is_some_and(|m| m.n_items() == 4)), "one leading row and three episodes");
            assert!(left.is_visible() && left.first_child().is_some(), "the left pane shows the info block");
            assert_eq!(list.margin_end(), 0, "a right side narrower than the list's widest leaves no spare room");
            // a big window: the list sits at the pane's edge at its widest, and the room past it goes to the right
            window.set_default_size(1900, 700);
            let spare = |page: i32| page - left.width_request() - 1 - crate::entry_core::constants::LIST_MAX_WIDTH;
            assert!(pump(&ctx, || page.width() >= 1800 && list.margin_end() == spare(page.width()) && list.margin_end() > 0), "spare room at the right, got {} at {}", list.margin_end(), page.width());
            window.set_child(None::<&gtk::Widget>);
            window.close();

            // 3000 episodes: the list view builds only the rows on screen, so the page opens without a stall
            let (_big_dir, big_store, big_id) = super::super::store::fixture::with_episodes(true, true, 3000, 10);
            let big_runner = Rc::new(runner::spawn(rt.handle(), big_store.clone(), kuro_prefs::store::PrefStore::in_memory(), Arc::new(Stub), EntryLoadContext::default()));
            let big = assemble(Rc::clone(&big_runner), true, env_of(&big_store));
            let big_window = gtk::Window::builder().default_width(1000).default_height(700).child(&big).build();
            big_window.present();
            big_runner.send(EntryCmd::Load { entry_id: big_id });
            let started = std::time::Instant::now();
            let big_list = descendants(big.upcast_ref()).into_iter().find_map(|w| w.downcast::<gtk::ListView>().ok()).expect("the list view");
            assert!(pump(&ctx, || big_list.model().is_some_and(|m| m.n_items() == 3001)), "3000 rows and the list head");
            assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
            for _ in 0..50 {
                while ctx.iteration(false) {}
                std::thread::sleep(Duration::from_millis(10));
            }
            // GTK prepares a few off-screen rows ahead; what matters is that 3000 rows do not make 3000 widgets
            let built = descendants(big_list.upcast_ref()).into_iter().filter(|w| w.type_().name() == "GtkListItemWidget").collect::<Vec<_>>();
            assert!(built.len() < 400, "only a window of rows is built, got {}", built.len());
            assert!(built.iter().filter(|w| w.is_mapped()).count() < 40, "only the visible rows are on screen");
            let newest = descendants(big_list.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).any(|l| l.label() == "Episode 3000" && l.is_mapped());
            assert!(newest, "the first row shows the newest episode");
            big_window.set_child(None::<&gtk::Widget>);
            big_window.close();

            let missing_id = 4242;
            let (missing, _r) = spawn(missing_id);
            let stack = stack_of(&missing);
            assert!(pump(&ctx, || stack.visible_child_name().as_deref() == Some("missing")), "a missing row ends in the missing state");
            let has_back = descendants(missing.upcast_ref()).iter().filter_map(|w| w.clone().downcast::<Button>().ok()).any(|b| b.label().as_deref() == Some("Back"));
            assert!(has_back, "the missing state offers Back");
        });
    }
}
