//! The one entry restore for both kinds (area 22, T-22a-011 to T-22a-025). `manga.rs` and `anime.rs` turn a backup row
//! into an [`Incoming`] entry (the wire shapes differ), and `restore_entry` writes it over `kuro_db::tables` inside the
//! caller's transaction, so a failure anywhere in it rolls the whole entry back.

use std::collections::{HashMap, HashSet};

use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_data::track::Track;
use kuro_db::codec::GenreList;
use kuro_db::tables::{self, EntryPatch, EntryRow, ItemPatch, ItemRow, TrackPatch, TrackRow, categories, entries, history, items, tracks};
use kuro_db::{DbResult, Transaction};
use kuro_domain::entries::fetch_interval::{ItemDates, Stored, to_update};
use kuro_platform::kind::Kind;

use super::categories::{differs_from_stored, membership_ids};
use super::merge::{RunClock, merge_progress};
use kuro_db::tables::excluded;

/// One backup history row, its item named by url.
pub struct HistoryIn {
    pub url: String,
    pub last_at: i64,
    pub duration_ms: i64,
}

/// A backup entry in the stored shape. `entry.kind` selects the kind; `items` and `tracks` carry id -1 and entry id 0.
pub struct Incoming<'a> {
    pub entry: Entry,
    pub items: Vec<Item>,
    pub tracks: Vec<Track>,
    pub history: Vec<HistoryIn>,
    /// Backup category `order` values.
    pub category_orders: &'a [i64],
    pub excluded_scanlators: &'a [String],
}

/// The `(source, url)` pairs of one kind already in `entries`, read once per section (T-22a-011).
pub fn existing_pairs(tx: &Transaction<'_>, kind: Kind) -> DbResult<HashSet<(i64, String)>> {
    Ok(entries::all_source_and_url(tx, kind)?.into_iter().collect())
}

/// Restores one entry: row, categories, items, tracks, history, excluded scanlators, fetch interval. `parent` is the
/// database id of the root for an anime season and `None` otherwise. Returns the database id.
pub fn restore_entry(
    tx: &Transaction<'_>,
    clock: &RunClock,
    inc: &Incoming<'_>,
    parent: Option<i64>,
    categories_by_order: &HashMap<i64, i64>,
) -> DbResult<i64> {
    let kind = inc.entry.kind;
    let id = match entries::by_url_and_source(tx, kind, &inc.entry.url, inc.entry.source)? {
        None => entries::insert(tx, &EntryRow { parent_id: parent, ..EntryRow::from(&inc.entry) })?,
        Some(existing) => {
            update_existing(tx, &inc.entry, &existing, parent)?;
            existing.id
        }
    };
    if let Some(ids) = membership_ids(categories_by_order, inc.category_orders) {
        let stored: Vec<i64> = categories::by_entry(tx, kind, id)?.into_iter().map(|c| c.id).collect();
        if differs_from_stored(stored, &ids) {
            // The membership insert trigger bumps `version`; the restore keeps the version it set.
            let version = entries::by_id(tx, id)?.version;
            entries::set_categories(tx, id, &ids)?;
            tx.execute("UPDATE entries SET version = ?1 WHERE _id = ?2", [version, id])?;
        }
    }
    restore_items(tx, id, &inc.items)?;
    restore_tracks(tx, id, &inc.tracks)?;
    restore_history(tx, id, &inc.history)?;
    restore_scanlators(tx, id, inc.excluded_scanlators)?;
    refresh_interval(tx, clock, id)?;
    Ok(id)
}

/// True when applying `p` would leave `d` as it is. The write is skipped then, so a repeat restore writes nothing.
fn update_is_noop(p: &EntryPatch, d: &EntryRow) -> bool {
    fn same<T: PartialEq>(p: &Option<T>, cur: &T) -> bool {
        p.as_ref().is_none_or(|v| v == cur)
    }
    fn same_opt<T: PartialEq>(p: &Option<T>, cur: &Option<T>) -> bool {
        p.as_ref().is_none_or(|v| Some(v) == cur.as_ref())
    }
    same(&p.favorite, &d.favorite)
        && same(&p.initialized, &d.initialized)
        && same(&p.version, &d.version)
        && same(&p.status, &d.status)
        && same(&p.title, &d.title)
        && same(&p.viewer, &d.viewer)
        && same(&p.item_flags, &d.item_flags)
        && same(&p.update_strategy, &d.update_strategy)
        && same(&p.date_added, &d.date_added)
        && same(&p.cover_last_modified, &d.cover_last_modified)
        && same_opt(&p.artist, &d.artist)
        && same_opt(&p.author, &d.author)
        && same_opt(&p.description, &d.description)
        && same_opt(&p.genre, &d.genre)
        && same_opt(&p.thumbnail_url, &d.thumbnail_url)
        && same_opt(&p.last_update, &d.last_update)
        && same_opt(&p.parent_id, &d.parent_id)
        && same_opt(&p.fetch_type, &d.fetch_type)
        && same_opt(&p.season_flags, &d.season_flags)
        && same_opt(&p.season_number, &d.season_number)
        && same_opt(&p.season_source_order, &d.season_source_order)
        && same_opt(&p.background_url, &d.background_url)
        && same_opt(&p.background_last_modified, &d.background_last_modified)
        && same_opt(&p.memo, &d.memo)
}

/// The merge of an existing row (T-22a-012, T-22a-013, T-22a-022). The newer side owns the metadata and the version;
/// the other side owns title, viewer, flags, strategy and dates (D5, parity). `favorite` and `initialized` are ORed.
/// The write is a coalesce update, so `next_update` and `calculate_interval` stay. The season columns exist for anime
/// only, and the parent is written only for a season (`parent`); a root leaves it as the row has it.
fn update_existing(tx: &Transaction<'_>, b: &Entry, d: &EntryRow, parent: Option<i64>) -> DbResult<()> {
    let anime = b.kind.is_anime();
    let mut p = EntryPatch {
        id: d.id,
        favorite: Some(d.favorite || b.favorite),
        initialized: Some(d.initialized || b.description.is_some()),
        parent_id: parent,
        ..Default::default()
    };
    if b.version > d.version {
        // Backup is newer: metadata and version from the backup. Title, viewer, flags, strategy, dates keep the row.
        p.artist = b.artist.clone();
        p.author = b.author.clone();
        p.description = b.description.clone();
        p.genre = b.genre.clone().map(GenreList);
        p.thumbnail_url = b.thumbnail_url.clone();
        p.status = Some(b.status);
        p.version = Some(b.version);
        if anime {
            p.fetch_type = b.fetch_type;
        }
    } else {
        // Row is newer or equal: metadata and version stay as they are. The other fields come from the backup. The
        // backup carries no last-update or cover date, so those become 0 (parity with the Kotlin mapping).
        p.title = Some(b.title.clone());
        p.viewer = Some(b.viewer_flags);
        p.item_flags = Some(b.item_flags);
        p.update_strategy = Some(b.update_strategy);
        p.date_added = Some(b.date_added);
        p.last_update = Some(0);
        p.cover_last_modified = Some(0);
        if anime {
            p.season_flags = b.season_flags;
            p.season_number = b.season_number;
            p.season_source_order = b.season_source_order;
            p.background_url = b.background_url.clone();
            p.background_last_modified = Some(0);
            p.memo = b.memo.clone();
        }
    }
    if !update_is_noop(&p, d) {
        entries::update(tx, &p)?;
    }
    Ok(())
}

/// Items (T-22a-015, T-22a-016, T-22a-024). Lookup is by url within this entry. An unchanged item is skipped.
fn restore_items(tx: &Transaction<'_>, entry_id: i64, backup: &[Item]) -> DbResult<()> {
    let db = items::by_entry(tx, entry_id, false)?;
    let by_url: HashMap<&str, &ItemRow> = db.iter().map(|r| (r.url.as_str(), r)).collect();
    for b in backup {
        let row = ItemRow { entry_id, ..ItemRow::from(b) };
        match by_url.get(row.url.as_str()) {
            None => {
                items::insert(tx, &row)?;
            }
            Some(d) if same_item_state(&row, d) => {}
            Some(d) => {
                let (done, progress) = merge_progress(row.done, row.progress, d.done, d.progress);
                items::update(
                    tx,
                    &ItemPatch {
                        id: d.id,
                        done: Some(done),
                        bookmark: Some(row.bookmark || d.bookmark),
                        fillermark: Some(row.fillermark || d.fillermark),
                        progress: Some(progress),
                        total_seconds: row.total_seconds,
                        version: Some(row.version),
                        memo: row.memo.clone(),
                        // a backup without it (Aniyomi's) keeps the stored count
                        page_count: row.page_count,
                        ..Default::default()
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// `forComparison`: every field but the ids, the fetch and upload dates, `lastModifiedAt` and `version`, and the page
/// count, which is no reading state (the reader writes it): a backup without it (Aniyomi's) is not a change.
fn same_item_state(a: &ItemRow, d: &ItemRow) -> bool {
    ItemRow {
        id: d.id,
        entry_id: d.entry_id,
        date_fetch: d.date_fetch,
        date_upload: d.date_upload,
        last_modified_at: d.last_modified_at,
        version: d.version,
        page_count: d.page_count,
        ..a.clone()
    } == *d
}

/// Tracks (T-22a-019, T-22a-025). Lookup is by tracker id within the entry. An existing track changes only the remote
/// id, the library id and the last item (the larger of the two).
fn restore_tracks(tx: &Transaction<'_>, entry_id: i64, backup: &[Track]) -> DbResult<()> {
    let db = tracks::by_entry(tx, entry_id)?;
    let by_tracker: HashMap<i64, &TrackRow> = db.iter().map(|r| (r.tracker_id, r)).collect();
    for b in backup {
        let row = TrackRow { entry_id, ..TrackRow::from(b) };
        match by_tracker.get(&row.tracker_id) {
            None => {
                tracks::insert(tx, &row)?;
            }
            Some(d) if same_track_state(&row, d) => {}
            Some(d) => {
                tracks::update_coalesced(
                    tx,
                    &TrackPatch {
                        id: d.id,
                        remote_id: Some(row.remote_id),
                        library_id: row.library_id,
                        last_item: Some(d.last_item.max(row.last_item)),
                        ..Default::default()
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// `forComparison` for a track: every field but the id and the entry id.
fn same_track_state(a: &TrackRow, d: &TrackRow) -> bool {
    TrackRow { id: d.id, entry_id: d.entry_id, ..a.clone() } == *d
}

/// History (T-22a-018, T-22a-025). The item is looked up in this entry only (D9 fixed); a row for an item that does
/// not exist here is skipped. The time ends at the later of the two, the duration at the larger (both kinds, E-42).
fn restore_history(tx: &Transaction<'_>, entry_id: i64, backup: &[HistoryIn]) -> DbResult<()> {
    let item_ids: HashMap<String, i64> = items::by_entry(tx, entry_id, false)?.into_iter().map(|r| (r.url, r.id)).collect();
    let existing: HashMap<i64, tables::HistoryRow> =
        history::by_entry(tx, entry_id)?.into_iter().map(|h| (h.item_id, h)).collect();
    for h in backup {
        let Some(&item_id) = item_ids.get(&h.url) else {
            continue;
        };
        let (at, delta) = match existing.get(&item_id) {
            None => (h.last_at, h.duration_ms),
            Some(d) => (h.last_at.max(d.last_at), h.duration_ms.max(d.duration_ms) - d.duration_ms),
        };
        if existing.get(&item_id).is_some_and(|d| d.last_at == at) && delta == 0 {
            continue;
        }
        history::upsert(tx, item_id, at, delta)?;
    }
    Ok(())
}

/// Excluded scanlators (T-22a-020): only the ones not stored yet. A repeated name in the backup is written once.
fn restore_scanlators(tx: &Transaction<'_>, entry_id: i64, backup: &[String]) -> DbResult<()> {
    if backup.is_empty() {
        return Ok(());
    }
    // ponytail: `kuro_db::tables` has no excluded_scanlators statements yet (phase E); this is the one legacy call.
    let mut seen: HashSet<String> = excluded::by_entry_id(tx, entry_id)?.into_iter().collect();
    for s in backup {
        if seen.insert(s.clone()) {
            excluded::insert(tx, entry_id, s)?;
        }
    }
    Ok(())
}

/// Recomputes `next_update` and `fetch_interval` from the items and the run clock (T-22a-021). Nothing else is
/// written. The window is the one captured in `clock`, so it is the same for every entry of a run (E-47).
pub fn refresh_interval(tx: &Transaction<'_>, clock: &RunClock, entry_id: i64) -> DbResult<()> {
    let row = entries::by_id(tx, entry_id)?;
    let dates: Vec<ItemDates> = items::by_entry(tx, entry_id, true)?
        .iter()
        .map(|r| ItemDates { date_upload: r.date_upload, date_fetch: r.date_fetch })
        .collect();
    let stored = Stored {
        fetch_interval: row.calculate_interval as i32,
        next_update: row.next_update.unwrap_or(0),
        last_update: row.last_update.unwrap_or(0),
    };
    let out = to_update(stored, &dates, &clock.now, clock.window, clock.clock_now_ms);
    if row.next_update == Some(out.next_update) && row.calculate_interval == i64::from(out.fetch_interval) {
        return Ok(());
    }
    entries::update(
        tx,
        &EntryPatch {
            id: entry_id,
            next_update: Some(out.next_update),
            calculate_interval: Some(i64::from(out.fetch_interval)),
            ..Default::default()
        },
    )?;
    Ok(())
}
