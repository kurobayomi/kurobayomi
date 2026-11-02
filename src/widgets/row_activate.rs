//! One callback for a `gtk::ListBoxRow`, fired by the mouse and by the keyboard.
//!
//! A mouse click on a row emits `row-activated` on the parent `gtk::ListBox` and never the row's own `activate`
//! signal. `activate` only comes from Enter or Space, and its class handler ends in the same `row-activated`.
//! So `row.connect_activate(..)` reacts to the keyboard but not to a click. [`on_activate`] listens on the parent
//! list instead, which covers both paths with one call per activation. A row that moves to another list is
//! followed, and a row that is not activatable stays silent.

use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

type Callback = Rc<dyn Fn(&gtk::ListBoxRow)>;
type Link = Rc<RefCell<Option<(glib::WeakRef<gtk::ListBox>, glib::SignalHandlerId)>>>;

/// Runs `f` once each time `row` is activated by a click, Enter or Space.
pub fn on_activate(row: &impl IsA<gtk::ListBoxRow>, f: impl Fn(&gtk::ListBoxRow) + 'static) {
    let row = row.upcast_ref::<gtk::ListBoxRow>();
    let f: Callback = Rc::new(f);
    let link: Link = Rc::default();
    attach(row, &f, &link);
    row.connect_parent_notify(move |row| attach(row, &f, &link));
}

/// Moves the listener from the old parent list (if any) to the current one (if any).
fn attach(row: &gtk::ListBoxRow, f: &Callback, link: &Link) {
    let old = link.borrow_mut().take();
    if let Some((list, id)) = old
        && let Some(list) = list.upgrade()
    {
        list.disconnect(id);
    }
    let Some(list) = row.parent().and_downcast::<gtk::ListBox>() else { return };
    let (mine, f) = (row.downgrade(), Rc::clone(f));
    let id = list.connect_row_activated(move |_, activated| {
        if activated.is_activatable() && mine.upgrade().as_ref() == Some(activated) {
            f(activated);
        }
    });
    *link.borrow_mut() = Some((list.downgrade(), id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn counted(row: &gtk::ListBoxRow) -> Rc<Cell<u32>> {
        let n = Rc::new(Cell::new(0));
        let seen = Rc::clone(&n);
        on_activate(row, move |_| seen.set(seen.get() + 1));
        n
    }

    /// What a mouse click does: GTK emits `row-activated` on the list.
    fn click(list: &gtk::ListBox, row: &gtk::ListBoxRow) {
        list.emit_by_name::<()>("row-activated", &[row]);
    }

    fn list_with(rows: &[&gtk::ListBoxRow]) -> gtk::ListBox {
        let list = gtk::ListBox::new();
        for r in rows {
            list.append(*r);
        }
        list
    }

    #[test]
    fn a_click_fires_once() {
        crate::testing::with_display(|| {
            let (a, b) = (gtk::ListBoxRow::new(), gtk::ListBoxRow::new());
            let (na, nb) = (counted(&a), counted(&b));
            let list = list_with(&[&a, &b]);
            click(&list, &a);
            assert_eq!((na.get(), nb.get()), (1, 0), "only the clicked row reacts, once");
            click(&list, &b);
            assert_eq!((na.get(), nb.get()), (1, 1));
        });
    }

    #[test]
    fn enter_or_space_fires_once() {
        crate::testing::with_display(|| {
            let row = gtk::ListBoxRow::new();
            let n = counted(&row);
            let _list = list_with(&[&row]);
            row.emit_by_name::<()>("activate", &[]);
            assert_eq!(n.get(), 1, "the row activate ends in row-activated; no second call");
        });
    }

    #[test]
    fn a_row_that_is_not_activatable_stays_silent() {
        crate::testing::with_display(|| {
            let row = gtk::ListBoxRow::new();
            let n = counted(&row);
            let list = list_with(&[&row]);
            row.set_activatable(false);
            click(&list, &row);
            row.emit_by_name::<()>("activate", &[]);
            assert_eq!(n.get(), 0);
        });
    }

    #[test]
    fn a_row_that_moves_to_another_list_follows_and_leaves_the_old_one() {
        crate::testing::with_display(|| {
            let row = gtk::ListBoxRow::new();
            let n = counted(&row);
            let first = list_with(&[&row]);
            first.remove(&row);
            click(&first, &row);
            assert_eq!(n.get(), 0, "a removed row no longer hears its old list");
            let second = list_with(&[&row]);
            click(&second, &row);
            assert_eq!(n.get(), 1);
            row.emit_by_name::<()>("activate", &[]);
            assert_eq!(n.get(), 2);
        });
    }

    #[test]
    fn two_callbacks_on_one_row_both_fire_once() {
        crate::testing::with_display(|| {
            let row = gtk::ListBoxRow::new();
            let (one, two) = (counted(&row), counted(&row));
            let list = list_with(&[&row]);
            click(&list, &row);
            assert_eq!((one.get(), two.get()), (1, 1));
        });
    }
}

/// Test helper for the screens: the same call GTK makes for a mouse click on `row`.
#[cfg(test)]
pub fn click_row(row: &impl IsA<gtk::ListBoxRow>) {
    let row = row.upcast_ref::<gtk::ListBoxRow>();
    let list = row.parent().and_downcast::<gtk::ListBox>().expect("the row is in a list");
    list.emit_by_name::<()>("row-activated", &[row]);
}

/// Test helper: the first row under `root` that shows a label reading `title`.
#[cfg(test)]
pub fn row_titled(root: &impl IsA<gtk::Widget>, title: &str) -> gtk::ListBoxRow {
    fn find(w: &gtk::Widget, keep: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
        if keep(w) {
            return Some(w.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if let Some(hit) = find(&c, keep) {
                return Some(hit);
            }
            child = c.next_sibling();
        }
        None
    }
    let shows_title = |w: &gtk::Widget| w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == title);
    find(root.upcast_ref(), &|w| w.is::<gtk::ListBoxRow>() && find(w, &shows_title).is_some())
        .and_downcast()
        .unwrap_or_else(|| panic!("no row titled {title}"))
}

#[cfg(test)]
mod site_tests {
    use super::*;
    use std::cell::Cell;

    fn keyboard(row: &impl IsA<gtk::ListBoxRow>) {
        row.emit_by_name::<()>("activate", &[]);
    }

    /// A settings row (the shared preference row behind every settings page).
    #[test]
    fn a_settings_row_runs_its_handler_once_per_click_and_once_per_key() {
        crate::testing::with_display(|| {
            let n = std::rc::Rc::new(Cell::new(0));
            let seen = std::rc::Rc::clone(&n);
            let common = crate::pref_model::model::ItemCommon::new("Reader");
            let row = crate::pref_model::widgets::text::build(&common, Some(std::rc::Rc::new(move || seen.set(seen.get() + 1))), None);
            let list = gtk::ListBox::new();
            list.append(&row);
            click_row(&row);
            assert_eq!(n.get(), 1);
            keyboard(&row);
            assert_eq!(n.get(), 2);
        });
    }

    #[test]
    fn a_history_row_resumes_once_per_click_and_once_per_key() {
        crate::testing::with_display(|| {
            use crate::updates::history_row::{RowActions, history_row};
            let n = std::rc::Rc::new(Cell::new(0));
            let seen = std::rc::Rc::clone(&n);
            let none: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(|| {});
            let actions = RowActions { open_entry: none.clone(), resume: std::rc::Rc::new(move || seen.set(seen.get() + 1)), remove: none.clone(), add_to_library: none };
            let row = history_row("Title", "Ch. 1", &gtk::Picture::new(), true, actions);
            let list = gtk::ListBox::new();
            list.append(&row);
            click_row(&row);
            assert_eq!(n.get(), 1);
            keyboard(&row);
            assert_eq!(n.get(), 2);
        });
    }

    /// A Browse row (sources, migrate, extensions, migrate entries all go through `browse::view::list_row`).
    #[test]
    fn a_browse_source_row_opens_once_per_click_and_once_per_key() {
        crate::testing::with_display(|| {
            let n = std::rc::Rc::new(Cell::new(0));
            let (seen, menu) = (std::rc::Rc::clone(&n), std::rc::Rc::new(Cell::new(0)));
            let long = std::rc::Rc::clone(&menu);
            let icon = crate::browse::icons::resolve(false, false, false);
            let child = crate::browse::view::source_row("Source", None, icon, None, false, None);
            let row = crate::browse::view::list_row(&child, move || seen.set(seen.get() + 1), move || {
                long.set(long.get() + 1);
                Vec::new()
            });
            let list = gtk::ListBox::new();
            list.append(&row);
            click_row(&row);
            assert_eq!(n.get(), 1);
            keyboard(&row);
            assert_eq!((n.get(), menu.get()), (2, 0), "activation never opens the menu");
        });
    }

    /// Receivers whose `connect_activate` is not a `gtk::ListBoxRow` (apps, actions, entries, buttons, list views).
    const NOT_ROWS: &[&str] = &["app", "action", "add", "rescan", "off", "entry", "field", "search", "button", "inner.grid", "inner.list", "this.widget"];

    /// A click never reaches `ListBoxRow::connect_activate`, so rows must go through [`on_activate`]. A new
    /// `connect_activate` on anything else is fine: add its receiver name to `NOT_ROWS`.
    #[test]
    fn no_row_uses_connect_activate() {
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("widgets/row_activate.rs") {
                    for (i, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                        let Some(at) = line.find(".connect_activate(") else { continue };
                        let receiver = line[..at].trim_start();
                        if !NOT_ROWS.contains(&receiver) {
                            out.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                        }
                    }
                }
            }
        }
        let mut found = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
        assert!(found.is_empty(), "use crate::widgets::row_activate::on_activate for rows:\n{}", found.join("\n"));
    }
}
