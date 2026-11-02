// Settings pages that are not plain preference lists (the main list and shell are in pref_model).
use crate::ui::DialogOpen as _;
use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;
use std::sync::Arc;

use crate::screens::{built, Built};
use crate::ui::*;
use adw::prelude::*;
use gtk::{Box as GBox, Button, Label, Orientation as O};
use kuro_data::entry::EntryRepo;
use kuro_data::source::StubSourceRepo;
use kuro_platform::kind::Kind;

pub fn body(id: &str) -> Option<Built> {
    // the settings shell, the language picker, the extension lists, the backup pages and Debug info (area 47b)
    if crate::pref_model::pages::IDS.contains(&id) {
        return crate::pref_model::pages::body(id);
    }
    match id {
        "settings_backup_schema" => backup_schema(id),
        "settings_clear_manga_database" | "settings_clear_anime_database" => clear_db(id),
        _ => None,
    }
}

/// The debug screen of the backup schema: the 13 `Backup` fields, and the Copy button puts the text on the clipboard (T-21a-035).
fn backup_schema(id: &str) -> Option<Built> {
    let text = kuro_backup::schema_text::TEXT;
    let l = Label::builder().label(text).xalign(0.0).yalign(0.0).selectable(true).css_classes(["monospace"]).build();
    margins(&l, 16);
    let tools = toolbar(&screen(id));
    for copy in tools.iter().filter_map(|w| w.downcast_ref::<Button>()) {
        copy.connect_clicked(|_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(text);
            }
        });
    }
    built(scrolled(&l), tools)
}

/// The clear-database screens: one row per source that has non-library entries in the database, a
/// check per row, and Delete (after the spec's confirm dialog) removes the checked sources' entries.
fn clear_db(id: &str) -> Option<Built> {
    let kind = Kind::from_anime(id == "settings_clear_anime_database");
    let db = crate::data::app().db.clone();
    // (source id, row title, non-library count), read from the database; a failed read shows nothing to clear.
    let entries_repo = EntryRepo::new(db.clone());
    let stubs = StubSourceRepo::new(db.clone(), kind);
    let sources: Vec<(i64, String, i64)> = entries_repo
        .non_library_counts_by_source(kind)
        .map(|rows| {
            rows.into_iter()
                .map(|(source, count)| {
                    let name = stubs.get(source).ok().flatten().map(|s| s.display_name());
                    (source, name.filter(|n| !n.is_empty()).unwrap_or_else(|| kuro_i18n::trf(kuro_i18n::MsgKey::ui_source_by_id, &[("id", kuro_i18n::Arg::Int(source))])), count)
                })
                .collect()
        })
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "cannot list the non-library entries to clear");
            Vec::new()
        });
    let sources = sorted_by_name(sources);
    let b = GBox::new(O::Vertical, 0);
    if sources.is_empty() {
        let text = screen(id).empty.unwrap_or_else(|| tr(MsgKey::database_clean).to_owned());
        let label = Label::builder().label(text).wrap(true).xalign(0.0).build();
        margins(&label, 12);
        b.append(&label);
        return built(b, vec![]);
    }
    let col = column();
    let l = boxed_list();
    let entries: Rc<Vec<(i64, gtk::CheckButton, adw::ActionRow)>> = Rc::new(
        sources
            .into_iter()
            .map(|(source, name, count)| {
                let check = gtk::CheckButton::new();
                let r = row(&name, Some(&kuro_i18n::trf(kuro_i18n::MsgKey::clear_database_source_item_count, &[("arg1", kuro_i18n::Arg::Int(count))])), Some(check.clone().upcast()));
                r.add_prefix(&gtk::Image::from_icon_name("application-x-addon-symbolic"));
                l.append(&r);
                (source, check, r)
            })
            .collect(),
    );
    // tapping anywhere on a row flips its check
    let tapped = Rc::clone(&entries);
    l.connect_row_activated(move |_, row| {
        if let Some((_, check, _)) = tapped.iter().find(|(_, _, r)| r == row) {
            check.set_active(!check.is_active());
        }
    });
    col.append(&l);
    b.append(&scrolled(&col));
    let del = Button::builder().label(&tr(MsgKey::action_delete)).css_classes(["destructive-action"]).sensitive(false).build();
    margins(&del, 12);
    {
        let (entries, db, list, del_btn, id) = (Rc::clone(&entries), Arc::clone(&db), l.clone(), del.clone(), id.to_string());
        del.connect_clicked(move |_| {
            let Some(dialog) = screen(&id).dialogs.into_iter().find(|d| d.id == "confirm_clear") else { return };
            let (entries, db, list, del_btn) = (Rc::clone(&entries), Arc::clone(&db), list.clone(), del_btn.clone());
            show_dialog_then(&dialog, move || {
                let selected: Vec<i64> = entries.iter().filter(|(_, c, _)| c.is_active()).map(|(s, _, _)| *s).collect();
                if selected.is_empty() {
                    return;
                }
                let item_count = || {
                    let sql = "SELECT count(*) FROM items i JOIN entries e ON e._id = i.entry_id WHERE e.kind = ?1";
                    db.read(|c| Ok(c.query_row(sql, [kind.as_str()], |r| r.get::<_, i64>(0))?)).map_err(|e| e.to_string())
                };
                let written = item_count().and_then(|before| {
                    // one path for both kinds: non-favourites go, favourite seasons are orphaned, reset history goes
                    EntryRepo::new(Arc::clone(&db)).clear_non_library(kind, &selected).map_err(|e| e.to_string())?;
                    Ok((before - item_count()?).max(0) as usize)
                });
                match written {
                    Ok(removed) => {
                        // saved browse results name the rows that just went
                        crate::browse::results_cache::clear();
                        after_clear(kind, removed);
                        for (source, check, r) in entries.iter() {
                            if selected.contains(source) {
                                list.remove(r);
                                check.set_active(false);
                            }
                        }
                        del_btn.set_sensitive(false);
                    }
                    Err(e) => tracing::warn!(error = %e, "clearing the selected sources failed"),
                }
            });
        });
    }
    b.append(&del);
    for (_, c, _) in entries.iter() {
        let (d, all) = (del.clone(), Rc::clone(&entries));
        c.connect_toggled(move |_| d.set_sensitive(all.iter().any(|(_, c, _)| c.is_active())));
    }
    let all = icon_button("edit-select-all-symbolic", &tr(MsgKey::action_select_all));
    let inv = icon_button("object-flip-horizontal-symbolic", &tr(MsgKey::action_select_inverse));
    let (e1, e2) = (Rc::clone(&entries), Rc::clone(&entries));
    all.connect_clicked(move |_| e1.iter().for_each(|(_, c, _)| c.set_active(true)));
    inv.connect_clicked(move |_| e2.iter().for_each(|(_, c, _)| c.set_active(!c.is_active())));
    built(b, vec![inv.upcast(), all.upcast()])
}

/// The clear-database rows in source-name order, ignoring case.
fn sorted_by_name(mut sources: Vec<(i64, String, i64)>) -> Vec<(i64, String, i64)> {
    sources.sort_by_cached_key(|(_, name, _)| name.to_lowercase());
    sources
}

/// Housekeeping after "Clear database" committed (T-03a-028): above 1000 removed chapters or episodes the WAL is
/// checkpointed and the user is offered VACUUM. Both run on their own connection to the file, off the open handle.
fn after_clear(_kind: Kind, removed: usize) {
    use crate::ui::{DialogButton, Role};
    use kuro_platform::paths::Paths as _;
    let Ok(paths) = kuro_platform::system_paths::SystemPaths::system() else { return };
    let path = kuro_db::schema_unified::db_path(&paths.data_dir());
    let after = match kuro_db::schema_unified::open_file(&path).and_then(|c| kuro_db::maintenance::after_chapter_delete(&c, removed)) {
        Ok(after) => after,
        Err(e) => {
            tracing::warn!(error = %e, "database housekeeping after the clear failed");
            return;
        }
    };
    if !after.offer_vacuum {
        return;
    }
    let compact = DialogButton::new(&tr(MsgKey::ui_compact), Role::Suggested, move || {
        let path = path.clone();
        std::thread::spawn(move || {
            if let Err(e) = kuro_db::schema_unified::open_file(&path).and_then(|c| kuro_db::maintenance::vacuum(&c)) {
                tracing::warn!(error = %e, "VACUUM failed");
            }
        });
    });
    let body = crate::ui::message(&tr(MsgKey::ui_this_can_take_a_while_the_app_stays_usable_b));
    crate::ui::dialog(&tr(MsgKey::ui_compact_the_database_to_free_the_space), &body, 420, vec![DialogButton::cancel(&tr(MsgKey::action_not_now)), compact]).open();
}

/// Advanced > "Clear WebView data" (T-12a-034): the toast text for the outcome of `clear_data`.
fn clear_webview_message(result: &std::io::Result<()>) -> String {
    use kuro_i18n::{MsgKey, tr};
    tr(if result.is_ok() { MsgKey::webview_data_deleted } else { MsgKey::cache_delete_error })
}

/// Deletes the WebKit data and cache directories and shows the outcome. The failure is logged by `clear_data`.
pub fn clear_webview_data() {
    let result = kuro_platform::system_paths::SystemPaths::system()
        .map_err(|e| {
            tracing::error!(error = %e, "Error occurred while clearing WebView data");
            std::io::Error::other(e)
        })
        .and_then(|paths| kuro_platform::webview::data::clear_data(&paths));
    crate::toast::notify(clear_webview_message(&result));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_database_rows_sort_by_source_name_ignoring_case() {
        let rows = vec![(1, "zeta".to_owned(), 3), (2, "Alpha".to_owned(), 1), (3, "beta".to_owned(), 2)];
        let names: Vec<String> = sorted_by_name(rows).into_iter().map(|(_, n, _)| n).collect();
        assert_eq!(names, ["Alpha", "beta", "zeta"]);
    }

    #[test]
    fn webview_clear_outcome_messages() {
        assert_eq!(clear_webview_message(&Ok(())), "WebView data cleared");
        let err = Err(std::io::Error::other("denied"));
        assert_eq!(clear_webview_message(&err), "Error occurred while clearing");
    }
}
