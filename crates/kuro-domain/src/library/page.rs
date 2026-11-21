//! The library page model: group the rows by category, decide which pages exist, and attach the badge fields
//! (`getLibraryFlow`, behaviour 2.3). Pure: no database, no GTK. Sorting, filtering and search are later steps.

use std::collections::BTreeMap;

use super::model::LibraryRow;
use crate::category::model::Category;

/// Which badges the user turned on. A badge that is off yields its zero value.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BadgeSettings {
    pub download: bool,
    pub unread: bool,
    pub local: bool,
    pub language: bool,
}

/// The value of a badge count that was not computed (its badge is off): renderers can tell it from a real zero.
pub const NOT_COMPUTED: i64 = -1;

/// One cover in a page: the row plus the badge values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryItem {
    pub row: LibraryRow,
    /// Downloaded items, when the download badge is on; [`NOT_COMPUTED`] otherwise.
    pub download_count: i64,
    /// Unread (unseen) items, when the unread badge is on; [`NOT_COMPUTED`] otherwise.
    pub unread_count: i64,
    /// `source == 0`, when the local badge is on; false otherwise.
    pub is_local: bool,
    /// Language of the source, when the language badge is on; empty otherwise.
    pub source_language: String,
}

/// One tab of the library: a visible category and its items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryPage {
    pub category: Category,
    pub items: Vec<LibraryItem>,
}

/// Builds the pages. `visible` is the visible category list in `sort, _id` order; `rows` are every library row.
///
/// The Default page (id 0) is dropped when the library has rows and none of them is in Default. The check looks
/// at the rows, not at the visible list. An empty library keeps every visible category. Rows whose categories are
/// all hidden land on no page.
pub fn build_pages(
    visible: &[Category],
    rows: &[LibraryRow],
    badges: BadgeSettings,
    downloads: impl Fn(&LibraryRow) -> i64,
    language: impl Fn(i64) -> String,
) -> Vec<LibraryPage> {
    let mut by_category: BTreeMap<i64, Vec<&LibraryRow>> = BTreeMap::new();
    for row in rows {
        by_category.entry(row.category).or_default().push(row);
    }
    let default_is_hidden = !by_category.is_empty() && !by_category.contains_key(&crate::SYSTEM_CATEGORY_ID);
    visible
        .iter()
        .filter(|c| !(default_is_hidden && c.is_system()))
        .map(|category| LibraryPage {
            category: category.clone(),
            items: by_category
                .get(&category.id)
                .map(|rows| rows.iter().map(|row| item(row, badges, &downloads, &language)).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// One library card's item: `row` with the counts its badges show, each computed only while its badge is on. The
/// Library's pages and Home's covers both build their items here, so the two show the same badges.
pub fn item(
    row: &LibraryRow,
    badges: BadgeSettings,
    downloads: &impl Fn(&LibraryRow) -> i64,
    language: &impl Fn(i64) -> String,
) -> LibraryItem {
    LibraryItem {
        row: row.clone(),
        download_count: if badges.download { downloads(row) } else { NOT_COMPUTED },
        unread_count: if badges.unread { row.pending() } else { NOT_COMPUTED },
        is_local: badges.local && row.is_local(),
        source_language: if badges.language { language(row.source) } else { String::new() },
    }
}

/// Distinct entries across the pages. An entry in three categories counts once.
pub fn distinct_entry_count(pages: &[LibraryPage]) -> usize {
    let mut ids: Vec<i64> = pages.iter().flat_map(|p| p.items.iter().map(|i| i.row.id)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// Whether the library shows nothing (`isLibraryEmpty`), counted over the displayed pages.
pub fn is_library_empty(pages: &[LibraryPage]) -> bool {
    distinct_entry_count(pages) == 0
}

/// Toolbar title and optional count (`getToolbarTitle`). With category tabs the title is the library name; without
/// them it is the active page's name. The count is the distinct entry count (tabs on) or the active page's item
/// count (tabs off), and it is shown only when `show_count` is on.
pub fn toolbar_title(
    show_category_tabs: bool,
    show_count: bool,
    default_title: &str,
    active_page_title: &str,
    page_items: usize,
    distinct_count: usize,
) -> (String, Option<usize>) {
    let title = if show_category_tabs { default_title } else { active_page_title };
    let count = match (show_count, show_category_tabs) {
        (false, _) => None,
        (true, true) => Some(distinct_count),
        (true, false) => Some(page_items),
    };
    (title.to_owned(), count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(id: i64, name: &str, hidden: bool) -> Category {
        Category { id, name: name.into(), order: id, flags: 0x40, hidden }
    }

    fn row(id: i64, category: i64, source: i64) -> LibraryRow {
        LibraryRow {
            id,
            source,
            title: format!("t{id}"),
            category,
            total: 10,
            done: 4,
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

    fn pages(visible: &[Category], rows: &[LibraryRow]) -> Vec<LibraryPage> {
        build_pages(visible, rows, BadgeSettings::default(), |_| 0, |_| String::new())
    }

    fn page_ids(p: &[LibraryPage]) -> Vec<(i64, Vec<i64>)> {
        p.iter().map(|p| (p.category.id, p.items.iter().map(|i| i.row.id).collect())).collect()
    }

    #[test]
    fn default_page_is_dropped_when_no_row_is_in_default() {
        let visible = [cat(0, "", false), cat(5, "Hold", false)];
        let rows = [row(1, 5, 7), row(2, 5, 7)];
        assert_eq!(page_ids(&pages(&visible, &rows)), vec![(5, vec![1, 2])]);
    }

    #[test]
    fn default_page_stays_when_a_row_is_in_default_and_for_an_empty_library() {
        let visible = [cat(0, "", false), cat(5, "Hold", false)];
        let with_default = [row(1, 0, 7), row(2, 5, 7)];
        assert_eq!(page_ids(&pages(&visible, &with_default)).len(), 2);
        let empty = pages(&visible, &[]);
        assert_eq!(page_ids(&empty), vec![(0, vec![]), (5, vec![])]);
        assert!(is_library_empty(&empty));
    }

    #[test]
    fn entry_in_two_categories_appears_once_per_page_and_hidden_only_entries_on_none() {
        let visible = [cat(0, "", false), cat(5, "Hold", false), cat(6, "Read", false)];
        let hidden_only = [row(3, 9, 7)];
        let rows = [row(1, 5, 7), row(1, 6, 7), row(3, 9, 7), row(2, 0, 7)];
        let p = pages(&visible, &rows);
        assert_eq!(page_ids(&p), vec![(0, vec![2]), (5, vec![1]), (6, vec![1])]);
        assert_eq!(distinct_entry_count(&p), 2, "entry 1 counts once, entry 3 is on no page");
        assert!(!is_library_empty(&p));
        assert!(!pages(&visible, &hidden_only).iter().any(|p| !p.items.is_empty()));
    }

    #[test]
    fn hidden_categories_are_not_pages_but_their_rows_do_not_hide_default() {
        let visible = [cat(5, "Hold", false)];
        let rows = [row(1, 0, 7), row(2, 9, 7)];
        let p = pages(&visible, &rows);
        assert_eq!(page_ids(&p), vec![(5, vec![])]);
    }

    #[test]
    fn badges_follow_their_settings() {
        let visible = [cat(0, "", false)];
        let rows = [row(1, 0, 0)];
        let on = BadgeSettings { download: true, unread: true, local: true, language: true };
        let p = build_pages(&visible, &rows, on, |_| 3, |s| format!("lang{s}"));
        let item = &p[0].items[0];
        assert_eq!((item.download_count, item.unread_count, item.is_local), (3, 6, true));
        assert_eq!(item.source_language, "lang0");
        let off = build_pages(&visible, &rows, BadgeSettings::default(), |_| 3, |_| "x".into());
        let item = &off[0].items[0];
        assert_eq!((item.download_count, item.unread_count, item.is_local), (NOT_COMPUTED, NOT_COMPUTED, false));
        assert_eq!(item.source_language, "");
    }

    #[test]
    fn toolbar_title_has_four_combinations() {
        assert_eq!(toolbar_title(true, false, "Manga", "Default", 4, 12), ("Manga".into(), None));
        assert_eq!(toolbar_title(true, true, "Manga", "Default", 4, 12), ("Manga".into(), Some(12)));
        assert_eq!(toolbar_title(false, false, "Manga", "Default", 4, 12), ("Default".into(), None));
        assert_eq!(toolbar_title(false, true, "Manga", "Default", 4, 12), ("Default".into(), Some(4)));
    }
}
