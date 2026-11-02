//! The one duplicate dialog of the app: "You have an entry in your library with the same name. Do you still wish to
//! continue?" with Cancel, Show (manga or anime), Migrate existing entry and Add anyway. The manga page, the anime page,
//! Browse and History all open it here. Nothing is written until a choice is pressed; a choice closes the dialog first
//! and then acts (T-DU1), and Cancel, Escape and the close button only close it.

use std::rc::Rc;

use gtk::glib;
use adw::prelude::*;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;

use crate::ui::{self, DialogButton, DialogOpen as _, Role};

/// What the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateChoice {
    /// Open the entry that is already in the library.
    Show,
    /// Start the migrate flow from the entry that is already in the library.
    Migrate,
    /// Add the new entry too, without the check.
    AddAnyway,
}

/// The choices in the order of the buttons, after Cancel.
pub const CHOICES: [DuplicateChoice; 3] = [DuplicateChoice::Show, DuplicateChoice::Migrate, DuplicateChoice::AddAnyway];

/// The button label of a choice for `kind`.
pub fn label(kind: Kind, choice: DuplicateChoice) -> String {
    tr(match choice {
        DuplicateChoice::Show if kind.is_anime() => MsgKey::action_show_anime,
        DuplicateChoice::Show => MsgKey::action_show_manga,
        DuplicateChoice::Migrate => MsgKey::action_migrate_duplicate,
        DuplicateChoice::AddAnyway => MsgKey::action_add_anyway,
    })
}

/// Opens the dialog. `on_closed` runs once however the dialog closes (a choice, Cancel, Escape or the close button),
/// before a choice acts; `on_choice` runs with the choice after the dialog is closed. The window is returned so a page
/// can keep it in its one dialog slot.
pub fn open(kind: Kind, on_choice: impl Fn(DuplicateChoice) + 'static, on_closed: impl Fn() + 'static) -> adw::Dialog {
    let on_choice: Rc<dyn Fn(DuplicateChoice)> = Rc::new(on_choice);
    let mut buttons = vec![DialogButton::cancel(tr(MsgKey::action_cancel))];
    for choice in CHOICES {
        let on_choice = Rc::clone(&on_choice);
        let role = if choice == DuplicateChoice::AddAnyway { Role::Suggested } else { Role::Normal };
        // the dialog closes after this returns; the idle runs once it is gone
        buttons.push(DialogButton::new(label(kind, choice), role, move || {
            let on_choice = Rc::clone(&on_choice);
            glib::idle_add_local_once(move || on_choice(choice));
        }));
    }
    let window = ui::dialog(&tr(MsgKey::are_you_sure), &ui::message(&tr(MsgKey::confirm_add_duplicate_manga)), 400, buttons);
    window.connect_closed(move |_| on_closed());
    window.open();
    window
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[test]
    fn the_show_button_names_the_kind() {
        assert_eq!(label(Kind::Manga, DuplicateChoice::Show), tr(MsgKey::action_show_manga));
        assert_eq!(label(Kind::Anime, DuplicateChoice::Show), tr(MsgKey::action_show_anime));
        assert_eq!(label(Kind::Anime, DuplicateChoice::Migrate), label(Kind::Manga, DuplicateChoice::Migrate));
    }

    fn buttons(window: &adw::Dialog) -> Vec<gtk::Button> {
        fn walk(w: &gtk::Widget, out: &mut Vec<gtk::Button>) {
            if let Some(b) = w.downcast_ref::<gtk::Button>() {
                out.push(b.clone());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                walk(&c, out);
                child = c.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(&ui::dialog_content(window).expect("content"), &mut out);
        out.into_iter().filter(|b| b.label().is_some()).collect()
    }

    #[test]
    fn a_choice_closes_the_dialog_first_and_then_acts_and_cancel_only_closes() {
        crate::testing::with_display(|| {
            let _nav = crate::testing::nav();
            let ctx = glib::MainContext::default();
            for (index, expected) in [(1usize, Some(DuplicateChoice::Show)), (2, Some(DuplicateChoice::Migrate)), (3, Some(DuplicateChoice::AddAnyway)), (0, None)] {
                let chosen = Rc::new(RefCell::new(None));
                let closed = Rc::new(Cell::new(0));
                let window = {
                    let (chosen, closed) = (Rc::clone(&chosen), Rc::clone(&closed));
                    open(Kind::Manga, move |c| *chosen.borrow_mut() = Some(c), move || closed.set(closed.get() + 1))
                };
                let labels: Vec<String> = buttons(&window).iter().filter_map(|b| b.label().map(|l| l.to_string())).collect();
                assert_eq!(labels, [tr(MsgKey::action_cancel), label(Kind::Manga, DuplicateChoice::Show), tr(MsgKey::action_migrate_duplicate), tr(MsgKey::action_add_anyway)]);
                buttons(&window)[index].emit_clicked();
                while ctx.iteration(false) {}
                assert_eq!(closed.get(), 1, "closed once");
                assert_eq!(*chosen.borrow(), expected);
            }
        });
    }
}
