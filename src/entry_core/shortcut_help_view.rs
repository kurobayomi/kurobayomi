//! The in-app shortcut help dialog (T-38a-046): a two-column list of `shortcuts::help()`, opened with F1 or `?` on the
//! manga and anime details pages.

use crate::ui::DialogOpen as _;
use kuro_i18n::{MsgKey, tr};
use gtk::prelude::*;
use gtk::Label;

use super::shortcuts::help;
use crate::ui::{self, DialogButton, Role};

/// Whether this key opens the help dialog.
pub fn is_help_key(key: gtk::gdk::Key) -> bool {
    matches!(key, gtk::gdk::Key::F1 | gtk::gdk::Key::question)
}

/// The grid of the help list: the keys on the left, what they do on the right.
pub fn grid() -> gtk::Grid {
    let grid = gtk::Grid::builder().row_spacing(8).column_spacing(24).build();
    for (row, (keys, what)) in help().iter().enumerate() {
        let keys = Label::builder().label(keys.as_str()).xalign(0.0).css_classes(["monospace"]).build();
        let what = Label::builder().label(what.as_str()).xalign(0.0).wrap(true).build();
        grid.attach(&keys, 0, row as i32, 1, 1);
        grid.attach(&what, 1, row as i32, 1, 1);
    }
    grid
}

/// Opens the dialog.
pub fn open() {
    let buttons = vec![DialogButton::new(&tr(MsgKey::action_close), Role::Suggested, || {})];
    ui::dialog(&tr(MsgKey::ui_keyboard_shortcuts), &grid(), 420, buttons).open();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_grid_has_a_row_for_every_shortcut() {
        crate::testing::with_display(|| {
            let grid = grid();
            let mut rows = 0;
            let mut child = grid.first_child();
            while let Some(w) = child {
                rows += 1;
                child = w.next_sibling();
            }
            assert_eq!(rows, help().len() * 2);
        });
    }

    #[test]
    fn f1_and_question_mark_open_the_help() {
        assert!(is_help_key(gtk::gdk::Key::F1));
        assert!(is_help_key(gtk::gdk::Key::question));
        assert!(!is_help_key(gtk::gdk::Key::f));
    }
}
