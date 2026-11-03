//! The rows of the Updates feed and how a gesture changes the selection (T-42a-030, T-42a-034). Pure. A feed row
//! carries its own selected flag, as [`Selection`] expects; a press is routed here by what it was (tap, long press,
//! Ctrl or Shift click) and whether the selection mode is already on.

use std::collections::HashMap;

use chrono::{NaiveDate, TimeZone};
use kuro_domain::dates::ListRow;
use kuro_domain::updates::model::UpdateRow;

use crate::updates::bulk::UnitFlags;
use crate::updates::dates::grouped;
use crate::updates::download_actions::UnitTarget;
use crate::updates::download_state::{self, Change, DiskCache, DiskKey, DownloadState, QueueEntry, RowDownload};
use crate::updates::selection::{Selectable, Selection, is_selection_mode};

#[derive(Clone, Debug, PartialEq)]
pub struct FeedRow {
    pub row: UpdateRow,
    pub selected: bool,
    /// What the download indicator shows (T-42a-029): the queue status, else the disk check, else not downloaded.
    pub download: RowDownload,
}

/// The state of a row with no queue entry and no files.
pub const NOT_DOWNLOADED: RowDownload = RowDownload { state: DownloadState::NotDownloaded, progress: 0 };

/// The disk-check key of a row: its source, entry title, unit name and scanlator.
pub fn disk_key(row: &UpdateRow) -> DiskKey {
    DiskKey { source_id: row.source_id, entry_title: row.entry_title.clone(), unit_name: row.item_name.clone(), scanlator: row.scanlator.clone() }
}

/// Derives the download state of every row from the queue snapshot and the (cached) disk check. Call it on a new
/// list and after each download-cache or queue signal, with `cache` invalidated first on the signal.
pub fn rederive(rows: &mut [FeedRow], queue: impl Fn(i64) -> Option<QueueEntry>, cache: &mut DiskCache, on_disk: impl Fn(&DiskKey) -> bool) {
    for row in rows {
        let key = disk_key(&row.row);
        let downloaded = cache.is_downloaded(key.clone(), || on_disk(&key));
        row.download = download_state::derive(queue(row.row.item_id), downloaded);
    }
}

/// Row positions by unit id. Rebuilt whenever the list is replaced, so a download event finds its row without a scan
/// (T-42b-040).
pub fn index_rows(rows: &[FeedRow]) -> HashMap<i64, usize> {
    rows.iter().enumerate().map(|(i, r)| (r.row.item_id, i)).collect()
}

/// Patches the one row an event names. Returns whether a row was found. A missing or stale index entry is a no-op.
pub fn apply_download(rows: &mut [FeedRow], index: &HashMap<i64, usize>, unit_id: i64, change: Change) -> bool {
    match index.get(&unit_id).and_then(|&i| rows.get_mut(i)).filter(|r| r.row.item_id == unit_id) {
        Some(row) => {
            download_state::apply_to(&mut row.download, change);
            true
        }
        None => false,
    }
}

/// The selected rows as download targets (entry, unit, state), for the bulk download actions.
pub fn selected_targets(rows: &[FeedRow]) -> Vec<UnitTarget> {
    rows.iter()
        .filter(|r| r.selected)
        .map(|r| UnitTarget { entry_id: r.row.entry_id, unit_id: r.row.item_id, state: r.download.state })
        .collect()
}

impl Selectable for FeedRow {
    fn id(&self) -> i64 {
        self.row.item_id
    }
    fn selected(&self) -> bool {
        self.selected
    }
    fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }
}

/// A new list from the query. Rows that were selected stay selected by unit id, and the bounds are recomputed from the
/// selected flags (EC-42-U-05, T-42a-006), so they never point past a shorter list. The source leaves them stale
/// (B 5.4, E-U46); our 42a rule wins here because a stale bound makes a long press select the wrong range.
pub fn replace_rows(rows: Vec<UpdateRow>, previous: &[FeedRow], selection: &mut Selection) -> Vec<FeedRow> {
    let was_selected = |id: i64| previous.iter().any(|p| p.selected && p.row.item_id == id);
    let fresh: Vec<FeedRow> = rows.into_iter().map(|row| FeedRow { selected: was_selected(row.item_id), row, download: NOT_DOWNLOADED }).collect();
    selection.sync(&fresh);
    fresh
}

/// The list under date headers by fetch day; each item is an index into `rows`.
pub fn layout<Tz: TimeZone>(
    rows: &[FeedRow],
    zone: &Tz,
    today: NaiveDate,
    relative: bool,
    date_pref: &str,
    locale: &str,
) -> Vec<ListRow<usize>> {
    grouped(rows.iter().enumerate().map(|(i, r)| (i, r.row.date_fetch)), zone, today, relative, date_pref, locale)
}

/// The flags of the selected rows, for the bulk planner.
pub fn selected_flags(rows: &[FeedRow]) -> Vec<UnitFlags> {
    rows.iter()
        .filter(|r| r.selected)
        .map(|r| UnitFlags {
            id: r.row.item_id,
            read: r.row.done,
            bookmark: r.row.bookmark,
            fillermark: r.row.fillermark,
            progress: r.row.progress,
        })
        .collect()
}

/// The widget name of the list row of unit `id`, so a key press can find the focused row's unit.
pub fn row_name(id: i64) -> String {
    format!("unit:{id}")
}

/// The unit of a row widget name made by [`row_name`].
pub fn unit_of_row_name(name: &str) -> Option<i64> {
    name.strip_prefix("unit:")?.parse().ok()
}

/// How a row was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Tap,
    LongPress,
    CtrlClick,
    ShiftClick,
}

/// What a press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Not in selection mode: the tap opens the unit.
    Open,
    /// The selection changed.
    Changed,
    /// Nothing to do (an unknown row).
    Nothing,
}

/// Routes a press on the row with unit `id`.
/// - Tap opens the unit, or toggles the row while the selection mode is on.
/// - Long press toggles the row and fills the gap to the existing selection (`fromLongPress`).
/// - Ctrl+click toggles the row, in or out of the selection mode.
/// - Shift+click selects the range from the existing selection to the row.
pub fn press(gesture: Gesture, selection: &mut Selection, rows: &mut [FeedRow], id: i64) -> Press {
    let Some(selected) = rows.iter().find(|r| r.row.item_id == id).map(|r| r.selected) else {
        return Press::Nothing;
    };
    let mode = is_selection_mode(rows);
    let changed = match gesture {
        Gesture::Tap if !mode => return Press::Open,
        Gesture::Tap | Gesture::CtrlClick => selection.toggle(rows, id, !selected, true, false),
        Gesture::LongPress => selection.toggle(rows, id, !selected, true, true),
        Gesture::ShiftClick => selection.toggle(rows, id, true, true, true),
    };
    if !mode {
        // a first selection that was not a long press leaves a half-set bound; read both from the flags
        selection.sync(rows);
    }
    if changed.is_empty() { Press::Nothing } else { Press::Changed }
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;

    use super::*;

    fn update(id: i64) -> UpdateRow {
        UpdateRow {
            kind: Kind::Manga,
            entry_id: 1,
            entry_title: "T".into(),
            item_id: id,
            item_name: format!("c{id}"),
            scanlator: None,
            done: false,
            bookmark: false,
            fillermark: false,
            progress: 0,
            total_seconds: 0,
            source_id: 1,
            date_fetch: 0,
            date_upload: 0,
            thumbnail_url: None,
            cover_last_modified: 0,
        }
    }

    fn rows(n: i64) -> Vec<FeedRow> {
        (0..n).map(|id| FeedRow { row: update(id), selected: false, download: NOT_DOWNLOADED }).collect()
    }

    fn flags(rows: &[FeedRow]) -> Vec<bool> {
        rows.iter().map(|r| r.selected).collect()
    }

    #[test]
    fn feed_tap_opens_until_the_selection_mode_is_on() {
        let (mut items, mut sel) = (rows(4), Selection::default());
        assert_eq!(press(Gesture::Tap, &mut sel, &mut items, 1), Press::Open);
        assert_eq!(press(Gesture::LongPress, &mut sel, &mut items, 1), Press::Changed);
        assert_eq!(press(Gesture::Tap, &mut sel, &mut items, 3), Press::Changed, "tap toggles while selecting");
        assert_eq!(flags(&items), [false, true, false, true]);
        assert_eq!(press(Gesture::Tap, &mut sel, &mut items, 3), Press::Changed);
        assert_eq!(flags(&items), [false, true, false, false]);
    }

    #[test]
    fn feed_long_press_fills_the_range() {
        let (mut items, mut sel) = (rows(6), Selection::default());
        press(Gesture::LongPress, &mut sel, &mut items, 1);
        press(Gesture::LongPress, &mut sel, &mut items, 4);
        assert_eq!(flags(&items), [false, true, true, true, true, false]);
    }

    #[test]
    fn feed_ctrl_click_toggles_without_the_selection_mode() {
        let (mut items, mut sel) = (rows(3), Selection::default());
        assert_eq!(press(Gesture::CtrlClick, &mut sel, &mut items, 2), Press::Changed);
        assert_eq!(flags(&items), [false, false, true]);
        assert_eq!(sel.bounds(), [2, 2]);
        press(Gesture::CtrlClick, &mut sel, &mut items, 0);
        assert_eq!(flags(&items), [true, false, true], "ctrl adds a single row, no range");
        press(Gesture::CtrlClick, &mut sel, &mut items, 2);
        assert_eq!(flags(&items), [true, false, false]);
    }

    #[test]
    fn feed_shift_click_selects_a_range_and_never_deselects() {
        let (mut items, mut sel) = (rows(6), Selection::default());
        press(Gesture::CtrlClick, &mut sel, &mut items, 1);
        press(Gesture::ShiftClick, &mut sel, &mut items, 4);
        assert_eq!(flags(&items), [false, true, true, true, true, false]);
        assert_eq!(press(Gesture::ShiftClick, &mut sel, &mut items, 4), Press::Nothing, "already selected");
    }

    #[test]
    fn feed_unknown_row_is_ignored() {
        let (mut items, mut sel) = (rows(2), Selection::default());
        assert_eq!(press(Gesture::Tap, &mut sel, &mut items, 99), Press::Nothing);
    }

    #[test]
    fn feed_new_list_keeps_selected_ids_and_recomputes_the_bounds() {
        let (mut items, mut sel) = (rows(5), Selection::default());
        press(Gesture::LongPress, &mut sel, &mut items, 2);
        press(Gesture::LongPress, &mut sel, &mut items, 3);
        let fresh = replace_rows(vec![update(9), update(3), update(2), update(8)], &items, &mut sel);
        assert_eq!(flags(&fresh), [false, true, true, false]);
        assert_eq!(sel.bounds(), [1, 2], "rows 3 and 2 moved to indices 1 and 2");
    }

    #[test]
    fn selection_bounds_are_recomputed_after_the_list_shrinks() {
        // EC-42-U-05: select 6 and 7 of 8, refresh to a list of 3 where row 6 is at index 0
        let (mut items, mut sel) = (rows(8), Selection::default());
        press(Gesture::LongPress, &mut sel, &mut items, 6);
        press(Gesture::LongPress, &mut sel, &mut items, 7);
        assert_eq!(sel.bounds(), [6, 7]);
        let mut fresh = replace_rows(vec![update(6), update(30), update(31)], &items, &mut sel);
        assert_eq!(sel.bounds(), [0, 0], "row 6 is the only selected row left");
        assert_eq!(flags(&fresh), [true, false, false]);
        // a long press on the last row now fills the gap up from the real bound: no stale range
        assert_eq!(press(Gesture::LongPress, &mut sel, &mut fresh, 31), Press::Changed);
        assert_eq!(flags(&fresh), [true, true, true]);
        // and a refresh that drops every selected row clears the bounds
        let none = replace_rows(vec![update(40), update(41)], &fresh, &mut sel);
        assert_eq!(flags(&none), [false, false]);
        assert_eq!(sel.bounds(), [-1, -1]);
    }

    #[test]
    fn feed_selected_flags_feed_the_bulk_planner() {
        let mut items = rows(3);
        items[1].selected = true;
        items[1].row.bookmark = true;
        items[1].row.progress = 7;
        let picked = selected_flags(&items);
        assert_eq!(picked.len(), 1);
        assert_eq!((picked[0].id, picked[0].bookmark, picked[0].progress), (1, true, 7));
    }

    #[test]
    fn feed_download_state_comes_from_the_queue_then_the_cached_disk_check() {
        let mut items = rows(3);
        let mut cache = DiskCache::default();
        let checks = std::cell::Cell::new(0);
        let queue = |id: i64| (id == 0).then_some(QueueEntry { status: DownloadState::Downloading, progress: 30 });
        let disk = |key: &DiskKey| {
            checks.set(checks.get() + 1);
            key.unit_name == "c1"
        };
        rederive(&mut items, queue, &mut cache, disk);
        let states: Vec<DownloadState> = items.iter().map(|r| r.download.state).collect();
        assert_eq!(states, [DownloadState::Downloading, DownloadState::Downloaded, DownloadState::NotDownloaded]);
        assert_eq!(items[0].download.progress, 30);
        let first = checks.get();
        rederive(&mut items, queue, &mut cache, disk);
        assert_eq!(checks.get(), first, "the second pass is served from the cache");
        cache.invalidate();
        rederive(&mut items, queue, &mut cache, disk);
        assert_eq!(checks.get(), first * 2, "the cache signal drops the checks");
    }

    #[test]
    fn feed_download_events_patch_only_the_matching_row() {
        let mut items = rows(3);
        let index = index_rows(&items);
        assert!(apply_download(&mut items, &index, 2, Change::Status(DownloadState::Queue)));
        assert!(apply_download(&mut items, &index, 2, Change::Progress(10)));
        assert!(!apply_download(&mut items, &index, 99, Change::Progress(10)));
        assert_eq!(items[2].download, RowDownload { state: DownloadState::Queue, progress: 10 });
        assert_eq!(items[0].download, NOT_DOWNLOADED);
    }

    #[test]
    fn feed_selected_targets_carry_the_download_state() {
        let mut items = rows(3);
        items[1].selected = true;
        items[1].download = RowDownload { state: DownloadState::Downloaded, progress: 0 };
        assert_eq!(selected_targets(&items), vec![UnitTarget { entry_id: 1, unit_id: 1, state: DownloadState::Downloaded }]);
    }

    #[test]
    fn feed_row_names_round_trip_and_other_names_are_not_units() {
        assert_eq!(unit_of_row_name(&row_name(42)), Some(42));
        assert_eq!(unit_of_row_name("screen:updates"), None);
        assert_eq!(unit_of_row_name("unit:x"), None);
    }
}
