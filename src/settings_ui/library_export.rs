//! The Library List row under Data and storage, Export (task T-47a-091). The row opens the "Select data to include"
//! dialog; Export asks for a save file and writes the favourites as CSV (`library_csv`). The favourites are read when the
//! dialog opens, so Export is never pressed while they are still loading.
use crate::ui::DialogOpen as _;
use kuro_i18n::{MsgKey, tr};
use adw::prelude::*;
use kuro_data::entry::EntryRepo;
use kuro_platform::kind::Kind;

use crate::pref_model::model::PreferenceItem;
use crate::settings_ui::library_csv::{self, Columns, Entry};
use crate::settings_ui::rows::text;
use crate::ui::{DialogButton, Role};
use crate::theme::tokens::{SPACE_S};

pub fn row_title() -> String {
    tr(MsgKey::library_list)
}
pub fn dialog_title() -> String {
    tr(MsgKey::migration_dialog_what_to_include)
}
pub fn group_title() -> String {
    tr(MsgKey::export)
}

/// The favourites of both databases, manga first, each in the repository's order.
fn favourites() -> Vec<Entry> {
    let app = crate::data::app();
    let repo = EntryRepo::new(app.db.clone());
    [(Kind::Manga, "manga"), (Kind::Anime, "anime")]
        .into_iter()
        .flat_map(|(kind, word)| {
            repo.favorites(kind)
                .unwrap_or_default()
                .into_iter()
                .map(move |e| Entry { title: e.title, kind: word, author: e.author, artist: e.artist })
        })
        .collect()
}

/// The row: a plain text row whose tap opens the dialog.
pub fn row() -> PreferenceItem {
    let mut row = text(&row_title(), None, true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(std::rc::Rc::new(open_dialog));
    }
    row
}

/// The check boxes as the columns they stand for, in Android order: title, type, author, artist.
fn read(boxes: &[gtk::CheckButton; 4]) -> Columns {
    Columns { title: boxes[0].is_active(), kind: boxes[1].is_active(), author: boxes[2].is_active(), artist: boxes[3].is_active() }
}

/// Shows `columns` in the check boxes and greys the three below Title while Title is off.
fn apply(boxes: &[gtk::CheckButton; 4], columns: Columns) {
    for (button, on) in boxes.iter().zip([columns.title, columns.kind, columns.author, columns.artist]) {
        if button.is_active() != on {
            button.set_active(on);
        }
    }
    for button in &boxes[1..] {
        button.set_sensitive(columns.title);
    }
}

fn open_dialog() {
    let entries = std::rc::Rc::new(favourites());
    let boxes: [gtk::CheckButton; 4] = ["Title", "Type", "Author", "Artist"].map(|label| gtk::CheckButton::with_label(label));
    apply(&boxes, Columns::ALL);

    let column = gtk::Box::new(gtk::Orientation::Vertical, SPACE_S);
    let heading = gtk::Label::new(Some(&dialog_title()));
    heading.add_css_class("heading");
    heading.set_halign(gtk::Align::Start);
    column.append(&heading);
    for button in &boxes {
        let all = boxes.clone();
        button.connect_toggled(move |_| {
            // Title off clears the rest; any other change is kept as the user set it
            let shown = read(&all);
            apply(&all, if shown.title { shown } else { shown.with_title(false) });
        });
        column.append(button);
    }

    // the Export button needs the window as the parent of the save dialog, which is built after the buttons
    let window: std::rc::Rc<std::cell::OnceCell<adw::Dialog>> = std::rc::Rc::default();
    let export_window = std::rc::Rc::clone(&window);
    let export = DialogButton::keep_open_unless(&tr(MsgKey::export), Role::Suggested, move || {
        let Some(close) = export_window.get().cloned() else { return false };
        let columns = read(&boxes);
        let body = library_csv::export(&entries, columns);
        let dialog = gtk::FileDialog::new();
        dialog.set_initial_name(Some(library_csv::FILE_NAME));
        let done = close.clone();
        dialog.save(Some(&crate::ui::window()), None::<&gtk::gio::Cancellable>, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else { return };
            match std::fs::write(&path, body.as_bytes()) {
                Ok(()) => crate::toast::notify(library_csv::exported()),
                Err(err) => {
                    tracing::warn!(%err, "the library was not exported");
                    crate::toast::notify(library_csv::failed());
                }
            }
            done.close();
        });
        false
    });
    let built = crate::ui::dialog(&row_title(), &column, 320, vec![DialogButton::cancel(&tr(MsgKey::action_cancel)), export]);
    let _ = window.set(built.clone());
    built.open();
}

/// The Export group with the Library List row.
pub fn group_row() -> crate::pref_model::model::Preference {
    crate::settings_ui::rows::group(&group_title(), vec![row()])
}
