//! The calendar widget of the Upcoming page (T-42b-027, T-42b-028, T-42b-029, T-42b-038): the month header with its
//! Previous and Next buttons and the 200 ms title slide, the week-day row, and the grid of day cells with the today ring,
//! the past dimming, the event dots and the circular click area. The numbers come from [`calendar`]; this file only
//! draws them. A month change moves the title and the grid, never the list (E-C10).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, Weekday};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, GestureClick, Label, Orientation as O};
use kuro_i18n::{MsgKey, tr};

use crate::updates::upcoming::calendar::{self, CalKey};

/// The title slide, forward from below and backward from above (B§8.2).
const TITLE_SLIDE_MS: u32 = 200;
/// The smallest side of a day cell, in pixels.
const CELL_SIZE: i32 = 40;

/// The calendar key a GDK key stands for.
pub fn cal_key(key: gdk::Key) -> Option<CalKey> {
    match key {
        gdk::Key::Left | gdk::Key::KP_Left => Some(CalKey::Left),
        gdk::Key::Right | gdk::Key::KP_Right => Some(CalKey::Right),
        gdk::Key::Up | gdk::Key::KP_Up => Some(CalKey::Up),
        gdk::Key::Down | gdk::Key::KP_Down => Some(CalKey::Down),
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => Some(CalKey::PageUp),
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => Some(CalKey::PageDown),
        _ => None,
    }
}

pub struct CalendarView {
    root: GBox,
    title: gtk::Stack,
    /// Which of the stack's two labels is on show.
    title_on_a: Cell<bool>,
    grid_slot: GBox,
    month: Cell<(i32, u32)>,
    first: Weekday,
    today: Cell<NaiveDate>,
    events: RefCell<BTreeMap<NaiveDate, u32>>,
    focus: Cell<NaiveDate>,
    cells: RefCell<HashMap<NaiveDate, gtk::Widget>>,
    on_day: Rc<dyn Fn(NaiveDate)>,
    /// The opt-in Home key (T-42b-046): off unless the setting turns it on.
    home_today: Cell<bool>,
}

impl CalendarView {
    /// A calendar on the month of `today`, with weeks that start on `first`. `on_day` runs for a click or Enter on a day.
    pub fn new(first: Weekday, today: NaiveDate, on_day: impl Fn(NaiveDate) + 'static) -> Rc<Self> {
        let month = (today.year(), today.month());
        let title = gtk::Stack::builder().transition_duration(TITLE_SLIDE_MS).halign(Align::Start).hexpand(true).build();
        for name in ["a", "b"] {
            title.add_named(&Label::builder().xalign(0.0).css_classes(["title-3"]).build(), Some(name));
        }
        let view = Rc::new(Self {
            root: GBox::new(O::Vertical, 4),
            title,
            title_on_a: Cell::new(true),
            grid_slot: GBox::new(O::Vertical, 0),
            month: Cell::new(month),
            first,
            today: Cell::new(today),
            events: RefCell::default(),
            focus: Cell::new(today),
            cells: RefCell::default(),
            on_day: Rc::new(on_day),
            home_today: Cell::new(false),
        });
        view.set_title_text("a", month);
        view.title.set_visible_child_name("a");
        view.build_header();
        view.root.append(&view.grid_slot);
        view.rebuild_grid();
        view.attach_keys();
        view
    }

    pub fn widget(&self) -> &GBox {
        &self.root
    }

    #[cfg(test)]
    /// The month on show.
    pub fn month(&self) -> (i32, u32) {
        self.month.get()
    }

    /// Replaces the per-day counts that draw the dots. The month on show stays.
    pub fn set_events(self: &Rc<Self>, events: BTreeMap<NaiveDate, u32>) {
        *self.events.borrow_mut() = events;
        self.rebuild_grid();
    }

    /// Turns the opt-in Home key on or off (T-42b-046). Off, Home is not taken by the calendar.
    pub fn set_home_returns_today(&self, on: bool) {
        self.home_today.set(on);
    }

    /// The day changed under a page that stayed open: the ring and the dimming move to the new today.
    pub fn set_today(self: &Rc<Self>, today: NaiveDate) {
        if self.today.replace(today) != today {
            self.rebuild_grid();
        }
    }

    /// The previous (`-1`) or next (`1`) month.
    pub fn step_month(self: &Rc<Self>, delta: i32) {
        let (year, month) = self.month.get();
        self.show_month(calendar::shift_month(year, month, delta));
    }

    fn show_month(self: &Rc<Self>, to: (i32, u32)) {
        let from = self.month.get();
        if to == from {
            return;
        }
        self.month.set(to);
        self.slide_title(from, to);
        self.rebuild_grid();
    }

    fn set_title_text(&self, name: &str, (year, month): (i32, u32)) {
        if let Some(label) = self.title.child_by_name(name).and_downcast::<Label>() {
            label.set_label(&calendar::month_title(year, month));
        }
    }

    /// Slides the new title in from below when moving forward and from above when moving back, in 200 ms.
    fn slide_title(&self, from: (i32, u32), to: (i32, u32)) {
        let next = if self.title_on_a.get() { "b" } else { "a" };
        self.set_title_text(next, to);
        let kind = if calendar::moving_forward(from, to) { gtk::StackTransitionType::SlideUp } else { gtk::StackTransitionType::SlideDown };
        self.title.set_transition_type(kind);
        self.title.set_visible_child_name(next);
        self.title_on_a.set(!self.title_on_a.get());
    }

    fn build_header(self: &Rc<Self>) {
        let header = GBox::new(O::Horizontal, 4);
        header.set_margin_start(12);
        header.append(&self.title);
        for (icon, tip, delta) in [("go-previous-symbolic", MsgKey::upcoming_calendar_prev, -1), ("go-next-symbolic", MsgKey::upcoming_calendar_next, 1)] {
            let button = crate::ui::icon_button(icon, &tr(tip));
            button.add_css_class("flat");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.step_month(delta);
                }
            });
            header.append(&button);
        }
        self.root.append(&header);
    }

    /// The week-day row and the weeks of the month on show.
    fn rebuild_grid(self: &Rc<Self>) {
        while let Some(child) = self.grid_slot.first_child() {
            self.grid_slot.remove(&child);
        }
        let grid = gtk::Grid::builder().column_homogeneous(true).row_spacing(2).column_spacing(2).build();
        for (column, day) in calendar::week_days(self.first).iter().enumerate() {
            grid.attach(&Label::builder().label(calendar::weekday_narrow(*day)).css_classes(["heading"]).build(), column as i32, 0, 1, 1);
        }
        let (year, month) = self.month.get();
        let (today, events) = (self.today.get(), self.events.borrow().clone());
        let mut cells = HashMap::new();
        for (i, cell) in calendar::month_grid(year, month, self.first).into_iter().enumerate() {
            let Some(date) = cell else { continue };
            let widget = self.day_cell(date, events.get(&date).copied().unwrap_or(0), today);
            grid.attach(&widget, (i % 7) as i32, (i / 7) as i32 + 1, 1, 1);
            cells.insert(date, widget);
        }
        *self.cells.borrow_mut() = cells;
        self.grid_slot.append(&grid);
    }

    /// One day: the number (dimmed when past), the ring when it is today, up to three dots, and a circular click area.
    fn day_cell(self: &Rc<Self>, date: NaiveDate, count: u32, today: NaiveDate) -> gtk::Widget {
        let cell = gtk::Overlay::builder().halign(Align::Center).valign(Align::Center).focusable(true).build();
        cell.set_size_request(CELL_SIZE, CELL_SIZE);
        cell.add_css_class("kuro-cal-day");
        if date == today {
            cell.add_css_class("kuro-cal-today");
        }
        let number = Label::builder().label(date.day().to_string()).css_classes(["heading"]).halign(Align::Center).valign(Align::Center).build();
        number.set_opacity(calendar::day_alpha(date, today));
        cell.set_child(Some(&number));
        let dots = GBox::new(O::Horizontal, 2);
        dots.set_halign(Align::Center);
        dots.set_valign(Align::End);
        dots.set_margin_bottom(4);
        dots.set_can_target(false);
        let diameter = calendar::DOT_DIAMETER.round() as i32;
        for alpha in calendar::dots(count) {
            let dot = GBox::new(O::Horizontal, 0);
            dot.set_size_request(diameter, diameter);
            dot.add_css_class("kuro-cal-dot");
            dot.set_opacity(alpha);
            dots.append(&dot);
        }
        cell.add_overlay(&dots);

        let click = GestureClick::new();
        let (weak, cell_weak) = (Rc::downgrade(self), cell.downgrade());
        click.connect_released(move |_, _, x, y| {
            let (Some(view), Some(cell)) = (weak.upgrade(), cell_weak.upgrade()) else { return };
            if calendar::in_circle(x, y, f64::from(cell.width()), f64::from(cell.height())) {
                view.focus.set(date);
                cell.grab_focus();
                (view.on_day)(date);
            }
        });
        cell.add_controller(click);
        cell.upcast()
    }

    /// Arrow keys move the focus by day and week, Page Up and Page Down by month; Enter or Space picks the focused day.
    fn attach_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(view) = weak.upgrade() else { return glib::Propagation::Proceed };
            if let Some(step) = cal_key(key) {
                view.move_focus(calendar::key_move(view.focus_date(), step));
                glib::Propagation::Stop
            } else if key == gdk::Key::Home && view.home() {
                glib::Propagation::Stop
            } else if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) && view.focused_cell().is_some() {
                (view.on_day)(view.focus_date());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.root.add_controller(keys);
    }

    /// The day the keys move from: the last one focused, else today when it is on show, else the first of the month.
    fn focus_date(&self) -> NaiveDate {
        let (year, month) = self.month.get();
        let focus = self.focus.get();
        if (focus.year(), focus.month()) == (year, month) {
            return focus;
        }
        let today = self.today.get();
        if (today.year(), today.month()) == (year, month) { today } else { NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(today) }
    }

    fn focused_cell(&self) -> Option<NaiveDate> {
        let cells = self.cells.borrow();
        cells.iter().find(|(_, widget)| widget.has_focus()).map(|(date, _)| *date)
    }

    /// Focuses `to`, changing the month first when it is in another one.
    fn move_focus(self: &Rc<Self>, to: NaiveDate) {
        self.focus.set(to);
        self.show_month((to.year(), to.month()));
        if let Some(widget) = self.cells.borrow().get(&to) {
            widget.grab_focus();
        }
    }

    /// Home, when the setting is on: shows the current month with today focused. Returns `false` (key not taken) when off.
    fn home(self: &Rc<Self>) -> bool {
        if !self.home_today.get() {
            return false;
        }
        self.move_focus(self.today.get());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("a date")
    }

    /// The dots drawn on a day cell: the overlay's `Box` of dot boxes.
    fn dots_of(cell: &gtk::Widget) -> Vec<f64> {
        let mut child = cell.first_child();
        while let Some(widget) = child {
            if widget.is::<GBox>() {
                let mut alphas = Vec::new();
                let mut dot = widget.first_child();
                while let Some(d) = dot {
                    alphas.push(d.opacity());
                    dot = d.next_sibling();
                }
                return alphas;
            }
            child = widget.next_sibling();
        }
        Vec::new()
    }

    fn number_of(cell: &gtk::Widget) -> Label {
        cell.first_child().and_downcast::<Label>().expect("the day number is the first child")
    }

    #[test]
    fn calendar_view_draws_the_ring_the_dimming_and_at_most_three_dots() {
        crate::testing::with_display(|| {
            let view = CalendarView::new(Weekday::Mon, d(2026, 10, 8), |_| {});
            view.set_events(BTreeMap::from([(d(2026, 10, 8), 5), (d(2026, 10, 9), 1)]));
            let cells = view.cells.borrow();
            assert_eq!(cells.len(), 31, "one cell per day of October");
            assert!(cells[&d(2026, 10, 8)].has_css_class("kuro-cal-today"), "today has the ring");
            assert!(!cells[&d(2026, 10, 9)].has_css_class("kuro-cal-today"));
            assert_eq!(dots_of(&cells[&d(2026, 10, 8)]).len(), 3, "five entries draw three dots");
            let alphas = dots_of(&cells[&d(2026, 10, 8)]);
            // GTK keeps an opacity in 8 bits, so 0.3 reads back as 77/255
            assert!((alphas[0] - 0.3).abs() < 0.005 && (alphas[1] - 0.6).abs() < 0.005 && (alphas[2] - 0.9).abs() < 0.005, "{alphas:?}");
            assert_eq!(dots_of(&cells[&d(2026, 10, 9)]).len(), 1);
            assert!(dots_of(&cells[&d(2026, 10, 10)]).is_empty());
            assert!((number_of(&cells[&d(2026, 10, 7)]).opacity() - 0.38).abs() < 0.005, "a past day is dimmed");
            assert!((number_of(&cells[&d(2026, 10, 8)]).opacity() - 1.0).abs() < 1e-6);
        });
    }

    #[test]
    fn calendar_view_months_and_keys_move_the_grid_and_keep_the_events() {
        crate::testing::with_display(|| {
            let view = CalendarView::new(Weekday::Mon, d(2026, 10, 8), |_| {});
            view.set_events(BTreeMap::from([(d(2026, 10, 8), 2), (d(2026, 11, 3), 1)]));
            view.step_month(1);
            assert_eq!(view.month(), (2026, 11));
            assert_eq!(view.cells.borrow().len(), 30, "November");
            assert_eq!(dots_of(&view.cells.borrow()[&d(2026, 11, 3)]).len(), 1, "the events belong to the page, not to a month");
            view.step_month(-2);
            assert_eq!(view.month(), (2026, 9));
            view.step_month(3);
            assert_eq!(view.month(), (2026, 12));
            view.step_month(1);
            assert_eq!(view.month(), (2027, 1), "the year rolls over");
            // Page Down from the focus moves a month; an arrow moves a day
            let focus = view.focus_date();
            view.move_focus(calendar::key_move(focus, CalKey::PageUp));
            assert_eq!(view.month(), (2026, 12));
            view.move_focus(calendar::key_move(view.focus_date(), CalKey::Right));
            assert_eq!(view.focus.get().month(), 12);
        });
    }

    #[test]
    fn calendar_view_title_shows_the_month_after_the_slide() {
        crate::testing::with_display(|| {
            let view = CalendarView::new(Weekday::Mon, d(2026, 10, 8), |_| {});
            let title = |view: &CalendarView| view.title.visible_child().and_downcast::<Label>().expect("a label").label().to_string();
            assert_eq!(title(&view), "October 2026");
            view.step_month(1);
            assert_eq!(title(&view), "November 2026", "the visible child switches at once; only its entry is animated");
            view.step_month(-1);
            assert_eq!(title(&view), "October 2026");
        });
    }

    #[test]
    fn calendar_view_maps_the_arrow_and_page_keys() {
        assert_eq!(cal_key(gdk::Key::Left), Some(CalKey::Left));
        assert_eq!(cal_key(gdk::Key::Page_Down), Some(CalKey::PageDown));
        assert_eq!(cal_key(gdk::Key::a), None);
    }
}
