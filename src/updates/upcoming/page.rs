//! The Upcoming page of one kind (T-42b-027 to T-42b-035, T-42b-038): the calendar and the list of entries with an
//! update coming, a boxed list per day under its date heading with a worded count badge. On a content width of 720 px or more the calendar and the list
//! sit in a resizable [`gtk::Paned`]; below that the calendar is the first row of the list. A click on a day scrolls the
//! list to that day's heading. The page has no empty-state text and no toast (U§2.1, U§3).
//!
//! The grouping and the numbers come from [`model`], [`layout`] and [`calendar`]; the widgets here stay thin.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{Local, NaiveDate, Weekday};
use gtk::gdk;
use gtk::glib;
#[cfg(test)]
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, Label, ListBoxRow, Orientation as O};
use kuro_platform::kind::Kind;
use kuro_i18n::{MsgKey, tr};

use crate::browse::look;
use crate::data;
use crate::updates::download_host;
use crate::misc::diet_defaults as diet;
use crate::updates::upcoming::calendar::week_start_for;
use crate::updates::upcoming::calendar_view::CalendarView;
use crate::updates::upcoming::layout::{self, Shape};
use crate::updates::upcoming::model::{self, Entry, Row, Upcoming};
use crate::updates::upcoming::{query, rows};

/// How often a shown page looks at the clock for a day change (E-C18).
const DAY_CHECK_SECONDS: u32 = 60;

thread_local! {
    /// The pages on the navigation stack; their widgets hold only weak references, so this keeps them alive until the
    /// page is destroyed.
    static PAGES: RefCell<Vec<Rc<UpcomingPage>>> = const { RefCell::new(Vec::new()) };
}

/// What the page reads from the app: the week start, the clock, the date preferences, and what a tap on an entry does.
/// The page takes it as a value so its layout and its scroll can run without the app data.
struct Env {
    first: Weekday,
    today: NaiveDate,
    relative: bool,
    date_pattern: String,
    locale: &'static str,
    /// The opt-in Home key (T-42b-046).
    home_today: bool,
    open: Rc<dyn Fn(i64)>,
}

impl Env {
    /// The live environment of one kind. Needs the app data.
    fn current(kind: Kind) -> Self {
        let locale = kuro_i18n::current_locale().tag;
        Self {
            first: week_start_for(diet::UPCOMING_WEEK_START, locale),
            today: Local::now().date_naive(),
            relative: diet::LIBRARY_RELATIVE_TIME,
            date_pattern: diet::LIBRARY_DATE_FORMAT.to_owned(),
            locale,
            home_today: diet::LIBRARY_UPCOMING_HOME_RETURNS_TODAY,
            open: Rc::new(move |id| open_entry(kind, id)),
        }
    }
}

/// The loaded entries and the day they were loaded for.
struct Loaded {
    up: Upcoming,
    day: NaiveDate,
}

pub struct UpcomingPage {
    kind: Kind,
    root: GBox,
    paned: gtk::Paned,
    scroll: gtk::ScrolledWindow,
    /// The list: the calendar row on a single column, then a heading and its boxed list per day.
    content: GBox,
    /// The widget of each list position of [`model::day_jump`]: the calendar row on a single column, then each
    /// heading and each entry row in index order.
    targets: RefCell<Vec<gtk::Widget>>,
    /// Holds the calendar, or the clamp that caps its width; it moves between the paned and the list.
    calendar_slot: GBox,
    calendar: Rc<CalendarView>,
    env: Env,
    loaded: RefCell<Option<Loaded>>,
    shape: Cell<Option<Shape>>,
}

impl UpcomingPage {
    fn new(kind: Kind, env: Env) -> Rc<Self> {
        let page = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let jump = weak.clone();
            let calendar = CalendarView::new(env.first, env.today, move |day| {
                if let Some(page) = jump.upgrade() {
                    page.jump(day);
                }
            });
            calendar.set_home_returns_today(env.home_today);
            let content = crate::ui::column();
            let scroll = crate::ui::scrolled(&content);
            scroll.set_hexpand(true);
            let paned = gtk::Paned::builder()
                .orientation(O::Horizontal)
                .wide_handle(true)
                .resize_start_child(true)
                .shrink_start_child(false)
                .resize_end_child(true)
                .shrink_end_child(false)
                .vexpand(true)
                .build();
            paned.set_end_child(Some(&scroll));
            let root = GBox::new(O::Vertical, 0);
            root.append(&paned);
            Self {
                kind,
                root,
                paned,
                scroll,
                content,
                targets: RefCell::default(),
                calendar_slot: GBox::new(O::Vertical, 0),
                calendar,
                env,
                loaded: RefCell::new(None),
                shape: Cell::new(None),
            }
        });
        page.watch_width();
        page
    }

    /// Starts the live behaviour: the query, and the look at the clock when the page is shown.
    fn start(self: &Rc<Self>) {
        self.watch_day();
        self.reload();
    }

    /// Applies the layout of the content width on the first frame and whenever it crosses a breakpoint.
    fn watch_width(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.paned.add_tick_callback(move |paned, _| {
            let Some(page) = weak.upgrade() else { return glib::ControlFlow::Break };
            let width = paned.width();
            if width > 0 {
                let shape = layout::shape(width);
                if page.shape.get() != Some(shape) {
                    page.apply_shape(shape, width);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Re-queries when the page is shown, or is open, on a later day than the one it was loaded for.
    fn watch_day(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.root.connect_map(move |_| {
            if let Some(page) = weak.upgrade() {
                page.refresh_if_new_day();
            }
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(DAY_CHECK_SECONDS, move || {
            let Some(page) = weak.upgrade() else { return glib::ControlFlow::Break };
            if page.root.is_mapped() {
                page.refresh_if_new_day();
            }
            glib::ControlFlow::Continue
        });
    }

    fn refresh_if_new_day(self: &Rc<Self>) {
        let today = Local::now().date_naive();
        let stale = self.loaded.borrow().as_ref().is_some_and(|loaded| model::needs_requery(loaded.day, today));
        if stale {
            self.calendar.set_today(today);
            self.reload();
        }
    }

    /// Reads the entries off the GTK thread and shows them. A failure leaves the page as it is: there is no toast.
    fn reload(self: &Rc<Self>) {
        let (db, kind) = (data::app().db.clone(), self.kind);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let now = Local::now();
            let result = download_host::blocking(move || query::load(&db, kind, now)).await;
            let Some(page) = weak.upgrade() else { return };
            match result {
                Some(Ok(up)) => page.show(up, now.date_naive()),
                Some(Err(error)) => tracing::warn!("the upcoming entries could not be read: {error}"),
                None => tracing::warn!("the upcoming query task was dropped"),
            }
        });
    }

    fn show(self: &Rc<Self>, up: Upcoming, day: NaiveDate) {
        self.calendar.set_today(day);
        self.calendar.set_events(up.index.events.clone());
        *self.loaded.borrow_mut() = Some(Loaded { up, day });
        self.fill();
    }

    /// Moves the calendar between the paned and the list and caps its width in the medium band.
    fn apply_shape(self: &Rc<Self>, shape: Shape, width: i32) {
        self.shape.set(Some(shape));
        // the slot leaves whichever container holds it: the paned (two panels) or the list (single column)
        if self.paned.start_child().as_ref() == Some(self.calendar_slot.upcast_ref()) {
            self.paned.set_start_child(None::<&gtk::Widget>);
        } else if self.calendar_slot.parent().is_some() {
            self.content.remove(&self.calendar_slot);
        }
        while let Some(child) = self.calendar_slot.first_child() {
            self.calendar_slot.remove(&child);
        }
        let calendar = self.calendar.widget();
        if calendar.parent().is_some() {
            calendar.unparent();
        }
        match shape.calendar_cap {
            Some(max) => self.calendar_slot.append(&crate::clamp::clamp(calendar, max)),
            None => self.calendar_slot.append(calendar),
        }
        self.calendar_slot.set_valign(if shape.two_panels { Align::Start } else { Align::Fill });
        if shape.two_panels {
            self.paned.set_start_child(Some(&self.calendar_slot));
            self.paned.set_position(layout::divider_start(width));
        }
        self.fill();
    }

    fn two_panels(&self) -> bool {
        self.shape.get().is_some_and(|shape| shape.two_panels)
    }

    /// Rebuilds the list: the calendar row first on a single column, then a heading and a boxed list per day.
    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let mut targets = self.targets.borrow_mut();
        targets.clear();
        if self.shape.get().is_none() {
            return;
        }
        if !self.two_panels() {
            self.content.append(&self.calendar_slot);
            targets.push(self.calendar_slot.clone().upcast());
        }
        let loaded = self.loaded.borrow();
        let Some(loaded) = loaded.as_ref() else { return };
        let env = &self.env;
        let mut list = crate::ui::boxed_list();
        for row in &loaded.up.index.rows {
            match row {
                Row::Header { date, count } => {
                    let line = heading(&rows::header_text(*date, loaded.day, env.relative, &env.date_pattern, env.locale), *count);
                    list = crate::ui::boxed_list();
                    self.content.append(&line);
                    self.content.append(&list);
                    targets.push(line.upcast());
                }
                Row::Item(i) => {
                    let item = self.item(&loaded.up.entries[*i]);
                    list.append(&item);
                    targets.push(item.upcast());
                }
            }
        }
    }

    /// One entry row: the book cover (initials until an image draws) and the title (two lines at most); activating
    /// the row (click, Enter or Space) opens the entry by id.
    fn item(&self, entry: &Entry) -> ListBoxRow {
        let art = look::cover_art(self.kind, entry.id, entry.source_id, entry.cover_url.clone(), true, entry.cover_last_modified);
        let cover = look::row_cover(&entry.title, Some(&art), look::MEDIUM.width, look::MEDIUM.height);
        let row = look::list_row(Some(cover), &entry.title, None, None);
        // a long title keeps its two lines
        if let Some(title) = crate::updates::row_texts(&row).first_child().and_downcast::<Label>() {
            title.set_wrap(true);
            title.set_lines(2);
        }
        let (open, id) = (Rc::clone(&self.env.open), entry.id);
        crate::widgets::row_activate::on_activate(&row, move |_| open(id));
        row
    }

    /// A click on a day: scrolls to its heading, one row further down on a single column where the calendar row comes
    /// first. A day with no heading does nothing.
    fn jump(&self, day: NaiveDate) {
        let target = self.loaded.borrow().as_ref().and_then(|loaded| model::day_jump(&loaded.up.index, day, !self.two_panels()));
        let Some(position) = target else { return };
        let widget = self.targets.borrow().get(position).cloned();
        if let Some(bounds) = widget.and_then(|w| w.compute_bounds(&self.content)) {
            self.scroll.vadjustment().set_value(f64::from(bounds.y()));
        }
    }
}

/// The heading of a day (`ui::heading`) and a badge that says how many entries it has.
fn heading(text: &str, count: u32) -> GBox {
    let line = GBox::new(O::Horizontal, 8);
    let label = crate::ui::heading(text);
    // the margin goes on the line, so the badge sits level with the text
    label.set_margin_top(0);
    line.set_margin_top(12);
    line.append(&label);
    line.append(&crate::browse::look::badge(&rows::badge_text(count), false));
    line
}

/// Pushes the entry screen of the entry by its id.
fn open_entry(kind: Kind, id: i64) {
    match kind {
        Kind::Anime => crate::entry_core::details::route::open(id),
        Kind::Manga => crate::entry_core::details::route::open_kind(Kind::Manga, id),
    }
}

/// The "more" menu: the Upcoming guide, which opens in the default browser.
fn help_menu() -> Vec<Vec<crate::ui::MenuEntry>> {
    vec![vec![crate::ui::MenuEntry::new(tr(MsgKey::upcoming_guide), || {
        crate::ui::open_link(layout::help_url(), Some(crate::ui::window().upcast_ref()));
    })]]
}

/// The navigation page of one kind: the title Upcoming, the back arrow, the guide in the more menu, and Escape that goes back. `None`
/// before the app data is open.
pub fn build(kind: Kind) -> Option<GBox> {
    if !data::is_installed() {
        return None;
    }
    let screen = UpcomingPage::new(kind, Env::current(kind));
    screen.start();
    Some(assemble(screen))
}

/// The navigation page around a screen, and the owner that keeps the screen alive until the page is destroyed.
fn assemble(screen: Rc<UpcomingPage>) -> GBox {
    let page = crate::ui::page(&tr(MsgKey::label_upcoming), &screen.root, crate::ui::HeaderActions { menu: help_menu(), ..Default::default() }.widgets(), true);
    crate::ui::tag(&page, "upcoming");
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, _| {
        if key == gdk::Key::Escape {
            crate::ui::pop();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    page.add_controller(keys);
    let owner = Rc::clone(&screen);
    PAGES.with(|pages| pages.borrow_mut().push(owner));
    let weak = Rc::downgrade(&screen);
    page.connect_destroy(move |_| {
        if let Some(screen) = weak.upgrade() {
            PAGES.with(|pages| pages.borrow_mut().retain(|p| !Rc::ptr_eq(p, &screen)));
        }
    });
    page
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).expect("a date")
    }

    fn at(day: u32, hour: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).single().expect("a time").timestamp_millis()
    }

    fn entry(id: i64, next_update_ms: i64) -> Entry {
        Entry { id, title: format!("Entry {id}"), status: 1, source_id: 7, next_update_ms, cover_url: None, cover_last_modified: 0 }
    }

    /// Entries 1 and 2 on the 8th, 3 on the 9th, 4 on the 15th.
    fn upcoming() -> Upcoming {
        model::build(vec![entry(1, at(8, 1)), entry(2, at(8, 20)), entry(3, at(9, 5)), entry(4, at(15, 12))], &Utc)
    }

    fn page(opened: &Rc<RefCell<Vec<i64>>>) -> Rc<UpcomingPage> {
        let log = Rc::clone(opened);
        let env = Env {
            first: Weekday::Mon,
            today: d(8),
            relative: true,
            date_pattern: String::new(),
            locale: "en",
            home_today: false,
            open: Rc::new(move |id| log.borrow_mut().push(id)),
        };
        let page = UpcomingPage::new(Kind::Anime, env);
        page.show(upcoming(), d(8));
        page
    }

    fn children(parent: &GBox) -> Vec<gtk::Widget> {
        let mut all = Vec::new();
        let mut child = parent.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            all.push(widget);
        }
        all
    }

    const SINGLE: Shape = Shape { two_panels: false, calendar_cap: None };
    const TWO: Shape = Shape { two_panels: true, calendar_cap: Some(360) };

    #[test]
    fn upcoming_page_single_column_starts_with_the_calendar_then_headings_and_rows() {
        crate::testing::with_display(|| {
            let page = page(&Rc::default());
            page.apply_shape(SINGLE, 500);
            let rows = children(&page.content);
            assert_eq!(rows.len(), 1 + 3 * 2, "the calendar row, then 3 headings each over its boxed list");
            assert_eq!(rows[0], page.calendar_slot.clone().upcast::<gtk::Widget>(), "the calendar is the first row");
            assert_eq!(page.calendar_slot.first_child().as_ref(), Some(page.calendar.widget().upcast_ref()), "no cap below 600 px");
            let heading = rows[1].first_child().and_downcast::<Label>().expect("the heading text");
            assert_eq!(heading.label(), "Today");
            let badge = rows[1].last_child().and_downcast::<Label>().expect("the badge");
            assert_eq!(badge.label(), "2 entries");
            assert!(badge.has_css_class("kb-badge"));
            let list = rows[2].clone().downcast::<gtk::ListBox>().expect("the day's entries are a list");
            assert!(list.has_css_class("boxed-list"));
            let row = list.row_at_index(0).expect("an entry row");
            let line = row.child().expect("the row line");
            let cover = line.first_child().expect("the cover leads the row");
            assert_eq!(cover.size_request(), (look::MEDIUM.width, look::MEDIUM.height), "the medium row cover");
            assert!(cover.has_css_class("kb-row-cover") && cover.has_css_class("kb-cover-initials"), "no stored cover: the initials");
        });
    }

    #[test]
    fn upcoming_page_two_panels_put_the_calendar_in_the_paned_and_cap_it() {
        crate::testing::with_display(|| {
            let page = page(&Rc::default());
            page.apply_shape(SINGLE, 500);
            page.apply_shape(TWO, 800);
            assert_eq!(page.paned.start_child().as_ref(), Some(page.calendar_slot.upcast_ref()), "the calendar is the start panel");
            assert_eq!(children(&page.content).len(), 3 * 2, "the list has no calendar row");
            let cap = page.calendar_slot.first_child().expect("the capped calendar");
            assert_eq!(cap.type_().name(), "KurobayomiClamp", "the width is capped in the medium band");
            assert_eq!(cap.first_child().as_ref(), Some(page.calendar.widget().upcast_ref()));
            page.apply_shape(Shape { two_panels: true, calendar_cap: None }, 1000);
            assert_eq!(page.calendar_slot.first_child().as_ref(), Some(page.calendar.widget().upcast_ref()), "uncapped above 840 px");
            page.apply_shape(SINGLE, 500);
            assert!(page.paned.start_child().is_none(), "back to one column empties the start panel");
            assert_eq!(children(&page.content).len(), 1 + 3 * 2);
        });
    }

    #[test]
    fn upcoming_page_a_row_click_or_key_opens_the_entry_by_id() {
        crate::testing::with_display(|| {
            use crate::widgets::row_activate::{click_row, row_titled};
            let opened: Rc<RefCell<Vec<i64>>> = Rc::default();
            let page = page(&opened);
            page.apply_shape(TWO, 800);
            click_row(&row_titled(&page.content, "Entry 2"));
            row_titled(&page.content, "Entry 4").emit_by_name::<()>("activate", &[]);
            assert_eq!(opened.borrow().as_slice(), [2, 4], "once per click and once per Enter or Space");
        });
    }

    #[test]
    fn upcoming_page_day_jump_scrolls_to_the_heading_and_ignores_empty_days() {
        crate::testing::with_display(|| {
            let page = page(&Rc::default());
            page.apply_shape(SINGLE, 500);
            page.root.size_allocate(&gtk::Allocation::new(0, 0, 500, 200), -1);
            let adjustment = page.scroll.vadjustment();
            assert_eq!(adjustment.value(), 0.0);
            page.jump(d(20));
            assert_eq!(adjustment.value(), 0.0, "a day with no heading does nothing");
            page.jump(d(15));
            let heading = children(&page.content)[1 + 2 * 2].clone();
            let top = heading.compute_bounds(&page.content).expect("bounds").y();
            assert!(adjustment.value() > 0.0, "the list scrolled");
            assert!((adjustment.value() - f64::from(top).min(adjustment.upper() - adjustment.page_size())).abs() < 1.0, "to the heading of the 15th, one row down for the calendar");
        });
    }

    #[test]
    fn upcoming_page_has_no_empty_state_text() {
        crate::testing::with_display(|| {
            let opened: Rc<RefCell<Vec<i64>>> = Rc::default();
            let page = page(&opened);
            page.show(model::build(Vec::new(), &Utc), d(8));
            page.apply_shape(SINGLE, 500);
            assert_eq!(children(&page.content).len(), 1, "only the calendar row; no message");
        });
    }

    #[test]
    fn upcoming_page_is_titled_upcoming_and_escape_pops_it() {
        crate::testing::with_display(|| {
            let (stack, _window) = crate::testing::nav();
            let base = GBox::new(O::Vertical, 0);
            stack.add_child(&base);
            stack.set_visible_child(&base);
            let page = assemble(page(&Rc::default()));
            crate::ui::push(&page);
            let title = page.first_child().and_downcast::<gtk::HeaderBar>().and_then(|bar| bar.title_widget()).and_downcast::<Label>().expect("the title");
            assert_eq!(title.label(), "Upcoming");
            assert_eq!(page.widget_name(), "screen:upcoming");
            assert_eq!(crate::ui::page_stack().1, 1, "the page is on show");
            let keys = page
                .observe_controllers()
                .into_iter()
                .filter_map(|c| c.ok())
                .find_map(|c| c.downcast::<gtk::EventControllerKey>().ok())
                .expect("the key controller");
            let _ = keys.emit_by_name::<bool>("key-pressed", &[&gdk::Key::Escape.into_glib(), &0u32, &gdk::ModifierType::empty()]);
            assert_eq!(crate::ui::page_stack().1, 0, "Escape went back to the page below");
        });
    }

    #[test]
    fn upcoming_page_help_entry_is_the_guide() {
        crate::testing::with_display(|| {
            let menu = crate::ui::menu_button(help_menu()).expect("the page has a menu");
            assert_eq!(crate::widgets::view_options::menu_labels(&menu.menu_model().expect("a menu")), ["Upcoming Guide"]);
            assert_eq!(layout::help_url(), "https://kurobayomi.github.io/docs/faq/updates#upcoming");
        });
    }
}
