//! The remove dialog of a library selection (`DeleteLibraryEntryDialog.kt`, E-37, E-28): "from library" and
//! "downloaded" are independent boxes, OK needs one checked, and the downloaded box is left out when a local entry is
//! selected. The rules are `actions::DeleteChoice` and `actions::downloads_checkbox_offered`.

use crate::ui::DialogOpen as _;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_i18n::{MsgKey, tr};

use crate::library::actions::DeleteChoice;
use crate::ui::{DialogButton, Role};

/// Shows the dialog. `on_confirm` gets the choice when OK is pressed.
pub fn show(kind: Kind, downloads_offered: bool, on_confirm: impl Fn(DeleteChoice) + 'static) {
    let (library_label, downloads_label) = match kind {
        Kind::Manga => (MsgKey::manga_from_library, MsgKey::downloaded_chapters),
        Kind::Anime => (MsgKey::anime_from_library, MsgKey::downloaded_episodes),
    };
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let from_library = gtk::CheckButton::with_label(&tr(library_label));
    column.append(&from_library);
    let downloads = gtk::CheckButton::with_label(&tr(downloads_label));
    if downloads_offered {
        column.append(&downloads);
    }
    let choice = {
        let (from_library, downloads) = (from_library.clone(), downloads.clone());
        Rc::new(move || DeleteChoice { from_library: from_library.is_active(), downloads: downloads_offered && downloads.is_active() })
    };
    // OK needs a box checked: until then it does nothing and the dialog stays open
    let ok = DialogButton::keep_open_unless(tr(MsgKey::action_ok), Role::Destructive, move || {
        let choice = choice();
        if !choice.confirmable() {
            return false;
        }
        on_confirm(choice);
        true
    });
    crate::ui::dialog(&tr(MsgKey::action_remove), &column, 360, vec![DialogButton::cancel(tr(MsgKey::action_cancel)), ok]).open();
}
