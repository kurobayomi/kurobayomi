//! T2-SEL-01 to T2-SEL-08: the selection set.

use kuro_domain::library::page::LibraryItem;

use crate::library::fixtures::{item, row};
use crate::library::select::Selection;

fn page(ids: &[i64]) -> Vec<LibraryItem> {
    ids.iter().map(|id| item(*id, 5)).collect()
}

fn ids(sel: &Selection) -> Vec<i64> {
    let mut v: Vec<i64> = sel.rows().map(|r| r.id).collect();
    v.sort_unstable();
    v
}

#[test]
fn t2_sel_01_toggle_twice_returns_to_empty() {
    let mut sel = Selection::default();
    sel.toggle(&row(1, 5));
    sel.toggle(&row(1, 5));
    assert!(sel.is_empty());
}

#[test]
fn t2_sel_02_a_range_with_nothing_selected_adds_only_the_clicked_row() {
    let mut sel = Selection::default();
    sel.toggle_range(&row(3, 5), &page(&[1, 2, 3, 4]));
    assert_eq!(ids(&sel), vec![3]);
}

#[test]
fn t2_sel_03_the_anchor_is_the_last_inserted_row() {
    let p = page(&[0, 1, 2, 3, 4]);
    let mut sel = Selection::default();
    sel.toggle(&row(1, 5));
    sel.toggle_range(&row(4, 5), &p);
    assert_eq!(ids(&sel), vec![1, 2, 3, 4]);
    sel.toggle_range(&row(0, 5), &p);
    assert_eq!(ids(&sel), vec![0, 1, 2, 3, 4]);
}

#[test]
fn t2_sel_04_a_stale_anchor_adds_only_the_clicked_row() {
    let mut sel = Selection::default();
    sel.toggle(&row(99, 5));
    sel.toggle_range(&row(3, 5), &page(&[1, 2, 3, 4]));
    assert_eq!(ids(&sel), vec![3, 99]);
}

#[test]
fn t2_sel_05_a_range_skips_items_the_filter_hid() {
    let visible = page(&[1, 3, 5]); // 2 and 4 are filtered out of the page
    let mut sel = Selection::default();
    sel.toggle(&row(1, 5));
    sel.toggle_range(&row(5, 5), &visible);
    assert_eq!(ids(&sel), vec![1, 3, 5]);
}

#[test]
fn t2_sel_06_select_all_adds_the_unselected_rows_and_invert_flips_the_page() {
    let p = page(&[1, 2, 3]);
    let mut sel = Selection::default();
    sel.toggle(&row(2, 5));
    sel.toggle(&row(9, 5)); // off the page
    sel.select_all(&p);
    assert_eq!(ids(&sel), vec![1, 2, 3, 9]);
    sel.clear();
    sel.toggle(&row(2, 5));
    sel.toggle(&row(9, 5));
    sel.invert(&p);
    assert_eq!(ids(&sel), vec![1, 3, 9], "only the page subset flips");
}

#[test]
fn t2_sel_07_the_selection_survives_a_search_and_counts_what_it_hides() {
    let mut sel = Selection::default();
    sel.toggle(&row(1, 5));
    sel.toggle(&row(2, 5));
    let after_search = page(&[2]); // the search filtered row 1 out
    assert_eq!(sel.len(), 2);
    assert_eq!(sel.hidden_count(&after_search), 1);
}

#[test]
fn t2_sel_08_rows_of_another_library_do_not_survive_a_refresh() {
    let mut sel = Selection::default();
    sel.toggle(&row(1, 5));
    sel.toggle(&row(500, 5)); // an id that belongs to the other kind's database
    sel.refresh(&[row(1, 5), row(2, 5)]);
    assert_eq!(ids(&sel), vec![1]);
}
