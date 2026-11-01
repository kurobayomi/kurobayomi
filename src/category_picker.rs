//! The one "Set categories" dialog of the app: the user's categories as a boxed list, one check per row, then Edit
//! categories, Cancel and OK. Every screen opens it here (the entry page, Library's selection, Browse and History) over
//! its own rules: a plain on/off list ([`crate::entry_core::dialog::CategoryDialog`]) or the tri-state of a selection
//! ([`crate::library::category_dialog::CategoryDialog`]).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use kuro_i18n::{MsgKey, tr};

use crate::ui::{self, DialogOpen as _};

/// The rules behind the rows: their names, how each looks, and what a click does.
pub trait Pick {
    fn names(&self) -> Vec<String>;
    /// `(checked, mixed)` of row `index`.
    fn state(&self, index: usize) -> (bool, bool);
    fn click(&mut self, index: usize);
}

impl Pick for crate::entry_core::dialog::CategoryDialog {
    fn names(&self) -> Vec<String> {
        self.items.iter().map(|i| i.name.clone()).collect()
    }
    fn state(&self, index: usize) -> (bool, bool) {
        (self.items.get(index).is_some_and(|i| i.checked), false)
    }
    fn click(&mut self, index: usize) {
        self.toggle(index);
    }
}

impl Pick for crate::library::category_dialog::CategoryDialog {
    fn names(&self) -> Vec<String> {
        self.rows().iter().map(|(c, _)| c.name.clone()).collect()
    }
    fn state(&self, index: usize) -> (bool, bool) {
        use kuro_domain::category::bulk::Choice;
        match self.rows().get(index).map(|(_, choice)| *choice) {
            Some(Choice::Checked | Choice::Include) => (true, false),
            Some(Choice::Untouched) => (false, true),
            _ => (false, false),
        }
    }
    fn click(&mut self, index: usize) {
        if let Some(id) = self.rows().get(index).map(|(c, _)| c.id) {
            // the inherent click, by category id
            crate::library::category_dialog::CategoryDialog::click(self, id);
        }
    }
}

/// Opens the dialog over `model`. OK runs `on_ok` with the model. Edit categories closes the dialog and then runs
/// `on_edit`. Every way out other than OK (Escape, the close button, Cancel, Edit categories) runs `on_dismiss` and
/// writes nothing. The window is returned, so a page can keep it in its one dialog slot.
pub fn open<M: Pick + 'static>(
    model: M,
    on_ok: impl Fn(&M) + 'static,
    on_edit: impl Fn() + 'static,
    on_dismiss: impl Fn() + 'static,
) -> adw::Dialog {
    let names = model.names();
    let model = Rc::new(RefCell::new(model));
    let body: gtk::Widget = if names.is_empty() {
        ui::message(&tr(MsgKey::information_empty_category_dialog)).upcast()
    } else {
        let list = ui::boxed_list();
        let checks: Vec<gtk::CheckButton> = names
            .iter()
            .map(|name| {
                let check = gtk::CheckButton::builder().label(name).margin_start(8).margin_end(8).margin_top(4).margin_bottom(4).build();
                list.append(&gtk::ListBoxRow::builder().child(&check).build());
                check
            })
            .collect();
        // the check drives the model (a click, the keyboard or a set_active), then shows the model's state, so a mixed
        // row of a selection moves through its tri-state; `painting` keeps the repaint from counting as a click
        let painting = Rc::new(std::cell::Cell::new(false));
        let paint = {
            let (model, checks, painting) = (Rc::clone(&model), checks.clone(), Rc::clone(&painting));
            Rc::new(move || {
                painting.set(true);
                for (i, check) in checks.iter().enumerate() {
                    let (active, mixed) = model.borrow().state(i);
                    check.set_inconsistent(mixed);
                    check.set_active(active);
                }
                painting.set(false);
            })
        };
        paint();
        for (i, check) in checks.iter().enumerate() {
            let (model, paint, painting) = (Rc::clone(&model), Rc::clone(&paint), Rc::clone(&painting));
            check.connect_toggled(move |_| {
                if !painting.get() {
                    model.borrow_mut().click(i);
                    paint();
                }
            });
        }
        // a click beside the label toggles the row's check too
        list.connect_row_activated(move |_, row| {
            if let Some(check) = usize::try_from(row.index()).ok().and_then(|i| checks.get(i)) {
                check.set_active(!check.is_active());
            }
        });
        list.upcast()
    };
    let on_edit = Rc::new(on_edit);
    let mut buttons = vec![
        ui::DialogButton {
            dismisses: true,
            ..ui::DialogButton::new(tr(MsgKey::action_edit_categories), ui::Role::Normal, move || {
                // after the close, so the dismiss runs first
                let on_edit = Rc::clone(&on_edit);
                gtk::glib::idle_add_local_once(move || on_edit());
            })
        },
        ui::DialogButton::cancel(tr(MsgKey::action_cancel)),
    ];
    if !names.is_empty() {
        buttons.push(ui::DialogButton::new(tr(MsgKey::action_ok), ui::Role::Suggested, move || on_ok(&model.borrow())));
    }
    let window = ui::dialog_or_dismiss(&tr(MsgKey::action_move_category), &body, 360, buttons, on_dismiss);
    window.open();
    window
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_domain::category::model::Category;

    #[test]
    fn a_selection_row_with_some_entries_in_it_shows_mixed_and_a_click_moves_it_on() {
        let categories = [Category { id: 1, name: "a".into(), order: 1, flags: 0, hidden: false }];
        let mut d = crate::library::category_dialog::CategoryDialog::new(vec![10, 11], &categories, &[vec![1], vec![]]);
        assert_eq!(Pick::state(&d, 0), (false, true));
        Pick::click(&mut d, 0);
        assert_ne!(Pick::state(&d, 0), (false, true));
    }

    #[test]
    fn an_entry_row_toggles() {
        let mut d = crate::entry_core::dialog::CategoryDialog::new(1, false, &[(1, "a".into())], &[]);
        assert_eq!(Pick::state(&d, 0), (false, false));
        Pick::click(&mut d, 0);
        assert_eq!(Pick::state(&d, 0), (true, false));
    }
}
