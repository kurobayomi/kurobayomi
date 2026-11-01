//! The history row (T-42a-014): a book-aspect cover (initials until an image draws) that opens the entry, a bold title (two lines, ellipsised), the
//! subtitle, a favorite button only when the entry is not in the library, and a delete button. A click on the row
//! resumes (T-42a-016); its right-click menu offers Resume, Open details and Remove from history.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{Align, Box as GBox, Button, GestureClick, Orientation as O};

use crate::browse::look::list_row;
use crate::widgets::context_menu::Item;
use kuro_domain::dates::item_row_text;
use kuro_i18n::{MsgKey, tr};

/// The right-click item that opens the entry's page (no catalogue key yet, as in `updates::copy`).
pub fn open_details() -> String {
    tr(MsgKey::history_row_open_details)
}
/// The right-click item that asks to remove the row, as the trash button does.
pub fn remove_from_history() -> String {
    tr(MsgKey::history_row_remove_from_history)
}

/// The subtitle: `Ch. {n} - {time}` (or `Ep.`) when the unit number is above -1, else the time only.
pub fn subtitle(prefix: &str, number: f64, time: &str) -> String {
    item_row_text(prefix, number, time)
}

/// The row's actions. Each closure is the screen's own; the row only calls it.
#[derive(Clone)]
pub struct RowActions {
    pub open_entry: Rc<dyn Fn()>,
    pub resume: Rc<dyn Fn()>,
    pub remove: Rc<dyn Fn()>,
    pub add_to_library: Rc<dyn Fn()>,
}

/// One history row (`look::list_row`). `in_library` decides whether the favorite button is shown. `cover` is the row's
/// cover widget (`look::row_cover` at `look::MEDIUM`); a press on it opens the entry.
pub fn history_row(title: &str, subtitle: &str, cover: &impl IsA<gtk::Widget>, in_library: bool, actions: RowActions) -> adw::ActionRow {
    let cover = cover.as_ref();
    let open = GestureClick::new();
    let open_entry = actions.open_entry.clone();
    open.connect_pressed(move |gesture, _, _, _| {
        // the cover opens the entry; the row's own click (resume) does not see it
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_entry();
    });
    cover.add_controller(open);

    let buttons = GBox::new(O::Horizontal, 4);
    if !in_library {
        let favorite = Button::from_icon_name("starred-symbolic");
        favorite.set_tooltip_text(Some(&tr(MsgKey::add_to_library)));
        favorite.set_valign(Align::Center);
        favorite.add_css_class("flat");
        let add = actions.add_to_library.clone();
        favorite.connect_clicked(move |_| add());
        buttons.append(&favorite);
    }
    let delete = Button::from_icon_name("user-trash-symbolic");
    delete.set_tooltip_text(Some(&tr(MsgKey::action_remove)));
    delete.set_valign(Align::Center);
    delete.add_css_class("flat");
    let remove = actions.remove.clone();
    delete.connect_clicked(move |_| remove());
    buttons.append(&delete);

    let row = list_row(Some(cover.clone()), title, Some(subtitle), Some(buttons.upcast()));
    // a long title keeps its two lines
    row.set_title_lines(2);
    row.set_activatable(true);
    let resume = actions.resume.clone();
    crate::widgets::row_activate::on_activate(&row, move |_| resume());
    crate::widgets::context_menu::attach(&row, move |_, _| menu(&actions));
    row
}

/// The row's right-click menu: Resume, Open details, then Remove from history (which asks first, as the trash button).
fn menu(actions: &RowActions) -> Vec<Item> {
    let (resume, open, remove) = (actions.resume.clone(), actions.open_entry.clone(), actions.remove.clone());
    vec![
        Item::new(tr(MsgKey::action_resume), 0, move || resume()),
        Item::new(open_details(), 0, move || open()),
        Item::new(remove_from_history(), 1, move || remove()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_menu_resumes_opens_the_details_and_asks_to_remove() {
        let log: Rc<std::cell::RefCell<Vec<&str>>> = Rc::default();
        let call = |name: &'static str| -> Rc<dyn Fn()> {
            let log = Rc::clone(&log);
            Rc::new(move || log.borrow_mut().push(name))
        };
        let actions = RowActions { open_entry: call("open"), resume: call("resume"), remove: call("remove"), add_to_library: call("add") };
        let items = menu(&actions);
        assert_eq!(items.iter().map(|i| i.label.clone()).collect::<Vec<_>>(), [tr(MsgKey::action_resume), open_details().to_owned(), remove_from_history().to_owned()]);
        for item in &items {
            (item.run)();
        }
        assert_eq!(*log.borrow(), ["resume", "open", "remove"]);
    }

    #[test]
    fn history_subtitle_rules() {
        assert_eq!(subtitle("Ch.", 12.0, "14:05"), "Ch. 12 - 14:05");
        assert_eq!(subtitle("Ep.", 3.5, "21:30"), "Ep. 3.5 - 21:30");
        assert_eq!(subtitle("Ch.", -1.0, "14:05"), "14:05");
    }

    #[test]
    fn t_42a_014_the_row_leads_with_its_cover_which_the_row_does_not_resize() {
        crate::testing::with_display(|| {
            let none: Rc<dyn Fn()> = Rc::new(|| {});
            let actions = RowActions { open_entry: none.clone(), resume: none.clone(), remove: none.clone(), add_to_library: none };
            let cover = crate::browse::look::row_cover("Title", None, crate::browse::look::MEDIUM.width, crate::browse::look::MEDIUM.height);
            let row = history_row("Title", "Ch. 1", &cover, false, actions);
            assert!(cover.is_ancestor(&row), "the cover leads the row");
            assert_eq!(cover.size_request(), (53, 80), "the medium row cover");
        });
    }

    /// The cover opens the entry and claims its press, so the row under it does not also resume.
    #[test]
    fn a_cover_press_opens_the_entry_and_does_not_resume() {
        crate::testing::with_display(|| {
            use std::cell::Cell;
            let (opened, resumed) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
            let (o, r) = (Rc::clone(&opened), Rc::clone(&resumed));
            let none: Rc<dyn Fn()> = Rc::new(|| {});
            let actions = RowActions { open_entry: Rc::new(move || o.set(o.get() + 1)), resume: Rc::new(move || r.set(r.get() + 1)), remove: none.clone(), add_to_library: none };
            let cover = gtk::Picture::new();
            let _row = history_row("Title", "Ch. 1", &cover, true, actions);
            let all = cover.observe_controllers();
            let press = (0..all.n_items()).filter_map(|i| all.item(i)).find_map(|c| c.downcast::<GestureClick>().ok()).expect("the cover's click");
            press.emit_by_name::<()>("pressed", &[&1_i32, &0.0_f64, &0.0_f64]);
            assert_eq!((opened.get(), resumed.get()), (1, 0), "the cover opens the entry; the resume is the row's");
        });
    }
}
