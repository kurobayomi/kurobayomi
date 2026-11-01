//! The source preference page: rows built from the list a configurable source declares (switch, list, multi choice,
//! text, and categories that nest rows), a spinner until the source registry has loaded, and every value written to the
//! source's own preference store, `source_<id>.<key>` (T-40a-023). A source that declares no rows is not configurable and
//! the page says `empty_screen`. Ports `MangaSourcePreferencesScreen`; the rules of what a row shows are pure.
//!
//! The descriptor (`kuro_source::SourcePref`) has no info-text or action-row variant yet, so those two rows of the
//! plan's list cannot be drawn until it has; a row's `summary` is its only explanatory text.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_domain::source::settings::SourcePrefs;
use kuro_i18n::{MsgKey, tr};
use kuro_source::SourcePref;
use tokio::sync::watch;

use super::dialogs::{ButtonStyle, DialogSlot, DialogSpec};
use super::glue::{follow_ready, keep_alive};

/// The entry a stored list value stands for. A value that matches none shows nothing.
pub fn list_label(entries: &[String], values: &[String], stored: &str) -> Option<String> {
    values.iter().position(|v| v == stored).and_then(|i| entries.get(i).cloned())
}

/// The entries of the selected values, in the order the source declared them, joined for the row's summary.
pub fn multi_label(entries: &[String], values: &[String], selected: &BTreeSet<String>) -> String {
    values.iter().zip(entries).filter(|(v, _)| selected.contains(*v)).map(|(_, e)| e.as_str()).collect::<Vec<_>>().join(", ")
}

/// `selected` with `value` in or out.
pub fn with_member(selected: &BTreeSet<String>, value: &str, on: bool) -> BTreeSet<String> {
    let mut next = selected.clone();
    if on {
        next.insert(value.to_owned());
    } else {
        next.remove(value);
    }
    next
}

/// The title of a dialog row's dialog: the row's own title. The descriptors have no separate dialog title, so the
/// fallback rule of the Android screen (title when no dialog title is set) always applies.
pub fn dialog_title(title: &str) -> String {
    title.to_owned()
}

/// What the text field of an edit-text row asks of the input method: no spell check and a private input, exactly
/// `NO_SPELLCHECK | PRIVATE` (T-41a-023, D6). GTK keeps no history for an entry, so the incognito input of the Android
/// screen needs nothing more (U§8.3).
pub fn edit_text_hints() -> gtk::InputHints {
    gtk::InputHints::NO_SPELLCHECK | gtk::InputHints::PRIVATE
}

/// The second line of a row: the declared summary, else what the row currently holds.
pub fn row_summary(pref: &SourcePref, prefs: &SourcePrefs) -> Option<String> {
    match pref {
        SourcePref::Switch { summary, .. } => summary.clone(),
        SourcePref::List { summary, entries, entry_values, default, key, .. } => summary
            .clone()
            .or_else(|| list_label(entries, entry_values, &prefs.get_string(key).unwrap_or_else(|| default.clone()))),
        SourcePref::MultiSelect { summary, entries, entry_values, default, key, .. } => summary.clone().or_else(|| {
            let selected = prefs.get_string_set(key).unwrap_or_else(|| default.iter().cloned().collect());
            Some(multi_label(entries, entry_values, &selected)).filter(|s| !s.is_empty())
        }),
        SourcePref::EditText { summary, default, key, .. } => {
            summary.clone().or_else(|| Some(prefs.get_string(key).unwrap_or_else(|| default.clone())).filter(|s| !s.is_empty()))
        }
        SourcePref::Category { .. } => None,
    }
}

struct Inner {
    prefs: SourcePrefs,
    /// `ConfigurableSource::on_settings_changed`, called with the key of each row that was written.
    on_changed: Rc<dyn Fn(&str)>,
    stack: gtk::Stack,
    column: gtk::Box,
    dialogs: DialogSlot,
    rows: RefCell<Vec<SourcePref>>,
    ready: RefCell<bool>,
}

/// Cloning shares the page; the widget keeps it alive.
#[derive(Clone)]
pub struct SourcePrefsPage {
    inner: Rc<Inner>,
    root: gtk::Box,
}

impl SourcePrefsPage {
    /// `ready` is `SourceRegistry::subscribe_initialized()`: the page shows a spinner until it is true, and fills in
    /// when it turns true. `prefs` is the store of the source; every row writes into it.
    pub fn new(prefs: SourcePrefs, on_changed: Rc<dyn Fn(&str)>, ready: watch::Receiver<bool>) -> Self {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroller = gtk::ScrolledWindow::builder().child(&column).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let stack = gtk::Stack::new();
        stack.add_named(&crate::widgets::status::loading(None), Some("loading"));
        stack.add_named(&scroller, Some("list"));
        stack.add_named(&crate::widgets::status::empty("view-list-symbolic", &tr(MsgKey::empty_screen), None), Some("empty"));
        stack.set_vexpand(true);
        stack.set_visible_child_name("loading");
        let inner = Rc::new(Inner {
            prefs,
            on_changed,
            stack,
            column,
            dialogs: DialogSlot::new(),
            rows: RefCell::new(Vec::new()),
            ready: RefCell::new(false),
        });
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&inner.stack);
        keep_alive(&root, &inner);
        let weak = Rc::downgrade(&inner);
        follow_ready(ready, move |ready| {
            let Some(inner) = weak.upgrade() else { return false };
            *inner.ready.borrow_mut() = ready;
            inner.render();
            true
        });
        Self { inner, root }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The rows the source declares (`ConfigurableSource::preferences`).
    pub fn set_rows(&self, rows: Vec<SourcePref>) {
        *self.inner.rows.borrow_mut() = rows;
        self.inner.render();
    }
}

impl Inner {
    fn render(self: &Rc<Self>) {
        if !*self.ready.borrow() {
            self.stack.set_visible_child_name("loading");
            return;
        }
        while let Some(child) = self.column.first_child() {
            self.column.remove(&child);
        }
        let rows = self.rows.borrow().clone();
        if rows.is_empty() {
            self.stack.set_visible_child_name("empty");
            return;
        }
        for row in &rows {
            self.add_row(&self.column, row);
        }
        self.stack.set_visible_child_name("list");
    }

    fn add_row(self: &Rc<Self>, into: &gtk::Box, pref: &SourcePref) {
        match pref {
            SourcePref::Category { title, children } => {
                into.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).margin_start(16).margin_top(16).margin_bottom(4).build());
                for child in children {
                    self.add_row(into, child);
                }
            }
            SourcePref::Switch { key, title, default, .. } => {
                let (row, summary) = self.row(title, pref);
                let switch = gtk::Switch::builder().active(self.prefs.get_bool(key).unwrap_or(*default)).valign(gtk::Align::Center).build();
                let (weak, key) = (Rc::downgrade(self), key.clone());
                switch.connect_state_set(move |switch, on| {
                    if let Some(inner) = weak.upgrade() {
                        inner.prefs.set_bool(&key, on);
                        (inner.on_changed)(&key);
                    }
                    switch.set_state(on);
                    gtk::glib::Propagation::Stop
                });
                row.append(&switch);
                drop(summary);
                into.append(&row);
            }
            SourcePref::List { .. } | SourcePref::MultiSelect { .. } | SourcePref::EditText { .. } => {
                let title = match pref {
                    SourcePref::List { title, .. } | SourcePref::MultiSelect { title, .. } | SourcePref::EditText { title, .. } => title.clone(),
                    _ => String::new(),
                };
                let (row, summary) = self.row(&title, pref);
                let (weak, pref, summary) = (Rc::downgrade(self), pref.clone(), summary);
                let click = gtk::GestureClick::new();
                click.connect_released(move |_, _, _, _| {
                    if let Some(inner) = weak.upgrade() {
                        inner.open_dialog(&pref, &summary);
                    }
                });
                row.add_controller(click);
                into.append(&row);
            }
        }
    }

    /// A row: title over summary, with room at the end for the control. Returns the row and its summary label.
    fn row(&self, title: &str, pref: &SourcePref) -> (gtk::Box, gtk::Label) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        row.set_margin_start(16);
        row.set_margin_end(16);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&gtk::Label::builder().label(title).xalign(0.0).build());
        let summary = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
        let line = row_summary(pref, &self.prefs);
        summary.set_visible(line.is_some());
        summary.set_label(line.as_deref().unwrap_or(""));
        text.append(&summary);
        row.append(&text);
        (row, summary)
    }

    fn written(&self, key: &str) {
        (self.on_changed)(key);
    }

    fn open_dialog(self: &Rc<Self>, pref: &SourcePref, summary: &gtk::Label) {
        let cancel = (tr(MsgKey::action_cancel), ButtonStyle::Plain);
        let ok = (tr(MsgKey::action_ok), ButtonStyle::Suggested);
        match pref {
            SourcePref::List { key, title, entries, entry_values, default, .. } => {
                let stored = self.prefs.get_string(key).unwrap_or_else(|| default.clone());
                let choices = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let mut group: Option<gtk::CheckButton> = None;
                let radios: Rc<Vec<(String, gtk::CheckButton)>> = Rc::new(
                    entries
                        .iter()
                        .zip(entry_values)
                        .map(|(entry, value)| {
                            let button = gtk::CheckButton::with_label(entry);
                            button.set_group(group.as_ref());
                            button.set_active(*value == stored);
                            group.get_or_insert(button.clone());
                            choices.append(&button);
                            (value.clone(), button)
                        })
                        .collect(),
                );
                // choosing an entry only moves the selection; OK writes it, Cancel and closing write nothing
                let (weak, key, pref, summary) = (Rc::downgrade(self), key.clone(), pref.clone(), summary.clone());
                let spec = DialogSpec { title: dialog_title(title), body: None, buttons: vec![ok, cancel] };
                self.dialogs.show_with(&self.column, spec, Some(choices.upcast()), move |chosen| {
                    let (Some(0), Some(inner)) = (chosen, weak.upgrade()) else { return };
                    let Some((value, _)) = radios.iter().find(|(_, button)| button.is_active()) else { return };
                    inner.prefs.set_string(&key, value.clone());
                    inner.written(&key);
                    let line = row_summary(&pref, &inner.prefs);
                    summary.set_visible(line.is_some());
                    summary.set_label(line.as_deref().unwrap_or(""));
                });
            }
            SourcePref::MultiSelect { key, title, entries, entry_values, default, .. } => {
                let selected = self.prefs.get_string_set(key).unwrap_or_else(|| default.iter().cloned().collect());
                let choices = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let boxes: Rc<Vec<(String, gtk::CheckButton)>> = Rc::new(
                    entries
                        .iter()
                        .zip(entry_values)
                        .map(|(entry, value)| {
                            let button = gtk::CheckButton::with_label(entry);
                            button.set_active(selected.contains(value));
                            choices.append(&button);
                            (value.clone(), button)
                        })
                        .collect(),
                );
                let (weak, key, pref, summary) = (Rc::downgrade(self), key.clone(), pref.clone(), summary.clone());
                let spec = DialogSpec { title: dialog_title(title), body: None, buttons: vec![ok, cancel] };
                self.dialogs.show_with(&self.column, spec, Some(choices.upcast()), move |chosen| {
                    let (Some(0), Some(inner)) = (chosen, weak.upgrade()) else { return };
                    let next = boxes.iter().fold(BTreeSet::new(), |set, (value, button)| with_member(&set, value, button.is_active()));
                    inner.prefs.set_string_set(&key, next);
                    inner.written(&key);
                    let line = row_summary(&pref, &inner.prefs);
                    summary.set_visible(line.is_some());
                    summary.set_label(line.as_deref().unwrap_or(""));
                });
            }
            SourcePref::EditText { key, title, default, .. } => {
                let entry = gtk::Entry::new();
                entry.set_text(&self.prefs.get_string(key).unwrap_or_else(|| default.clone()));
                entry.set_input_hints(edit_text_hints());
                entry.set_activates_default(true);
                let field = entry.clone();
                let (weak, key, pref, summary) = (Rc::downgrade(self), key.clone(), pref.clone(), summary.clone());
                let spec = DialogSpec { title: dialog_title(title), body: None, buttons: vec![ok, cancel] };
                self.dialogs.show_with(&self.column, spec, Some(entry.upcast()), move |chosen| {
                    let (Some(0), Some(inner)) = (chosen, weak.upgrade()) else { return };
                    inner.prefs.set_string(&key, field.text().to_string());
                    inner.written(&key);
                    let line = row_summary(&pref, &inner.prefs);
                    summary.set_visible(line.is_some());
                    summary.set_label(line.as_deref().unwrap_or(""));
                });
            }
            SourcePref::Switch { .. } | SourcePref::Category { .. } => {}
        }
    }
}

#[cfg(test)]
mod pure_tests {
    use kuro_prefs::store::PrefStore;

    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_list_value_shows_its_entry_and_an_unknown_one_nothing() {
        let (entries, values) = (strings(&["Low", "High"]), strings(&["l", "h"]));
        assert_eq!(list_label(&entries, &values, "h").as_deref(), Some("High"));
        assert_eq!(list_label(&entries, &values, "x"), None);
    }

    #[test]
    fn a_multi_choice_lists_its_selected_entries_in_declared_order() {
        let (entries, values) = (strings(&["One", "Two", "Three"]), strings(&["1", "2", "3"]));
        let selected: BTreeSet<String> = strings(&["3", "1"]).into_iter().collect();
        assert_eq!(multi_label(&entries, &values, &selected), "One, Three");
        assert_eq!(multi_label(&entries, &values, &BTreeSet::new()), "");
        assert_eq!(with_member(&selected, "2", true).len(), 3);
        assert_eq!(with_member(&selected, "1", false).len(), 1);
    }

    #[test]
    fn a_dialog_takes_the_title_of_its_row_and_text_asks_for_no_spellcheck_and_a_private_input() {
        assert_eq!(dialog_title("Quality"), "Quality");
        assert_eq!(edit_text_hints(), gtk::InputHints::NO_SPELLCHECK | gtk::InputHints::PRIVATE, "T-41a-023 lists exactly these two");
    }

    #[test]
    fn the_summary_is_the_declared_one_else_the_current_value() {
        let prefs = SourcePrefs::new(PrefStore::in_memory(), 7);
        let list = SourcePref::List {
            key: "q".into(),
            title: "Q".into(),
            summary: None,
            entries: strings(&["Low", "High"]),
            entry_values: strings(&["l", "h"]),
            default: "l".into(),
        };
        assert_eq!(row_summary(&list, &prefs).as_deref(), Some("Low"), "the default shows until a value is stored");
        prefs.set_string("q", "h");
        assert_eq!(row_summary(&list, &prefs).as_deref(), Some("High"));
        let declared = SourcePref::Switch { key: "s".into(), title: "S".into(), summary: Some("Because".into()), default: false };
        assert_eq!(row_summary(&declared, &prefs).as_deref(), Some("Because"));
        let empty = SourcePref::EditText { key: "t".into(), title: "T".into(), summary: None, default: String::new() };
        assert_eq!(row_summary(&empty, &prefs), None);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use kuro_prefs::store::PrefStore;

    use super::*;
    use adw::prelude::*;

    fn declared() -> Vec<SourcePref> {
        let strings = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        vec![
            SourcePref::Category {
                title: "Images".into(),
                children: vec![
                    SourcePref::Switch { key: "hd".into(), title: "High quality".into(), summary: None, default: true },
                    SourcePref::List {
                        key: "size".into(),
                        title: "Size".into(),
                        summary: None,
                        entries: strings(&["Small", "Large"]),
                        entry_values: strings(&["s", "l"]),
                        default: "s".into(),
                    },
                ],
            },
            SourcePref::MultiSelect {
                key: "langs".into(),
                title: "Languages".into(),
                summary: None,
                entries: strings(&["English", "French"]),
                entry_values: strings(&["en", "fr"]),
                default: strings(&["en"]),
            },
            SourcePref::EditText { key: "host".into(), title: "Host".into(), summary: None, default: "example.org".into() },
        ]
    }

    fn all<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Vec<T> {
        fn walk<T: IsA<gtk::Widget>>(widget: &gtk::Widget, out: &mut Vec<T>) {
            if let Some(found) = widget.downcast_ref::<T>() {
                out.push(found.clone());
            }
            let mut child = widget.first_child();
            while let Some(w) = child {
                walk(&w, out);
                child = w.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(widget.upcast_ref(), &mut out);
        out
    }

    fn click(widget: &impl IsA<gtk::Widget>) {
        let list = widget.observe_controllers();
        let gesture = (0..list.n_items()).filter_map(|i| list.item(i)).find_map(|c| c.downcast::<gtk::GestureClick>().ok()).expect("click");
        gesture.emit_by_name::<()>("released", &[&1_i32, &0.0_f64, &0.0_f64]);
    }

    /// The row boxes of the page, in order: those with a click controller or a switch.
    fn top_rows(page: &SourcePrefsPage) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut child = page.inner.column.first_child();
        while let Some(w) = child {
            child = w.next_sibling();
            if w.is::<gtk::Box>() {
                out.push(w);
            }
        }
        out
    }

    fn a_spinner_shows_until_the_registry_loads_then_the_rows_appear() {
        let (tx, rx) = watch::channel(false);
        let page = SourcePrefsPage::new(SourcePrefs::new(PrefStore::in_memory(), 7), Rc::new(|_| {}), rx);
        page.set_rows(declared());
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("loading"), "the registry is not ready");
        tx.send(true).expect("ready");
        let context = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while page.inner.stack.visible_child_name().as_deref() != Some("list") && start.elapsed() < std::time::Duration::from_secs(3) {
            while context.iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("list"));
        assert_eq!(top_rows(&page).len(), 4, "the category's heading is a label; its two rows, the multi and the text follow");
    }

    fn every_row_type_writes_into_the_sources_own_store_and_tells_the_source() {
        let store = PrefStore::in_memory();
        let prefs = SourcePrefs::new(store.clone(), 7);
        let changed: Rc<RefCell<Vec<String>>> = Rc::default();
        let log = Rc::clone(&changed);
        let page = SourcePrefsPage::new(prefs.clone(), Rc::new(move |key| log.borrow_mut().push(key.to_owned())), watch::channel(true).1);
        page.set_rows(declared());
        let rows = top_rows(&page);
        assert_eq!(rows.len(), 4);

        // switch
        let switch = all::<gtk::Switch>(&rows[0]).remove(0);
        assert!(switch.is_active(), "the declared default shows until a value is stored");
        switch.emit_by_name::<bool>("state-set", &[&false]);
        assert_eq!(prefs.get_bool("hd"), Some(false));
        assert!(store.get_raw("source_7.hd").is_some(), "stored under the source's namespace");

        // list: choosing an entry only selects it; Cancel writes nothing, OK writes it
        click(&rows[1]);
        let window = page.inner.dialogs.current().expect("the list dialog");
        assert_eq!(window.title(), "Size", "the dialog title is the row title");
        let radios = all::<gtk::CheckButton>(&crate::ui::dialog_content(&window).expect("child"));
        assert_eq!(radios.len(), 2);
        assert_eq!(crate::browse::dialogs::buttons_of(&window).len(), 2, "OK and Cancel");
        radios[1].set_active(true);
        assert_eq!(prefs.get_string("size"), None, "choosing alone writes nothing");
        assert!(page.inner.dialogs.current().is_some(), "and the dialog stays open");
        crate::browse::dialogs::buttons_of(&window)[1].emit_clicked();
        assert_eq!(prefs.get_string("size"), None, "Cancel writes nothing");
        assert!(page.inner.dialogs.current().is_none());
        click(&rows[1]);
        let window = page.inner.dialogs.current().expect("the list dialog");
        all::<gtk::CheckButton>(&crate::ui::dialog_content(&window).expect("child"))[1].set_active(true);
        crate::browse::dialogs::buttons_of(&window)[0].emit_clicked();
        assert_eq!(prefs.get_string("size").as_deref(), Some("l"));
        assert!(page.inner.dialogs.current().is_none());
        assert!(all::<gtk::Label>(&rows[1]).iter().any(|l| l.label().as_str() == "Large"), "the summary follows the value");

        // multi choice: OK writes the set
        click(&rows[2]);
        let window = page.inner.dialogs.current().expect("the multi dialog");
        let checks = all::<gtk::CheckButton>(&crate::ui::dialog_content(&window).expect("child"));
        assert!(checks[0].is_active() && !checks[1].is_active(), "the declared default is preselected");
        checks[1].set_active(true);
        checks[0].set_active(false);
        crate::browse::dialogs::buttons_of(&window)[0].emit_clicked();
        assert_eq!(prefs.get_string_set("langs"), Some(["fr".to_owned()].into()));

        // text: the field has the input hints, Cancel writes nothing, OK writes the text
        click(&rows[3]);
        let window = page.inner.dialogs.current().expect("the text dialog");
        let entry = all::<gtk::Entry>(&crate::ui::dialog_content(&window).expect("child")).remove(0);
        assert_eq!(entry.text().as_str(), "example.org");
        assert_eq!(entry.input_hints(), gtk::InputHints::NO_SPELLCHECK | gtk::InputHints::PRIVATE);
        entry.set_text("mirror.test");
        crate::browse::dialogs::buttons_of(&window)[1].emit_clicked();
        assert_eq!(prefs.get_string("host"), None, "Cancel stores nothing");
        click(&rows[3]);
        let window = page.inner.dialogs.current().expect("dialog");
        all::<gtk::Entry>(&crate::ui::dialog_content(&window).expect("child")).remove(0).set_text("mirror.test");
        crate::browse::dialogs::buttons_of(&window)[0].emit_clicked();
        assert_eq!(prefs.get_string("host").as_deref(), Some("mirror.test"));

        assert_eq!(*changed.borrow(), ["hd", "size", "langs", "host"], "the source hears each written key once");
    }

    fn a_source_with_no_rows_is_not_configurable_and_the_page_says_so() {
        let page = SourcePrefsPage::new(SourcePrefs::new(PrefStore::in_memory(), 7), Rc::new(|_| {}), watch::channel(true).1);
        page.set_rows(Vec::new());
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("empty"));
        let label = all::<gtk::Label>(page.inner.stack.visible_child().as_ref().expect("child")).remove(0);
        assert_eq!(label.label().as_str(), tr(MsgKey::empty_screen), "Well, this is awkward");
        page.set_rows(declared());
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("list"), "rows replace the empty state");
    }

    pub(crate) fn smoke() {
        a_source_with_no_rows_is_not_configurable_and_the_page_says_so();
        a_spinner_shows_until_the_registry_loads_then_the_rows_appear();
        every_row_type_writes_into_the_sources_own_store_and_tells_the_source();
    }
}
