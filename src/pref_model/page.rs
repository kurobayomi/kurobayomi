//! Settings page scaffold: a scrolled column with one boxed list per group (a `heading` above it, 24 px
//! between groups, 16 px gutters, at most `ui::PAGE_WIDTH` wide and centred; the same layout as `ui::column`), and
//! the highlight target. Groups that are disabled are not built.

use gtk::prelude::*;
use gtk::{Box as GBox, ListBox, ListBoxRow, Orientation, PolicyType, Revealer, RevealerTransitionType, ScrolledWindow, SelectionMode};

use super::enabled::EnabledFlag;
use super::highlight;
use super::model::{Preference, PreferenceItem};
use super::render;
use super::widgets::{group, GROUP_SPACER, PAGE_MAX_WIDTH};

/// Slide-in time of a row that becomes enabled.
const REVEAL_MS: u32 = 150;

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
    let mut loose: Option<ListBox> = None;
    for entry in &entries {
        match entry {
            Preference::Item(item) => {
                let list = loose.get_or_insert_with(|| new_list(&content));
                append_item(list, item, &mut targets);
            }
            Preference::Group(group_entry) => {
                loose = None;
                let block = GBox::new(Orientation::Vertical, 0);
                if let Some(header) = group::header(&group_entry.title) {
                    block.append(&header);
                }
                let list = new_list(&block);
                for item in &group_entry.items {
                    append_item(&list, item, &mut targets);
                }
                // a request for the group's title (the restore link asks for "Backup and restore") flashes its
                // first row, because a group header is not a row
                if let Some(first) = list.row_at_index(0) {
                    if !group_entry.title.trim().is_empty() {
                        targets.push((group_entry.title.clone(), first));
                    }
                }
                content.append(&block);
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

fn new_list(parent: &GBox) -> ListBox {
    let list = ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(SelectionMode::None);
    parent.append(&list);
    list
}

/// Adds the item's row. Its title is registered for highlight unless it is an info block or custom widget.
fn append_item(list: &ListBox, item: &PreferenceItem, targets: &mut Vec<(String, ListBoxRow)>) {
    let Some(row) = render::item_row(item) else {
        return;
    };
    if !item.enabled() || item.enabled_flag().is_some() {
        animate_enabled(&row, item.enabled_flag());
    }
    let highlightable = !matches!(item, PreferenceItem::Info { .. } | PreferenceItem::Custom { .. });
    if highlightable {
        targets.push((item.title().to_owned(), row.clone()));
    }
    list.append(&row);
}

/// Wraps the row's content in a slide-down `Revealer` (150 ms), makes the row insensitive while disabled, and follows
/// `flag`: turning it on makes the row sensitive and slides it in, with no page rebuild (T-47ba-004).
fn animate_enabled(row: &ListBoxRow, flag: Option<&EnabledFlag>) {
    row.set_sensitive(flag.map_or(false, EnabledFlag::get));
    let Some(child) = row.child() else {
        return;
    };
    row.set_child(None::<&gtk::Widget>);
    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .transition_duration(REVEAL_MS)
        .reveal_child(true)
        .child(&child)
        .build();
    row.set_child(Some(&revealer));
    let Some(flag) = flag else {
        return;
    };
    let row = row.clone();
    flag.subscribe(move |enabled| {
        row.set_sensitive(enabled);
        if enabled {
            revealer.set_transition_duration(0);
            revealer.set_reveal_child(false);
            revealer.set_transition_duration(REVEAL_MS);
            revealer.set_reveal_child(true);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pref_model::model::ItemCommon;

    #[test]
    fn a_bound_disabled_row_is_insensitive_in_a_revealer_and_enables_live() {
        crate::testing::with_display(|| {
            let flag = EnabledFlag::new(false);
            let item = PreferenceItem::Text { common: ItemCommon::new("Row").with_enabled_flag(flag.clone()), on_click: None };
            let list = ListBox::new();
            let mut targets = Vec::new();
            append_item(&list, &item, &mut targets);
            let row = list.row_at_index(0).expect("row");
            let revealer = row.child().and_downcast::<Revealer>().expect("revealer");
            assert_eq!(revealer.transition_type(), RevealerTransitionType::SlideDown);
            assert_eq!(revealer.transition_duration(), 150);
            assert!(!row.is_sensitive());
            flag.set(true);
            assert!(row.is_sensitive());
            assert!(revealer.reveals_child());
            flag.set(false);
            assert!(!row.is_sensitive());
        });
    }

    #[test]
    fn an_always_enabled_row_is_not_wrapped() {
        crate::testing::with_display(|| {
            let item = PreferenceItem::Text { common: ItemCommon::new("Row"), on_click: None };
            let list = ListBox::new();
            append_item(&list, &item, &mut Vec::new());
            let row = list.row_at_index(0).expect("row");
            assert!(row.child().and_downcast::<Revealer>().is_none());
            assert!(row.is_sensitive());
        });
    }
}
