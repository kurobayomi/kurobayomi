//! The chapter navigator (T-043a-041): previous and next buttons around a page pill with a slider, centred at the bottom
//! and at most [`MAX_WIDTH`] wide (libadwaita's clamp), so on a wide window the buttons sit beside the pill instead of
//! at the window's edges. The decisions (slider range, labels, RTL mirror, padding) are pure functions; [`Navigator`]
//! only applies them.

use kuro_i18n::{MsgKey, tr};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use super::nav::Step;

pub const ICON_PREVIOUS: &str = "media-skip-backward-symbolic";
pub const ICON_NEXT: &str = "media-skip-forward-symbolic";

/// The slider value for the current page. Before the first page (`current_page` below 1) it shows page 1 (PROPOSAL).
pub fn slider_value(current_page: i32, total: i32) -> f64 {
    f64::from(current_page.clamp(1, total.max(1)))
}

/// The widest the navigator gets, in px: a floating bar under the page, not a strip across a wide window.
pub const MAX_WIDTH: i32 = 600;

/// The current page number label: `3`, or `2-3` with page 2 shown `beside` it. A dash before the first page event
/// (`ReaderViewModel.kt:962`).
pub fn page_text(current_page: i32, beside: Option<i32>) -> String {
    crate::widgets::page_header::page_span(current_page, beside)
}

/// The pill is shown only with more than one page (`ChapterNavigator.kt:131-133`).
pub fn slider_visible(total: i32) -> bool {
    total > 1
}

/// The viewer index a slider value asks for. Equal values do nothing; otherwise the index is the value minus one.
pub fn slider_target(value: f64, current_page: i32) -> Option<usize> {
    let page = value.round() as i32;
    if page == current_page { None } else { Some((page - 1).max(0) as usize) }
}

/// Above this many pages the slider shows no page marks: in the capped bar they would sit too close to tell apart.
pub const MAX_PAGE_MARKS: i32 = 30;

/// The slider values that get a mark, one per page, so the slider reads as a row of page parts. None for a single
/// page or past [`MAX_PAGE_MARKS`].
pub fn page_marks(total: i32) -> Vec<f64> {
    if !slider_visible(total) || total > MAX_PAGE_MARKS {
        return Vec::new();
    }
    (1..=total).map(f64::from).collect()
}

/// The horizontal padding of the navigator for a window size: 24 px in a wide window, 8 px otherwise.
pub fn horizontal_padding(window_width: i32, window_height: i32) -> i32 {
    if crate::window_size::is_tablet_ui(window_width, window_height) { 24 } else { 8 }
}

/// One of the two chapter buttons: the action it runs, whether it is enabled, its label and its icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonPlan {
    pub step: Step,
    pub enabled: bool,
    pub description: String,
    pub icon: &'static str,
}

/// Left and right buttons. In LTR the left one is previous; in RTL the positions swap, and each icon follows its
/// action so the glyph matches what the button does (PROPOSAL, `p1-ui-parity.md` section 3.1.1).
pub fn button_plan(rtl: bool, previous_enabled: bool, next_enabled: bool) -> [ButtonPlan; 2] {
    let previous = ButtonPlan {
        step: Step::Previous,
        enabled: previous_enabled,
        description: tr(MsgKey::action_previous_chapter),
        icon: ICON_PREVIOUS,
    };
    let next = ButtonPlan { step: Step::Next, enabled: next_enabled, description: tr(MsgKey::action_next_chapter), icon: ICON_NEXT };
    if rtl { [next, previous] } else { [previous, next] }
}

/// What the navigator shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigatorState {
    pub total: i32,
    pub current_page: i32,
    /// The page shown beside the current one (two pages side by side).
    pub beside: Option<i32>,
    pub previous_enabled: bool,
    pub next_enabled: bool,
    pub rtl: bool,
    /// An adjacent load is running: the buttons are disabled and a spinner shows in the pill.
    pub loading: bool,
}

/// The navigator widgets. `on_step` runs a chapter button, `on_page` moves the viewer to a 0-based index.
pub struct Navigator {
    pub root: adw::Clamp,
    /// The left and right buttons by position; what each runs follows the RTL plan in `update`.
    left: gtk::Button,
    right: gtk::Button,
    left_step: Cell<Step>,
    right_step: Cell<Step>,
    pill: gtk::Box,
    slider: gtk::Scale,
    current_label: gtk::Label,
    total_label: gtk::Label,
    spinner: gtk::Spinner,
    current: Cell<i32>,
    /// The page total the slider's marks were drawn for, so they are redrawn only when it changes.
    marks_for: Cell<i32>,
    /// Set while the navigator writes its own value, so the slider does not call back.
    syncing: Cell<bool>,
    on_step: RefCell<Rc<dyn Fn(Step)>>,
    on_page: RefCell<Rc<dyn Fn(usize)>>,
}

impl Navigator {
    pub fn new() -> Rc<Self> {
        let left = gtk::Button::from_icon_name(ICON_PREVIOUS);
        let right = gtk::Button::from_icon_name(ICON_NEXT);
        for button in [&left, &right] {
            button.add_css_class("flat");
        }
        let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 2.0, 1.0);
        slider.set_draw_value(false);
        slider.set_digits(0);
        slider.set_hexpand(true);
        let current_label = gtk::Label::new(Some("-"));
        let total_label = gtk::Label::new(Some(""));
        current_label.set_width_chars(3);
        total_label.set_width_chars(3);
        // the numbers keep off the pill's rounded ends
        current_label.set_margin_start(8);
        total_label.set_margin_end(8);
        let spinner = gtk::Spinner::new();
        let pill = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        pill.add_css_class("osd");
        pill.set_margin_start(8);
        pill.set_margin_end(8);
        pill.set_hexpand(true);
        for child in [current_label.upcast_ref::<gtk::Widget>(), slider.upcast_ref(), total_label.upcast_ref(), spinner.upcast_ref()] {
            pill.append(child);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&left);
        row.append(&pill);
        row.append(&right);
        let root = adw::Clamp::builder().maximum_size(MAX_WIDTH).tightening_threshold(MAX_WIDTH).child(&row).build();
        root.set_widget_name("navigator");

        let navigator = Rc::new(Self {
            root,
            left: left.clone(),
            right: right.clone(),
            left_step: Cell::new(Step::Previous),
            right_step: Cell::new(Step::Next),
            pill,
            slider: slider.clone(),
            current_label,
            total_label,
            spinner,
            current: Cell::new(-1),
            marks_for: Cell::new(-1),
            syncing: Cell::new(false),
            on_step: RefCell::new(Rc::new(|_: Step| {})),
            on_page: RefCell::new(Rc::new(|_: usize| {})),
        });
        for (button, is_left) in [(left, true), (right, false)] {
            let weak = Rc::downgrade(&navigator);
            button.connect_clicked(move |_| {
                if let Some(nav) = weak.upgrade() {
                    let step = if is_left { nav.left_step.get() } else { nav.right_step.get() };
                    let run = nav.on_step.borrow().clone();
                    run(step);
                }
            });
        }
        let weak = Rc::downgrade(&navigator);
        slider.connect_value_changed(move |s| {
            let Some(nav) = weak.upgrade() else { return };
            if nav.syncing.get() {
                return;
            }
            if let Some(index) = slider_target(s.value(), nav.current.get()) {
                let run = nav.on_page.borrow().clone();
                run(index);
            }
        });
        navigator
    }

    pub fn connect(&self, on_step: impl Fn(Step) + 'static, on_page: impl Fn(usize) + 'static) {
        *self.on_step.borrow_mut() = Rc::new(on_step);
        *self.on_page.borrow_mut() = Rc::new(on_page);
    }

    /// Applies a state. Every value is computed from `state`; nothing here calls back into the reader.
    pub fn update(&self, state: &NavigatorState, window_size: (i32, i32)) {
        let padding = horizontal_padding(window_size.0, window_size.1);
        self.root.set_margin_start(padding);
        self.root.set_margin_end(padding);
        let plan = button_plan(state.rtl, state.previous_enabled, state.next_enabled);
        for (button, step, action) in [(&self.left, &self.left_step, &plan[0]), (&self.right, &self.right_step, &plan[1])] {
            step.set(action.step);
            button.set_icon_name(action.icon);
            button.set_tooltip_text(Some(&action.description));
            button.update_property(&[gtk::accessible::Property::Label(&action.description)]);
            button.set_sensitive(action.enabled && !state.loading);
        }
        self.pill.set_direction(if state.rtl { gtk::TextDirection::Rtl } else { gtk::TextDirection::Ltr });
        self.pill.set_visible(slider_visible(state.total) || state.loading);
        self.spinner.set_spinning(state.loading);
        self.current.set(state.current_page);
        self.current_label.set_label(&page_text(state.current_page, state.beside));
        self.total_label.set_label(&state.total.max(0).to_string());
        self.slider.set_visible(slider_visible(state.total));
        self.syncing.set(true);
        self.slider.set_range(1.0, f64::from(state.total.max(1)));
        self.slider.set_value(slider_value(state.current_page, state.total));
        self.syncing.set(false);
        if self.marks_for.replace(state.total) != state.total {
            self.slider.clear_marks();
            for value in page_marks(state.total) {
                self.slider.add_mark(value, gtk::PositionType::Bottom, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slider_is_one_based_and_equal_values_do_nothing() {
        assert_eq!(slider_target(3.0, 3), None);
        assert_eq!(slider_target(3.0, 1), Some(2));
        assert_eq!(slider_target(1.0, -1), Some(0));
    }

    #[test]
    fn before_the_first_page_the_slider_stays_in_range_and_the_label_is_a_dash() {
        assert_eq!(slider_value(-1, 20), 1.0);
        assert_eq!(slider_value(25, 20), 20.0);
        assert_eq!(page_text(-1, None), "-");
        assert_eq!(page_text(4, None), "4");
        assert_eq!(page_text(5, Some(4)), "4-5", "two pages side by side");
    }

    #[test]
    fn the_pill_needs_more_than_one_page() {
        assert!(!slider_visible(1));
        assert!(!slider_visible(0));
        assert!(slider_visible(2));
    }

    #[test]
    fn every_page_gets_a_mark_up_to_the_limit() {
        assert_eq!(page_marks(3), vec![1.0, 2.0, 3.0]);
        assert_eq!(page_marks(19).len(), 19);
        assert!(page_marks(86).is_empty(), "a long chapter gets no comb of marks");
        assert!(page_marks(1).is_empty(), "a single page has no slider");
        assert_eq!(page_marks(MAX_PAGE_MARKS).len(), MAX_PAGE_MARKS as usize);
        assert!(page_marks(MAX_PAGE_MARKS + 1).is_empty(), "too many to tell apart");
    }

    #[test]
    fn rtl_swaps_the_buttons_and_their_actions_and_icons() {
        let ltr = button_plan(false, true, false);
        assert_eq!(ltr[0].step, Step::Previous);
        assert!(ltr[0].enabled && !ltr[1].enabled);
        let rtl = button_plan(true, true, false);
        assert_eq!(rtl[0].step, Step::Next);
        assert!(!rtl[0].enabled, "the left button follows enabledNext in RTL");
        assert_eq!(rtl[0].icon, ICON_NEXT);
        assert_eq!(rtl[1].description, "Previous chapter");
    }

    #[test]
    fn wide_window_uses_24_px_padding_otherwise_8() {
        assert_eq!(horizontal_padding(1000, 800), 24);
        assert_eq!(horizontal_padding(719, 800), 8);
        assert_eq!(horizontal_padding(1000, 719), 8, "the smaller side decides");
    }
}
