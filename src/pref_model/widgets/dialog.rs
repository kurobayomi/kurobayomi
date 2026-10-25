//! The one dialog helper every preference dialog uses: the shared dialog of `crate::ui`, with a confirm button that
//! the caller enables or disables.

use std::cell::{Cell, OnceCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, Box as GBox, Button, Label, Orientation};

use super::super::strings;
use super::DIALOG_WIDTH;
use crate::ui::{DialogButton, Role};

/// A dialog built by [`show`]. The caller owns the body and decides when to close it.
#[derive(Clone)]
pub struct Dialog {
    pub window: gtk::Window,
    pub confirm: Option<Button>,
    error: Label,
    done: Rc<Cell<bool>>,
}

impl Dialog {
    /// Enables or disables the confirm button. Call on every change that affects the rule.
    pub fn set_confirm_enabled(&self, enabled: bool) {
        if let Some(confirm) = &self.confirm {
            confirm.set_sensitive(enabled);
        }
    }

    /// Shows `message` under the body (a failed or vetoed-with-error write), or hides it for `None`.
    pub fn set_error(&self, message: Option<&str>) {
        self.error.set_text(message.unwrap_or(""));
        self.error.set_visible(message.is_some());
    }

    /// Closes the dialog after an accepted write. The cancel callback does not run.
    pub fn finish(&self) {
        self.done.set(true);
        self.window.destroy();
    }
}

/// Return on the main block or the keypad.
pub fn is_enter(key: gtk::gdk::Key) -> bool {
    matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::ISO_Enter)
}

/// Builds and shows a dialog. `confirm` is the confirm label, or `None` for a Cancel-only dialog.
/// `on_confirm` gets the dialog so it can run a veto and call [`Dialog::finish`] on acceptance.
/// `on_cancel` runs once, on Escape, Cancel or the window close button, never after `finish`.
pub fn show(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    body: &impl IsA<gtk::Widget>,
    confirm: Option<&str>,
    on_confirm: impl Fn(Dialog) + 'static,
    on_cancel: impl Fn() + 'static,
) -> Dialog {
    let done = Rc::new(Cell::new(false));
    let column = GBox::new(Orientation::Vertical, 16);
    // the window title shows the title once; the body does not repeat it
    column.append(body);
    let error = Label::new(None);
    error.add_css_class("pref-error");
    error.set_wrap(true);
    error.set_xalign(0.0);
    error.set_visible(false);
    column.append(&error);

    // the confirm button needs the dialog, which is built after the window that holds the button
    let cell: Rc<OnceCell<Dialog>> = Rc::default();
    let mut buttons = vec![DialogButton::cancel(strings::cancel())];
    if let Some(label) = confirm {
        let cell = cell.clone();
        buttons.push(DialogButton::keep_open_unless(label, Role::Suggested, move || {
            if let Some(dialog) = cell.get() {
                on_confirm(dialog.clone());
            }
            false
        }));
    }
    let close_done = done.clone();
    let window = crate::ui::dialog_or_dismiss(title, &column, DIALOG_WIDTH, buttons, move || {
        if !close_done.replace(true) {
            on_cancel();
        }
    });
    window.set_title(Some(title));
    window.set_transient_for(Some(parent));
    // the confirm button is the last button of the window: its row comes last
    let confirm_button = confirm.and_then(|_| last_button(window.child()?.upcast_ref()));
    let dialog = Dialog { window: window.clone(), confirm: confirm_button.clone(), error, done };
    let _ = cell.set(dialog.clone());

    // Enter submits an enabled dialog (T-19a-067); a field or button that handles Enter itself never gets here
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if let Some(confirm) = confirm_button.as_ref().filter(|_| is_enter(key)) {
            if confirm.is_sensitive() {
                confirm.emit_clicked();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);

    window.present();
    dialog
}

/// The last button at or below `widget` in tree order.
fn last_button(widget: &gtk::Widget) -> Option<Button> {
    let mut found = None;
    let mut child = widget.first_child();
    while let Some(next) = child {
        found = last_button(&next).or(found);
        child = next.next_sibling();
    }
    found.or_else(|| widget.clone().downcast::<Button>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many labels at or below `root` read exactly `text`.
    fn labels_reading(root: &gtk::Widget, text: &str) -> usize {
        let mut n = usize::from(root.downcast_ref::<Label>().is_some_and(|l| l.text() == text));
        let mut child = root.first_child();
        while let Some(c) = child {
            n += labels_reading(&c, text);
            child = c.next_sibling();
        }
        n
    }

    // the dialog title is the window title only; the body does not say it a second time
    #[test]
    fn the_title_is_shown_once_in_the_window_title_and_not_repeated_in_the_body() {
        crate::testing::with_display(|| {
            let (_, parent) = crate::testing::nav();
            let body = Label::new(Some("Some text"));
            let dialog = show(&parent, "Pick one", &body, Some("OK"), |_| {}, || {});
            assert_eq!(dialog.window.title().as_deref(), Some("Pick one"));
            let content = dialog.window.child().expect("the dialog content");
            assert_eq!(labels_reading(&content, "Pick one"), 0);
            assert_eq!(labels_reading(&content, "Some text"), 1);
            dialog.window.destroy();
        });
    }

    #[test]
    fn t_19a_067_every_enter_key_submits_and_other_keys_do_not() {
        use gtk::gdk::Key;
        assert!(is_enter(Key::Return) && is_enter(Key::KP_Enter) && is_enter(Key::ISO_Enter));
        assert!(!is_enter(Key::Escape) && !is_enter(Key::space) && !is_enter(Key::n));
    }
}
