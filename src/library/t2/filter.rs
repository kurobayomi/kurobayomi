//! T2-FILTER-01 to T2-FILTER-07 and T2-TRACK-01 to T2-TRACK-05: the filter engine and the tracker filter.

use std::collections::BTreeSet;

use kuro_domain::library::filter::{EntryFacts, LibraryFilters};
use kuro_domain::library::model::LibraryRow;
use kuro_prefs::tristate::TriState;

use crate::library::fixtures::row;

fn keep(filters: &LibraryFilters, row: &LibraryRow, downloads: i64, trackers: &[i64], logged_in: &[i64]) -> bool {
    let logged: BTreeSet<i64> = logged_in.iter().copied().collect();
    filters.keep(row, EntryFacts { downloads, tracker_ids: trackers }, &logged)
}

fn with(f: impl FnOnce(&mut LibraryFilters)) -> LibraryFilters {
    let mut filters = LibraryFilters::default();
    f(&mut filters);
    filters
}

#[test]
fn t2_filter_01_tri_state_cycle() {
    assert_eq!(TriState::DISABLED.next(), TriState::ENABLED_IS);
    assert_eq!(TriState::ENABLED_IS.next(), TriState::ENABLED_NOT);
    assert_eq!(TriState::ENABLED_NOT.next(), TriState::DISABLED);
}

#[test]
fn t2_filter_02_apply_truth_table() {
    for predicate in [true, false] {
        assert!(TriState::DISABLED.apply_filter(|| predicate));
        assert_eq!(TriState::ENABLED_IS.apply_filter(|| predicate), predicate);
        assert_eq!(TriState::ENABLED_NOT.apply_filter(|| predicate), !predicate);
    }
}

#[test]
fn t2_filter_03_downloaded_uses_the_cache_count_and_passes_local_entries() {
    let is = with(|f| f.downloaded = TriState::ENABLED_IS);
    let mut local = row(1, 0);
    local.source = 0;
    assert!(keep(&is, &local, 0, &[], &[]), "a local entry passes Is");
    assert!(keep(&is, &row(2, 0), 2, &[], &[]), "a cache count of 2 passes Is whatever the badge shows");
    assert!(!keep(&is, &row(3, 0), 0, &[], &[]));
}

#[test]
fn t2_filter_04_unseen_uses_the_raw_count() {
    let is = with(|f| f.unread = TriState::ENABLED_IS);
    let mut pending = row(1, 0);
    pending.total = 3;
    pending.done = 1;
    let mut finished = row(2, 0);
    finished.total = 3;
    finished.done = 3;
    assert!(keep(&is, &pending, 0, &[], &[]));
    assert!(!keep(&is, &finished, 0, &[], &[]));
}

#[test]
fn t2_filter_07_filters_are_anded() {
    let both = with(|f| {
        f.unread = TriState::ENABLED_IS;
        f.downloaded = TriState::ENABLED_IS;
    });
    let a = row(1, 0);
    let b = row(2, 0);
    let mut c = row(3, 0);
    c.done = c.total;
    assert!(keep(&both, &a, 1, &[], &[]));
    assert!(!keep(&both, &b, 0, &[], &[]));
    assert!(!keep(&both, &c, 1, &[], &[]));
}

#[test]
fn t2_track_01_no_logged_in_trackers_passes_everything() {
    let filters = with(|f| {
        f.tracking.insert(1, TriState::ENABLED_IS);
    });
    assert!(keep(&filters, &row(1, 0), 0, &[], &[]));
}

#[test]
fn t2_track_02_include_needs_the_tracker() {
    let filters = with(|f| {
        f.tracking.insert(1, TriState::ENABLED_IS);
    });
    assert!(keep(&filters, &row(1, 0), 0, &[1], &[1]));
    assert!(!keep(&filters, &row(1, 0), 0, &[2], &[1]));
}

#[test]
fn t2_track_03_exclude_drops_the_tracker() {
    let filters = with(|f| {
        f.tracking.insert(1, TriState::ENABLED_NOT);
    });
    assert!(!keep(&filters, &row(1, 0), 0, &[1], &[1]));
    assert!(keep(&filters, &row(1, 0), 0, &[2], &[1]));
}

#[test]
fn t2_track_04_exclude_wins_over_include() {
    let filters = with(|f| {
        f.tracking.insert(1, TriState::ENABLED_IS);
        f.tracking.insert(2, TriState::ENABLED_NOT);
    });
    assert!(!keep(&filters, &row(1, 0), 0, &[1, 2], &[1, 2]));
    assert!(keep(&filters, &row(1, 0), 0, &[1], &[1, 2]));
}

#[test]
fn t2_track_05_untracked_entries() {
    let include = with(|f| {
        f.tracking.insert(1, TriState::ENABLED_IS);
    });
    let exclude = with(|f| {
        f.tracking.insert(1, TriState::ENABLED_NOT);
    });
    assert!(!keep(&include, &row(1, 0), 0, &[], &[1]), "no tracks and an Is filter fails");
    assert!(keep(&exclude, &row(1, 0), 0, &[], &[1]), "no tracks and only a Not filter passes");
}
