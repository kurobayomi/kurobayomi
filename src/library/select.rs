//! Selection of library entries (behaviour 10, T-37a-025). Pure: the view feeds it the displayed page and the
//! rows it clicks. The selection keeps a copy of each selected row, so bulk actions still have the entry after the
//! page or the search changed. The rules are generic over what is selected, so Browse covers (issue #7) follow the
//! same toggle, range, select all and invert as the Library; `Selection` is the Library's.

use std::collections::HashMap;

use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::page::LibraryItem;

/// Something a selection holds: its id, and the group a range stays inside (the Library's categories).
pub trait Selectable: Clone {
    fn key(&self) -> i64;
    /// A range never crosses groups. One group by default.
    fn group(&self) -> i64 {
        0
    }
}

/// One cell of a displayed page and the entry it shows.
pub trait Shown {
    type Entry: Selectable;
    fn entry(&self) -> &Self::Entry;
}

impl Selectable for LibraryRow {
    fn key(&self) -> i64 {
        self.id
    }
    fn group(&self) -> i64 {
        self.category
    }
}

impl Shown for LibraryItem {
    type Entry = LibraryRow;
    fn entry(&self) -> &LibraryRow {
        &self.row
    }
}

/// The Library's selection.
pub type Selection = SelectionOf<LibraryRow>;

#[derive(Clone, Debug)]
pub struct SelectionOf<T> {
    order: Vec<i64>,
    rows: HashMap<i64, T>,
}

impl<T> Default for SelectionOf<T> {
    fn default() -> Self {
        Self { order: Vec::new(), rows: HashMap::new() }
    }
}

impl<T: Selectable> SelectionOf<T> {
    pub fn contains(&self, id: i64) -> bool {
        self.rows.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.rows.clear();
    }

    /// The selected rows, in the order they were selected.
    pub fn rows(&self) -> impl Iterator<Item = &T> {
        self.order.iter().filter_map(|id| self.rows.get(id))
    }

    /// The selected ids, in selection order.
    pub fn ids(&self) -> &[i64] {
        &self.order
    }

    /// Selected entries that are not on the displayed page (E-10: the header reports them).
    pub fn hidden_count<P: Shown<Entry = T>>(&self, visible: &[P]) -> usize {
        self.order.iter().filter(|id| !visible.iter().any(|item| item.entry().key() == **id)).count()
    }

    /// Plain click in selection mode: flips the entry.
    pub fn toggle(&mut self, row: &T) {
        if self.contains(row.key()) {
            self.remove(row.key());
        } else {
            self.add(row);
        }
    }

    /// Long press or right click: selects the inclusive range from the last selected entry to `row` on the
    /// displayed page. It never deselects. When the anchor and `row` are not both on the page, or they belong to
    /// different categories, only `row` is added.
    pub fn toggle_range<P: Shown<Entry = T>>(&mut self, row: &T, page: &[P]) {
        let anchor = self.order.last().and_then(|id| self.rows.get(id)).cloned();
        let Some(anchor) = anchor.filter(|a| a.group() == row.group()) else {
            self.add(row);
            return;
        };
        let from = page.iter().position(|item| item.entry().key() == anchor.key());
        let to = page.iter().position(|item| item.entry().key() == row.key());
        let (Some(from), Some(to)) = (from, to) else {
            self.add(row);
            return;
        };
        let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
        for item in &page[lo..=hi] {
            self.add(item.entry());
        }
    }

    pub fn select_all<P: Shown<Entry = T>>(&mut self, page: &[P]) {
        for item in page {
            self.add(item.entry());
        }
    }

    /// Flips every entry of the page: selected ones are removed, the others are added.
    pub fn invert<P: Shown<Entry = T>>(&mut self, page: &[P]) {
        for item in page {
            let row = item.entry();
            if self.contains(row.key()) {
                self.remove(row.key());
            } else {
                self.add(row);
            }
        }
    }

    /// Replaces each snapshot with the current row and drops entries that left the library.
    pub fn refresh(&mut self, library: &[T]) {
        let fresh: HashMap<i64, &T> = library.iter().map(|r| (r.key(), r)).collect();
        self.order.retain(|id| fresh.contains_key(id));
        self.rows.retain(|id, row| match fresh.get(id) {
            Some(current) => {
                *row = (**current).clone();
                true
            }
            None => false,
        });
    }

    fn add(&mut self, row: &T) {
        if !self.rows.contains_key(&row.key()) {
            self.order.push(row.key());
            self.rows.insert(row.key(), row.clone());
        }
    }

    fn remove(&mut self, id: i64) {
        if self.rows.remove(&id).is_some() {
            self.order.retain(|x| *x != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, category: i64) -> LibraryRow {
        LibraryRow {
            id,
            source: 7,
            title: format!("t{id}"),
            category,
            total: 10,
            done: 0,
            bookmarks: 0,
            fillermarks: 0,
            latest_upload: 0,
            item_fetched_at: 0,
            last_activity: 0,
            last_update: 0,
            date_added: 0,
            status: 0,
            fetch_interval: 0,
            viewer_flags: 0,
            always_update: true,
            next_update: 0,
        }
    }

    fn page(ids: &[(i64, i64)]) -> Vec<LibraryItem> {
        ids.iter()
            .map(|&(id, cat)| LibraryItem {
                row: row(id, cat),
                download_count: 0,
                unread_count: 0,
                is_local: false,
                source_language: String::new(),
            })
            .collect()
    }

    fn ids(sel: &Selection) -> Vec<i64> {
        sel.rows().map(|r| r.id).collect()
    }

    #[test]
    fn toggle_flips_membership_and_keeps_selection_order() {
        let mut sel = Selection::default();
        sel.toggle(&row(3, 0));
        sel.toggle(&row(1, 0));
        assert_eq!(ids(&sel), vec![3, 1]);
        sel.toggle(&row(3, 0));
        assert_eq!(ids(&sel), vec![1]);
        assert!(!sel.contains(3));
    }

    #[test]
    fn range_fills_the_inclusive_span_within_one_category_in_either_direction() {
        let p = page(&[(1, 5), (2, 5), (3, 5), (4, 5), (5, 5)]);
        let mut sel = Selection::default();
        sel.toggle(&row(2, 5));
        sel.toggle_range(&row(4, 5), &p);
        assert_eq!(ids(&sel), vec![2, 3, 4]);
        sel.clear();
        sel.toggle(&row(4, 5));
        sel.toggle_range(&row(2, 5), &p);
        assert_eq!(ids(&sel), vec![4, 2, 3], "the range is added in page order");
    }

    #[test]
    fn range_across_categories_or_missing_anchor_adds_only_the_row() {
        let p = page(&[(1, 5), (2, 6), (3, 6)]);
        let mut sel = Selection::default();
        sel.toggle(&row(1, 5));
        sel.toggle_range(&row(3, 6), &p);
        assert_eq!(ids(&sel), vec![1, 3]);
        let mut other = Selection::default();
        other.toggle(&row(99, 6));
        other.toggle_range(&row(3, 6), &p);
        assert_eq!(ids(&other), vec![99, 3]);
    }

    #[test]
    fn range_never_deselects() {
        let p = page(&[(1, 0), (2, 0), (3, 0)]);
        let mut sel = Selection::default();
        sel.toggle(&row(1, 0));
        sel.toggle(&row(3, 0));
        sel.toggle_range(&row(2, 0), &p);
        assert_eq!(sel.len(), 3);
    }

    #[test]
    fn select_all_and_invert_work_on_the_page_only() {
        let p = page(&[(1, 0), (2, 0), (3, 0)]);
        let mut sel = Selection::default();
        sel.toggle(&row(9, 0));
        sel.select_all(&p);
        assert_eq!(ids(&sel), vec![9, 1, 2, 3]);
        sel.invert(&p);
        assert_eq!(ids(&sel), vec![9]);
        sel.invert(&p);
        assert_eq!(ids(&sel), vec![9, 1, 2, 3]);
    }

    #[test]
    fn hidden_count_reports_selected_entries_off_the_page() {
        let mut sel = Selection::default();
        sel.toggle(&row(1, 0));
        sel.toggle(&row(7, 0));
        assert_eq!(sel.hidden_count(&page(&[(1, 0), (2, 0)])), 1);
        sel.clear();
        assert!(sel.is_empty());
    }

    #[test]
    fn refresh_updates_snapshots_and_drops_entries_that_left_the_library() {
        let mut sel = Selection::default();
        sel.toggle(&row(1, 0));
        sel.toggle(&row(2, 0));
        let mut changed = row(1, 0);
        changed.title = "renamed".into();
        sel.refresh(&[changed]);
        assert_eq!(ids(&sel), vec![1]);
        assert_eq!(sel.rows().next().map(|r| r.title.as_str()), Some("renamed"));
    }
}
