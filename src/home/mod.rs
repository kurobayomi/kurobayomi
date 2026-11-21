//! Home (docs/design/home.md): the page the app opens on. Two rows of covers, anime and manga together: Continue (what
//! the user is in the middle of, from the History of both kinds) and New for you (new units of library titles, from the
//! Updates feed of both kinds). "See all" opens the full screen.
//!
//! Home draws nothing of its own that the app already has (fix pass of docs/design/home.md): each cover is the Library's
//! card (`library::view::card`) built from the Library's item (`kuro_domain::library::page::item`), so the badges, the
//! score, the cover size and the display mode are the Library's; a row holds as many covers as the Library's column
//! rule fits (`widgets::cover_grid::columns`); the right-click menu is the Library's (`library::mount::cover_menu`); the
//! header is the shared page header (`ui::page`, built by the tab host) with the shared actions; the refresh button is
//! the Updates page's (`updates::mount::update_libraries`); the welcome view is the Library's. The rules are in
//! [`data`]; this file reads the database, draws and routes clicks.

pub mod data;
pub mod discovery;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use chrono::{Local, Months};
use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, Orientation as O};
use kuro_db::handle::Db;
use kuro_domain::history::model::HistoryWithRelations;
use kuro_domain::history::repo::HistoryRepo;
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::page::LibraryItem;
use kuro_domain::updates::model::UpdateRow;
use kuro_domain::updates::repo::UpdatesRepo;
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_images::art::ArtRef;
use kuro_platform::kind::Kind;
use kuro_prefs::library::LibraryPrefs;

use crate::browse::look;
use crate::screens::Built;
use crate::theme::tokens::{SPACE_M, SPACE_S, SPACE_XS};
use crate::updates::coalesce::watch_coalesced;
use crate::updates::history::{COALESCE_MS, Resume};
use crate::updates::history_data;
use crate::widgets::cover_grid::{self, CELL_MARGIN_PX};
use data::{NewCard, ROW_LIMIT, Spot};

thread_local! {
    /// The Home screen; its widgets hold only weak references, so this keeps it alive with the page.
    static HOME: RefCell<Option<Rc<Home>>> = const { RefCell::new(None) };
}

/// The Home page: its content and the shared header actions (search across sources, refresh the library). The tab host
/// wraps it in the shared page header. `None` before the app data is open.
pub fn page() -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    let home = Home::new();
    let content = home.stack.clone().upcast();
    let search = crate::ui::icon_button("system-search-symbolic", &tr(MsgKey::action_global_search));
    search.connect_clicked(|_| crate::ui::open("global_search"));
    let refresh = crate::ui::refresh_button(&tr(MsgKey::action_update_library));
    refresh.connect_clicked(crate::updates::mount::update_libraries);
    let options = view_options(&home.prefs);
    let end = crate::ui::HeaderActions { refresh: Some(refresh.upcast()), extra: vec![search.upcast()], options: Some(options), ..Default::default() }.widgets();
    HOME.with(|h| *h.borrow_mut() = Some(home));
    Some(Built { content, end })
}

/// The Library's view options on Home (user, 2026-11-21: the same pieces as the Library): View, Grid style and Cover
/// size (`widgets::view_look`), writing the Library's own settings, so both pages always look alike. Home follows them
/// through its watches of the display mode and the cover size.
fn view_options(prefs: &Rc<LibraryPrefs>) -> gtk::Widget {
    use crate::widgets::view_look;
    let menu = crate::widgets::view_options::ViewOptions::new(&tr(MsgKey::ui_view_options), &view_look::sections());
    view_look::install(&menu, &prefs.cover_size);
    let written = Rc::clone(prefs);
    menu.connect(move |pick| {
        if let Some(next) = view_look::picked(&pick, written.display_mode.get()) {
            kuro_domain::category::service::set_display_mode(&written, next);
        }
    });
    // the radios show what is stored, also after a change on the Library's own menu
    let shown = menu.downgrade();
    crate::bind::bind(&menu.widget, &prefs.display_mode, move |_, mode| {
        if let Some(menu) = shown() {
            view_look::sync(&menu, mode);
        }
    });
    menu.widget.clone().upcast()
}

fn slot(kind: Kind) -> usize {
    usize::from(kind == Kind::Manga)
}

/// The rows each query last published, per kind (anime first).
#[derive(Default)]
struct Rows {
    history: [Option<Vec<HistoryWithRelations>>; 2],
    updates: [Option<Vec<UpdateRow>>; 2],
    /// The community score of each entry, per kind.
    scores: [HashMap<i64, f64>; 2],
    /// The last answer of the suggestions (`discovery`).
    discovery: Option<Arc<discovery::Cached>>,
}

/// What a click (or Enter) on a cover does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    /// Opens this unit of the entry, as a History or an Updates row does.
    Unit(i64),
    /// A suggestion, not on a source yet: global search for its title, for its kind.
    Search,
}

/// The source id of a suggestion's cover: no source has it, so the image cache fetches it with the shared client and no
/// source headers (`kuro_images::fetch::SourceHttp`, edge E-IMG-31).
const NO_SOURCE: i64 = -1;

/// One cover as drawn: everything it shows and the unit it opens. Equal lists draw the same row, so a change elsewhere
/// in the tables redraws nothing.
#[derive(Clone, Debug, PartialEq)]
struct Cover {
    kind: Kind,
    item: LibraryItem,
    in_library: bool,
    art: ArtRef,
    score: Option<f64>,
    line: String,
    progress: Option<f64>,
    action: Action,
}

/// A row of covers: its group (heading, "See all", the covers), and what it shows now.
struct Row {
    group: GBox,
    flow: gtk::FlowBox,
    covers: RefCell<Vec<Cover>>,
}

impl Row {
    /// `see_all` is the link's text and the screen it opens; a row with no full list behind it has none.
    fn new(title: &str, see_all: Option<(String, &'static str)>) -> Rc<Self> {
        let heading = gtk::Label::builder().label(title).xalign(0.0).hexpand(true).css_classes(["heading"]).build();
        let top = GBox::new(O::Horizontal, SPACE_S);
        top.append(&heading);
        if let Some((text, screen)) = see_all {
            let link = gtk::Button::with_label(&text);
            link.add_css_class("flat");
            link.connect_clicked(move |_| crate::ui::open(screen));
            top.append(&link);
        }
        // one line of covers: at most the columns that fit (`fit`), each cell the Library's column; its minimum is one cell
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(1)
            .max_children_per_line(2)
            .column_spacing(0)
            .row_spacing(0)
            .valign(Align::Start)
            .activate_on_single_click(true)
            .build();
        let group = GBox::new(O::Vertical, SPACE_S);
        group.append(&top);
        group.append(&flow);
        group.set_visible(false);
        let row = Rc::new(Self { group, flow, covers: RefCell::default() });
        // a click or Enter on a cover runs its action
        let weak = Rc::downgrade(&row);
        row.flow.connect_child_activated(move |_, child| {
            let Some(row) = weak.upgrade() else { return };
            let Some(cover) = usize::try_from(child.index()).ok().and_then(|i| row.covers.borrow().get(i).cloned()) else { return };
            match cover.action {
                Action::Unit(unit_id) => crate::updates::history_screen::open_unit_of(cover.kind, cover.item.row.id, unit_id),
                Action::Search => crate::browse::pending::open(crate::browse::route::BrowseRoute::GlobalSearch {
                    kind: cover.kind,
                    source_id: None,
                    query: cover.item.row.title.clone(),
                }),
            }
        });
        row
    }

    /// Draws `covers` when they differ from what the row shows (or always, with `force`: a new cover size or look). The
    /// Library's List mode draws the Library's list rows, one under the other; a grid mode its cards.
    fn draw(&self, covers: Vec<Cover>, mode: kuro_prefs::library::LibraryDisplayMode, force: bool) {
        if !force && *self.covers.borrow() == covers {
            return;
        }
        while let Some(child) = self.flow.first_child() {
            self.flow.remove(&child);
        }
        for cover in &covers {
            let built = if mode == kuro_prefs::library::LibraryDisplayMode::List { list_row(cover) } else { card(cover, mode) };
            self.flow.append(&built);
        }
        // cells are the Library's: its margin on every side
        let mut index = 0;
        while let Some(child) = self.flow.child_at_index(index) {
            crate::ui::margins(&child, CELL_MARGIN_PX);
            index += 1;
        }
        *self.covers.borrow_mut() = covers;
    }

    /// Lays the titles out at most `per_line` to a line and shows the first `shown`, hiding the rest. A short row keeps
    /// the Library's cell width: a card asks for no more than its cover (`card::title_label`), so the line still counts
    /// `per_line` cells.
    fn fit(&self, (per_line, shown): (i32, i32)) {
        let per_line = u32::try_from(per_line.max(1)).unwrap_or(1);
        self.flow.set_max_children_per_line(per_line);
        let mut index = 0;
        while let Some(child) = self.flow.child_at_index(index) {
            child.set_visible(index < shown);
            index += 1;
        }
    }
}

struct Home {
    /// Loading, then the content: the shared state switcher.
    stack: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    /// The column's side margins, taken off the scroller's width when counting columns.
    gutter: i32,
    continue_row: Rc<Row>,
    new_row: Rc<Row>,
    /// The suggestions: AniList's recommendations for the recent titles, and what is trending.
    recommended_row: Rc<Row>,
    trending_row: Rc<Row>,
    empty: gtk::Widget,
    /// The first run's welcome view (the Library's own), shown while no source repository exists.
    welcome: GBox,
    prefs: Rc<LibraryPrefs>,
    rows: RefCell<Rows>,
    watches: RefCell<Vec<glib::JoinHandle<()>>>,
    /// Something changed while Home was not on screen: it redraws when it shows again.
    stale: Cell<bool>,
    /// The next draw rebuilds every cover (the cover size or the look changed).
    force: Cell<bool>,
}

impl Home {
    fn new() -> Rc<Self> {
        let continue_row = Row::new(&tr(MsgKey::action_continue), Some((tr(MsgKey::home_see_all_history), "history")));
        let new_row = Row::new(&tr(MsgKey::home_new_for_you), Some((tr(MsgKey::home_see_all_updates), "updates")));
        let recommended_row = Row::new(&tr(MsgKey::home_recommended_for_you), None);
        let trending_row = Row::new(&tr(MsgKey::home_trending_now), None);
        let find = gtk::Button::with_label(&tr(MsgKey::home_find_something));
        find.add_css_class("suggested-action");
        find.add_css_class("pill");
        find.connect_clicked(|_| crate::ui::open("browse"));
        let empty: gtk::Widget = crate::widgets::status::page(Some("view-grid-symbolic"), &tr(MsgKey::home_empty), None, std::slice::from_ref(&find)).upcast();
        let welcome = crate::library::view::welcome::Welcome::new().widget().clone();
        welcome.set_visible(false);

        let gutter = SPACE_M + SPACE_S;
        let column = GBox::new(O::Vertical, SPACE_M * 2);
        column.set_margin_start(gutter);
        column.set_margin_end(gutter);
        column.set_margin_top(SPACE_M);
        column.set_margin_bottom(SPACE_M);
        column.append(&welcome);
        column.append(&continue_row.group);
        column.append(&new_row.group);
        column.append(&recommended_row.group);
        column.append(&trending_row.group);
        column.append(&empty);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&column).build();

        let stack = crate::widgets::status::switcher();
        stack.add_named(&crate::widgets::status::loading(None), Some("loading"));
        stack.add_named(&scroller, Some("content"));
        stack.set_visible_child_name("loading");

        let prefs = Rc::new(LibraryPrefs::new(crate::data::app().base.store().clone()));
        let home = Rc::new(Self {
            stack,
            scroller,
            gutter,
            continue_row,
            new_row,
            recommended_row,
            trending_row,
            empty,
            welcome,
            prefs,
            rows: RefCell::default(),
            watches: RefCell::default(),
            stale: Cell::new(true),
            force: Cell::new(false),
        });
        home.wire();
        home
    }

    fn wire(self: &Rc<Self>) {
        // the rows follow the width of the page, as the Library's grid does
        let weak = Rc::downgrade(self);
        self.scroller.hadjustment().connect_page_size_notify(move |_| {
            if let Some(home) = weak.upgrade() {
                home.fit();
            }
        });
        // shown again after a change while away: draw it now
        let weak = Rc::downgrade(self);
        self.stack.connect_map(move |_| {
            if let Some(home) = weak.upgrade().filter(|home| home.stale.get()) {
                home.render();
            }
        });
        // the Library's cover size and look apply here too, at once (the first call of each is the seed), and so does
        // Show (Settings > Appearance): with one kind shown alone, every row holds only it
        for redraw in [Self::bind_cover_size, Self::bind_display_mode, Self::bind_kinds_shown] {
            redraw(self);
        }
        self.start();
        // adding (or removing) a repository loads the sources again: the welcome view gives way to the rows, or back
        let weak = Rc::downgrade(self);
        crate::browse::glue::follow_changes(crate::recipe_sources::changes(), move |_| match weak.upgrade() {
            Some(home) => {
                home.changed();
                true
            }
            None => false,
        });
        // the suggestions' cache: its last answer now, then each refresh
        let weak = Rc::downgrade(self);
        crate::browse::glue::follow_changes(discovery::follow(), move |cached| match weak.upgrade() {
            Some(home) => {
                home.rows.borrow_mut().discovery = cached;
                home.changed();
                true
            }
            None => false,
        });
        for kind in [Kind::Anime, Kind::Manga] {
            let Some(scores) = crate::community::follow_scores(kind) else { continue };
            let weak = Rc::downgrade(self);
            crate::browse::glue::follow_changes(scores, move |scores| match weak.upgrade() {
                Some(home) => {
                    home.rows.borrow_mut().scores[slot(kind)] = scores;
                    home.changed();
                    true
                }
                None => false,
            });
        }
    }

    fn bind_cover_size(self: &Rc<Self>) {
        let (weak, seeded) = (Rc::downgrade(self), Cell::new(false));
        crate::bind::bind(&self.stack, &self.prefs.cover_size, move |_, _| {
            if !seeded.replace(true) {
                return;
            }
            if let Some(home) = weak.upgrade() {
                home.force.set(true);
                home.changed();
            }
        });
    }

    fn bind_display_mode(self: &Rc<Self>) {
        let (weak, seeded) = (Rc::downgrade(self), Cell::new(false));
        crate::bind::bind(&self.stack, &self.prefs.display_mode, move |_, _| {
            if !seeded.replace(true) {
                return;
            }
            if let Some(home) = weak.upgrade() {
                home.force.set(true);
                home.changed();
            }
        });
    }

    fn bind_kinds_shown(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        crate::shell::kind_choice::on_change(move || {
            if let Some(home) = weak.upgrade() {
                home.force.set(true);
                home.changed();
            }
        });
    }

    fn db(&self) -> Arc<Db> {
        crate::data::app().db.clone()
    }

    /// Follows the History and the Updates of both kinds.
    fn start(self: &Rc<Self>) {
        let months = u32::try_from(kuro_domain::UPDATES_WINDOW_MONTHS).unwrap_or(3);
        let since = Local::now().checked_sub_months(Months::new(months)).map_or(0, |t| t.timestamp_millis());
        let mut watches = Vec::new();
        for kind in [Kind::Anime, Kind::Manga] {
            let weak = Rc::downgrade(self);
            watches.push(watch_coalesced(HistoryRepo::new(self.db()).watch_list(kind, String::new()), COALESCE_MS, move |published| {
                let Some(home) = weak.upgrade() else { return };
                match published {
                    Ok(rows) => home.rows.borrow_mut().history[slot(kind)] = Some((*rows).clone()),
                    Err(error) => tracing::warn!(%error, ?kind, "home: the history could not be read"),
                }
                home.changed();
            }));
            let weak = Rc::downgrade(self);
            watches.push(watch_coalesced(UpdatesRepo::new(self.db()).watch_recent(kind, since), COALESCE_MS, move |published| {
                let Some(home) = weak.upgrade() else { return };
                match published {
                    Ok(rows) => home.rows.borrow_mut().updates[slot(kind)] = Some((*rows).clone()),
                    Err(error) => tracing::warn!(%error, ?kind, "home: the updates could not be read"),
                }
                home.changed();
            }));
        }
        *self.watches.borrow_mut() = watches;
    }

    /// Something Home shows may have changed: it redraws now when it is on screen, else when it shows again. The player
    /// saves its position every few seconds; while it plays, Home is not on screen and reads nothing.
    fn changed(self: &Rc<Self>) {
        if self.stack.is_mapped() {
            self.render();
        } else {
            self.stale.set(true);
        }
    }

    fn render(self: &Rc<Self>) {
        self.stale.set(false);
        let answered = {
            let rows = self.rows.borrow();
            rows.history.iter().all(Option::is_some) && rows.updates.iter().all(Option::is_some)
        };
        let repositories = crate::library::view::repositories_exist();
        // the spinner until the first answers, unless the welcome view has the page anyway
        self.stack.set_visible_child_name(if answered || !repositories { "content" } else { "loading" });
        if !answered && repositories {
            return;
        }
        let (continue_covers, new_covers, recommended, trending) = self.covers();
        let shown = data::shown(repositories, answered, !continue_covers.is_empty(), !new_covers.is_empty());
        // the suggestions show beside the rows (or on their own), never with the welcome view
        let (show_recommended, show_trending) = (!shown.welcome && !recommended.is_empty(), !shown.welcome && !trending.is_empty());
        let mode = self.prefs.display_mode.get();
        let force = self.force.replace(false);
        self.continue_row.draw(continue_covers, mode, force);
        self.new_row.draw(new_covers, mode, force);
        self.recommended_row.draw(recommended, mode, force);
        self.trending_row.draw(trending, mode, force);
        self.welcome.set_visible(shown.welcome);
        self.continue_row.group.set_visible(shown.continue_row);
        self.new_row.group.set_visible(shown.new_row);
        self.recommended_row.group.set_visible(show_recommended);
        self.trending_row.group.set_visible(show_trending);
        self.empty.set_visible(shown.empty && !show_recommended && !show_trending);
        self.fit();
    }

    /// Every row's covers from the published rows: Continue, New for you, Recommended for you, Trending now.
    fn covers(&self) -> (Vec<Cover>, Vec<Cover>, Vec<Cover>, Vec<Cover>) {
        // the kinds Show allows (Settings > Appearance): a hidden kind's titles leave every row before it is cut to size
        let shown = crate::shell::kind_choice::is_shown;
        let (history, updates, scores, cached) = {
            let rows = self.rows.borrow();
            let updates: Vec<UpdateRow> =
                [Kind::Anime, Kind::Manga].into_iter().filter(|kind| shown(*kind)).flat_map(|kind| rows.updates[slot(kind)].iter().flatten().cloned()).collect();
            (rows.history.clone(), updates, rows.scores.clone(), rows.discovery.clone())
        };
        let items = Items::new(self.db());
        let score = |kind: Kind, id: i64| scores[slot(kind)].get(&id).copied();

        let [anime, manga] = history;
        // what the user has, so the suggestions leave it out: the titles of the library and the history, and the seeds
        let mut known = discovery::Known::default();
        for kind in [Kind::Anime, Kind::Manga] {
            known.titles.extend(items.library_rows(kind).values().map(|row| discovery::fold(&row.title)));
        }
        known.titles.extend(anime.iter().chain(manga.iter()).flatten().map(|row| discovery::fold(&row.title)));
        if let Some(cached) = &cached {
            known.mal_ids.extend(cached.seeds.iter().map(|(_, id)| *id));
        }
        let db = self.db();
        let item_repo = kuro_data::item::ItemRepo::new(db.clone());
        // more rows than a row holds: titles with nothing left drop out below
        let of_kind = |kind: Kind, rows: Option<Vec<_>>| if shown(kind) { rows.unwrap_or_default() } else { Vec::new() };
        let continue_covers: Vec<Cover> = data::merge_by_time(of_kind(Kind::Anime, anime), of_kind(Kind::Manga, manga), |row| row.at, ROW_LIMIT * 2)
            .into_iter()
            .filter_map(|(kind, row)| {
                let Resume::Open(unit) = history_data::resume_row(&db, kind, row.entry_id, row.item_id) else { return None };
                let stored = item_repo.by_id(unit.id).ok().flatten();
                let (progress, total) = stored.as_ref().map_or((0, None), |item| (item.progress, item.total_seconds));
                let spot = data::spot(kind, unit.id == row.item_id, progress, total);
                let (item, in_library) = items.get(kind, row.entry_id, None)?;
                let art = look::cover_art(kind, row.cover.entry_id, row.cover.source_id, row.cover.url.clone(), row.cover.favorite, row.cover.cover_last_modified);
                Some(Cover { kind, item, in_library, art, score: score(kind, row.entry_id), line: spot_line(&unit.name, spot), progress: data::fraction(spot), action: Action::Unit(unit.id) })
            })
            .take(ROW_LIMIT)
            .collect();

        let new_covers: Vec<Cover> = data::group_new(&updates, ROW_LIMIT)
            .into_iter()
            .filter_map(|card: NewCard| {
                // the unread badge counts the new units here
                let (item, in_library) = items.get(card.kind, card.entry_id, Some(i64::try_from(card.count).unwrap_or(i64::MAX)))?;
                let art = look::cover_art(card.kind, card.entry_id, card.source_id, card.thumbnail_url.clone(), true, card.cover_last_modified);
                Some(Cover { kind: card.kind, item, in_library, art, score: score(card.kind, card.entry_id), line: card.first_unit_name.clone(), progress: None, action: Action::Unit(card.first_unit_id) })
            })
            .collect();

        let adult = kuro_prefs::source::SourcePrefs::new(crate::data::app().base.store().clone()).show_nsfw_source.get();
        let (recommended, trending) = match &cached {
            Some(cached) => (
                discovery::recommended(&discovery::of_kinds(&cached.answer.recommended, shown), &known, adult, ROW_LIMIT),
                discovery::trending(
                    if shown(Kind::Anime) { cached.answer.trending_anime.as_slice() } else { &[] },
                    if shown(Kind::Manga) { cached.answer.trending_manga.as_slice() } else { &[] },
                    &known,
                    adult,
                    ROW_LIMIT,
                ),
            ),
            None => (Vec::new(), Vec::new()),
        };
        (continue_covers, new_covers, recommended.iter().map(suggestion_cover).collect(), trending.iter().map(suggestion_cover).collect())
    }

    /// Fits every row to the page's width with the Library's column rule: one line each, as many covers as fit.
    fn fit(&self) {
        let width = self.scroller.width() - 2 * self.gutter;
        if width <= 0 {
            return;
        }
        let layout = data::layout_of(self.prefs.display_mode.get(), cover_grid::columns(width, self.prefs.cover_size.get()));
        for row in [&self.continue_row, &self.new_row, &self.recommended_row, &self.trending_row] {
            row.fit(layout);
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for watch in self.watches.borrow_mut().drain(..) {
            watch.abort();
        }
    }
}

/// The Library's items for Home's covers: a library title exactly as the Library reads it; a title only in the history
/// gets the same row from its entry, its units counted. The badges are the Library's switches.
struct Items {
    db: Arc<Db>,
    library: RefCell<HashMap<Kind, HashMap<i64, LibraryRow>>>,
    badges: kuro_domain::library::page::BadgeSettings,
    downloads: HashMap<Kind, Arc<dyn Fn(&LibraryRow) -> i64 + Send + Sync>>,
}

impl Items {
    fn new(db: Arc<Db>) -> Self {
        let index = crate::shell::host::Shell::get().and_then(|shell| shell.download_service()).map(|service| service.index().clone());
        let downloads = [Kind::Anime, Kind::Manga].into_iter().map(|kind| (kind, crate::library::download_facts::lookup(kind, index.clone()))).collect();
        Self { db, library: RefCell::default(), badges: crate::library::load::badges(), downloads }
    }

    /// The item of entry `id` and whether it is in the library; `unread` replaces the unread count (New for you counts
    /// its new units). `None` for an entry that cannot be read.
    fn get(&self, kind: Kind, id: i64, unread: Option<i64>) -> Option<(LibraryItem, bool)> {
        let library_row = self.library_rows(kind).get(&id).cloned();
        let in_library = library_row.is_some();
        let row = match library_row {
            Some(row) => row,
            None => self.history_only_row(kind, id)?,
        };
        let db = self.db.clone();
        let language = move |source: i64| crate::library::mount::source_info(&db, kind, source).language;
        let downloads = self.downloads.get(&kind)?;
        let mut item = kuro_domain::library::page::item(&row, self.badges, &|row: &LibraryRow| downloads(row), &language);
        if let Some(count) = unread.filter(|_| self.badges.unread) {
            item.unread_count = count;
        }
        Some((item, in_library))
    }

    /// The library rows of `kind`, read once per draw: the first row of each entry (an entry in two categories has two).
    fn library_rows(&self, kind: Kind) -> std::cell::Ref<'_, HashMap<i64, LibraryRow>> {
        if !self.library.borrow().contains_key(&kind) {
            let rows: HashMap<i64, LibraryRow> = kuro_data::entry::EntryRepo::new(self.db.clone())
                .library(kind)
                .unwrap_or_default()
                .iter()
                .rev()
                .map(|entry| (entry.entry.id, LibraryRow::from_entry(entry)))
                .collect();
            self.library.borrow_mut().insert(kind, rows);
        }
        std::cell::Ref::map(self.library.borrow(), |all| &all[&kind])
    }

    /// A title outside the library, as a library row: its entry with its units counted.
    fn history_only_row(&self, kind: Kind, id: i64) -> Option<LibraryRow> {
        let entry = kuro_data::entry::EntryRepo::new(self.db.clone()).by_id(id).ok()?;
        let units = history_data::ordered_units(&self.db, kind, id);
        let total = i64::try_from(units.len()).unwrap_or(i64::MAX);
        let done = i64::try_from(units.iter().filter(|unit| unit.done).count()).unwrap_or(0);
        let library_entry = kuro_data::entry::LibraryEntry { entry, category: 0, total, done, bookmarks: 0, fillermarks: 0, latest_upload: 0, item_fetched_at: 0, last_activity: 0 };
        Some(LibraryRow::from_entry(&library_entry))
    }
}

/// A suggestion's cover: the Library's card drawn from a row that holds only the title (no counts, so no unread or
/// download badge), AniList's cover and score, the kind under it, and a search for it on click.
fn suggestion_cover(title: &kuro_track::tracker::anilist::discovery::Suggestion) -> Cover {
    let row = LibraryRow {
        id: 0,
        source: NO_SOURCE,
        title: title.title.clone(),
        category: 0,
        total: 0,
        done: 0,
        bookmarks: 0,
        fillermarks: 0,
        latest_upload: 0,
        item_fetched_at: 0,
        last_activity: 0,
        last_update: 0,
        date_added: 0,
        status: 0,
        fetch_interval: 0,
        viewer_flags: 0,
        always_update: false,
        next_update: 0,
    };
    let not_computed = kuro_domain::library::page::NOT_COMPUTED;
    let item = LibraryItem { row, download_count: not_computed, unread_count: not_computed, is_local: false, source_language: String::new() };
    let art = look::cover_art(title.kind, 0, NO_SOURCE, title.cover.clone(), false, 0);
    let line = tr(if title.kind.is_anime() { MsgKey::label_anime } else { MsgKey::label_manga });
    Cover { kind: title.kind, item, in_library: false, art, score: title.score, line, progress: None, action: Action::Search }
}

/// The line under a Continue cover: the unit a click opens, and how far into it.
fn spot_line(unit: &str, spot: Spot) -> String {
    match spot {
        Spot::Start => unit.to_owned(),
        Spot::Watched { seconds, total } => format!("{unit} · {}", trf(MsgKey::home_minutes_left, &[("count", Arg::Int(data::minutes_left(seconds, total)))])),
        Spot::Page { last_read } => format!("{unit} · {}", trf(MsgKey::home_page, &[("page", Arg::Int(last_read + 1))])),
    }
}

/// A cover: the Library's card for the stored cover size and look, the theme's progress bar at the foot of the cover
/// for an episode under way, Home's line under the card, and the Library's right-click menu.
fn card(cover: &Cover, mode: kuro_prefs::library::LibraryDisplayMode) -> gtk::Widget {
    let cover_size = crate::data::app().library.cover_size.get();
    let extras = crate::library::view::card::Extras {
        art: Some(&cover.art),
        kind: cover.kind,
        season_parent: false,
        width: cover_grid::cover_width(cover_size),
        score: cover.score,
    };
    let built = crate::library::view::card::build(&cover.item, mode, false, &extras);
    add_progress(&built.root, cover.progress);
    if let Some(root) = built.root.downcast_ref::<GBox>() {
        let line = gtk::Label::builder().label(&cover.line).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(1).css_classes(["dim-label", "caption"]).build();
        line.set_margin_top(-SPACE_XS);
        root.append(&line);
    }
    // the Library's right-click menu, for a title on a source (a suggestion is not one yet)
    if matches!(cover.action, Action::Unit(_)) {
        let (kind, row, in_library) = (cover.kind, cover.item.row.clone(), cover.in_library);
        crate::widgets::context_menu::attach(&built.root, move |_, _| crate::library::mount::cover_menu(kind, &row, in_library));
    }
    built.root
}

/// The thin line of how far a title is (`kb-cover-progress`), on the bottom edge of the cover that leads `root`.
fn add_progress(root: &gtk::Widget, progress: Option<f64>) {
    let Some(fraction) = progress else { return };
    let Some(overlay) = root.first_child().and_downcast::<gtk::Overlay>() else { return };
    let bar = gtk::ProgressBar::builder().fraction(fraction).valign(Align::End).halign(Align::Fill).can_target(false).css_classes(["kb-cover-progress"]).build();
    overlay.add_overlay(&bar);
}

/// A title as the Library's List mode row (`card::build_list`, its row height), with Home's line at its end and the
/// Library's right-click menu.
fn list_row(cover: &Cover) -> gtk::Widget {
    let extras = crate::library::view::card::Extras {
        art: Some(&cover.art),
        kind: cover.kind,
        season_parent: false,
        width: 0,
        score: cover.score,
    };
    let window = crate::ui::window();
    let row_height = crate::library::layout::page_list_row_height(window.width(), f64::from(window.height()));
    let built = crate::library::view::card::build_list(&cover.item, false, row_height, &extras);
    add_progress(&built.root, cover.progress);
    if let Some(root) = built.root.downcast_ref::<GBox>() {
        let line = gtk::Label::builder().label(&cover.line).xalign(1.0).valign(Align::Center).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label", "caption"]).build();
        root.append(&line);
    }
    if matches!(cover.action, Action::Unit(_)) {
        let (kind, row, in_library) = (cover.kind, cover.item.row.clone(), cover.in_library);
        crate::widgets::context_menu::attach(&built.root, move |_, _| crate::library::mount::cover_menu(kind, &row, in_library));
    }
    built.root
}
