//! Dispatch from one settings item to its row. The match is exhaustive over the 13 variants (no `_` arm). Link rows end
//! in the dim arrow and choice rows in the dim drop-down arrow, so a row says what a click does.

use gtk::prelude::*;
use gtk::{Image, ListBoxRow};

use super::model::PreferenceItem;
use super::strings;
use super::widgets::dialog;
use super::widgets::{info, list, multi_select, edit_text, row, slider, switch, text, TRACKER_CHECK_SIZE};

/// The dim end marker of a row that opens a chooser (choice and multi-select rows): the same `pan-down` arrow a drop
/// down shows, so every choice row reads the same way.
fn choice_marker() -> gtk::Widget {
    Image::builder().icon_name("pan-down-symbolic").css_classes(["dim-label"]).build().upcast()
}

/// Puts `marker` in the row's empty trailing slot.
fn mark(built: row::PrefRow, marker: gtk::Widget) -> ListBoxRow {
    if built.trailing.first_child().is_none() {
        built.trailing.append(&marker);
    }
    built.row
}

/// The row for one item, or `None` when the item renders nothing (a blank info block).
pub fn item_row(item: &PreferenceItem) -> Option<ListBoxRow> {
    let built = match item {
        PreferenceItem::Text { common, on_click } if common.link => {
            mark(text::build(common, on_click.clone(), None), crate::ui::chevron().upcast())
        }
        PreferenceItem::Text { common, on_click } => text::build(common, on_click.clone(), None).row,
        PreferenceItem::Switch { common, pref, veto } => switch::build(common, pref.clone(), veto.clone()).row,
        PreferenceItem::Slider { common, value, range, steps, on_change } => {
            slider::row(common, *value, range.clone(), *steps, on_change.clone())
        }
        PreferenceItem::List { common, pref, entries, veto } => {
            mark(list::row(common, pref.clone(), entries.clone(), veto.clone()), choice_marker())
        }
        PreferenceItem::BasicList { common, value, entries, on_change } => {
            mark(list::basic_row(common, value.clone(), entries.clone(), on_change.clone()), choice_marker())
        }
        PreferenceItem::MultiSelect { common, pref, entries, veto } => {
            mark(multi_select::row(common, pref.clone(), entries.clone(), veto.clone()), choice_marker())
        }
        PreferenceItem::EditText { common, pref, dialog_subtitle, can_be_blank, validate, error_message, veto } => {
            edit_text::row(
                common,
                pref.clone(),
                dialog_subtitle.clone(),
                *can_be_blank,
                validate.clone(),
                error_message.clone(),
                veto.clone(),
            )
            .row
        }
        PreferenceItem::EditTextInfo { common, pref, dialog_subtitle, can_be_blank, validate, error_message, veto } => {
            edit_text::row(
                common,
                pref.clone(),
                dialog_subtitle.clone(),
                *can_be_blank,
                validate.clone(),
                error_message.clone(),
                veto.clone(),
            )
            .row
        }
        PreferenceItem::Tracker { name, logged_in, on_login, on_logout } => {
            tracker_row(name, *logged_in, on_login.clone(), on_logout.clone())
        }
        PreferenceItem::Info { text, .. } => return info::build(text),
        PreferenceItem::Custom { build, .. } => custom_row(build()),
    };
    Some(built)
}

fn tracker_row(name: &str, logged_in: bool, on_login: std::rc::Rc<dyn Fn()>, on_logout: std::rc::Rc<dyn Fn()>) -> ListBoxRow {
    let check = logged_in.then(|| {
        let label = strings::logged_in();
        let image = Image::from_icon_name("object-select-symbolic");
        image.set_pixel_size(TRACKER_CHECK_SIZE);
        image.set_tooltip_text(Some(&label));
        image.add_css_class("tracker-check");
        image.set_accessible_role(gtk::AccessibleRole::Img);
        image.update_property(&[gtk::accessible::Property::Label(&label)]);
        image
    });
    let trailing = check.as_ref().map(|image| image.clone().upcast::<gtk::Widget>());
    let built = row::build(row::RowSpec {
        title: name,
        subtitle: None,
        icon: None,
        trailing: trailing.as_ref(),
        on_click: Some(std::rc::Rc::new({
            let name = name.to_owned();
            move || if logged_in { confirm_logout(&name, on_logout.clone()) } else { on_login() }
        })),
    });
    if let Some(logo) = tracker_logo(name) {
        if let Some(content) = built.row.child().and_downcast::<gtk::Box>() {
            content.prepend(&logo);
        }
    }
    built.row
}

/// The bundled logo of the login row named `name`, when one is bundled.
fn tracker_logo(name: &str) -> Option<Image> {
    let id = kuro_track::login_rows::LOGIN_ROWS.iter().find(|row| row.name == name)?.tracker;
    match kuro_track::logo::logo_asset(id)? {
        kuro_track::tracker::LogoRef::Embedded(bytes) => crate::settings_ui::rows::logo_image(bytes),
        _ => None,
    }
}

/// Logging out asks first (T-47ba-050, Proposal T-E42). The login stays until the user confirms.
fn confirm_logout(name: &str, on_logout: std::rc::Rc<dyn Fn()>) {
    let body = gtk::Label::builder().label(name).xalign(0.0).build();
    dialog::show(
        &crate::ui::window(),
        &strings::log_out_of_tracker(),
        &body,
        Some(&strings::ok()),
        move |confirmed| {
            on_logout();
            confirmed.finish();
        },
        || {},
    );
}

fn custom_row(widget: gtk::Widget) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    row.set_child(Some(&widget));
    row
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use kuro_prefs::prefs::Prefs;

    use super::*;
    use crate::pref_model::model::{ItemCommon, Preference};

    /// The icon of the last image in `row`: what the row ends in.
    fn end_icon(row: &gtk::Widget) -> Option<String> {
        fn walk(w: &gtk::Widget, last: &mut Option<String>) {
            if let Some(image) = w.downcast_ref::<Image>() {
                *last = image.icon_name().map(|n| n.to_string());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                walk(&c, last);
                child = c.next_sibling();
            }
        }
        let mut last = None;
        walk(row, &mut last);
        last
    }

    fn items(tree: Vec<Preference>) -> Vec<PreferenceItem> {
        tree.into_iter()
            .flat_map(|entry| match entry {
                Preference::Item(item) => vec![item],
                Preference::Group(g) => g.items,
            })
            .collect()
    }

    fn ends_in(item: &PreferenceItem) -> Option<String> {
        end_icon(item_row(item).expect("a row").upcast_ref())
    }

    /// Needs a display, and skips itself without one.
    #[test]
    fn link_rows_end_in_the_arrow_and_choice_rows_in_the_drop_down_marker() {
        crate::testing::with_display(|| {
            let link = PreferenceItem::Text { common: ItemCommon::new("Reader").with_link(), on_click: Some(Rc::new(|| {})) };
            assert_eq!(ends_in(&link).as_deref(), Some("go-next-symbolic"));
            let action = PreferenceItem::Text { common: ItemCommon::new("Clear"), on_click: Some(Rc::new(|| {})) };
            assert_ne!(ends_in(&action).as_deref(), Some("go-next-symbolic"), "an action row has no arrow");
            let prefs = Prefs::in_memory();
            let lists: Vec<PreferenceItem> = items(crate::settings_ui::downloads::page_rows(&prefs))
                .into_iter()
                .chain(items(crate::settings_ui::library::page_rows(&prefs, &[], &[], Rc::new(|_| {}))))
                .filter(|i| matches!(i, PreferenceItem::List { .. } | PreferenceItem::BasicList { .. } | PreferenceItem::MultiSelect { .. }))
                .collect();
            assert!(!lists.is_empty());
            for list in &lists {
                assert_eq!(ends_in(list).as_deref(), Some("pan-down-symbolic"), "{}", list.title());
            }
        });
    }

    /// Every typed row that opens a page ends in the arrow, like the spec link rows.
    #[test]
    fn rows_that_open_a_page_end_in_the_arrow() {
        crate::testing::with_display(|| {
            let prefs = Prefs::in_memory();
            let browse = items(crate::settings_ui::browse::page_rows(&prefs, crate::settings_ui::browse::RepoCounts::default()));
            let debug = crate::pref_model::debug_info::tree().into_iter().filter_map(|e| match e {
                Preference::Item(item) => Some(item),
                Preference::Group(_) => None,
            });
            let mut opening = 0;
            for item in browse.iter().chain(debug.collect::<Vec<_>>().iter()) {
                let PreferenceItem::Text { on_click: Some(on_click), .. } = item else { continue };
                if crate::ui::opened_during(|| on_click()).is_empty() {
                    continue;
                }
                opening += 1;
                assert_eq!(ends_in(item).as_deref(), Some("go-next-symbolic"), "{}", item.title());
            }
            assert!(opening >= 3, "the two repo rows and the debug info links");
            let library = items(crate::settings_ui::library::page_rows(&prefs, &[], &[], Rc::new(|_| {})));
            for title in ["Edit anime categories", "Edit manga categories"] {
                let item = library.iter().find(|i| i.title() == title).unwrap_or_else(|| panic!("{title}"));
                assert_eq!(ends_in(item).as_deref(), Some("go-next-symbolic"), "{title}");
            }
        });
    }
}
