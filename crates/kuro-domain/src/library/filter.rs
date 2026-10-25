//! Library filter engine (`applyFilters`, behaviour 2.4). Pure: the caller supplies what the row does not carry
//! (the download count and the tracker ids of the entry) and the set of logged-in tracker ids.
//!
//! Filters run in the Kotlin short-circuit order: downloaded, unread, started, bookmarked, completed,
//! interval-custom, tracking. The tracking rule only sees trackers that are logged in.

use std::collections::{BTreeMap, BTreeSet};

use super::model::LibraryRow;
use crate::flags::{TriState, apply_filter};

/// `SManga.COMPLETED` (and `SAnime.COMPLETED`).
pub use crate::entries::flags::STATUS_COMPLETED as COMPLETED_STATUS;

/// The filter preferences of one kind, already read from `LibraryPrefs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryFilters {
    pub downloaded: TriState,
    pub unread: TriState,
    pub started: TriState,
    pub bookmarked: TriState,
    pub completed: TriState,
    pub interval_custom: TriState,
    /// `filter_tracked_*` per tracker id. Entries for trackers that are not logged in are ignored.
    pub tracking: BTreeMap<i64, TriState>,
    /// The global "downloaded only" switch (`pref_downloaded_only`), which forces the downloaded filter.
    pub downloaded_only: bool,
    /// `manga_outside_release_period` is in the item restrictions; the interval filter only applies then.
    pub skip_outside_release_period: bool,
}

impl Default for LibraryFilters {
    fn default() -> Self {
        Self {
            downloaded: TriState::DISABLED,
            unread: TriState::DISABLED,
            started: TriState::DISABLED,
            bookmarked: TriState::DISABLED,
            completed: TriState::DISABLED,
            interval_custom: TriState::DISABLED,
            tracking: BTreeMap::new(),
            downloaded_only: false,
            skip_outside_release_period: false,
        }
    }
}

/// The facts about one entry that the filters need besides its row.
#[derive(Clone, Copy, Debug)]
pub struct EntryFacts<'a> {
    /// Downloaded chapters or episodes on disk (`downloadManager.getDownloadCount`).
    pub downloads: i64,
    /// Every tracker id the entry is tracked on, logged in or not.
    pub tracker_ids: &'a [i64],
}

impl LibraryFilters {
    /// The downloaded filter the list actually applies: forced to `ENABLED_IS` by "downloaded only".
    pub fn effective_downloaded(&self) -> TriState {
        if self.downloaded_only { TriState::ENABLED_IS } else { self.downloaded }
    }

    /// `hasActiveFilters`: the raw stored values, so a forced "downloaded only" does not count. The interval
    /// filter counts only while its restriction is on (fix Q21).
    pub fn has_active(&self) -> bool {
        let interval = self.skip_outside_release_period && self.interval_custom != TriState::DISABLED;
        interval
            || [self.downloaded, self.unread, self.started, self.bookmarked, self.completed]
                .iter()
                .any(|f| *f != TriState::DISABLED)
            || self.tracking.values().any(|f| *f != TriState::DISABLED)
    }

    /// `applyFilters` for one entry: true when the entry stays on its page.
    pub fn keep(&self, row: &LibraryRow, facts: EntryFacts<'_>, logged_in: &BTreeSet<i64>) -> bool {
        apply_filter(self.effective_downloaded(), || row.is_local() || facts.downloads > 0)
            && apply_filter(self.unread, || row.pending() > 0)
            && apply_filter(self.started, || row.has_started())
            && apply_filter(self.bookmarked, || row.has_bookmarks())
            && apply_filter(self.completed, || row.status == COMPLETED_STATUS)
            && (!self.skip_outside_release_period || apply_filter(self.interval_custom, || row.fetch_interval < 0))
            && self.tracking_passes(facts.tracker_ids, logged_in)
    }

    /// The tracking rule: an entry on an excluded tracker is out; with any included tracker, the entry must be
    /// on one of them. Only logged-in trackers take part; with none active the rule has no effect.
    fn tracking_passes(&self, entry_trackers: &[i64], logged_in: &BTreeSet<i64>) -> bool {
        let active = self.tracking.iter().filter(|(id, _)| logged_in.contains(id));
        let mut excluded = Vec::new();
        let mut included = Vec::new();
        for (id, state) in active {
            match state {
                TriState::ENABLED_NOT => excluded.push(*id),
                TriState::ENABLED_IS => included.push(*id),
                TriState::DISABLED => {}
            }
        }
        if excluded.is_empty() && included.is_empty() {
            return true;
        }
        let is_excluded = entry_trackers.iter().any(|t| excluded.contains(t));
        let is_included = included.is_empty() || entry_trackers.iter().any(|t| included.contains(t));
        !is_excluded && is_included
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, total: i64, done: i64) -> LibraryRow {
        LibraryRow {
            id,
            source: 7,
            title: format!("t{id}"),
            category: 0,
            total,
            done,
            bookmarks: 0,
            fillermarks: 0,
            latest_upload: 0,
            item_fetched_at: 0,
            last_activity: 0,
            last_update: 0,
            date_added: 0,
            status: 1,
            fetch_interval: 0,
            viewer_flags: 0,
            always_update: true,
            next_update: 0,
        }
    }

    /// The fixture of `p1-edge-cases-and-tests.md` 2.2: id, total, read, status, source.
    fn fixture() -> Vec<LibraryRow> {
        let mut alpha = row(1, 10, 10);
        alpha.status = 2;
        let mut beta = row(2, 8, 2);
        beta.bookmarks = 1;
        let gamma = row(3, 5, 0);
        let mut emile = row(4, 3, 3);
        emile.source = 0;
        vec![alpha, beta, gamma, emile]
    }

    fn ids(
        filters: &LibraryFilters,
        logged_in: &BTreeSet<i64>,
        rows: &[LibraryRow],
        tracks: &BTreeMap<i64, Vec<i64>>,
    ) -> Vec<i64> {
        rows.iter()
            .filter(|r| {
                let t = tracks.get(&r.id).cloned().unwrap_or_default();
                filters.keep(r, EntryFacts { downloads: 0, tracker_ids: &t }, logged_in)
            })
            .map(|r| r.id)
            .collect()
    }

    #[test]
    fn tri_state_next_cycles_and_disabled_passes_without_calling_the_predicate() {
        assert_eq!(TriState::DISABLED.next(), TriState::ENABLED_IS);
        assert_eq!(TriState::ENABLED_IS.next(), TriState::ENABLED_NOT);
        assert_eq!(TriState::ENABLED_NOT.next(), TriState::DISABLED);
        let f = LibraryFilters::default();
        assert!(f.keep(&row(1, 1, 0), EntryFacts { downloads: 0, tracker_ids: &[] }, &BTreeSet::new()));
    }

    #[test]
    fn unread_is_and_started_not_match_the_fixture() {
        let rows = fixture();
        let none = BTreeMap::new();
        let logged = BTreeSet::new();
        let unread = LibraryFilters { unread: TriState::ENABLED_IS, ..Default::default() };
        assert_eq!(ids(&unread, &logged, &rows, &none), vec![2, 3]);
        let started_not = LibraryFilters { started: TriState::ENABLED_NOT, ..Default::default() };
        assert_eq!(ids(&started_not, &logged, &rows, &none), vec![3]);
        let completed = LibraryFilters { completed: TriState::ENABLED_IS, ..Default::default() };
        assert_eq!(ids(&completed, &logged, &rows, &none), vec![1]);
    }

    #[test]
    fn downloaded_keeps_local_entries_and_downloaded_only_forces_is() {
        let rows = fixture();
        let logged = BTreeSet::new();
        let f = LibraryFilters { downloaded: TriState::ENABLED_IS, ..Default::default() };
        let local_and_downloaded: Vec<i64> = rows
            .iter()
            .filter(|r| f.keep(r, EntryFacts { downloads: if r.id == 2 { 3 } else { 0 }, tracker_ids: &[] }, &logged))
            .map(|r| r.id)
            .collect();
        assert_eq!(local_and_downloaded, vec![2, 4], "download count > 0 or source 0");
        let forced = LibraryFilters { downloaded: TriState::DISABLED, downloaded_only: true, ..Default::default() };
        assert_eq!(forced.effective_downloaded(), TriState::ENABLED_IS);
        assert!(!forced.has_active(), "a forced downloaded-only does not count as active");
    }

    #[test]
    fn bookmarked_uses_the_bookmark_count() {
        let rows = fixture();
        let f = LibraryFilters { bookmarked: TriState::ENABLED_IS, ..Default::default() };
        assert_eq!(ids(&f, &BTreeSet::new(), &rows, &BTreeMap::new()), vec![2]);
    }

    #[test]
    fn interval_custom_is_ignored_without_the_release_period_restriction() {
        let mut custom = row(9, 1, 0);
        custom.fetch_interval = -3;
        let on = LibraryFilters {
            interval_custom: TriState::ENABLED_IS,
            skip_outside_release_period: true,
            ..Default::default()
        };
        let plain = row(8, 1, 0);
        assert!(on.keep(&custom, EntryFacts { downloads: 0, tracker_ids: &[] }, &BTreeSet::new()));
        assert!(!on.keep(&plain, EntryFacts { downloads: 0, tracker_ids: &[] }, &BTreeSet::new()));
        let off = LibraryFilters { interval_custom: TriState::ENABLED_IS, ..Default::default() };
        assert!(off.keep(&plain, EntryFacts { downloads: 0, tracker_ids: &[] }, &BTreeSet::new()));
        assert!(!off.has_active(), "Q21: the interval filter counts only with its restriction");
        assert!(on.has_active());
    }

    #[test]
    fn tracking_rules_follow_the_excluded_and_included_sets() {
        let rows = fixture();
        let logged: BTreeSet<i64> = [1, 2].into_iter().collect();
        let f = LibraryFilters {
            tracking: BTreeMap::from([(1, TriState::ENABLED_IS), (2, TriState::ENABLED_NOT)]),
            ..Default::default()
        };
        let tracks = BTreeMap::from([(1, vec![1, 2]), (2, vec![1]), (3, vec![])]);
        assert_eq!(ids(&f, &logged, &rows, &tracks), vec![2], "excluded wins; 4 has no track so it is out");
        let untracked_only = BTreeMap::from([(2, vec![1])]);
        assert_eq!(ids(&f, &logged, &rows, &untracked_only), vec![2]);
    }

    #[test]
    fn tracking_is_ignored_without_logged_in_trackers() {
        let rows = fixture();
        let f = LibraryFilters { tracking: BTreeMap::from([(1, TriState::ENABLED_IS)]), ..Default::default() };
        assert_eq!(ids(&f, &BTreeSet::new(), &rows, &BTreeMap::new()).len(), 4);
        assert!(f.has_active(), "the raw preference counts even while no tracker is logged in");
    }
}
