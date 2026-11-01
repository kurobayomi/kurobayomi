//! Modal dialogs of the browse pages: the untrusted-extension dialog, the remove confirm and the duplicate dialog all
//! come through `DialogSlot`, which keeps at most one of them open (B-EC-27: a second request replaces the first). GTK
//! only; every decision that follows an answer is in the pure modules (`trust`, `favorite`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

use crate::ui::{DialogButton, DialogOpen as _, Role};

/// How a dialog button looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    Plain,
    Suggested,
    Destructive,
}

/// What a dialog says and offers. Buttons are answered by index; closing the window without a press answers `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogSpec {
    pub title: String,
    pub body: Option<String>,
    pub buttons: Vec<(String, ButtonStyle)>,
}

/// The one dialog a page may have open.
#[derive(Clone, Default)]
pub struct DialogSlot {
    current: Rc<RefCell<Option<adw::Dialog>>>,
}

impl DialogSlot {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    /// Shows `spec` over `anchor`'s window and replaces any dialog this slot still has open. `on_answer` runs once for
    /// the dialog: `Some(index)` for a button, `None` when it was closed without one (also when a newer dialog
    /// replaced it), and then the dialog is gone.
    pub fn show(&self, anchor: &impl IsA<gtk::Widget>, spec: DialogSpec, on_answer: impl Fn(Option<usize>) + 'static) -> adw::Dialog {
        self.show_with(anchor, spec, None, on_answer)
    }

    /// As `show`, with `extra` between the body and the buttons (the category chooser's check buttons).
    pub fn show_with(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        spec: DialogSpec,
        extra: Option<gtk::Widget>,
        on_answer: impl Fn(Option<usize>) + 'static,
    ) -> adw::Dialog {
        // the borrow ends before `close`, which runs the old dialog's close handler
        let old = self.current.borrow_mut().take();
        if let Some(old) = old {
            old.close();
        }
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if let Some(body) = &spec.body {
            column.append(&crate::ui::message(body));
        }
        if let Some(extra) = &extra {
            column.append(extra);
        }

        let answered = Rc::new(Cell::new(false));
        let on_answer: Rc<dyn Fn(Option<usize>)> = Rc::new(on_answer);
        let buttons = spec
            .buttons
            .iter()
            .enumerate()
            .map(|(index, (label, style))| {
                let role = match style {
                    ButtonStyle::Plain => Role::Normal,
                    ButtonStyle::Suggested => Role::Suggested,
                    ButtonStyle::Destructive => Role::Destructive,
                };
                let (answered, on_answer) = (Rc::clone(&answered), Rc::clone(&on_answer));
                DialogButton::new(label, role, move || {
                    if !answered.replace(true) {
                        on_answer(Some(index));
                    }
                })
            })
            .collect();
        // Escape, the close button and a replacement all dismiss: they answer `None`
        let dismissed = (Rc::clone(&answered), Rc::clone(&on_answer));
        let window = crate::ui::dialog_or_dismiss(&spec.title, &column, 360, buttons, move || {
            if !dismissed.0.replace(true) {
                dismissed.1(None);
            }
        });
        {
            let slot = Rc::clone(&self.current);
            let this = window.clone();
            window.connect_closed(move |_| {
                let mut current = slot.borrow_mut();
                if current.as_ref() == Some(&this) {
                    current.take();
                }
            });
        }
        *self.current.borrow_mut() = Some(window.clone());
        window.open_over(anchor);
        window
    }

    /// Registers a window the caller built itself (a dialog that stays open while it works): any dialog this slot has
    /// open is closed first, and the slot forgets the window when it closes. A dialog that must not close while it works sets `can-close` itself.
    pub fn adopt(&self, window: &adw::Dialog) {
        let old = self.current.borrow_mut().take();
        if let Some(old) = old {
            old.close();
        }
        *self.current.borrow_mut() = Some(window.clone());
        let (slot, this) = (Rc::clone(&self.current), window.clone());
        window.connect_closed(move |_| {
            let mut current = slot.borrow_mut();
            if current.as_ref() == Some(&this) {
                current.take();
            }
        });
    }

    #[cfg(test)]
    /// The dialog on screen, if any.
    pub fn current(&self) -> Option<adw::Dialog> {
        self.current.borrow().clone()
    }
}

/// The buttons of a dialog window, in order. Used by the widget checks.
#[cfg(test)]
pub(crate) fn buttons_of(window: &adw::Dialog) -> Vec<gtk::Button> {
    fn walk(widget: &gtk::Widget, out: &mut Vec<gtk::Button>) {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            out.push(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            walk(&w, out);
            child = w.next_sibling();
        }
    }
    let mut out = Vec::new();
    if let Some(child) = crate::ui::dialog_content(window) {
        walk(&child, &mut out);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn spec(title: &str) -> DialogSpec {
        DialogSpec {
            title: title.into(),
            body: Some("body".into()),
            buttons: vec![("One".into(), ButtonStyle::Suggested), ("Two".into(), ButtonStyle::Destructive)],
        }
    }

    fn a_button_answers_with_its_index_once_and_closing_without_one_answers_none() {
        let slot = DialogSlot::new();
        let anchor = gtk::Label::new(None);
        let answers: Rc<RefCell<Vec<Option<usize>>>> = Rc::default();
        let log = Rc::clone(&answers);
        let window = slot.show(&anchor, spec("Ask"), move |a| log.borrow_mut().push(a));
        let buttons = buttons_of(&window);
        assert_eq!(buttons.len(), 2);
        assert!(buttons[1].has_css_class("destructive-action"));
        buttons[1].emit_clicked();
        assert_eq!(*answers.borrow(), [Some(1)], "the window closing after a press does not answer again");
        assert!(slot.current().is_none());

        let log = Rc::clone(&answers);
        slot.show(&anchor, spec("Dismissed"), move |a| log.borrow_mut().push(a)).close();
        assert_eq!(*answers.borrow(), [Some(1), None], "dismissing is its own answer");
    }

    fn a_second_dialog_replaces_the_open_one() {
        let slot = DialogSlot::new();
        let anchor = gtk::Label::new(None);
        let answers: Rc<RefCell<Vec<(&'static str, Option<usize>)>>> = Rc::default();
        let log = Rc::clone(&answers);
        let first = slot.show(&anchor, spec("First"), move |a| log.borrow_mut().push(("first", a)));
        let log = Rc::clone(&answers);
        let second = slot.show(&anchor, spec("Second"), move |a| log.borrow_mut().push(("second", a)));
        assert_ne!(first, second);
        assert_eq!(slot.current().as_ref(), Some(&second), "only the newest dialog is open");
        assert_eq!(*answers.borrow(), [("first", None)], "the replaced dialog changed nothing");
        second.close();
    }

    pub(crate) fn smoke() {
        a_button_answers_with_its_index_once_and_closing_without_one_answers_none();
        a_second_dialog_replaces_the_open_one();
    }
}
