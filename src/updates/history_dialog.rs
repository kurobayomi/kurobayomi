//! The confirm dialog of the History screen (T-42a-018, T-42a-024): a text, an optional checkbox whose state reaches
//! the confirm callback, Cancel and a confirm button. `show_dialog_then` of the shell passes no value to its callback,
//! so the remove dialog with its "Reset all" checkbox is built here. Escape closes the dialog. A modal window has no scrim, so a click outside it cannot cancel; the parent window is blocked instead.


use crate::ui::DialogOpen as _;
use adw::prelude::*;
use gtk::{Box as GBox, CheckButton, Orientation as O};

use crate::ui::{DialogButton, Role};
use crate::theme::tokens::{SPACE_M};

/// What the dialog says. `checkbox` is the label of the optional switch.
pub struct ConfirmCopy {
    pub title: String,
    pub text: String,
    pub checkbox: Option<String>,
    pub confirm: String,
    pub cancel: String,
}

/// Shows the dialog. `on_ok` gets the checkbox state (false when there is no checkbox). `on_close` runs once the
/// dialog is gone, whichever button closed it, so the screen can drop its dialog state.
pub fn show(copy: ConfirmCopy, destructive: bool, on_ok: impl Fn(bool) + 'static, on_close: impl Fn() + 'static) {
    let col = GBox::new(O::Vertical, SPACE_M);
    col.append(&crate::ui::message(&copy.text));
    let check = copy.checkbox.as_deref().map(|label| CheckButton::with_label(label));
    if let Some(check) = &check {
        col.append(check);
    }
    // a second click while the window is closing does not confirm again (a double click deletes once)
    let confirmed = std::cell::Cell::new(false);
    let confirm = DialogButton::new(&copy.confirm, if destructive { Role::Destructive } else { Role::Suggested }, move || {
        if confirmed.replace(true) {
            return;
        }
        on_ok(check.as_ref().is_some_and(CheckButton::is_active));
    });
    let win = crate::ui::dialog(&copy.title, &col, 420, vec![DialogButton::cancel(&copy.cancel), confirm]);
    win.connect_closed(move |_| on_close());
    win.open();
}
