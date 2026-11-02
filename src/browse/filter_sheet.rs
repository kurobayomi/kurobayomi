//! The filter sheet of a browse page: a native popover under the Filter button, with the rows of `SourceFilters` in a
//! boxed list and Reset and Apply under them (T-40b-050, T-40b-053; no separate window since 2026-10-12). Headers and
//! separators draw as a heading and a divider, a group expands recursively, and select, text, check box, tri-state and
//! sort rows write their changes through the sheet's edit methods. The rules are `kuro_source::filter_sheet` and
//! `sheet`; nothing here decides anything.

use std::rc::Rc;

use gtk::prelude::*;
use kuro_i18n::{MsgKey, tr};
use kuro_source::filter::TriStateState;
use kuro_source::filter_list::FilterList;
use kuro_source::filter_sheet::{ItemView, SheetItem, sort_descending};

use super::filters::SourceFilters;
use super::sheet::{SheetButton, on_button, select_label};
use crate::theme::tokens::{SPACE_M, SPACE_S};

const INDENT_PX: i32 = 16;
/// The popover's width, and the height past which its rows scroll.
const WIDTH_PX: i32 = 360;
const MAX_HEIGHT_PX: i32 = 480;

/// The sheet container. `on_filter` runs with the current filter values when Apply is pressed; `close` runs after it
/// (it closes the popover that holds the container).
pub struct FilterSheetView {
    pub root: gtk::Box,
    pub reset: gtk::Button,
    pub apply: gtk::Button,
    body: gtk::ListBox,
    filters: Rc<SourceFilters>,
}

impl FilterSheetView {
    pub fn new(filters: Rc<SourceFilters>, on_filter: impl Fn(FilterList) + 'static, close: impl Fn() + 'static) -> Rc<Self> {
        let body = crate::ui::boxed_list();
        crate::ui::margins(&body, 12);
        let reset = gtk::Button::with_label(&tr(MsgKey::action_reset));
        let apply = gtk::Button::with_label(&tr(MsgKey::action_apply));
        apply.add_css_class("suggested-action");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
        buttons.set_margin_start(SPACE_M);
        buttons.set_margin_end(SPACE_M);
        buttons.set_margin_bottom(SPACE_M);
        reset.set_hexpand(true);
        reset.set_halign(gtk::Align::Start);
        buttons.append(&reset);
        buttons.append(&apply);
        // a long filter list scrolls inside the popover; a short one keeps its natural height
        let scroller = gtk::ScrolledWindow::builder()
            .child(&body)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(MAX_HEIGHT_PX)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_width_request(WIDTH_PX);
        root.append(&scroller);
        root.append(&buttons);

        let view = Rc::new(Self { root, reset, apply, body, filters });
        view.render();
        let weak = Rc::downgrade(&view);
        view.reset.connect_clicked(move |_| {
            let Some(view) = weak.upgrade() else { return };
            let effect = on_button(SheetButton::Reset);
            if effect.reset_state {
                view.filters.reset();
                view.render();
            }
        });
        let weak = Rc::downgrade(&view);
        let (on_filter, close) = (Rc::new(on_filter), Rc::new(close));
        view.apply.connect_clicked(move |_| {
            let Some(view) = weak.upgrade() else { return };
            let effect = on_button(SheetButton::Filter);
            if effect.search {
                on_filter(view.filters.current());
            }
            if effect.close {
                close();
            }
        });
        view
    }

    /// Draws the rows again from the sheet's state.
    pub fn render(self: &Rc<Self>) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        for item in self.filters.items() {
            self.body.append(&self.item(&item, 0));
        }
    }

    /// Runs an edit and redraws, for the rows whose look depends on their state.
    fn edit_and_render(self: &Rc<Self>, edit: impl FnOnce(&mut kuro_source::filter_sheet::FilterSheet) -> Result<(), kuro_source::filter_sheet::SheetError>) {
        if let Err(error) = self.filters.edit(edit) {
            tracing::warn!(%error, "a filter edit did not apply");
        }
        self.render();
    }

    fn item(self: &Rc<Self>, item: &SheetItem, depth: i32) -> gtk::Widget {
        let path = item.path.clone();
        let row = match &item.view {
            ItemView::Heading { name } => {
                let label = gtk::Label::builder().label(name).xalign(0.0).css_classes(["heading"]).build();
                label.set_margin_top(SPACE_S);
                label.upcast::<gtk::Widget>()
            }
            ItemView::Divider => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
            ItemView::CheckBox { label, checked } => {
                let check = gtk::CheckButton::with_label(label);
                check.set_active(*checked);
                let weak = Rc::downgrade(self);
                // the box already shows its state, so there is nothing to redraw
                check.connect_toggled(move |_| {
                    if let Some(view) = weak.upgrade() {
                        if let Err(error) = view.filters.edit(|sheet| sheet.toggle_check(&path)) {
                            tracing::warn!(%error, "a check box edit did not apply");
                        }
                    }
                });
                check.upcast()
            }
            ItemView::TriState { label, state } => {
                let (icon, class) = match state {
                    TriStateState::Ignore => ("checkbox-symbolic", None),
                    TriStateState::Include => ("checkbox-checked-symbolic", None),
                    TriStateState::Exclude => ("checkbox-mixed-symbolic", Some("error")),
                };
                let content = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
                let image = gtk::Image::from_icon_name(icon);
                if let Some(class) = class {
                    image.add_css_class(class);
                }
                content.append(&image);
                content.append(&gtk::Label::new(Some(label)));
                let button = gtk::Button::builder().child(&content).css_classes(["flat"]).halign(gtk::Align::Start).build();
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(view) = weak.upgrade() {
                        view.edit_and_render(|sheet| sheet.cycle_tri(&path));
                    }
                });
                button.upcast()
            }
            ItemView::Text { label, value } => {
                let entry = gtk::Entry::builder().text(value).placeholder_text(label).hexpand(true).build();
                let weak = Rc::downgrade(self);
                // every keystroke is written, and nothing is searched until Filter (EC-FI-06); no redraw, so focus stays
                entry.connect_changed(move |entry| {
                    if let Some(view) = weak.upgrade() {
                        let text = entry.text().to_string();
                        if let Err(error) = view.filters.edit(|sheet| sheet.set_text(&path, text)) {
                            tracing::warn!(%error, "a text filter edit did not apply");
                        }
                    }
                });
                let line = gtk::Box::new(gtk::Orientation::Vertical, 2);
                line.append(&gtk::Label::builder().label(label).xalign(0.0).css_classes(["dim-label"]).build());
                line.append(&entry);
                line.upcast()
            }
            ItemView::Select { label, options, selected } => {
                // the shown value is values[state], and the first one when the stored index is out of range (B-EC-21)
                let shown = select_label(options, selected.map_or(-1, |i| i as i32)).and_then(|v| options.iter().position(|o| o == v));
                let names: Vec<&str> = options.iter().map(String::as_str).collect();
                let drop = gtk::DropDown::from_strings(&names);
                drop.set_selected(shown.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
                let weak = Rc::downgrade(self);
                let current = shown;
                drop.connect_selected_notify(move |drop| {
                    let index = drop.selected() as usize;
                    if let (Some(view), true) = (weak.upgrade(), Some(index) != current) {
                        if let Err(error) = view.filters.edit(|sheet| sheet.set_select(&path, index)) {
                            tracing::warn!(%error, "a select filter edit did not apply");
                        }
                    }
                });
                let line = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
                line.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
                line.append(&drop);
                line.upcast()
            }
            ItemView::Sort { label, options, selection, expanded } => {
                let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
                column.append(&self.expander(label, *expanded, &path));
                if *expanded {
                    for (index, option) in options.iter().enumerate() {
                        let icon = match sort_descending(*selection, index) {
                            Some(true) => "view-sort-descending-symbolic",
                            Some(false) => "view-sort-ascending-symbolic",
                            None => "content-loading-symbolic",
                        };
                        let content = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
                        content.append(&gtk::Image::from_icon_name(icon));
                        content.append(&gtk::Label::new(Some(option)));
                        let button = gtk::Button::builder().child(&content).css_classes(["flat"]).halign(gtk::Align::Start).build();
                        button.set_margin_start(INDENT_PX);
                        let (weak, path) = (Rc::downgrade(self), path.clone());
                        button.connect_clicked(move |_| {
                            if let Some(view) = weak.upgrade() {
                                view.edit_and_render(|sheet| sheet.tap_sort(&path, index));
                            }
                        });
                        column.append(&button);
                    }
                }
                column.upcast()
            }
            ItemView::Group { label, expanded, children } => {
                let column = gtk::Box::new(gtk::Orientation::Vertical, SPACE_S);
                column.append(&self.expander(label, *expanded, &path));
                if *expanded {
                    for child in children {
                        column.append(&self.item(child, depth + 1));
                    }
                }
                column.upcast()
            }
        };
        row.set_margin_start(depth * INDENT_PX);
        if depth == 0 {
            // a top-level item is one row of the boxed list, with the list's inner padding
            row.set_margin_start(SPACE_M);
            row.set_margin_end(SPACE_M);
            row.set_margin_top(SPACE_S);
            row.set_margin_bottom(SPACE_S);
        }
        row
    }

    /// The header of a group or sort box: its name and an arrow that opens and closes it.
    fn expander(self: &Rc<Self>, label: &str, expanded: bool, path: &[usize]) -> gtk::Button {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
        content.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
        content.append(&gtk::Image::from_icon_name(if expanded { "pan-up-symbolic" } else { "pan-down-symbolic" }));
        let button = gtk::Button::builder().child(&content).css_classes(["flat"]).build();
        let (weak, path) = (Rc::downgrade(self), path.to_vec());
        button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.edit_and_render(|sheet| sheet.toggle_expanded(&path));
            }
        });
        button
    }

    /// The sheet in a native popover, for the Filter menu button. The rows are drawn again each time it opens (a search
    /// may have reset the values meanwhile); Escape or a click outside closes it and keeps the edits.
    pub fn popover(self: &Rc<Self>) -> gtk::Popover {
        let popover = gtk::Popover::builder().child(&self.root).build();
        let weak = Rc::downgrade(self);
        popover.connect_show(move |_| {
            if let Some(view) = weak.upgrade() {
                view.render();
            }
        });
        // the rows reach the sheet through weak links: the popover owns it until it is destroyed
        super::glue::keep_alive(&popover, self);
        popover
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::{Cell, RefCell};

    use kuro_source::filter::Filter;

    use super::*;

    fn defaults() -> FilterList {
        FilterList(vec![
            Filter::header("Genres"),
            Filter::separator(""),
            Filter::check_box("Completed"),
            Filter::tri_state("Action"),
            Filter::text("Author"),
            Filter::select("Status", vec!["Any".into(), "Ongoing".into(), "Done".into()]),
            Filter::group("More", vec![Filter::check_box("Inner"), Filter::group("Deep", vec![Filter::check_box("Deepest")])]),
            Filter::sort("Order", vec!["Title".into(), "Date".into()]),
        ])
    }

    struct Rig {
        view: Rc<FilterSheetView>,
        filters: Rc<SourceFilters>,
        searched: Rc<RefCell<Vec<FilterList>>>,
        closed: Rc<Cell<u32>>,
    }

    fn rig() -> Rig {
        let filters = Rc::new(SourceFilters::new(defaults()));
        let searched: Rc<RefCell<Vec<FilterList>>> = Rc::default();
        let closed = Rc::new(Cell::new(0));
        let (log, count) = (Rc::clone(&searched), Rc::clone(&closed));
        let view = FilterSheetView::new(Rc::clone(&filters), move |list| log.borrow_mut().push(list), move || count.set(count.get() + 1));
        Rig { view, filters, searched, closed }
    }

    fn walk<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>) -> Vec<T> {
        fn go<T: IsA<gtk::Widget>>(w: &gtk::Widget, out: &mut Vec<T>) {
            if let Some(t) = w.downcast_ref::<T>() {
                out.push(t.clone());
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

    fn labels(rig: &Rig) -> Vec<String> {
        walk::<gtk::Label>(&rig.view.body).iter().map(|l| l.label().to_string()).collect()
    }

    fn headers_and_separators_render_as_a_heading_and_a_divider_and_groups_expand_recursively() {
        let rig = rig();
        let body = &rig.view.body;
        let headings: Vec<_> = walk::<gtk::Label>(body).into_iter().filter(|l| l.has_css_class("heading")).collect();
        assert_eq!(headings.len(), 1, "the header row is a heading");
        assert_eq!(headings[0].label().as_str(), "Genres");
        assert_eq!(walk::<gtk::Separator>(body).len(), 1, "the separator row is a divider");
        assert!(!labels(&rig).contains(&"Inner".to_owned()), "groups start closed");
        let more = walk::<gtk::Button>(body).into_iter().find(|b| walk::<gtk::Label>(b).iter().any(|l| l.label() == "More")).expect("More");
        more.emit_clicked();
        assert!(labels(&rig).contains(&"Inner".to_owned()) && labels(&rig).contains(&"Deep".to_owned()), "children draw once open");
        assert!(!labels(&rig).contains(&"Deepest".to_owned()));
        let deep = walk::<gtk::Button>(&rig.view.body).into_iter().find(|b| walk::<gtk::Label>(b).iter().any(|l| l.label() == "Deep")).expect("Deep");
        deep.emit_clicked();
        assert!(labels(&rig).contains(&"Deepest".to_owned()), "a group inside a group expands too");
    }

    fn a_check_box_a_tri_state_and_a_text_write_into_the_sheet() {
        let rig = rig();
        let body = &rig.view.body;
        let check = walk::<gtk::CheckButton>(body).into_iter().find(|c| c.label().as_deref() == Some("Completed")).expect("check");
        check.set_active(true);
        assert!(matches!(&rig.filters.current().0[2], Filter::CheckBox { state: true, .. }), "toggling reports the change");

        let tri = walk::<gtk::Button>(&rig.view.body).into_iter().find(|b| walk::<gtk::Label>(b).iter().any(|l| l.label() == "Action")).expect("tri");
        tri.emit_clicked();
        assert!(matches!(&rig.filters.current().0[3], Filter::TriState { state: TriStateState::Include, .. }));
        let tri = walk::<gtk::Button>(&rig.view.body).into_iter().find(|b| walk::<gtk::Label>(b).iter().any(|l| l.label() == "Action")).expect("tri");
        tri.emit_clicked();
        assert!(matches!(&rig.filters.current().0[3], Filter::TriState { state: TriStateState::Exclude, .. }));

        let entry = walk::<gtk::Entry>(&rig.view.body).into_iter().next().expect("text");
        entry.set_text("Oda");
        assert!(matches!(&rig.filters.current().0[4], Filter::Text { state, .. } if state == "Oda"), "every keystroke is written");
        assert!(rig.searched.borrow().is_empty(), "and nothing is searched until Filter");
    }

    fn a_select_shows_its_indexed_value_and_the_first_one_when_the_index_is_out_of_range() {
        let rig = rig();
        let drop = walk::<gtk::DropDown>(&rig.view.body).into_iter().next().expect("select");
        assert_eq!(drop.selected(), 0);
        drop.set_selected(2);
        assert!(matches!(&rig.filters.current().0[5], Filter::Select { state: 2, .. }), "choosing writes values[state]");

        let broken = Rc::new(SourceFilters::new(FilterList(vec![Filter::select_at("S", vec!["A".into(), "B".into()], 9)])));
        let view = FilterSheetView::new(broken, |_| {}, || {});
        assert_eq!(walk::<gtk::DropDown>(&view.body).into_iter().next().expect("select").selected(), 0, "index 0 stands in");
    }

    fn reset_keeps_the_sheet_open_and_filter_searches_with_the_values_then_closes() {
        let rig = rig();
        rig.filters.edit(|sheet| sheet.toggle_check(&[2]).expect("check"));
        rig.view.render();
        rig.view.reset.emit_clicked();
        assert_eq!(rig.filters.current(), defaults(), "Reset restores the defaults");
        assert!(rig.searched.borrow().is_empty() && rig.closed.get() == 0, "and neither searches nor closes");
        rig.filters.edit(|sheet| sheet.toggle_check(&[2]).expect("check"));
        rig.view.apply.emit_clicked();
        assert_eq!(rig.searched.borrow().len(), 1);
        assert!(matches!(&rig.searched.borrow()[0].0[2], Filter::CheckBox { state: true, .. }), "Filter searches with the current values");
        assert_eq!(rig.closed.get(), 1);
    }

    fn a_sort_row_taps_through_the_sheet_and_shows_its_arrow() {
        let rig = rig();
        let open = walk::<gtk::Button>(&rig.view.body).into_iter().find(|b| walk::<gtk::Label>(b).iter().any(|l| l.label() == "Order")).expect("Order");
        open.emit_clicked();
        let date = walk::<gtk::Button>(&rig.view.body).into_iter().find(|b| walk::<gtk::Label>(b).iter().any(|l| l.label() == "Date")).expect("Date");
        date.emit_clicked();
        assert!(matches!(&rig.filters.current().0[7], Filter::Sort { state: Some(s), .. } if s.index == 1 && s.ascending));
        let icons: Vec<String> = walk::<gtk::Image>(&rig.view.body).iter().filter_map(|i| i.icon_name()).map(|n| n.to_string()).collect();
        assert!(icons.contains(&"view-sort-ascending-symbolic".to_owned()), "{icons:?}");
    }

    fn the_sheet_is_a_popover_with_a_boxed_list_and_apply() {
        let rig = rig();
        rig.filters.edit(|sheet| sheet.toggle_check(&[2]).expect("check"));
        let popover = rig.view.popover();
        assert_eq!(popover.child(), Some(rig.view.root.clone().upcast::<gtk::Widget>()), "no separate window");
        assert!(rig.view.body.has_css_class("boxed-list"));
        assert_eq!(rig.view.apply.label().as_deref(), Some(tr(MsgKey::action_apply).as_str()));
        popover.popdown();
        assert!(matches!(&rig.filters.current().0[2], Filter::CheckBox { state: true, .. }), "closing reverts nothing");
        assert!(rig.searched.borrow().is_empty(), "and searches nothing");
    }

    pub(crate) fn smoke() {
        the_sheet_is_a_popover_with_a_boxed_list_and_apply();
        headers_and_separators_render_as_a_heading_and_a_divider_and_groups_expand_recursively();
        a_check_box_a_tri_state_and_a_text_write_into_the_sheet();
        a_select_shows_its_indexed_value_and_the_first_one_when_the_index_is_out_of_range();
        reset_keeps_the_sheet_open_and_filter_searches_with_the_values_then_closes();
        a_sort_row_taps_through_the_sheet_and_shows_its_arrow();
    }
}
