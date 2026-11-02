//! The page actions sheet (T-43a-045, `ReaderPageActionsDialog.kt`). It opens on a long press or right-click on a
//! page, and on Shift+F10. Four actions in a fixed order. Copy, Share and Save close the sheet; Set as cover stays
//! open until its result toast (E-UI-5).

use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;

use gtk::prelude::*;

use super::model::PageStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SetCover,
    Copy,
    Share,
    Save,
}

/// The actions in the order the sheet shows them.
pub const ACTIONS: [Action; 4] = [Action::SetCover, Action::Copy, Action::Share, Action::Save];

pub fn label(action: Action) -> String {
    match action {
        Action::SetCover => tr(MsgKey::set_as_cover),
        Action::Copy => tr(MsgKey::action_copy_to_clipboard),
        Action::Share => tr(MsgKey::action_share),
        Action::Save => tr(MsgKey::action_save),
    }
}

/// Whether the sheet closes when the action is picked.
pub fn closes_sheet(action: Action) -> bool {
    action != Action::SetCover
}

/// Whether an action does anything for a page in `status`: only a `Ready` page has bytes to copy, share, save or use as
/// the cover. It is checked again when the action runs, not only when the sheet opens.
pub fn runs_for(status: PageStatus) -> bool {
    status == PageStatus::Ready
}

/// The sheet's buttons. `on_action` gets the picked action; the caller runs it and closes the sheet when
/// [`closes_sheet`] says so.
pub fn build(on_action: Rc<dyn Fn(Action)>) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_start(16);
    column.set_margin_end(16);
    column.set_margin_top(12);
    column.set_margin_bottom(12);
    for action in ACTIONS {
        let button = gtk::Button::with_label(&label(action));
        button.add_css_class("flat");
        let on_action = Rc::clone(&on_action);
        button.connect_clicked(move |_| on_action(action));
        column.append(&button);
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_actions_in_order_with_copy_share_and_save_closing_the_sheet() {
        let labels: Vec<String> = ACTIONS.iter().map(|a| label(*a)).collect();
        assert_eq!(labels, ["Set as cover", "Copy to clipboard", "Share", "Save"]);
        assert!(!closes_sheet(Action::SetCover));
        assert!(closes_sheet(Action::Copy) && closes_sheet(Action::Share) && closes_sheet(Action::Save));
    }

    #[test]
    fn actions_do_nothing_unless_the_page_is_ready() {
        assert!(runs_for(PageStatus::Ready));
        for status in [PageStatus::Queued, PageStatus::LoadPage, PageStatus::DownloadImage, PageStatus::Error] {
            assert!(!runs_for(status), "{status:?}");
        }
    }
}
