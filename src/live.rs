//! The live Categories screen (T-08a-033, T-08a-034). Rows come from the app's databases ([`crate::data`]). The screen
//! renders once, then again on the GTK main loop whenever its live query publishes (see [`crate::watch::bind_watch`]).
//! The library, history and updates screens have their own modules (`library`, `updates`).

use crate::ui::DialogOpen as _;
use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;

use gtk::glib;
use adw::prelude::*;
use gtk::{Box as GBox, Button, Label, ListBox, Orientation as O};
use kuro_domain::category::model::Category;
use kuro_platform::kind::Kind;

use crate::data;
use crate::screens::dialog_then_named;
use crate::ui::{boxed_list, column, icon_button, scrolled, screen, DialogButton, Role};
use crate::watch::bind_watch;
use crate::theme::tokens::{SPACE_XS};
use crate::theme::tokens::{SPACE_S};

/// A category write that failed is a toast; the list stays as the database has it.
fn report<T, E: std::fmt::Display>(result: Result<T, E>) {
    if let Err(e) = result {
        tracing::error!(error = %e, "a category change failed");
        crate::toast::notify(e.to_string());
    }
}

fn clear_list(list: &ListBox) {
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
}

/// The error shown when a category name is already taken (`spec/app.json`, categories screen notes).
fn duplicate_category() -> String {
    tr(MsgKey::error_category_exists)
}

/// The name dialog of the categories screen: the spec's title and buttons, with a text entry. The last button
/// calls `on_ok` with the trimmed name. An empty name keeps the dialog open, and so does a name that `taken`
/// reports as used, with [`duplicate_category()`] shown under the entry.
fn name_dialog(dialog: &str, initial: &str, taken: impl Fn(&str) -> bool + 'static, on_ok: impl Fn(String) + 'static) {
    let Some(spec) = screen("categories").dialogs.into_iter().find(|d| d.id == dialog) else { return };
    let labels = if spec.buttons.is_empty() { vec!["Cancel".to_owned(), "OK".to_owned()] } else { spec.buttons.clone() };
    let body = GBox::new(O::Vertical, SPACE_S);
    let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
    body.append(&entry);
    let error = Label::builder().css_classes(["error"]).xalign(0.0).wrap(true).visible(false).build();
    body.append(&error);
    let (cancel, ok) = (labels.first().cloned().unwrap_or_default(), labels.last().cloned().unwrap_or_default());
    let save = {
        let entry = entry.clone();
        move || {
            let name = entry.text().trim().to_owned();
            if name.is_empty() {
                return false;
            }
            if taken(&name) {
                error.set_text(&duplicate_category());
                error.set_visible(true);
                return false;
            }
            on_ok(name);
            true
        }
    };
    let win = crate::ui::dialog(&spec.title, &body, 380, vec![DialogButton::cancel(cancel), DialogButton::keep_open_unless(ok, Role::Suggested, save)]);
    win.open();
    entry.grab_focus();
}

/// One category row list of one kind. Rename, hide and delete write through the category service. With no user
/// category the list hides and `empty` (the empty state with its Add button) shows.
struct CategoryList {
    kind: Kind,
    list: ListBox,
    empty: adw::StatusPage,
}

/// Whether a user category other than `except` already has the trimmed name `new` (the same exact-name rule the
/// category service applies on create and rename; the system category is never a match).
fn user_names_taken(kind: Kind, new: &str, except: Option<i64>) -> bool {
    let all = data::app().categories(kind).repo().all().unwrap_or_default();
    taken_by_user(&all, new, except)
}

/// The pure part of [`user_names_taken`] over a list of categories.
fn taken_by_user(all: &[Category], new: &str, except: Option<i64>) -> bool {
    all.iter().any(|c| !c.is_system() && Some(c.id) != except && c.name.trim() == new)
}

impl CategoryList {
    fn render(self: &Rc<Self>) {
        let app = data::app();
        // a failed read is a toast and leaves the list as it was
        let all = match app.categories(self.kind).repo().all() {
            Ok(all) => all,
            Err(e) => return report::<(), _>(Err(e)),
        };
        clear_list(&self.list);
        let cats: Vec<_> = all.into_iter().filter(|c| !c.is_system()).collect();
        self.list.set_visible(!cats.is_empty());
        self.empty.set_visible(cats.is_empty());
        for c in cats {
            let (id, name, kind) = (c.id, c.name.clone(), self.kind);
            let edit = icon_button("document-edit-symbolic", &tr(MsgKey::action_rename_category));
            let rename_from = name.clone();
            edit.connect_clicked(move |_| {
                let app = data::app();
                let taken = move |new: &str| user_names_taken(kind, new, Some(id));
                name_dialog("category_rename", &rename_from, taken, move |new| {
                    let app = Rc::clone(&app);
                    glib::spawn_future_local(async move {
                        report(app.categories(kind).rename(id, &new).await);
                    });
                });
            });
            let hide = gtk::ToggleButton::builder()
                .icon_name(if c.hidden { "view-conceal-symbolic" } else { "view-reveal-symbolic" })
                .tooltip_text(&tr(MsgKey::action_hide))
                .active(c.hidden)
                .build();
            hide.connect_toggled(move |_| {
                let app = data::app();
                glib::spawn_future_local(async move {
                    report(app.categories(kind).toggle_hidden(id).await);
                });
            });
            let del = icon_button("user-trash-symbolic", &tr(MsgKey::action_delete));
            let delete_name = name.clone();
            del.connect_clicked(move |_| {
                dialog_then_named("categories", "category_delete", &delete_name, move || {
                    let app = data::app();
                    glib::spawn_future_local(async move {
                        report(app.categories(kind).delete(id).await);
                    });
                });
            });
            let handle = gtk::Image::from_icon_name("list-drag-handle-symbolic");
            let drag = gtk::DragSource::new();
            drag.set_actions(gtk::gdk::DragAction::MOVE);
            drag.connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&id.to_value())));
            handle.add_controller(drag);
            let actions = GBox::new(O::Horizontal, SPACE_XS);
            for button in [edit.upcast_ref::<gtk::Widget>(), hide.upcast_ref(), del.upcast_ref()] {
                actions.append(button);
            }
            let row = crate::browse::look::list_row(Some(handle.upcast()), &name, c.hidden.then_some("Hidden"), Some(actions.upcast()));
            // Dropping a dragged category onto a row moves it before that row, through the category service.
            let drop = gtk::DropTarget::new(<i64 as gtk::glib::prelude::StaticType>::static_type(), gtk::gdk::DragAction::MOVE);
            drop.connect_drop(move |_, value, _, _| {
                let Ok(dragged) = value.get::<i64>() else { return false };
                if dragged == id {
                    return false;
                }
                let app = data::app();
                glib::spawn_future_local(async move {
                    report(app.categories(kind).move_before(dragged, Some(id)).await);
                });
                true
            });
            row.add_controller(drop);
            self.list.append(&row);
        }
    }
}

/// The categories screen of one kind: a live list of the non-system categories in the page column, or the empty
/// state with an Add button. The header gets the name search and an Add button, which runs [`add_category`] too.
pub fn categories_content(kind: Kind) -> crate::screens::Built {
    let add = Button::with_label(&kuro_i18n::tr(kuro_i18n::MsgKey::action_add_category));
    add.connect_clicked(move |_| add_category(kind));
    let empty = crate::widgets::status::empty_with("tag-symbolic", &screen("categories").empty.unwrap_or_default(), None, std::slice::from_ref(&add));
    let column = column();
    column.append(&boxed_list());
    column.append(&empty);
    let list = column.first_child().and_downcast::<ListBox>().expect("the list");
    let search = crate::ui::list_search(&list);
    let view = Rc::new(CategoryList { kind, list, empty });
    view.render();
    bind_watch(data::app().categories(kind).repo().watch_all(), move |_| view.render());
    let header_add = icon_button("list-add-symbolic", &tr(MsgKey::action_add_category));
    header_add.connect_clicked(move |_| add_category(kind));
    crate::screens::Built { content: scrolled(&column).upcast(), end: vec![search.upcast(), header_add.upcast()] }
}

/// The Add category dialog of one kind; the name is created through the category service.
pub fn add_category(kind: Kind) {
    name_dialog("category_create", "", move |new| user_names_taken(kind, new, None), move |name| {
        let app = data::app();
        glib::spawn_future_local(async move {
            report(app.categories(kind).create(&name).await);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::alone::{self, descendants, pump};

    fn cat(id: i64, name: &str) -> Category {
        Category { id, name: name.to_owned(), order: id, flags: 0, hidden: false }
    }

    #[test]
    fn a_name_used_by_another_user_category_is_taken_but_its_own_name_is_not() {
        let all = [cat(0, "Default"), cat(1, "Reading"), cat(2, "Later ")];
        assert!(taken_by_user(&all, "Reading", None));
        assert!(taken_by_user(&all, "Later", None), "stored names are compared trimmed");
        assert!(!taken_by_user(&all, "Reading", Some(1)), "renaming to the current name is not a duplicate");
        assert!(!taken_by_user(&all, "Default", None), "the system category never blocks a name");
        assert!(!taken_by_user(&all, "reading", None), "names are exact, not case-folded, as in the service");
    }

    /// The dialogs open over the main window, oldest first.
    fn open_dialogs() -> Vec<adw::Dialog> {
        descendants(crate::ui::window().upcast_ref()).into_iter().filter_map(|w| w.downcast::<adw::Dialog>().ok()).collect()
    }

    /// The dialog on screen and its own buttons in order (the last one is OK).
    fn confirm() -> (adw::Dialog, Vec<Button>) {
        let window = open_dialogs().pop().expect("a dialog is on screen");
        // the dialog's own buttons have labels; the header's close button is an icon
        let content = crate::ui::dialog_content(&window).expect("the dialog content");
        let buttons = descendants(&content).into_iter().filter_map(|w| w.downcast::<Button>().ok()).filter(|b| b.label().is_some()).collect();
        (window, buttons)
    }

    /// [`confirm`] for a name dialog, with its entry.
    fn dialog() -> (adw::Dialog, gtk::Entry, Vec<Button>) {
        let (window, buttons) = confirm();
        let entry = descendants(&crate::ui::dialog_content(&window).expect("the dialog content")).into_iter().find_map(|w| w.downcast::<gtk::Entry>().ok()).expect("the name entry");
        (window, entry, buttons)
    }

    fn named(root: &gtk::Widget, tooltip: &str) -> Vec<gtk::Widget> {
        descendants(root).into_iter().filter(|w| w.tooltip_text().as_deref() == Some(tooltip)).collect()
    }

    fn row_names(root: &gtk::Widget) -> Vec<String> {
        let rows = descendants(root).into_iter().filter(|w| w.is::<gtk::ListBoxRow>());
        rows.filter_map(|row| descendants(&row).into_iter().filter_map(|w| w.downcast::<Label>().ok()).map(|l| l.text().to_string()).next()).collect()
    }

    /// Needs a display and runs in a child process with fresh app data (see [`alone`]).
    #[test]
    fn the_categories_screen_creates_renames_hides_reorders_and_deletes_and_shows_errors() {
        alone::run(module_path!(), "the_categories_screen_creates_renames_hides_reorders_and_deletes_and_shows_errors", |ctx| {
            let (_stack, _window) = crate::testing::nav();
            let toast = alone::toast_label();
            let page = categories_content(Kind::Manga).content;
            let names = || row_names(&page);
            let kind = Kind::Manga;
            let create = |name: &str| {
                add_category(Kind::Manga);
                let (window, entry, buttons) = dialog();
                entry.set_text(name);
                buttons.last().expect("OK").emit_clicked();
                (window, buttons)
            };
            assert!(names().is_empty(), "no category yet: the empty state shows");

            // create
            create("Reading");
            assert!(pump(ctx, || names() == ["Reading"]), "the new category is listed, got {:?}; stored {:?}; toast {:?}", names(), data::app().categories(kind).repo().all().map(|a| a.into_iter().map(|c| c.name).collect::<Vec<_>>()), toast.text());

            // a taken name keeps the dialog open and says why
            let (window, buttons) = create("Reading");
            let (_, _, again) = dialog();
            assert_eq!(again.len(), buttons.len(), "the dialog is still the one on screen");
            let error = descendants(window.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).find(|l| l.text() == duplicate_category()).expect("the duplicate error label");
            assert!(error.get_visible(), "the duplicate-name error shows");
            buttons.first().expect("Cancel").emit_clicked();
            assert!(pump(ctx, || open_dialogs().is_empty()), "Cancel closes the dialog");
            assert_eq!(names(), ["Reading"], "the duplicate was not created");

            // rename
            named(page.upcast_ref(), "Rename category")[0].clone().downcast::<Button>().expect("button").emit_clicked();
            let (_, entry, buttons) = dialog();
            assert_eq!(entry.text(), "Reading", "the dialog starts on the current name");
            entry.set_text("Watching");
            buttons.last().expect("OK").emit_clicked();
            assert!(pump(ctx, || names() == ["Watching"]), "the rename shows, got {:?}", names());

            // hide
            let hide = named(page.upcast_ref(), "Hide")[0].clone().downcast::<gtk::ToggleButton>().expect("toggle");
            hide.set_active(true);
            let hidden = || descendants(page.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).any(|l| l.text() == "Hidden");
            assert!(pump(ctx, hidden), "a hidden category says Hidden");
            assert!(data::app().categories(kind).repo().all().expect("read").iter().any(|c| c.name == "Watching" && c.hidden), "stored as hidden");

            // reorder: a second category, dropped before the first
            let app = data::app();
            glib::spawn_future_local(async move { report(app.categories(kind).create("Later").await) });
            assert!(pump(ctx, || names() == ["Watching", "Later"]), "the second category is listed, got {:?}", names());
            let ids = |name: &str| data::app().categories(kind).repo().all().expect("read").iter().find(|c| c.name == name).map(|c| c.id).expect("category");
            let first = descendants(page.upcast_ref()).into_iter().find(|w| w.is::<gtk::ListBoxRow>()).expect("first row");
            let target = first.observe_controllers().iter::<glib::Object>().flatten().find_map(|c| c.downcast::<gtk::DropTarget>().ok()).expect("the row takes drops");
            let dropped = target.emit_by_name::<bool>("drop", &[&glib::BoxedValue(ids("Later").to_value()), &0.0_f64, &0.0_f64]);
            assert!(dropped, "the drop is accepted");
            assert!(pump(ctx, || names() == ["Later", "Watching"]), "the dragged category moved before the other, got {:?}", names());

            // delete, after the confirmation
            named(page.upcast_ref(), "Delete")[0].clone().downcast::<Button>().expect("button").emit_clicked();
            let (window, buttons) = confirm();
            assert!(descendants(window.upcast_ref()).into_iter().filter_map(|w| w.downcast::<Label>().ok()).any(|l| l.text().contains("\"Later\"")), "the question names the category");
            buttons.last().expect("Delete").emit_clicked();
            assert!(pump(ctx, || names() == ["Watching"]), "the category is gone, got {:?}", names());

            // a failed service call is a toast
            report::<(), _>(Err("the database is locked"));
            assert!(pump(ctx, || toast.text() == "the database is locked"), "the error reaches a toast, got {:?}", toast.text());
        });
    }
}
