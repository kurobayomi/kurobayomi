//! The one row builder of the app. Every list row of the settings pages and of the spec screens is made here, on
//! libadwaita's rows: `adw::ActionRow` (prefix icon, title, subtitle, suffix widgets), `adw::SwitchRow`,
//! `adw::ComboRow` and `adw::EntryRow`, grouped in an `adw::PreferencesGroup`. Spacing, height, focus and the hover
//! look come from libadwaita; nothing here sets a pixel size.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{Image, ListBoxRow};

/// What an action row shows. `subtitle` is already formatted; blank text hides the subtitle (T-E22).
pub struct RowSpec<'a> {
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    pub icon: Option<&'a str>,
    /// Widgets at the end of the row.
    pub trailing: Option<&'a gtk::Widget>,
    /// The row is activatable only when a click handler is given (T-E21).
    pub on_click: Option<Rc<dyn Fn()>>,
}

/// Rows read their text literally: a title with an ampersand or angle bracket is not markup.
fn plain_text(row: &impl IsA<adw::PreferencesRow>) {
    row.set_use_markup(false);
}

/// A leading icon: a decoration, so it is not announced.
fn prefix_icon(row: &impl IsA<adw::ActionRow>, icon: Option<&str>) {
    if let Some(name) = icon {
        let image = Image::from_icon_name(name);
        image.set_accessible_role(gtk::AccessibleRole::Presentation);
        row.add_prefix(&image);
    }
}

/// An action row: icon, title, subtitle, a trailing widget, and a click, Enter or Space handler.
pub fn build(spec: RowSpec) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    plain_text(&row);
    row.set_title(spec.title);
    set_subtitle(&row, spec.subtitle);
    prefix_icon(&row, spec.icon);
    if let Some(widget) = spec.trailing {
        widget.set_valign(gtk::Align::Center);
        row.add_suffix(widget);
    }
    row.set_activatable(spec.on_click.is_some());
    if let Some(on_click) = spec.on_click {
        crate::widgets::row_activate::on_activate(&row, move |_| on_click());
    }
    row
}

/// A title, a subtitle and a trailing widget, with no handler: what a page of plain facts and controls is made of.
pub fn plain(title: &str, subtitle: Option<&str>, trailing: Option<&gtk::Widget>) -> adw::ActionRow {
    build(RowSpec { title, subtitle, icon: None, trailing, on_click: None })
}

/// An info block: an icon beside dim text, not activatable. Blank text gives no row (T-E43).
pub fn info(text: &str) -> Option<adw::ActionRow> {
    if text.trim().is_empty() {
        return None;
    }
    let row = build(RowSpec { title: "", subtitle: Some(text), icon: Some("dialog-information-symbolic"), trailing: None, on_click: None });
    Some(row)
}

/// A switch row. A click, Enter or Space on the row flips the switch.
pub fn switch(title: &str, subtitle: Option<&str>, icon: Option<&str>, active: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    plain_text(&row);
    row.set_title(title);
    set_subtitle(&row, subtitle);
    prefix_icon(&row, icon);
    row.set_active(active);
    row
}

/// A row that picks one of `labels` from a drop-down list.
pub fn choice(title: &str, subtitle: Option<&str>, labels: &[&str], selected: usize) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    plain_text(&row);
    row.set_title(title);
    set_subtitle(&row, subtitle);
    row.set_model(Some(&gtk::StringList::new(labels)));
    row.set_selected(u32::try_from(selected).unwrap_or(0));
    row
}

/// A row with a text field in it.
pub fn entry(title: &str, text: &str) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    plain_text(&row);
    row.set_title(title);
    row.set_text(text);
    row
}

/// A titled group of rows (a blank title gives a group with no heading). Rows go in with `add`.
pub fn group(title: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    if !title.trim().is_empty() {
        group.set_title(title);
    }
    group
}

/// Shows `text` as the row's subtitle, or hides the subtitle when the text is missing or blank.
pub fn set_subtitle(row: &impl IsA<adw::ActionRow>, text: Option<&str>) {
    row.set_subtitle(text.filter(|t| !t.trim().is_empty()).unwrap_or_default());
}

/// Puts a row that is not a libadwaita row (a custom widget) in a list row.
pub fn wrap(widget: &impl IsA<gtk::Widget>) -> ListBoxRow {
    match widget.dynamic_cast_ref::<ListBoxRow>() {
        Some(row) => row.clone(),
        None => {
            let row = ListBoxRow::new();
            row.set_selectable(false);
            row.set_activatable(false);
            row.set_child(Some(widget));
            row
        }
    }
}
