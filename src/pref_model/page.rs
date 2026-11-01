//! Settings page scaffold: a scrolled column with one `adw::PreferencesGroup` per group (24 px between groups, 16 px
//! gutters, at most `ui::PAGE_WIDTH` wide and centred; the same layout as `ui::column`), and the highlight target.
//! Groups that are disabled are not built.

use adw::prelude::*;
use gtk::{ListBoxRow, PolicyType, ScrolledWindow};

use super::enabled::EnabledFlag;
use super::highlight;
use super::model::{Preference, PreferenceItem};
use super::render;
use super::widgets::{GROUP_SPACER, PAGE_MAX_WIDTH};
use crate::widgets::row;

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// A built page. `targets` maps each highlightable row title to its row (first registration wins).
pub struct Built {
    pub widget: gtk::Widget,
    pub scrolled: ScrolledWindow,
    pub targets: Vec<(String, ListBoxRow)>,
}

/// Builds `tree` without a header: the scrolled column of groups, for a caller that has a header of its own. The
/// pending highlight request is taken here, so the page that is built next shows it.
pub fn body(tree: &[Preference]) -> Built {
    let pending = highlight::take();

    // the page column of `ui::column` (its gutters), with a group-sized gap between the groups
    let content = crate::ui::column();
    content.set_spacing(GROUP_SPACER);
    content.set_hexpand(true);
    let mut targets: Vec<(String, ListBoxRow)> = Vec::new();

    let sections = tree.iter().filter(|entry| match entry {
        Preference::Group(g) => g.enabled,
        Preference::Item(_) => true,
    });
    let entries: Vec<&Preference> = sections.collect();
    let mut loose: Option<adw::PreferencesGroup> = None;
    for entry in &entries {
        match entry {
            Preference::Item(item) => {
                let group = loose.get_or_insert_with(|| new_group("", &content));
                append_item(group, item, &mut targets);
            }
            Preference::Group(group_entry) => {
                loose = None;
                let group = new_group(&group_entry.title, &content);
                let first_target = targets.len();
                for item in &group_entry.items {
                    append_item(&group, item, &mut targets);
                }
                // a request for the group's title (the restore link asks for "Backup and restore") flashes its
                // first row, because a group header is not a row
                if let Some((_, first)) = targets.get(first_target).cloned() {
                    if !group_entry.title.trim().is_empty() {
                        targets.push((group_entry.title.clone(), first));
                    }
                }
            }
        }
    }

    let scrolled = ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(PolicyType::Never);
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&crate::ui::clamp(&content, PAGE_MAX_WIDTH)));

    if let Some(key) = pending {
        let titles: Vec<&str> = targets.iter().map(|(t, _)| t.as_str()).collect();
        if let Some(index) = highlight::first_match(titles, &key) {
            highlight::reveal(&scrolled, &targets[index].1);
        }
    }

    Built { widget: scrolled.clone().upcast(), scrolled, targets }
}

fn new_group(title: &str, parent: &gtk::Box) -> adw::PreferencesGroup {
    let group = row::group(title);
    parent.append(&group);
    group
}

/// Adds the item's row. Its title is registered for highlight unless it is an info block or custom widget.
fn append_item(group: &adw::PreferencesGroup, item: &PreferenceItem, targets: &mut Vec<(String, ListBoxRow)>) {
    let Some(row) = render::item_row(item) else {
        return;
    };
    if !item.enabled() || item.enabled_flag().is_some() {
        follow_enabled(&row, item.enabled_flag());
    }
    let highlightable = !matches!(item, PreferenceItem::Info { .. } | PreferenceItem::Custom { .. });
    if highlightable {
        targets.push((item.title().to_owned(), row.clone()));
    }
    group.add(&row);
}

/// Makes the row insensitive while disabled and follows `flag`: turning it on makes the row sensitive, with no page
/// rebuild (T-47ba-004).
fn follow_enabled(row: &ListBoxRow, flag: Option<&EnabledFlag>) {
    row.set_sensitive(flag.map_or(false, EnabledFlag::get));
    let Some(flag) = flag else {
        return;
    };
    let row = row.clone();
    flag.subscribe(move |enabled| row.set_sensitive(enabled));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pref_model::model::ItemCommon;

    #[test]
    fn a_bound_disabled_row_is_insensitive_and_enables_live() {
        crate::testing::with_display(|| {
            let flag = EnabledFlag::new(false);
            let item = PreferenceItem::Text { common: ItemCommon::new("Row").with_enabled_flag(flag.clone()), on_click: None };
            let group = adw::PreferencesGroup::new();
            let mut targets = Vec::new();
            append_item(&group, &item, &mut targets);
            let row = targets.first().expect("row").1.clone();
            assert!(!row.is_sensitive());
            flag.set(true);
            assert!(row.is_sensitive());
            flag.set(false);
            assert!(!row.is_sensitive());
        });
    }

    #[test]
    fn an_always_enabled_row_is_sensitive() {
        crate::testing::with_display(|| {
            let item = PreferenceItem::Text { common: ItemCommon::new("Row"), on_click: None };
            let group = adw::PreferencesGroup::new();
            let mut targets = Vec::new();
            append_item(&group, &item, &mut targets);
            assert!(targets[0].1.is_sensitive());
        });
    }
}
