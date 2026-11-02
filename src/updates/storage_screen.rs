//! The Storage page of one kind (T-42a-048, T-42a-051, T-42a-052, T-42a-054): one column with the category dropdown,
//! the half donut with the total, and the entry rows. The rows and the categories come from the caller, which owns the
//! download cache and the library; the page builds its snapshot with [`storage::build_snapshot`] once per push. While
//! the cache initializes the page shows "Scanning downloads…" (never a stuck spinner).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, DrawingArea, DropDown, Label, ListBox, Orientation as O, Overlay, StringList};
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;

use crate::updates::copy;
use crate::updates::history_dialog::{ConfirmCopy, show as show_confirm};
use crate::updates::history::NOT_FOUND_TOAST_MS;
use crate::updates::storage::{self, CategoryOption, DeleteOutcome, PageState, Selection, StorageRow, StorageSnapshot};
use crate::theme::tokens::{SPACE_M, SPACE_S, SPACE_XL};

/// Stroke of the donut arcs, in pixels.
const DONUT_STROKE: f64 = 15.0;

/// What the caller hands the page on each push: the rows of the cache and the library, the categories in dropdown
/// order (id and name), and the ids of the categories that are visible (after the hidden-categories setting).
pub struct StorageInput {
    pub rows: Vec<StorageRow>,
    pub categories: Vec<(i64, String)>,
    pub visible: Vec<i64>,
    pub initializing: bool,
}

pub struct StorageScreen {
    kind: Kind,
    root: GBox,
    /// The two columns (dropdown and donut on the left, rows on the right); one column when narrow (T-42a-053).
    split: GBox,
    left: GBox,
    right: GBox,
    chart: Overlay,
    width: Cell<i32>,
    overlay_total: Label,
    donut: DrawingArea,
    dropdown: DropDown,
    options: RefCell<Vec<CategoryOption>>,
    selection: Cell<Selection>,
    input: RefCell<StorageInput>,
    snapshot: RefCell<StorageSnapshot>,
    list: ListBox,
    /// The title filter of the rows, for the page header (`mount::storage`).
    search: gtk::SearchEntry,
    scanning: GBox,
    helper: Label,
    /// Answers whether the entry's source is installed and deletes its downloads when it is (the download manager is
    /// the caller's). Set by the caller; without it a confirmed delete reports the missing source.
    on_delete: RefCell<Option<Box<dyn Fn(&StorageRow) -> DeleteOutcome>>>,
}

impl StorageScreen {
    /// Builds the page with nothing in it. The caller pushes its data with [`StorageScreen::set_input`].
    pub fn new(kind: Kind) -> Rc<Self> {
        Rc::new_cyclic(|weak: &Weak<Self>| {
            // the column spacing of the other pages (`ui::column`), unclamped: the page goes two columns from 720 px
            let root = GBox::new(O::Vertical, SPACE_M);
            crate::ui::margins(&root, 16);
            root.set_margin_bottom(SPACE_XL);

            let category = Label::builder().label(&tr(MsgKey::label_category)).xalign(0.0).css_classes(["heading"]).build();
            let dropdown = DropDown::from_strings(&[]);
            let weak_drop = weak.clone();
            dropdown.connect_selected_notify(move |d| {
                if let Some(screen) = weak_drop.upgrade() {
                    let id = storage::id_at(&screen.options.borrow(), d.selected()).unwrap_or(storage::ALL_ID);
                    screen.selection.set(Selection::from_id(id));
                    screen.rebuild();
                }
            });

            let donut = DrawingArea::new();
            donut.set_content_width(260);
            donut.set_content_height(130);
            donut.set_halign(Align::Center);
            let weak_draw = weak.clone();
            donut.set_draw_func(move |_, cr, width, height| {
                if let Some(screen) = weak_draw.upgrade() {
                    screen.draw_donut(cr, f64::from(width), f64::from(height));
                }
            });
            let overlay_total = Label::builder().label("0 B").css_classes(["title-2"]).valign(Align::Start).margin_top(SPACE_M).build();
            let chart = Overlay::new();
            chart.set_child(Some(&donut));
            chart.add_overlay(&overlay_total);

            let scanning = GBox::new(O::Horizontal, SPACE_S);
            scanning.set_halign(Align::Center);
            scanning.append(&crate::ui::inline_spinner());
            scanning.append(&Label::new(Some(&copy::storage_scanning())));

            let list = crate::ui::boxed_list();
            let search = crate::ui::list_search(&list);
            let helper = crate::ui::dim(&copy::storage_empty_category());
            helper.set_visible(false);

            let split = GBox::new(O::Vertical, SPACE_M);
            let left = GBox::new(O::Vertical, SPACE_M);
            let right = GBox::new(O::Vertical, SPACE_M);
            left.append(&category);
            left.append(&dropdown);
            left.append(&chart);
            right.append(&scanning);
            right.append(&helper);
            right.append(&list);
            split.append(&left);
            split.append(&right);
            root.append(&split);
            let weak_tick = weak.clone();
            split.add_tick_callback(move |widget, _| {
                if let Some(screen) = weak_tick.upgrade() {
                    screen.reflow(widget.width());
                }
                gtk::glib::ControlFlow::Continue
            });

            Self {
                kind,
                root,
                split,
                left,
                right,
                chart,
                width: Cell::new(0),
                overlay_total,
                donut,
                dropdown,
                options: RefCell::default(),
                selection: Cell::new(Selection::All),
                input: RefCell::new(StorageInput { rows: Vec::new(), categories: Vec::new(), visible: Vec::new(), initializing: true }),
                snapshot: RefCell::default(),
                list,
                search,
                scanning,
                helper,
                on_delete: RefCell::default(),
            }
        })
    }

    /// The title search for the page header.
    pub fn search(&self) -> &gtk::SearchEntry {
        &self.search
    }

    /// The page widget: the column.
    pub fn widget(&self) -> &GBox {
        &self.root
    }

    /// Lays the page out for a content width: two columns at 2 to 3 from 720 px (the chart stays under the dropdown),
    /// else one column with the chart first. Runs on each frame and only acts when the width changed.
    fn reflow(&self, width: i32) {
        if width == self.width.get() {
            return;
        }
        self.width.set(width);
        if storage::two_column(width) {
            let (left, _) = storage::column_widths(width);
            self.split.set_orientation(O::Horizontal);
            self.left.set_size_request(left, -1);
            self.left.set_margin_end(storage::LEFT_COLUMN_GAP);
            self.left.set_hexpand(false);
            self.right.set_hexpand(true);
            self.left.reorder_child_after(&self.chart, Some(&self.dropdown));
        } else {
            self.split.set_orientation(O::Vertical);
            self.left.set_size_request(-1, -1);
            self.left.set_margin_end(0);
            self.left.set_hexpand(true);
            self.right.set_hexpand(true);
            self.left.reorder_child_after(&self.chart, Option::<&gtk::Widget>::None);
        }
    }

    /// Sets the delete hook (see the field's comment).
    pub fn connect_delete(&self, hook: impl Fn(&StorageRow) -> DeleteOutcome + 'static) {
        *self.on_delete.borrow_mut() = Some(Box::new(hook));
    }

    /// Replaces the data and redraws. The dropdown keeps its selection when the category is still there.
    pub fn set_input(self: &Rc<Self>, input: StorageInput) {
        let options = storage::category_options(&input.categories);
        let selected = self.selection.get().id();
        let index = storage::selected_index(&options, selected);
        *self.options.borrow_mut() = options.clone();
        let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
        self.dropdown.set_model(Some(&StringList::new(&labels)));
        self.dropdown.set_selected(index);
        self.selection.set(Selection::from_id(storage::id_at(&options, index).unwrap_or(storage::ALL_ID)));
        *self.input.borrow_mut() = input;
        self.rebuild();
    }

    fn rebuild(self: &Rc<Self>) {
        let input = self.input.borrow();
        let snapshot = storage::build_snapshot(input.rows.clone(), self.selection.get(), &input.visible);
        let state = storage::page_state(input.initializing);
        drop(input);
        *self.snapshot.borrow_mut() = snapshot;
        self.render(state);
    }

    fn render(self: &Rc<Self>, state: PageState) {
        let snapshot = self.snapshot.borrow().clone();
        self.scanning.set_visible(state == PageState::Scanning);
        self.overlay_total.set_label(&snapshot.total_text());
        self.helper.set_visible(state == PageState::Ready && snapshot.rows.is_empty());
        self.list.set_visible(state == PageState::Ready);
        self.donut.queue_draw();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for row in &snapshot.rows {
            let delete = Rc::new({
                let weak = Rc::downgrade(self);
                let row = row.clone();
                move || {
                    if let Some(screen) = weak.upgrade() {
                        screen.confirm_delete(&row);
                    }
                }
            });
            self.list.append(&crate::updates::storage_row::storage_row(row, self.kind, delete));
        }
    }

    /// The donut: arcs from the left end of the half circle to the right end, largest entry first.
    fn draw_donut(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let snapshot = self.snapshot.borrow();
        let radius = (width / 2.0).min(height) - DONUT_STROKE;
        let (cx, cy) = (width / 2.0, height);
        cr.set_line_width(DONUT_STROKE);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        for arc in storage::donut_arcs(&snapshot) {
            let [r, g, b, _] = arc.color;
            cr.set_source_rgb(f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0);
            let from = std::f64::consts::PI * (1.0 + arc.start);
            let to = std::f64::consts::PI * (1.0 + arc.start + arc.sweep);
            cr.new_path();
            cr.arc(cx, cy, radius, from, to);
            let _ = cr.stroke();
        }
    }

    /// The per-row confirm (T-42a-051): OK runs the delete hook, Cancel changes nothing.
    fn confirm_delete(self: &Rc<Self>, row: &StorageRow) {
        let title = if self.kind.is_anime() { copy::delete_title_episodes() } else { copy::delete_title_chapters() };
        let copy_text = ConfirmCopy {
            title,
            text: storage::delete_text(&row.title),
            checkbox: None,
            confirm: tr(MsgKey::action_ok),
            cancel: tr(MsgKey::action_cancel),
        };
        let weak: Weak<Self> = Rc::downgrade(self);
        let row = row.clone();
        show_confirm(
            copy_text,
            true,
            move |_| {
                let Some(screen) = weak.upgrade() else { return };
                let outcome = match &*screen.on_delete.borrow() {
                    Some(hook) => hook(&row),
                    None => DeleteOutcome::SourceMissing,
                };
                if outcome == DeleteOutcome::SourceMissing {
                    crate::toast::notify_for(&copy::storage_source_missing(), Duration::from_millis(NOT_FOUND_TOAST_MS));
                }
            },
            || {},
        );
    }
}
