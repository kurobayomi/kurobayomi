//! The chapter and episode list, one widget for both kinds: a `gio::ListStore` behind a `gtk::ListView`, so only the
//! rows on screen exist (3000 rows cost the same as 30). The rows are drawn as one card, centred and at most
//! `constants::LIST_MAX_WIDTH` wide: the unread dot (or the selection check), the title over a dim second line, an optional
//! summary, the bookmark and filler marks, and the download button. The store may hold leading slots before the rows
//! (widgets that scroll with the list, such as the info block of the narrow anime layout).
//!
//! `sync` diffs the new rows against the shown ones (`list_model::diff`): changed rows are one splice of new objects
//! over their span, and a changed structure is one splice. Rows report clicks (with Ctrl and
//! Shift), Enter, download presses and the items of their right-click menu (`list_model::row_menu`, through the app's
//! one `context_menu`); a long press (mouse or touch) reports the row's Select. The page decides what they mean.
//!
//! No callback into the page runs while the list holds a borrow of its own state: the page redraws the list from
//! inside a press (`act -> render -> sync`), so every handler copies the row out of its store item first.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::gdk::ModifierType;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, Label, Orientation as O, gio, glib, pango};
use kuro_platform::kind::Kind;
use kuro_i18n::{MsgKey, tr};

use crate::entry_core::list_model::{DOT_SIZE, Diff, RowAction, RowItem, changed_span, diff, row_alpha, row_menu};
use crate::entry_core::selection::Mods;
use crate::entry_core::text::COLLAPSED_LINES;
use crate::widgets::context_menu::{self, Item as MenuItem};
use crate::widgets::download_button::{DownloadButton, Request};

/// A click or Enter on a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    /// Alt was held (the anime page then opens the other player).
    pub alt: bool,
    pub mods: Mods,
}

/// What the rows report to the page, by chapter or episode id.
#[derive(Clone)]
pub struct Callbacks {
    /// A click or Enter: a plain one opens the row, Ctrl toggles it into the selection, Shift selects a range.
    pub on_row: Rc<dyn Fn(i64, Press)>,
    /// An item of the row's right-click menu (`list_model::row_menu`). It runs after the menu closed.
    pub on_menu: Rc<dyn Fn(i64, RowAction)>,
    /// A press on the row's download button.
    pub on_download: Rc<dyn Fn(i64, Request)>,
    /// A tap on an episode summary.
    pub on_summary: Rc<dyn Fn(i64)>,
}

/// Where a row sits in the card: the first and the last row get the rounded corners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Edge {
    first: bool,
    last: bool,
}

fn edge(index: usize, len: usize) -> Edge {
    Edge { first: index == 0, last: index + 1 == len }
}

/// One item of the store.
#[derive(Clone)]
enum Item {
    /// The n-th leading widget.
    Slot(usize),
    Row(RowItem, Edge),
}

fn item_of(list_item: &gtk::ListItem) -> Option<Item> {
    list_item.item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Item>().clone())
}

/// The row behind a list item, when it is a chapter or an episode (not a slot, not a marker).
fn row_of(list_item: &gtk::ListItem) -> Option<RowItem> {
    match item_of(list_item) {
        Some(Item::Row(row, _)) if row.missing.is_none() => Some(row),
        _ => None,
    }
}

fn mods_of(state: ModifierType) -> Mods {
    Mods { shift: state.contains(ModifierType::SHIFT_MASK), ctrl: state.contains(ModifierType::CONTROL_MASK) }
}

fn press_of(state: ModifierType) -> Press {
    Press { alt: state.contains(ModifierType::ALT_MASK), mods: mods_of(state) }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// The widgets of one recycled list item: the row card, or a host for a leading slot.
struct RowWidgets {
    stack: gtk::Stack,
    card: GBox,
    lead: gtk::Image,
    text: GBox,
    title: Label,
    subtitle: Label,
    summary: Label,
    marker: Label,
    bookmark: gtk::Image,
    filler: Label,
    download: DownloadButton,
    slot: GBox,
}

fn row_widgets(on_download: Rc<dyn Fn(Request)>) -> RowWidgets {
    let lead = gtk::Image::builder().width_request(16).halign(Align::Center).valign(Align::Center).build();
    let title = Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).single_line_mode(true).build();
    let subtitle = Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).single_line_mode(true).css_classes(["dim-label", "caption"]).build();
    let summary = Label::builder().xalign(0.0).wrap(true).ellipsize(pango::EllipsizeMode::End).css_classes(["dim-label", "caption"]).build();
    let text = GBox::new(O::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(Align::Center);
    text.append(&title);
    text.append(&subtitle);
    text.append(&summary);
    let marker = Label::builder().xalign(0.0).hexpand(true).visible(false).css_classes(["dim-label", "caption"]).build();
    let bookmark = gtk::Image::builder().icon_name("starred-symbolic").css_classes(["chapter-primary"]).valign(Align::Center).build();
    let filler = Label::builder().label(tr(MsgKey::filler)).css_classes(["caption", "episode-tertiary"]).valign(Align::Center).build();
    let download = DownloadButton::new(move |request| on_download(request));
    let card = GBox::new(O::Horizontal, 12);
    card.add_css_class("kb-item");
    card.set_size_request(-1, crate::ui::ROW_MIN_HEIGHT);
    card.set_margin_start(16);
    card.set_margin_end(16);
    for w in [lead.upcast_ref::<gtk::Widget>(), text.upcast_ref(), marker.upcast_ref(), filler.upcast_ref(), bookmark.upcast_ref(), download.widget.upcast_ref()] {
        card.append(w);
    }
    let slot = GBox::new(O::Vertical, 0);
    // a leading slot (the info block) is never the start of a drag
    crate::widgets::drag_select::mark(&slot, None);
    // a row is as tall as what it shows: a stack is as tall as its tallest child by default, which padded a leading
    // slot (the "11 episodes" line) to the height of an episode card
    let stack = gtk::Stack::builder().vhomogeneous(false).hhomogeneous(false).build();
    stack.add_named(&crate::clamp::clamp(&card, crate::entry_core::constants::LIST_MAX_WIDTH), Some("row"));
    stack.add_named(&slot, Some("slot"));
    RowWidgets { stack, card, lead, text, title, subtitle, summary, marker, bookmark, filler, download, slot }
}

impl RowWidgets {
    fn show_row(&self, row: &RowItem, edge: Edge) {
        self.empty_slot();
        self.stack.set_visible_child_name("row");
        set_class(&self.card, "kb-item-first", edge.first);
        set_class(&self.card, "kb-item-last", edge.last);
        set_class(&self.card, "kb-item-selected", row.selected);
        let item = row.missing.is_none();
        self.stack.set_widget_name(&if item { format!("row:{}", row.id) } else { String::new() });
        // the card is the row's own press area; a drag that starts beside it selects (`widgets::drag_select`)
        crate::widgets::drag_select::mark(&self.card, item.then_some(row.id));
        for w in [self.lead.upcast_ref::<gtk::Widget>(), self.text.upcast_ref()] {
            w.set_visible(item);
        }
        self.marker.set_visible(!item);
        if !item {
            // a marker is one dim line: no marks, no download, nothing to select
            self.marker.set_label(&row.subtitle);
            self.bookmark.set_visible(false);
            self.filler.set_visible(false);
            self.download.widget.set_visible(false);
            return;
        }
        // the lead keeps its width in every state, so the titles line up
        if row.selected {
            self.lead.set_icon_name(Some("object-select-symbolic"));
            self.lead.set_pixel_size(16);
        } else if !row.read {
            self.lead.set_icon_name(Some("media-record-symbolic"));
            self.lead.set_pixel_size(DOT_SIZE);
        } else {
            self.lead.set_icon_name(None);
        }
        set_class(&self.lead, "chapter-primary", row.selected || !row.read);
        self.title.set_label(&row.title);
        self.subtitle.set_label(&row.subtitle);
        self.subtitle.set_visible(!row.subtitle.is_empty());
        self.summary.set_visible(row.summary.is_some());
        self.summary.set_label(row.summary.as_deref().unwrap_or(""));
        self.summary.set_lines(if row.summary_expanded { -1 } else { COLLAPSED_LINES });
        self.text.set_opacity(row_alpha(row.read));
        self.bookmark.set_visible(row.bookmark);
        self.filler.set_visible(row.filler);
        self.download.widget.set_visible(row.indicator);
        // the button answers presses only while nothing is selected
        self.download.widget.set_sensitive(row.indicator_enabled);
        self.download.set(row.download, row.progress);
    }

    fn show_slot(&self, widget: &gtk::Widget) {
        self.empty_slot();
        self.stack.set_widget_name("");
        if let Some(parent) = widget.parent().and_downcast::<GBox>() {
            parent.remove(widget);
        }
        self.slot.append(widget);
        self.stack.set_visible_child_name("slot");
    }

    fn empty_slot(&self) {
        while let Some(child) = self.slot.first_child() {
            self.slot.remove(&child);
        }
    }
}

/// The empty state of a chapter or episode list, for both kinds: `message` over a Retry button (the shared
/// `browse::look::empty_state_with`). A fetch that finds nothing says so here, not in a notification. Returns the
/// state and its message label, for a page that changes the message.
pub fn empty_list(message: &str, on_retry: impl Fn() + 'static) -> (GBox, Label) {
    let retry = gtk::Button::with_label(&tr(MsgKey::action_retry));
    retry.connect_clicked(move |_| on_retry());
    crate::browse::look::empty_state("view-list-symbolic", message, &[retry])
}

/// The items of the right-click menu of the row behind `list_item`, built when the menu opens so the labels match the
/// row as it is now. The row is copied out of the store item before anything runs.
fn menu_items(list_item: &glib::WeakRef<gtk::ListItem>, kind: Kind, on_menu: &Rc<dyn Fn(i64, RowAction)>) -> Vec<MenuItem> {
    let Some(row) = list_item.upgrade().and_then(|item| row_of(&item)) else { return Vec::new() };
    row_menu(&row, kind)
        .into_iter()
        .map(|(label, section, action)| {
            let (on_menu, id) = (Rc::clone(on_menu), row.id);
            MenuItem::new(tr(label), section, move || on_menu(id, action))
        })
        .collect()
}

/// Brings the row part of `store` (everything from `offset`) from `shown` to `rows`; see the module text.
fn sync_store(store: &gio::ListStore, offset: u32, shown: &[RowItem], rows: &[RowItem]) {
    let item = |i: usize| Item::Row(rows[i].clone(), edge(i, rows.len()));
    match diff(shown, rows) {
        Diff::Same => {}
        // always fresh objects: GTK does not rebind a row whose object is the same, so an edit in place (a download
        // turning into progress, then a tick) would never reach the row's widgets
        Diff::Rows(indices) => {
            if let Some((start, len)) = changed_span(&indices) {
                let objects: Vec<glib::BoxedAnyObject> = (start..start + len).map(|i| glib::BoxedAnyObject::new(item(i))).collect();
                store.splice(offset + start as u32, len as u32, &objects);
            }
        }
        Diff::Reset => {
            let objects: Vec<glib::BoxedAnyObject> = (0..rows.len()).map(|i| glib::BoxedAnyObject::new(item(i))).collect();
            store.splice(offset, store.n_items() - offset, &objects);
        }
    }
}

pub struct ItemList {
    pub widget: gtk::ListView,
    store: gio::ListStore,
    leading: RefCell<Vec<gtk::Widget>>,
    shown: RefCell<Vec<RowItem>>,
    /// The page's row press: a drag selects through it with Ctrl held, as a Ctrl+click does.
    on_row: Rc<dyn Fn(i64, Press)>,
}

impl ItemList {
    /// The list of `kind`'s rows: the kind only words the menu (read or watched).
    pub fn new(kind: Kind, callbacks: Callbacks) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let registry: Rc<RefCell<HashMap<usize, Rc<RowWidgets>>>> = Rc::default();
        let list: Rc<RefCell<Weak<Self>>> = Rc::default();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup({
            let (registry, callbacks) = (Rc::clone(&registry), callbacks.clone());
            move |_, object| {
                let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else { return };
                // Enter is the list's own (`key_controller`), so the item does not activate on it or on a double click
                list_item.set_activatable(false);
                let on_download: Rc<dyn Fn(Request)> = {
                    let (list_item, on_download) = (list_item.clone(), Rc::clone(&callbacks.on_download));
                    Rc::new(move |request: Request| {
                        if let Some(row) = row_of(&list_item) {
                            on_download(row.id, request);
                        }
                    })
                };
                let widgets = Rc::new(row_widgets(on_download));
                // set by a long press, so the release that ends it does not also open the row
                let long_pressed = Rc::new(std::cell::Cell::new(false));
                let left = gtk::GestureClick::builder().button(1).build();
                left.connect_pressed({
                    let long_pressed = Rc::clone(&long_pressed);
                    move |_, _, _, _| long_pressed.set(false)
                });
                left.connect_released({
                    let (list_item, on_row, long_pressed) = (list_item.clone(), Rc::clone(&callbacks.on_row), Rc::clone(&long_pressed));
                    move |gesture, _, _, _| {
                        if long_pressed.get() {
                            return;
                        }
                        if let Some(row) = row_of(&list_item) {
                            on_row(row.id, press_of(gesture.current_event_state()));
                        }
                    }
                });
                widgets.card.add_controller(left);
                // a long press, with the mouse or on a touch screen, starts the selection on this row (the user's choice)
                let long = gtk::GestureLongPress::new();
                long.connect_pressed({
                    let (list_item, on_menu) = (list_item.downgrade(), Rc::clone(&callbacks.on_menu));
                    move |gesture, _, _| {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        long_pressed.set(true);
                        // the id is copied out of the store item, and the page runs on idle, so no borrow is held
                        // while it redraws the list (`act -> render -> sync`)
                        let Some(id) = list_item.upgrade().and_then(|item| row_of(&item)).map(|row| row.id) else { return };
                        let on_menu = Rc::clone(&on_menu);
                        glib::idle_add_local_once(move || on_menu(id, RowAction::Select));
                    }
                });
                widgets.card.add_controller(long);
                // the right click (and the Menu key) open the row's menu
                context_menu::attach_without_long_press(&widgets.card, {
                    let (list_item, on_menu) = (list_item.downgrade(), Rc::clone(&callbacks.on_menu));
                    move |_, _| menu_items(&list_item, kind, &on_menu)
                });
                // a tap on the summary toggles it and claims the press, so the row does not open (T-39a-012)
                let summary_tap = gtk::GestureClick::builder().button(1).build();
                summary_tap.connect_released({
                    let (list_item, on_summary) = (list_item.clone(), Rc::clone(&callbacks.on_summary));
                    move |gesture, _, _, _| {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        if let Some(row) = row_of(&list_item) {
                            on_summary(row.id);
                        }
                    }
                });
                widgets.summary.add_controller(summary_tap);
                list_item.set_child(Some(&widgets.stack));
                registry.borrow_mut().insert(list_item.as_ptr() as usize, widgets);
            }
        });
        factory.connect_bind({
            let (registry, list) = (Rc::clone(&registry), Rc::clone(&list));
            move |_, object| {
                let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else { return };
                let (Some(item), Some(widgets)) = (item_of(list_item), registry.borrow().get(&(list_item.as_ptr() as usize)).cloned()) else { return };
                match item {
                    Item::Row(row, edge) => widgets.show_row(&row, edge),
                    Item::Slot(i) => {
                        let Some(this) = list.borrow().upgrade() else { return };
                        if let Some(w) = this.leading.borrow().get(i) {
                            widgets.show_slot(w);
                        }
                    }
                }
            }
        });
        factory.connect_unbind({
            let registry = Rc::clone(&registry);
            move |_, object| {
                if let Some(list_item) = object.downcast_ref::<gtk::ListItem>()
                    && let Some(widgets) = registry.borrow().get(&(list_item.as_ptr() as usize))
                {
                    widgets.empty_slot();
                }
            }
        });
        factory.connect_teardown({
            let registry = Rc::clone(&registry);
            move |_, object| {
                if let Some(list_item) = object.downcast_ref::<gtk::ListItem>() {
                    registry.borrow_mut().remove(&(list_item.as_ptr() as usize));
                }
            }
        });
        let widget = gtk::ListView::new(Some(gtk::NoSelection::new(Some(store.clone()))), Some(factory));
        widget.set_focusable(true);
        widget.add_css_class("kb-item-list");
        let this = Rc::new(Self {
            widget,
            store,
            leading: RefCell::new(Vec::new()),
            shown: RefCell::new(Vec::new()),
            on_row: Rc::clone(&callbacks.on_row),
        });
        *list.borrow_mut() = Rc::downgrade(&this);
        this.widget.add_controller(key_controller(Rc::downgrade(&this), callbacks.on_row));
        this
    }

    /// What a drag over the list (`widgets::drag_select`) asks: a row's selected state as shown, and a flip of one row,
    /// which is a Ctrl+click of it. Clearing flips every selected row on show; one a filter hides keeps its state.
    pub fn drag_hooks(self: &Rc<Self>) -> crate::widgets::drag_select::Hooks {
        let ctrl = Press { alt: false, mods: Mods { shift: false, ctrl: true } };
        let (shown, toggle, clear) = (Rc::downgrade(self), Rc::clone(&self.on_row), Rc::downgrade(self));
        crate::widgets::drag_select::Hooks {
            is_selected: Rc::new(move |id| shown.upgrade().is_some_and(|list| list.shown.borrow().iter().any(|row| row.id == id && row.selected))),
            toggle: Rc::new(move |id| toggle(id, ctrl)),
            clear: Rc::new(move || {
                let Some(list) = clear.upgrade() else { return };
                let selected: Vec<i64> = list.shown.borrow().iter().filter(|row| row.selected).map(|row| row.id).collect();
                for id in selected {
                    (list.on_row)(id, ctrl);
                }
            }),
        }
    }

    /// Replaces the leading slots (widgets that scroll with the rows). One splice of the head of the store.
    pub fn set_leading(&self, widgets: Vec<gtk::Widget>) {
        let old = self.leading.borrow().len() as u32;
        let objects: Vec<glib::BoxedAnyObject> = (0..widgets.len()).map(|i| glib::BoxedAnyObject::new(Item::Slot(i))).collect();
        *self.leading.borrow_mut() = widgets;
        self.store.splice(0, old, &objects);
    }

    /// Shows `rows`: see `sync_store`. The shown rows are swapped in before the store changes, so a handler that the
    /// change runs (a rebind, a focus move) finds no borrow of them held.
    pub fn sync(&self, rows: Vec<RowItem>) {
        let offset = self.leading.borrow().len() as u32;
        let old = self.shown.replace(rows.clone());
        // a row is replaced by a fresh object; when that row holds keyboard focus (it was just clicked, as in a
        // selection), GTK loses its place and scrolls the list to the top. Focus leaves the row first and comes back
        // to the same row after, so the list stays where it was.
        let focused = self.focused_id();
        if focused.is_some()
            && let Some(root) = self.widget.root()
        {
            root.set_focus(None::<&gtk::Widget>);
        }
        sync_store(&self.store, offset, &old, &rows);
        // once GTK has made the new row's widget, which it does on its next turn: the list's cursor goes to the row, and
        // the row (its content is named `row:<id>`) takes keyboard focus again
        if let Some((id, index)) = focused.and_then(|id| rows.iter().position(|r| r.id == id).map(|index| (id, index))) {
            let widget = self.widget.downgrade();
            glib::idle_add_local_once(move || {
                let Some(widget) = widget.upgrade() else { return };
                widget.scroll_to(offset + index as u32, gtk::ListScrollFlags::FOCUS, None);
                let name = format!("row:{id}");
                let mut stack = vec![widget.upcast::<gtk::Widget>()];
                while let Some(w) = stack.pop() {
                    if w.widget_name() == name {
                        // the named row content cannot take focus; the list's item around it can
                        if !w.grab_focus()
                            && let Some(item) = w.parent()
                        {
                            item.grab_focus();
                        }
                        return;
                    }
                    let mut child = w.first_child();
                    while let Some(c) = child {
                        child = c.next_sibling();
                        stack.push(c);
                    }
                }
            });
        }
    }

    /// The id of the row that holds keyboard focus.
    pub fn focused_id(&self) -> Option<i64> {
        let row_id = |w: &gtk::Widget| w.widget_name().strip_prefix("row:").and_then(|n| n.parse().ok());
        let mut cur = Some(self.widget.root()?.focus()?);
        while let Some(w) = cur {
            if let Some(id) = row_id(&w).or_else(|| w.first_child().and_then(|c| row_id(&c))) {
                return Some(id);
            }
            if w == *self.widget.upcast_ref::<gtk::Widget>() {
                break;
            }
            cur = w.parent();
        }
        None
    }

    /// Moves keyboard focus to the row of `id`.
    pub fn focus_row(&self, id: i64) {
        let offset = self.leading.borrow().len();
        let index = self.shown.borrow().iter().position(|r| r.id == id);
        if let Some(index) = index {
            self.widget.scroll_to((offset + index) as u32, gtk::ListScrollFlags::FOCUS, None);
        }
    }
}

/// Enter on the focused row is a plain press of it, with the modifiers held.
fn key_controller(list: Weak<ItemList>, on_row: Rc<dyn Fn(i64, Press)>) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, state| {
        if !matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
            return glib::Propagation::Proceed;
        }
        match list.upgrade().and_then(|list| list.focused_id()) {
            Some(id) => {
                on_row(id, press_of(state));
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });
    keys
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn rows(n: i64) -> Vec<RowItem> {
        (1..=n).map(|id| RowItem::new(id, format!("Episode {id}"))).collect()
    }

    /// A store with two leading slots and `shown`, and a counter of its `items_changed` signals.
    fn store_with(shown: &[RowItem]) -> (gio::ListStore, Rc<Cell<u32>>) {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let items: Vec<glib::BoxedAnyObject> = (0..2).map(|i| glib::BoxedAnyObject::new(Item::Slot(i))).collect();
        store.splice(0, 0, &items);
        let items: Vec<glib::BoxedAnyObject> = shown.iter().enumerate().map(|(i, r)| glib::BoxedAnyObject::new(Item::Row(r.clone(), edge(i, shown.len())))).collect();
        store.splice(2, 0, &items);
        let signals = Rc::new(Cell::new(0));
        store.connect_items_changed({
            let signals = Rc::clone(&signals);
            move |_, _, _, _| signals.set(signals.get() + 1)
        });
        (store, signals)
    }

    #[test]
    fn one_changed_row_is_one_signal_and_an_identical_list_is_none() {
        let shown = rows(5);
        let (store, signals) = store_with(&shown);
        let mut next = shown.clone();
        next[3].read = true;
        sync_store(&store, 2, &shown, &next);
        assert_eq!(signals.get(), 1);
        sync_store(&store, 2, &next, &next);
        assert_eq!(signals.get(), 1, "nothing changed");
    }

    #[test]
    fn one_changed_row_gets_a_new_object_so_the_row_is_bound_again() {
        let shown = rows(3);
        let (store, signals) = store_with(&shown);
        let before = store.item(3).unwrap();
        let mut changed = shown.clone();
        changed[1].progress = 40;
        sync_store(&store, 2, &shown, &changed);
        assert_eq!(signals.get(), 1);
        let after = store.item(3).unwrap();
        assert_ne!(before, after, "GTK skips the rebind of an unchanged object");
        assert!(matches!(&*after.downcast::<glib::BoxedAnyObject>().unwrap().borrow::<Item>(), Item::Row(r, _) if r.progress == 40));
    }

    #[test]
    fn a_bulk_change_is_one_splice_that_keeps_the_list_the_same_length() {
        let shown = rows(300);
        let (store, signals) = store_with(&shown);
        let all_selected: Vec<RowItem> = shown.iter().cloned().map(|r| RowItem { selected: true, ..r }).collect();
        sync_store(&store, 2, &shown, &all_selected);
        assert_eq!(signals.get(), 1, "select all is one signal, not 300");
        assert_eq!(store.n_items(), 302);
        let last = store.item(301).and_downcast::<glib::BoxedAnyObject>().unwrap();
        assert!(matches!(&*last.borrow::<Item>(), Item::Row(r, e) if r.selected && e.last && !e.first));
        let slot = store.item(1).and_downcast::<glib::BoxedAnyObject>().unwrap();
        assert!(matches!(&*slot.borrow::<Item>(), Item::Slot(1)), "the leading slots are untouched");
    }

    #[test]
    fn a_different_set_of_rows_is_one_splice_over_the_row_part() {
        let shown = rows(4);
        let (store, signals) = store_with(&shown);
        sync_store(&store, 2, &shown, &rows(2));
        assert_eq!((signals.get(), store.n_items()), (1, 4));
        let first = store.item(2).and_downcast::<glib::BoxedAnyObject>().unwrap();
        assert!(matches!(&*first.borrow::<Item>(), Item::Row(_, e) if e.first && !e.last));
    }

    #[test]
    fn the_first_and_last_rows_round_the_card() {
        assert_eq!(edge(0, 1), Edge { first: true, last: true });
        assert_eq!(edge(0, 3), Edge { first: true, last: false });
        assert_eq!(edge(1, 3), Edge { first: false, last: false });
        assert_eq!(edge(2, 3), Edge { first: false, last: true });
    }

    /// One display test: a chapter and an episode draw the same row, a read one is dimmed with no dot, a marker shows
    /// only its line, and a local item has no download button.
    #[test]
    fn chapters_and_episodes_draw_the_same_row() {
        crate::testing::with_display(|| {
            let widgets = row_widgets(Rc::new(|_: Request| {}));
            let chapter = RowItem { subtitle: "2024-01-02 \u{2022} Group".into(), indicator: true, indicator_enabled: true, ..RowItem::new(1, "Chapter 1".into()) };
            widgets.show_row(&chapter, edge(0, 3));
            assert!(widgets.card.has_css_class("kb-item-first") && !widgets.card.has_css_class("kb-item-last"));
            assert_eq!(widgets.lead.icon_name().as_deref(), Some("media-record-symbolic"), "the unread dot");
            assert!(widgets.download.widget.is_visible() && widgets.subtitle.is_visible());
            assert_eq!(widgets.stack.widget_name().as_str(), "row:1");
            let episode = RowItem { read: true, filler: true, bookmark: true, ..RowItem::new(2, "Episode 2".into()) };
            widgets.show_row(&episode, edge(2, 3));
            assert!(widgets.card.has_css_class("kb-item-last") && !widgets.card.has_css_class("kb-item-first"));
            assert_eq!(widgets.lead.icon_name(), None, "no dot on a seen episode");
            assert!((widgets.text.opacity() - 0.38).abs() < 0.01, "a seen episode is dimmed");
            assert!(widgets.filler.is_visible() && widgets.bookmark.is_visible());
            assert!(!widgets.download.widget.is_visible(), "no download button without an indicator");
            assert!(!widgets.subtitle.is_visible(), "an empty second line takes no space");
            widgets.show_row(&RowItem::marker("1-4".into(), 2, "Missing 2 chapters".into()), edge(1, 3));
            assert!(widgets.marker.is_visible() && !widgets.text.is_visible() && !widgets.download.widget.is_visible());
            assert_eq!(widgets.marker.label().as_str(), "Missing 2 chapters");
        });
    }

    /// Selecting a row redraws it as a fresh object while it holds keyboard focus; GTK then lost its place and the
    /// list jumped to the top (user, 2026-10-27). It stays where it was now.
    #[test]
    fn redrawing_the_focused_row_keeps_the_list_where_it_was() {
        crate::testing::with_display(|| {
            let ctx = glib::MainContext::default();
            let settle = || {
                for _ in 0..200 {
                    while ctx.iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };
            let rows = |selected: Option<i64>| -> Vec<RowItem> {
                (1..=200).map(|id| RowItem { selected: Some(id) == selected, ..RowItem::new(id, format!("Episode {id}")) }).collect()
            };
            let list = ItemList::new(
                Kind::Anime,
                Callbacks {
                    on_row: Rc::new(|_: i64, _: Press| {}),
                    on_menu: Rc::new(|_: i64, _: RowAction| {}),
                    on_download: Rc::new(|_: i64, _: Request| {}),
                    on_summary: Rc::new(|_: i64| {}),
                },
            );
            list.sync(rows(None));
            let scroller = gtk::ScrolledWindow::builder().child(&list.widget).build();
            let window = gtk::Window::builder().default_width(600).default_height(400).child(&scroller).build();
            window.present();
            settle();
            let adjustment = scroller.vadjustment();
            adjustment.set_value(adjustment.upper() / 2.0);
            settle();
            let before = adjustment.value();
            // the row a click would focus: one on screen
            let id = list.shown.borrow().iter().map(|r| r.id).nth((before / adjustment.upper() * 200.0) as usize + 2).expect("a row on screen");
            list.focus_row(id);
            assert!(crate::entry_core::page_shell::test_util::pump(&ctx, || list.focused_id() == Some(id)), "the row holds focus");
            settle();
            let focused_at = adjustment.value();
            list.sync(rows(Some(id)));
            assert!(crate::entry_core::page_shell::test_util::pump(&ctx, || list.focused_id() == Some(id)), "focus is back on the row");
            settle();
            // putting focus back may nudge the row fully into view, never more than about a row
            assert!((adjustment.value() - focused_at).abs() < 100.0, "the list stays put: {focused_at} -> {}", adjustment.value());
            assert!(adjustment.value() > before / 2.0, "not back at the top");
            window.destroy();
        });
    }

    /// The crash of round 3: a press whose page redraws the list from inside the handler (`act -> render -> sync`)
    /// panicked with "already borrowed". A plain click, a right click, a touch long press and the menu's Select now
    /// each run a handler that re-syncs the list, and nothing panics.
    #[test]
    fn a_press_that_redraws_the_list_from_its_handler_does_not_panic() {
        crate::testing::with_display(|| {
            use crate::entry_core::page_shell::test_util::{descendants, pump};
            let ctx = glib::MainContext::default();
            let shown = |selected: Option<i64>| -> Vec<RowItem> {
                (1..=3).map(|id| RowItem { indicator: true, indicator_enabled: true, selected: Some(id) == selected, ..RowItem::new(id, format!("Chapter {id}")) }).collect()
            };
            let slot: Rc<RefCell<Weak<ItemList>>> = Rc::default();
            let pressed: Rc<RefCell<Vec<(i64, Option<RowAction>)>>> = Rc::default();
            let redraw = {
                let slot = Rc::clone(&slot);
                move |id: i64| {
                    if let Some(list) = slot.borrow().upgrade() {
                        list.sync(shown(Some(id)));
                    }
                }
            };
            let list = ItemList::new(
                Kind::Manga,
                Callbacks {
                    on_row: Rc::new({
                        let (pressed, redraw) = (Rc::clone(&pressed), redraw.clone());
                        move |id: i64, _: Press| {
                            pressed.borrow_mut().push((id, None));
                            redraw(id);
                        }
                    }),
                    on_menu: Rc::new({
                        let pressed = Rc::clone(&pressed);
                        move |id: i64, action: RowAction| {
                            pressed.borrow_mut().push((id, Some(action)));
                            redraw(id);
                        }
                    }),
                    on_download: Rc::new(|_: i64, _: Request| {}),
                    on_summary: Rc::new(|_: i64| {}),
                },
            );
            *slot.borrow_mut() = Rc::downgrade(&list);
            list.sync(shown(None));
            let window = gtk::Window::builder().default_width(600).default_height(400).child(&list.widget).build();
            window.present();
            let card = || descendants(list.widget.upcast_ref()).into_iter().find(|w| w.has_css_class("kb-item") && w.is_visible());
            assert!(pump(&ctx, || card().is_some()), "the list draws its rows");
            let first = card().expect("a row");
            let model = first.observe_controllers();
            let controllers: Vec<gtk::EventController> = (0..model.n_items()).filter_map(|i| model.item(i).and_downcast::<gtk::EventController>()).collect();
            let click = |button: u32| controllers.iter().filter_map(|c| c.downcast_ref::<gtk::GestureClick>()).find(|g| g.button() == button).expect("a click gesture").clone();
            let longs: Vec<gtk::GestureLongPress> = controllers.iter().filter_map(|c| c.downcast_ref::<gtk::GestureLongPress>().cloned()).collect();
            assert_eq!(longs.len(), 1, "one long press: the row's own, not the menu's");
            let long = longs[0].clone();
            assert!(!long.is_touch_only(), "a mouse long press selects too");
            let selects = || pressed.borrow().iter().filter(|(_, a)| *a == Some(RowAction::Select)).count();

            click(1).emit_by_name::<()>("released", &[&1i32, &4.0f64, &4.0f64]);
            assert_eq!(pressed.borrow().len(), 1, "a plain click reached the page");
            // the click redrew the row, and the focus it held came back to a new widget: press the row on screen now
            let shown_now = card().expect("a row");
            let model = shown_now.observe_controllers();
            let controllers: Vec<gtk::EventController> = (0..model.n_items()).filter_map(|i| model.item(i).and_downcast::<gtk::EventController>()).collect();
            let press = controllers.iter().filter_map(|c| c.downcast_ref::<gtk::GestureClick>()).find(|g| g.button() == 1).expect("a click gesture").clone();
            let long = controllers.iter().find_map(|c| c.downcast_ref::<gtk::GestureLongPress>().cloned()).expect("the long press");
            press.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
            long.emit_by_name::<()>("pressed", &[&4.0f64, &4.0f64]);
            assert!(pump(&ctx, || selects() == 1), "the long press selects the row, with no borrow held");
            press.emit_by_name::<()>("released", &[&1i32, &4.0f64, &4.0f64]);
            assert_eq!(pressed.borrow().len(), 2, "the release that ends a long press does not open the row");
            let menu = || descendants(list.widget.upcast_ref()).into_iter().find_map(|w| w.downcast::<gtk::PopoverMenu>().ok());
            assert!(menu().is_none(), "the long press opens no menu");
            // the select redrew the row with a new item, so look the row's widget up again
            let card = card().expect("a row");
            let model = card.observe_controllers();
            let right = (0..model.n_items())
                .filter_map(|i| model.item(i).and_downcast::<gtk::GestureClick>())
                .find(|g| g.button() == 3)
                .expect("a right-click gesture");
            right.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
            assert!(pump(&ctx, || menu().is_some()), "the right click opens the row's menu");
            let select = row_menu(&shown(None)[0], Kind::Manga).iter().position(|(_, _, a)| *a == RowAction::Select).expect("a Select item");
            menu().expect("the menu").activate_action(&format!("ctx.item{select}"), None).expect("the Select item");
            assert!(pump(&ctx, || selects() == 2), "Select from the menu reached the page");
            window.close();
        });
    }

    /// The one empty state of both kinds' lists: the message and a Retry that runs `on_retry`.
    #[test]
    fn the_empty_list_shows_its_message_with_retry() {
        crate::testing::with_display(|| {
            let retried = Rc::new(std::cell::Cell::new(0));
            let count = Rc::clone(&retried);
            let (state, label) = empty_list("No chapters found", move || count.set(count.get() + 1));
            assert_eq!(label.text(), "No chapters found");
            let retry = state.last_child().and_downcast::<gtk::Button>().expect("the Retry button under the message");
            assert_eq!(retry.label().as_deref(), Some("Retry"));
            retry.emit_clicked();
            assert_eq!(retried.get(), 1);
        });
    }
}

