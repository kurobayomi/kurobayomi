//! GTK glue for local titles (area 43, wire-local-anime): the library menu's "Link a … folder…" and "Open the Local
//! folder", and the link shortcut. The Local folder needs no action: its new titles join the library before each library
//! update (a launch, the refresh button), whose pass reads every local title's files. A link stores its folder on a worker
//! thread, then starts that pass for its kind.

use kuro_i18n::{Arg, MsgKey, tr, trf, tr_plural};

use gtk::gio;
use gtk::prelude::*;
use kuro_platform::kind::Kind;

use super::titles::{self, Report};

/// The window action that links a local folder of the kind on screen, anime or manga (`win.add-local-folder`,
/// `Ctrl+Shift+O`).
pub const ADD_ACTION: &str = "add-local-folder";

/// Registers the link window action and its shortcut. It follows the app's one Anime | Manga choice.
pub fn attach(window: &gtk::ApplicationWindow) {
    let add = gio::SimpleAction::new(ADD_ACTION, None);
    add.connect_activate(|_, _| link_folder(crate::shell::kind_choice::current()));
    window.add_action(&add);
    if let Some(app) = window.application() {
        app.set_accels_for_action(&format!("win.{ADD_ACTION}"), &["<Primary><Shift>o"]);
    }
}

/// Asks for a folder and links it where it is: a manga folder is one title; an anime folder is one series when it holds
/// episode files, else each of its folders is one.
pub fn link_folder(kind: Kind) {
    let title = tr(match kind {
        Kind::Manga => MsgKey::ui_add_local_manga_folder,
        Kind::Anime => MsgKey::ui_add_local_anime_folder,
    });
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    dialog.select_folder(Some(&crate::ui::window()), gio::Cancellable::NONE, move |picked| {
        let Some(folder) = picked.ok().and_then(|file| file.path()) else { return };
        let db = crate::data::app().db.clone();
        let prefs = crate::data::app().base.store().clone();
        crate::background::run(
            move || titles::link(&db, &prefs, kind, &folder, chrono::Utc::now().timestamp_millis()).map_err(|e| e.to_string()),
            move |result| {
            crate::toast::notify(message(&result));
            if result.is_ok_and(|report| !report.added.is_empty()) {
                crate::library::mount::refresh(kind);
                // the local source reads the new titles' chapters or episodes in the library update
                crate::updates::library_trigger::start(kind);
            }
            },
        );
    });
}

/// Opens the Local folder of `kind` in the file manager: what is put in it joins the library at the next refresh.
pub fn open_local_folder(kind: Kind) {
    match super::local_root(kind) {
        Some(root) => crate::ui::open_local(&root, |error| crate::toast::notify(error)),
        None => crate::toast::notify(tr(MsgKey::local_folder_unavailable)),
    }
}

/// The toast of a link: how many titles joined the library, why a folder was passed over, or why nothing could be read.
fn message(result: &Result<Report, String>) -> String {
    match result {
        Err(error) => trf(MsgKey::local_scan_failed, &[("error", Arg::Str(error))]),
        Ok(report) if report.added.is_empty() => match report.skipped.first() {
            Some((folder, why)) => trf(MsgKey::local_read_failed, &[("error", Arg::Str(&format!("{folder}: {why}")))]),
            None => tr(MsgKey::local_link_nothing_new),
        },
        Ok(report) => tr_plural(MsgKey::local_linked, i64::try_from(report.added.len()).unwrap_or(i64::MAX), &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_says_how_many_titles_joined_or_why_none_did() {
        let report = |added: &[&str], skipped: &[(&str, &str)]| -> Result<Report, String> {
            Ok(Report { added: added.iter().map(|s| (*s).to_owned()).collect(), skipped: skipped.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect() })
        };
        assert_eq!(message(&report(&["A"], &[])), "Linked 1 title");
        assert_eq!(message(&report(&["A", "B"], &[])), "Linked 2 titles");
        assert_eq!(message(&report(&[], &[])), tr(MsgKey::local_link_nothing_new));
        assert_eq!(message(&report(&[], &[("Locked", "permission denied")])), "Could not read the folder: Locked: permission denied");
        assert_eq!(message(&Err("disk gone".into())), "Could not scan the folder: disk gone");
    }
}
