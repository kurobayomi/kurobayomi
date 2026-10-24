//! The one view-options menu of the app (user decision 2026-10-12, like Nautilus's): a `gtk::MenuButton` with a native
//! `gio::Menu` of stateful actions, built from a small description. A [`Section`] holds toggles (boolean actions), radio
//! groups (one string action per group, each item carries its value), plain actions and submenus. A `filter` section
//! ("Show only") puts a dot on the button while any of its toggles is on.
//!
//! A pick sets the action's state at once and reports a [`Pick`]; the owner writes it and later calls `set_toggle` /
//! `set_radio` with what is stored, so the menu always ends up showing the store. The library, the details pages
//! (manga chapters, anime episodes and seasons) all build their menu here.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

/// The prefix of the action group on the button.
const GROUP: &str = "viewopt";

/// One menu item. `name` is the action name; radios of one group share it.
pub enum Item {
    Toggle { name: &'static str, label: String },
    Radio { name: &'static str, value: String, label: String },
    Action { name: &'static str, label: String },
    Submenu { label: String, items: Vec<Item> },
    /// A widget of the owner's in the menu (a slider): `id` names the slot, `ViewOptions::set_custom` gives the widget.
    Custom { id: &'static str },
}

impl Item {
    pub fn toggle(name: &'static str, label: impl Into<String>) -> Self {
        Item::Toggle { name, label: label.into() }
    }

    pub fn radio(name: &'static str, value: impl Into<String>, label: impl Into<String>) -> Self {
        Item::Radio { name, value: value.into(), label: label.into() }
    }

    pub fn action(name: &'static str, label: impl Into<String>) -> Self {
        Item::Action { name, label: label.into() }
    }

    pub fn submenu(label: impl Into<String>, items: Vec<Item>) -> Self {
        Item::Submenu { label: label.into(), items }
    }

    pub fn custom(id: &'static str) -> Self {
        Item::Custom { id }
    }
}

/// A titled (or untitled) block of the menu. `filter` marks the "Show only" block whose toggles light the dot.
pub struct Section {
    pub title: Option<String>,
    pub filter: bool,
    pub items: Vec<Item>,
}

impl Section {
    pub fn new(title: impl Into<String>, items: Vec<Item>) -> Self {
        Section { title: Some(title.into()), filter: false, items }
    }

    /// A "Show only" block: any toggle of it that is on lights the dot.
    pub fn filter(title: impl Into<String>, items: Vec<Item>) -> Self {
        Section { title: Some(title.into()), filter: true, items }
    }

    #[cfg(test)]
    pub fn untitled(items: Vec<Item>) -> Self {
        Section { title: None, filter: false, items }
    }
}

/// What the user picked: a toggle and its new state, a radio group and the picked value, or a plain action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Toggle(&'static str, bool),
    Radio(&'static str, String),
    Action(&'static str),
}

/// The toggle names of the filter sections, submenus included: the ones that light the dot.
pub fn filter_names(sections: &[Section]) -> Vec<&'static str> {
    fn walk(items: &[Item], out: &mut Vec<&'static str>) {
        for item in items {
            match item {
                Item::Toggle { name, .. } => out.push(*name),
                Item::Submenu { items, .. } => walk(items, out),
                Item::Radio { .. } | Item::Action { .. } | Item::Custom { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    for section in sections.iter().filter(|s| s.filter) {
        walk(&section.items, &mut out);
    }
    out
}

fn menu_of(items: &[Item]) -> gio::Menu {
    let menu = gio::Menu::new();
    for item in items {
        match item {
            Item::Toggle { name, label } | Item::Action { name, label } => menu.append(Some(label.as_str()), Some(&format!("{GROUP}.{name}"))),
            Item::Radio { name, value, label } => {
                let entry = gio::MenuItem::new(Some(label.as_str()), None);
                entry.set_action_and_target_value(Some(&format!("{GROUP}.{name}")), Some(&value.to_variant()));
                menu.append_item(&entry);
            }
            Item::Submenu { label, items } => menu.append_submenu(Some(label.as_str()), &menu_of(items)),
            Item::Custom { id } => {
                let entry = gio::MenuItem::new(None, None);
                entry.set_attribute_value("custom", Some(&id.to_variant()));
                menu.append_item(&entry);
            }
        }
    }
    menu
}

fn bool_state(action: &gio::SimpleAction) -> bool {
    action.state().and_then(|v| v.get::<bool>()).unwrap_or(false)
}

struct Inner {
    dot: gtk::Box,
    actions: RefCell<HashMap<&'static str, gio::SimpleAction>>,
    /// The toggles of the filter sections in the current menu.
    filters: RefCell<Vec<&'static str>>,
    /// A filter the menu has no toggle for (the manga's scanlator exclusions) is on.
    extra: Cell<bool>,
    handler: RefCell<Option<Rc<dyn Fn(Pick)>>>,
    /// The custom slots and what builds their widget. A rebuilt menu is a new popover, so each gets a fresh widget.
    customs: RefCell<Vec<(&'static str, Rc<dyn Fn() -> gtk::Widget>)>>,
}

impl Inner {
    fn refresh_dot(&self) {
        let actions = self.actions.borrow();
        let on = self.extra.get() || self.filters.borrow().iter().any(|name| actions.get(name).is_some_and(bool_state));
        self.dot.set_visible(on);
    }

    /// Reports a pick. The handler is copied out first, so it may call back into the menu.
    fn emit(&self, pick: Pick) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(pick);
        }
    }
}

/// The button with its dot, and the actions behind the menu. Cheap to clone (GObjects and one `Rc`).
#[derive(Clone)]
pub struct ViewOptions {
    /// What a header shows: the menu button with the dot over its corner.
    pub widget: gtk::Overlay,
    button: gtk::MenuButton,
    group: gio::SimpleActionGroup,
    inner: Rc<Inner>,
}

impl ViewOptions {
    /// The button (`tooltip` is also its accessible name) and the menu of `sections`.
    pub fn new(tooltip: &str, sections: &[Section]) -> Self {
        let group = gio::SimpleActionGroup::new();
        let button = crate::ui::quiet_menu(gtk::MenuButton::builder().icon_name("view-sort-descending-symbolic").tooltip_text(tooltip).build());
        button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
        button.insert_action_group(GROUP, Some(&group));
        // the dot sits over the button's corner and never takes a click
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("kb-dot");
        dot.set_halign(gtk::Align::End);
        dot.set_valign(gtk::Align::Start);
        dot.set_can_target(false);
        dot.set_visible(false);
        let widget = gtk::Overlay::new();
        widget.set_child(Some(&button));
        widget.add_overlay(&dot);
        let inner = Rc::new(Inner { dot, actions: RefCell::default(), filters: RefCell::default(), extra: Cell::new(false), handler: RefCell::default(), customs: RefCell::default() });
        // the actions reach the menu's state weakly, so the button keeps it alive: a caller may drop this value once the
        // button is placed
        let keep = Rc::clone(&inner);
        button.connect_destroy(move |_| {
            let _ = &keep;
        });
        let this = Self { widget, button, group, inner };
        this.set_sections(sections);
        this
    }

    /// The one handler of every pick. A later call replaces it.
    pub fn connect(&self, f: impl Fn(Pick) + 'static) {
        *self.inner.handler.borrow_mut() = Some(Rc::new(f));
    }

    /// Rebuilds the menu. Actions are kept by name, so a state set earlier survives; call it only when the
    /// description changed (it replaces an open menu).
    pub fn set_sections(&self, sections: &[Section]) {
        let menu = gio::Menu::new();
        for section in sections {
            self.ensure(&section.items);
            menu.append_section(section.title.as_deref(), &menu_of(&section.items));
        }
        *self.inner.filters.borrow_mut() = filter_names(sections);
        self.button.set_menu_model(Some(&menu));
        self.place_customs();
        self.inner.refresh_dot();
    }

    /// Gives the custom slot `id` the widget `build` makes, now and each time the menu is rebuilt.
    pub fn set_custom(&self, id: &'static str, build: impl Fn() -> gtk::Widget + 'static) {
        self.inner.customs.borrow_mut().retain(|(slot, _)| *slot != id);
        self.inner.customs.borrow_mut().push((id, Rc::new(build)));
        self.place_customs();
    }

    /// Puts a fresh widget in every custom slot of the current popover.
    fn place_customs(&self) {
        let Some(popover) = self.button.popover().and_downcast::<gtk::PopoverMenu>() else { return };
        for (id, build) in self.inner.customs.borrow().iter() {
            popover.add_child(&build(), id);
        }
    }

    /// Creates the actions the items name and that do not exist yet.
    fn ensure(&self, items: &[Item]) {
        for item in items {
            let (name, action) = match item {
                Item::Submenu { items, .. } => {
                    self.ensure(items);
                    continue;
                }
                Item::Custom { .. } => continue,
                Item::Toggle { name, .. } | Item::Radio { name, .. } | Item::Action { name, .. } if self.inner.actions.borrow().contains_key(name) => continue,
                Item::Toggle { name, .. } => (*name, self.toggle_action(*name)),
                Item::Radio { name, .. } => (*name, self.radio_action(*name)),
                Item::Action { name, .. } => (*name, self.plain_action(*name)),
            };
            self.group.add_action(&action);
            self.inner.actions.borrow_mut().insert(name, action);
        }
    }

    fn toggle_action(&self, name: &'static str) -> gio::SimpleAction {
        let action = gio::SimpleAction::new_stateful(name, None, &false.to_variant());
        let weak = Rc::downgrade(&self.inner);
        action.connect_activate(move |action, _| {
            let Some(inner) = weak.upgrade() else { return };
            let on = !bool_state(action);
            action.set_state(&on.to_variant());
            inner.refresh_dot();
            inner.emit(Pick::Toggle(name, on));
        });
        action
    }

    fn radio_action(&self, name: &'static str) -> gio::SimpleAction {
        let action = gio::SimpleAction::new_stateful(name, Some(glib::VariantTy::STRING), &"".to_variant());
        let weak = Rc::downgrade(&self.inner);
        action.connect_activate(move |action, target| {
            let Some(inner) = weak.upgrade() else { return };
            let Some(value) = target.and_then(|t| t.get::<String>()) else { return };
            action.set_state(&value.to_variant());
            inner.emit(Pick::Radio(name, value));
        });
        action
    }

    fn plain_action(&self, name: &'static str) -> gio::SimpleAction {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(&self.inner);
        action.connect_activate(move |_, _| {
            if let Some(inner) = weak.upgrade() {
                inner.emit(Pick::Action(name));
            }
        });
        action
    }

    fn action(&self, name: &str) -> Option<gio::SimpleAction> {
        self.inner.actions.borrow().get(name).cloned()
    }

    /// Shows a stored toggle state (no pick is reported).
    pub fn set_toggle(&self, name: &str, on: bool) {
        if let Some(action) = self.action(name) {
            action.set_state(&on.to_variant());
        }
        self.inner.refresh_dot();
    }

    /// Shows the stored value of a radio group; a value no radio has leaves every radio of the group off.
    pub fn set_radio(&self, name: &str, value: &str) {
        if let Some(action) = self.action(name) {
            action.set_state(&value.to_variant());
        }
    }

    /// Lights the dot for a filter the menu has no toggle for (it opens elsewhere, such as the scanlator list).
    pub fn set_extra_filter(&self, on: bool) {
        self.inner.extra.set(on);
        self.inner.refresh_dot();
    }

    /// Greys an item out (or back in). A disabled toggle still shows its state.
    pub fn set_enabled(&self, name: &str, enabled: bool) {
        if let Some(action) = self.action(name) {
            action.set_enabled(enabled);
        }
    }

    #[cfg(test)]
    pub fn toggle(&self, name: &str) -> bool {
        self.action(name).is_some_and(|a| bool_state(&a))
    }

    #[cfg(test)]
    pub fn radio(&self, name: &str) -> Option<String> {
        self.action(name).and_then(|a| a.state()).and_then(|v| v.get::<String>())
    }

    /// Opens the menu (a keyboard shortcut of the page).
    pub fn popup(&self) {
        self.button.popup();
    }

    #[cfg(test)]
    pub fn button(&self) -> &gtk::MenuButton {
        &self.button
    }

    #[cfg(test)]
    /// The action group, so a test can activate an item the way the menu does.
    pub fn group(&self) -> &gio::SimpleActionGroup {
        &self.group
    }

    #[cfg(test)]
    pub fn dot_visible(&self) -> bool {
        self.inner.dot.is_visible()
    }
}

/// Every label of a menu model, section titles and submenus included, for the page tests.
#[cfg(test)]
pub fn menu_labels(model: &gio::MenuModel) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..model.n_items() {
        if let Some(label) = model.item_attribute_value(i, "label", Some(glib::VariantTy::STRING)).and_then(|v| v.get::<String>()) {
            out.push(label);
        }
        for link in ["section", "submenu"] {
            if let Some(child) = model.item_link(i, link) {
                out.extend(menu_labels(&child));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn a_custom_slot_holds_the_owner_widget_and_keeps_it_when_the_menu_is_rebuilt() {
        crate::testing::with_display(|| {
            let slider_in = |menu: &ViewOptions| {
                let popover = menu.button().popover().and_downcast::<gtk::PopoverMenu>().expect("the popover");
                let mut stack = vec![popover.upcast::<gtk::Widget>()];
                while let Some(widget) = stack.pop() {
                    if widget.is::<gtk::Scale>() {
                        return true;
                    }
                    let mut child = widget.first_child();
                    while let Some(c) = child {
                        child = c.next_sibling();
                        stack.push(c);
                    }
                }
                false
            };
            let with_slot = || vec![Section::new("Cover size", vec![Item::custom("size")])];
            let menu = ViewOptions::new("View", &with_slot());
            menu.set_custom("size", || gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1).upcast());
            assert!(slider_in(&menu));
            menu.set_sections(&with_slot());
            assert!(slider_in(&menu), "a rebuilt menu gets a fresh slider");
        });
    }

    fn sections() -> Vec<Section> {
        vec![
            Section::filter("Show only", vec![Item::toggle("unread", "Unread"), Item::toggle("downloaded", "Downloaded")]),
            Section::new("Sort by", vec![Item::radio("sort", "number", "Number"), Item::radio("sort", "date", "Upload date"), Item::toggle("reverse", "Reverse order")]),
            Section::untitled(vec![Item::submenu("Items per row", vec![Item::radio("size", "0", "Auto")]), Item::action("default", "Use as default")]),
        ]
    }

    #[test]
    fn only_the_toggles_of_a_filter_section_light_the_dot() {
        assert_eq!(filter_names(&sections()), vec!["unread", "downloaded"]);
        let nested = vec![Section::filter("Show only", vec![Item::submenu("More", vec![Item::toggle("deep", "Deep")]), Item::action("x", "X")])];
        assert_eq!(filter_names(&nested), vec!["deep"]);
    }

    #[test]
    fn picks_report_and_set_the_state_and_the_dot_follows_the_filters() {
        crate::testing::with_display(|| {
            let options = ViewOptions::new("View options", &sections());
            let picks: Rc<RefCell<Vec<Pick>>> = Rc::default();
            options.connect({
                let picks = Rc::clone(&picks);
                move |pick| picks.borrow_mut().push(pick)
            });
            assert!(!options.dot_visible());
            options.group().activate_action("unread", None);
            assert!(options.toggle("unread") && options.dot_visible(), "a Show only toggle lights the dot");
            options.group().activate_action("reverse", None);
            options.group().activate_action("sort", Some(&"date".to_variant()));
            options.group().activate_action("default", None);
            assert_eq!(
                *picks.borrow(),
                vec![Pick::Toggle("unread", true), Pick::Toggle("reverse", true), Pick::Radio("sort", "date".into()), Pick::Action("default")]
            );
            assert_eq!(options.radio("sort").as_deref(), Some("date"));
            options.set_toggle("unread", false);
            assert!(!options.dot_visible(), "reverse is not a filter");
            // the stored state is shown without a pick, and survives a rebuilt menu
            options.set_toggle("downloaded", true);
            options.set_enabled("downloaded", false);
            options.set_sections(&sections());
            assert!(options.toggle("downloaded") && options.dot_visible());
            assert_eq!(picks.borrow().len(), 4, "a stored state reports nothing");
            options.set_radio("size", "0");
            assert_eq!(options.radio("size").as_deref(), Some("0"));
            options.set_toggle("downloaded", false);
            options.set_extra_filter(true);
            assert!(options.dot_visible(), "a filter outside the menu lights the dot too");
            let labels = menu_labels(&options.button().menu_model().expect("a menu"));
            for want in ["Show only", "Unread", "Sort by", "Upload date", "Items per row", "Auto", "Use as default"] {
                assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
            }
        });
    }
}
