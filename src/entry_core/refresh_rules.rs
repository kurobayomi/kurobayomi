//! The source refresh rules of the manga page and the anime page, one set for both: what started a refresh, the guard
//! that stops an entry with no items from refetching on every open, which new items a manual refresh queues, and the
//! single-flight gate. Pure (the one read, `new_item_targets`, runs on the blocking pool). Upstream:
//! `MangaScreenModel.fetchChaptersFromSource` and `AnimeScreenModel.fetchEpisodesFromSource` (`manualFetch`), fix 23.24
//! (the 10-minute guard), E-REF-3, E-REF-5 and E-CONC-1.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use kuro_data::DataResult;
use kuro_data::category::CategoryRepo;
use kuro_data::entry::EntryRepo;
use kuro_data::item::{Item, ItemRepo};
use kuro_domain::download::filter::{CategoryDecision, filter_new, should_download_new};
use kuro_domain::download::prefs::DownloadPrefs;
use kuro_platform::kind::Kind;
use kuro_prefs::store::PrefStore;

/// What started a refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The refresh button or F5.
    Manual,
    /// The first open of the page.
    Open,
}

/// Only a manual refresh auto-downloads the new items (E-REF-5, `manualFetch`).
pub fn auto_download_allowed(trigger: Trigger) -> bool {
    trigger == Trigger::Manual
}

/// A zero-item entry is fetched again on open only after this long (fix 23.24).
pub const EMPTY_OPEN_REFRESH_GAP_MS: i64 = 10 * 60 * 1000;

/// When an open-time refresh last started for an entry with no items, in Unix ms. Process-wide, so every page of the same
/// entry shares the guard.
static EMPTY_OPEN_FETCHES: LazyLock<Mutex<HashMap<(Kind, i64), i64>>> = LazyLock::new(Mutex::default);

/// The time of the last open-time refresh of the entry that found no items, if any.
pub fn last_empty_open_fetch(kind: Kind, id: i64) -> Option<i64> {
    EMPTY_OPEN_FETCHES.lock().unwrap_or_else(PoisonError::into_inner).get(&(kind, id)).copied()
}

/// Remembers that an open-time refresh of the entry started at `now_ms` for an entry with no items.
pub fn record_empty_open_fetch(kind: Kind, id: i64, now_ms: i64) {
    EMPTY_OPEN_FETCHES.lock().unwrap_or_else(PoisonError::into_inner).insert((kind, id), now_ms);
}

/// An open-time refresh of an entry with no items waits for the 10-minute guard. A manual refresh always runs, and a list
/// that already has items is only fetched when the open asks for it (E-REF-3).
pub fn open_refresh_due(items_empty: bool, last_empty_fetch_ms: Option<i64>, now_ms: i64) -> bool {
    match last_empty_fetch_ms {
        Some(last) if items_empty => now_ms - last >= EMPTY_OPEN_REFRESH_GAP_MS,
        _ => true,
    }
}

/// The new items to queue after a refresh (T-REF-4, behaviour 5.1): only a manual refresh auto-downloads (E-REF-5).
/// `enabled` is the `download_new` preference of the kind, `eligible` the category rule for this entry, and `unread_only`
/// the "skip duplicate read items" rule. `existing` is the stored list the numbers are compared against.
pub fn auto_download_targets(trigger: Trigger, enabled: bool, eligible: bool, unread_only: bool, new_items: &[Item], existing: &[Item]) -> Vec<Item> {
    if !auto_download_allowed(trigger) {
        return Vec::new();
    }
    filter_new(enabled, eligible, unread_only, new_items, existing)
}

/// The new items a refresh of this entry queues, from the stored entry, its categories, its items and the preferences.
/// Runs on the blocking pool. Empty for an open-time refresh and for an entry outside the library.
pub fn new_item_targets(
    entries: &EntryRepo,
    categories: &CategoryRepo,
    items: &ItemRepo,
    prefs: &PrefStore,
    kind: Kind,
    id: i64,
    trigger: Trigger,
    new_items: &[Item],
) -> DataResult<Vec<Item>> {
    if new_items.is_empty() || !auto_download_allowed(trigger) {
        return Ok(Vec::new());
    }
    let downloads = DownloadPrefs::new(prefs);
    let enabled = match kind {
        Kind::Manga => downloads.download_new_chapters.get(),
        Kind::Anime => downloads.download_new_episodes.get(),
    };
    let favorite = crate::entry_core::entry_ops::entry_or_none(entries, id)?.is_some_and(|entry| entry.favorite);
    // the category lists are cut from the settings: every category is eligible
    let rule = CategoryDecision { included: Default::default(), excluded: Default::default() };
    let linked: Vec<i64> = categories.by_entry(id)?.into_iter().map(|c| c.id).collect();
    let eligible = should_download_new(favorite, &linked, &rule);
    Ok(auto_download_targets(
        trigger,
        enabled,
        eligible,
        crate::misc::diet_defaults::DOWNLOAD_NEW_UNREAD_ONLY,
        new_items,
        &items.by_entry_id(id, false)?,
    ))
}

/// Single-flight refreshes per entry id: a second request while one runs joins it and does not fetch again (E-CONC-1).
/// Cloning shares the gate.
#[derive(Clone, Default)]
pub struct RefreshGate(Arc<Mutex<HashSet<i64>>>);

/// A refresh in flight. The gate opens again when it is dropped, however the refresh ended, a panic included.
pub struct Running {
    gate: RefreshGate,
    id: i64,
}

impl RefreshGate {
    /// `Some` when this refresh is the one that runs, `None` when it joins the one already in flight.
    pub fn begin(&self, id: i64) -> Option<Running> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(id).then(|| Running { gate: self.clone(), id })
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.gate.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, number: f64, done: bool) -> Item {
        Item { number, done, ..Item::create(kind) }
    }

    #[test]
    fn the_open_refresh_never_auto_downloads_and_a_manual_one_follows_the_preference_for_both_kinds() {
        for kind in Kind::ALL {
            let new = [item(kind, 3.0, false), item(kind, 2.0, false)];
            let existing = [item(kind, 2.0, true)];
            assert!(auto_download_targets(Trigger::Open, true, true, false, &new, &existing).is_empty(), "E-REF-5");
            assert_eq!(auto_download_targets(Trigger::Manual, true, true, false, &new, &existing).len(), 2);
            assert!(auto_download_targets(Trigger::Manual, false, true, false, &new, &existing).is_empty(), "the preference is off");
        }
    }

    #[test]
    fn only_manual_refresh_auto_downloads() {
        assert!(auto_download_allowed(Trigger::Manual));
        assert!(!auto_download_allowed(Trigger::Open));
    }

    #[test]
    fn a_zero_item_open_refresh_waits_ten_minutes() {
        let t0 = 1_000_000;
        assert!(open_refresh_due(true, None, t0), "never fetched: fetch");
        assert!(!open_refresh_due(true, Some(t0), t0 + EMPTY_OPEN_REFRESH_GAP_MS - 1), "second open within 10 min");
        assert!(open_refresh_due(true, Some(t0), t0 + EMPTY_OPEN_REFRESH_GAP_MS), "10 min later: fetch again");
        assert!(open_refresh_due(false, Some(5), 6), "a non-empty list is never throttled");
    }

    #[test]
    fn the_empty_open_fetch_time_is_remembered_per_kind_and_entry() {
        let id = 910_001;
        assert_eq!(last_empty_open_fetch(Kind::Manga, id), None);
        record_empty_open_fetch(Kind::Manga, id, 5_000);
        assert_eq!(last_empty_open_fetch(Kind::Manga, id), Some(5_000));
        assert_eq!(last_empty_open_fetch(Kind::Manga, id + 1), None, "another entry has its own guard");
        assert_eq!(last_empty_open_fetch(Kind::Anime, id), None, "so does the other kind");
        assert!(!open_refresh_due(true, last_empty_open_fetch(Kind::Manga, id), 5_000 + 1));
    }

    #[test]
    fn a_second_refresh_of_the_same_entry_joins_the_first_and_the_gate_reopens_when_it_ends() {
        let gate = RefreshGate::default();
        let first = gate.begin(7);
        assert!(first.is_some());
        assert!(gate.begin(7).is_none(), "joined");
        assert!(gate.begin(8).is_some(), "another entry is independent");
        drop(first);
        assert!(gate.begin(7).is_some(), "after the end a new refresh starts");
    }
}
