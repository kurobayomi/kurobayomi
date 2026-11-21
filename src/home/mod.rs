//! Home (docs/design/home.md): the page the app opens on. Two rows of covers, anime and manga together: Continue (what
//! the user is in the middle of, from the History of both kinds) and New for you (new units of library titles, from the
//! Updates feed of both kinds). Each row shows as many covers as fit the window; "See all" opens the full screen.
//! The rules are in [`data`]; this file reads the database, draws and routes clicks.

pub mod data;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use chrono::{Local, Months};
use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, Orientation as O};
use kuro_db::handle::Db;
use kuro_domain::history::model::HistoryWithRelations;
use kuro_domain::history::repo::HistoryRepo;
use kuro_domain::updates::model::UpdateRow;
use kuro_domain::updates::repo::UpdatesRepo;
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_platform::kind::Kind;

use crate::browse::look;
use crate::screens::Built;
use crate::theme::tokens::{SPACE_M, SPACE_S, SPACE_XS};
use crate::updates::coalesce::watch_coalesced;
use crate::updates::history::{COALESCE_MS, Resume};
use crate::updates::history_data;
use data::{NewCard, ROW_LIMIT, Spot};

/// The space between two covers of a row.
const GAP: i32 = 12;

thread_local! {
    /// The Home screen; its widgets hold only weak references, so this keeps it alive with the page.
    static HOME: RefCell<Option<Rc<Home>>> = const { RefCell::new(None) };
}

/// The Home page. `None` before the app data is open.
pub fn page() -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    let home = Home::new();
    let content = home.root.clone().upcast();
    HOME.with(|h| *h.borrow_mut() = Some(home));
    Some(Built { content, end: vec![] })
}

/// The rows each query last published, per kind (anime first).
#[derive(Default)]
struct Rows {
    history: [Option<Vec<HistoryWithRelations>>; 2],
    updates: [Option<Vec<UpdateRow>>; 2],
}

fn slot(kind: Kind) -> usize {
    usize::from(kind == Kind::Manga)
}

struct Home {
    root: GBox,
    /// Continue and New for you: each a heading, a row of covers and a "See all" link, hidden while it has nothing.
    continue_group: GBox,
    continue_row: GBox,
    new_group: GBox,
    new_row: GBox,
    empty: gtk::Widget,
    /// The first run's welcome view (the Library's own), shown while no source repository exists.
    welcome: GBox,
    rows: RefCell<Rows>,
    watches: RefCell<Vec<glib::JoinHandle<()>>>,
    /// The width the rows were last fitted to.
    width: std::cell::Cell<i32>,
}

impl Home {
    fn new() -> Rc<Self> {
        let header = crate::ui::header_bar();
        header.set_title_widget(Some(&gtk::Label::new(None)));
        crate::ui::window_buttons(&header);

        let (continue_group, continue_row) = group(&tr(MsgKey::action_continue), &tr(MsgKey::home_see_all_history), "history");
        let (new_group, new_row) = group(&tr(MsgKey::home_new_for_you), &tr(MsgKey::home_see_all_updates), "updates");
        let find = gtk::Button::with_label(&tr(MsgKey::home_find_something));
        find.add_css_class("suggested-action");
        find.add_css_class("pill");
        find.connect_clicked(|_| crate::ui::open("browse"));
        let empty: gtk::Widget = crate::widgets::status::page(Some("view-grid-symbolic"), &tr(MsgKey::home_empty), None, std::slice::from_ref(&find)).upcast();

        let column = GBox::new(O::Vertical, SPACE_M * 2);
        column.set_margin_start(SPACE_M + SPACE_S);
        column.set_margin_end(SPACE_M + SPACE_S);
        column.set_margin_top(SPACE_M);
        column.set_margin_bottom(SPACE_M);
        let welcome = crate::library::view::welcome::Welcome::new().widget().clone();
        welcome.set_visible(false);
        column.append(&welcome);
        column.append(&continue_group);
        column.append(&new_group);
        column.append(&empty);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&column).build();

        let root = GBox::new(O::Vertical, 0);
        root.append(&header);
        root.append(&scroller);

        let home = Rc::new(Self {
            root,
            continue_group,
            continue_row,
            new_group,
            new_row,
            empty,
            welcome,
            rows: RefCell::default(),
            watches: RefCell::default(),
            width: std::cell::Cell::new(0),
        });
        // the rows follow the width of the page: as many covers as fit
        let weak = Rc::downgrade(&home);
        scroller.hadjustment().connect_page_size_notify(move |adjustment| {
            if let Some(home) = weak.upgrade() {
                home.fit(adjustment.page_size() as i32);
            }
        });
        home.start();
        // adding (or removing) a repository loads the sources again: the welcome view gives way to the rows, or back
        let weak = Rc::downgrade(&home);
        crate::browse::glue::follow_changes(crate::recipe_sources::changes(), move |_| match weak.upgrade() {
            Some(home) => {
                home.render();
                true
            }
            None => false,
        });
        home
    }

    fn db(&self) -> Arc<Db> {
        crate::data::app().db.clone()
    }

    /// Follows the History and the Updates of both kinds; each change redraws the page.
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
                home.render();
            }));
            let weak = Rc::downgrade(self);
            watches.push(watch_coalesced(UpdatesRepo::new(self.db()).watch_recent(kind, since), COALESCE_MS, move |published| {
                let Some(home) = weak.upgrade() else { return };
                match published {
                    Ok(rows) => home.rows.borrow_mut().updates[slot(kind)] = Some((*rows).clone()),
                    Err(error) => tracing::warn!(%error, ?kind, "home: the updates could not be read"),
                }
                home.render();
            }));
        }
        *self.watches.borrow_mut() = watches;
    }

    fn render(self: &Rc<Self>) {
        let (history, updates) = {
            let rows = self.rows.borrow();
            let history = rows.history.clone();
            let updates: Vec<UpdateRow> = rows.updates.iter().flatten().flatten().cloned().collect();
            (history, updates)
        };
        let [anime, manga] = history;
        let continue_cards = self.continue_cards(anime.unwrap_or_default(), manga.unwrap_or_default());
        let new_cards = data::group_new(&updates, ROW_LIMIT);

        clear(&self.continue_row);
        for card in &continue_cards {
            self.continue_row.append(card);
        }
        clear(&self.new_row);
        for card in &new_cards {
            self.new_row.append(&new_cover(card));
        }
        let answered = {
            let rows = self.rows.borrow();
            rows.history.iter().all(Option::is_some) && rows.updates.iter().all(Option::is_some)
        };
        let shown = data::shown(crate::library::view::repositories_exist(), answered, !continue_cards.is_empty(), !new_cards.is_empty());
        self.welcome.set_visible(shown.welcome);
        self.continue_group.set_visible(shown.continue_row);
        self.new_group.set_visible(shown.new_row);
        self.empty.set_visible(shown.empty);
        self.fit(self.width.get());
    }

    /// The Continue covers: the newest history row of each title, both kinds, without the titles that have nothing
    /// left (the resume a History row would make finds no unit).
    fn continue_cards(&self, anime: Vec<HistoryWithRelations>, manga: Vec<HistoryWithRelations>) -> Vec<gtk::Widget> {
        let db = self.db();
        let items = kuro_data::item::ItemRepo::new(db.clone());
        // more rows than shown: finished titles drop out below
        data::merge_by_time(anime, manga, |row| row.at, ROW_LIMIT * 2)
            .into_iter()
            .filter_map(|(kind, row)| {
                let Resume::Open(unit) = history_data::resume_row(&db, kind, row.entry_id, row.item_id) else { return None };
                let stored = items.by_id(unit.id).ok().flatten();
                let (progress, total) = stored.as_ref().map_or((0, None), |item| (item.progress, item.total_seconds));
                let spot = data::spot(kind, unit.id == row.item_id, progress, total);
                Some(continue_cover(kind, &row, &unit, spot))
            })
            .take(ROW_LIMIT)
            .collect()
    }

    /// Shows the covers of each row that fit `width`, hides the rest.
    fn fit(&self, width: i32) {
        if width <= 0 {
            return;
        }
        self.width.set(width);
        let shown = data::fitting(width, cover_width(), GAP);
        for row in [&self.continue_row, &self.new_row] {
            let mut child = row.first_child();
            let mut index = 0;
            while let Some(widget) = child {
                widget.set_visible(index < shown);
                index += 1;
                child = widget.next_sibling();
            }
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

/// The width of a Home cover: the Library's, for the stored cover size.
fn cover_width() -> i32 {
    crate::widgets::cover_grid::cover_width(crate::data::app().library.cover_size.get())
}

fn clear(row: &GBox) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
}

/// A group: its heading with "See all" at the end, and the row its covers go in.
fn group(title: &str, see_all: &str, screen: &'static str) -> (GBox, GBox) {
    let heading = gtk::Label::builder().label(title).xalign(0.0).hexpand(true).css_classes(["heading"]).build();
    let link = gtk::Button::with_label(see_all);
    link.add_css_class("flat");
    link.connect_clicked(move |_| crate::ui::open(screen));
    let top = GBox::new(O::Horizontal, SPACE_S);
    top.append(&heading);
    top.append(&link);
    let row = GBox::new(O::Horizontal, GAP);
    let group = GBox::new(O::Vertical, SPACE_S);
    group.append(&top);
    group.append(&row);
    group.set_visible(false);
    (group, row)
}

/// The kind's name for the tag on a cover.
fn kind_label(kind: Kind) -> String {
    tr(if kind.is_anime() { MsgKey::label_anime } else { MsgKey::label_manga })
}

/// A cover with its badges over it, the title and the line under it. A click on the cover runs `open`; a click on the
/// title opens the title's page.
fn card(kind: Kind, entry_id: i64, title: &str, art: &kuro_images::art::ArtRef, badges: &[gtk::Widget], progress: Option<f64>, line: &str, open: impl Fn() + 'static) -> gtk::Widget {
    let width = cover_width();
    let height = crate::library::layout::cover_height(width);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&look::cover(title, Some(art), width, height)));
    overlay.add_css_class("kb-card-cover");
    overlay.set_overflow(gtk::Overflow::Hidden);
    let tags = GBox::new(O::Horizontal, SPACE_XS);
    tags.set_halign(Align::Start);
    tags.set_valign(Align::Start);
    tags.set_margin_start(SPACE_XS);
    tags.set_margin_top(SPACE_XS);
    for badge in badges {
        tags.append(badge);
    }
    overlay.add_overlay(&tags);
    if let Some(fraction) = progress {
        let bar = gtk::ProgressBar::builder().fraction(fraction).valign(Align::End).css_classes(["osd"]).build();
        overlay.add_overlay(&bar);
    }
    let click = gtk::GestureClick::new();
    click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open();
    });
    overlay.add_controller(click);
    overlay.set_cursor_from_name(Some("pointer"));

    let name = gtk::Label::builder().label(title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["kb-card-title"]).build();
    let title_button = gtk::Button::builder().child(&name).css_classes(["flat"]).halign(Align::Fill).build();
    title_button.connect_clicked(move |_| crate::screens::open_entry(kind, entry_id));
    let under = gtk::Label::builder().label(line).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label", "caption"]).build();

    let card = GBox::new(O::Vertical, SPACE_XS);
    card.set_size_request(width, -1);
    card.set_hexpand(false);
    card.append(&overlay);
    card.append(&title_button);
    card.append(&under);
    card.upcast()
}

/// A Continue cover: a click resumes as the History row would, straight into the player or the reader.
fn continue_cover(kind: Kind, row: &HistoryWithRelations, unit: &history_data::Unit, spot: Spot) -> gtk::Widget {
    let art = look::cover_art(kind, row.cover.entry_id, row.cover.source_id, row.cover.url.clone(), row.cover.favorite, row.cover.cover_last_modified);
    let line = match spot {
        Spot::Start => unit.name.clone(),
        Spot::Watched { seconds, total } => format!("{} · {}", unit.name, trf(MsgKey::home_minutes_left, &[("count", Arg::Int(data::minutes_left(seconds, total)))])),
        Spot::Page { last_read } => format!("{} · {}", unit.name, trf(MsgKey::home_page, &[("page", Arg::Int(last_read + 1))])),
    };
    let tag: gtk::Widget = look::badge(&kind_label(kind), false).upcast();
    let (entry_id, unit_id) = (row.entry_id, unit.id);
    card(kind, row.entry_id, &row.title, &art, &[tag], data::fraction(spot), &line, move || crate::updates::history_screen::open_unit_of(kind, entry_id, unit_id))
}

/// A New for you cover: a click opens the oldest new unit.
fn new_cover(card_data: &NewCard) -> gtk::Widget {
    let NewCard { kind, entry_id, first_unit_id, .. } = *card_data;
    let art = look::cover_art(kind, entry_id, card_data.source_id, card_data.thumbnail_url.clone(), true, card_data.cover_last_modified);
    let tag: gtk::Widget = look::badge(&kind_label(kind), false).upcast();
    let count = i64::try_from(card_data.count).unwrap_or(i64::MAX);
    let pill: gtk::Widget = look::badge(&trf(MsgKey::home_new_count, &[("count", Arg::Int(count))]), true).upcast();
    card(kind, entry_id, &card_data.title, &art, &[tag, pill], None, &card_data.first_unit_name, move || crate::updates::history_screen::open_unit_of(kind, entry_id, first_unit_id))
}
