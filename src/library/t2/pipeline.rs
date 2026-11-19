//! T2-PIPE-01 to T2-PIPE-13: the pipeline from rows to pages (`query::compute`, `page::build_pages`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use kuro_domain::category::model::Category;
use kuro_platform::kind::Kind;
use kuro_domain::library::active::clamp_page;
use kuro_domain::library::filter::LibraryFilters;
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::page::{BadgeSettings, NOT_COMPUTED, build_pages};
use kuro_prefs::tristate::TriState;

use crate::library::fixtures::row;
use crate::library::layout::tabs_visible;
use crate::library::query::{Built, Facts, Request, Snapshot, SourceInfo, compute};

const ASC_ALPHA: i64 = 0x40;

fn cat(id: i64) -> Category {
    Category { id, name: format!("c{id}"), order: id, flags: ASC_ALPHA, hidden: false }
}

fn facts(downloads: i64) -> Facts {
    Facts {
        downloads: Arc::new(move |_| downloads),
        source: Arc::new(|_| SourceInfo { name: "Src".into(), language: "en".into() }),
    }
}

fn request() -> Request {
    Request {
        kind: Kind::Manga,
        filters: LibraryFilters::default(),
        badges: BadgeSettings::default(),
        query: None,
        random_seed: 0,
        logged_in: BTreeSet::new(),
    }
}

fn snapshot(categories: Vec<Category>, rows: Vec<LibraryRow>) -> Snapshot {
    Snapshot { categories, rows, details: HashMap::new(), tracks: BTreeMap::new(), community: HashMap::new() }
}

fn ids(built: &Built) -> Vec<Vec<i64>> {
    built.pages.iter().map(|p| p.items.iter().map(|i| i.row.id).collect()).collect()
}

#[test]
fn t2_pipe_01_a_badge_off_leaves_the_count_unset_but_the_filter_still_reads_the_cache() {
    let snap = snapshot(vec![cat(1)], vec![row(1, 1)]);
    let mut req = request();
    req.badges = BadgeSettings { download: false, unread: false, local: false, language: false };
    req.filters.downloaded = TriState::ENABLED_IS;
    let built = compute(&req, &snap, &facts(2));
    assert_eq!(ids(&built), vec![vec![1]], "the cache count of 2 still satisfies the filter");
    let item = &built.pages[0].items[0];
    assert_eq!(item.download_count, NOT_COMPUTED, "badge off: the count is not computed");
    assert_eq!(item.unread_count, NOT_COMPUTED);
    // the raw unseen count still drives the unread filter
    let mut unread = request();
    unread.badges.unread = false;
    unread.filters.unread = TriState::ENABLED_IS;
    assert_eq!(ids(&compute(&unread, &snap, &facts(0))), vec![vec![1]]);
}

#[test]
fn t2_pipe_02_grouping_keeps_the_database_order_within_a_category() {
    let rows = vec![row(3, 1), row(1, 1), row(2, 1)];
    let pages = build_pages(&[cat(1)], &rows, BadgeSettings::default(), |_| 0, |_| String::new());
    assert_eq!(pages[0].items.iter().map(|i| i.row.id).collect::<Vec<_>>(), vec![3, 1, 2]);
}

#[test]
fn t2_pipe_03_a_local_entry_passes_the_downloaded_filter() {
    let mut local = row(1, 1);
    local.source = 0;
    let snap = snapshot(vec![cat(1)], vec![local, row(2, 1)]);
    let mut req = request();
    req.filters.downloaded = TriState::ENABLED_IS;
    assert_eq!(ids(&compute(&req, &snap, &facts(0))), vec![vec![1]]);
}

#[test]
fn t2_pipe_04_an_entry_in_two_categories_is_on_both_pages_and_counts_once() {
    let snap = snapshot(vec![cat(1), cat(2)], vec![row(1, 1), row(1, 2)]);
    let built = compute(&request(), &snap, &facts(0));
    assert_eq!(ids(&built), vec![vec![1], vec![1]]);
    assert_eq!(built.distinct_count, 1);
}

#[test]
fn t2_pipe_05_the_interval_gate_holds_through_the_whole_pipeline() {
    let mut custom = row(1, 1);
    custom.fetch_interval = -2;
    let snap = snapshot(vec![cat(1)], vec![custom, row(2, 1)]);
    let mut req = request();
    req.filters.interval_custom = TriState::ENABLED_IS;
    assert_eq!(ids(&compute(&req, &snap, &facts(0))), vec![vec![1, 2]], "restriction off: ignored");
    req.filters.skip_outside_release_period = true;
    assert_eq!(ids(&compute(&req, &snap, &facts(0))), vec![vec![1]]);
}

#[test]
fn t2_pipe_06_an_entry_only_in_a_hidden_category_is_on_no_page() {
    // hidden categories are not in the visible list the pipeline receives
    let snap = snapshot(vec![cat(1)], vec![row(1, 9)]);
    let built = compute(&request(), &snap, &facts(0));
    assert!(built.pages.iter().all(|p| p.items.is_empty()));
    assert_eq!(built.distinct_count, 0);
}

#[test]
fn t2_pipe_07_downloaded_only_alone_counts_as_an_active_filter() {
    let snap = snapshot(vec![cat(1)], vec![row(1, 1)]);
    let mut req = request();
    req.filters.downloaded_only = true;
    assert!(compute(&req, &snap, &facts(0)).has_active_filters);
}

#[test]
fn t2_pipe_08_a_saved_page_index_past_the_end_clamps_to_the_last_page() {
    assert_eq!(clamp_page(9, 3), 2);
    assert_eq!(clamp_page(2, 3), 2);
    assert_eq!(clamp_page(-4, 3), 0);
    assert_eq!(clamp_page(5, 0), 0);
}

#[test]
fn t2_pipe_09_tabs_show_for_the_setting_or_an_active_search() {
    for (setting, search) in [(true, true), (true, false), (false, true), (false, false)] {
        assert_eq!(tabs_visible(setting, search), setting || search);
    }
}

#[test]
fn t2_pipe_10_a_logged_out_tracker_is_ignored_until_it_logs_in_again() {
    let mut snap = snapshot(vec![cat(1)], vec![row(1, 1), row(2, 1)]);
    snap.tracks.insert(1, vec![(10, 8.0)]);
    let mut req = request();
    req.filters.tracking.insert(10, TriState::ENABLED_IS);
    assert_eq!(ids(&compute(&req, &snap, &facts(0))), vec![vec![1, 2]], "logged out: no effect");
    req.logged_in = BTreeSet::from([10]);
    assert_eq!(ids(&compute(&req, &snap, &facts(0))), vec![vec![1]], "logged in again: restored");
}

#[test]
fn t2_pipe_12_no_visible_category_with_rows_means_no_pages_and_the_empty_library_state() {
    let snap = snapshot(vec![], vec![row(1, 1)]);
    let built = compute(&request(), &snap, &facts(0));
    assert!(built.pages.is_empty());
    assert!(built.is_library_empty());
}

#[test]
fn t2_pipe_13_the_default_category_is_dropped_when_no_row_is_in_it() {
    let default = Category { id: 0, name: String::new(), order: -1, flags: ASC_ALPHA, hidden: false };
    let rows = vec![row(1, 1)];
    let pages = build_pages(&[default.clone(), cat(1)], &rows, BadgeSettings::default(), |_| 0, |_| String::new());
    assert_eq!(pages.iter().map(|p| p.category.id).collect::<Vec<_>>(), vec![1]);
    let kept = build_pages(&[default, cat(1)], &[row(1, 0)], BadgeSettings::default(), |_| 0, |_| String::new());
    assert_eq!(kept.iter().map(|p| p.category.id).collect::<Vec<_>>(), vec![0, 1]);
}
