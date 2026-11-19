//! The database operations the manga and the anime details pages run the same way, over the shared `EntryRepo`,
//! `ItemRepo` and `CategoryRepo`: the favorite write, adding to the library with the library defaults, marking items
//! done, the fetch interval, the category reads and the tracker count. Each takes the repositories and works for an entry
//! of either kind; the two stores call them and keep only what is theirs (the page rows, the seasons, the viewer bits).

use kuro_data::DataResult;
use kuro_data::category::{Category, CategoryRepo};
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_data::item::{Item, ItemRepo};
use kuro_data::track::TrackRepo;
use kuro_domain::entries::defaults::{apply_item_defaults, apply_season_defaults};
use kuro_domain::entries::fetch_interval::{get_window, set_fetch_interval as store_fetch_interval};
use kuro_domain::items::status::{done_update, items_to_change};
use kuro_platform::kind::Kind;
use kuro_prefs::store::PrefStore;

use crate::entry_core::tracking::facts::Summary;
use crate::entry_core::tracking::session;

/// The row, or `None` when it does not exist.
pub fn entry_or_none(repo: &EntryRepo, id: i64) -> DataResult<Option<Entry>> {
    match repo.by_id(id) {
        Ok(entry) => Ok(Some(entry)),
        Err(kuro_data::DataError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The library entries other than this one with the same title, case folded, for either kind (the duplicate check of an
/// add, E-FAV-1: the first one is the duplicate the page names). Empty when the row is gone.
pub fn duplicates(repo: &EntryRepo, id: i64) -> DataResult<Vec<Entry>> {
    let Some(entry) = entry_or_none(repo, id)? else { return Ok(Vec::new()) };
    repo.duplicates(entry.kind, entry.id, &entry.title)
}

/// The favorite write: `favorite` and `date_added` (now when it is added, 0 when it is removed) in one update. `Ok(false)`
/// when the row is gone.
pub fn set_favorite(repo: &EntryRepo, id: i64, favorite: bool, now_ms: i64) -> DataResult<bool> {
    if entry_or_none(repo, id)?.is_none() {
        return Ok(false);
    }
    repo.update(&EntryUpdate {
        favorite: Some(favorite),
        date_added: Some(if favorite { now_ms } else { 0 }),
        ..EntryUpdate::new(id)
    })?;
    Ok(true)
}

/// Adds the entry to the library in the given categories (none for "no category"), then writes the library defaults to its
/// item flags and, for an anime, to its season flags (T-39b-012: the defaults are written when it joins the library, not
/// when it is opened). The links are written before the favorite, so a failed link leaves the entry out (T-39b-026,
/// E-LIB-6). `Ok(false)` when the row is gone.
pub fn add_to_library(repo: &EntryRepo, prefs: &PrefStore, id: i64, category_ids: &[i64], now_ms: i64) -> DataResult<bool> {
    let Some(entry) = entry_or_none(repo, id)? else { return Ok(false) };
    if !category_ids.is_empty() {
        repo.set_categories(id, category_ids)?;
    }
    set_favorite(repo, id, true, now_ms)?;
    apply_item_defaults(repo, prefs, &entry)?;
    if entry.kind.is_anime() {
        apply_season_defaults(repo, prefs, &entry)?;
    }
    Ok(true)
}

/// Marks `rows` done or undone in one transaction, as `SetDoneStatus` does: a done mark writes the rows that are not done,
/// an undone mark the rows that are done or have progress and resets the progress. Returns the rows that were written.
pub fn mark_done(items: &ItemRepo, rows: &[Item], done: bool) -> DataResult<Vec<Item>> {
    let targets = items_to_change(done, rows);
    if !targets.is_empty() {
        let updates: Vec<_> = targets.iter().map(|item| done_update(done, item)).collect();
        items.update_all(&updates)?;
    }
    Ok(targets)
}

/// `setFetchInterval`: stores `-wheel` (0 is automatic) and recomputes the next update through the domain (T-38b-022,
/// T-39a-026).
pub fn set_fetch_interval(repo: &EntryRepo, items: &ItemRepo, id: i64, wheel: i32) -> DataResult<()> {
    let entry = repo.by_id(id)?;
    let now = chrono::Local::now();
    let window = get_window(&now);
    store_fetch_interval(repo, items, &entry, wheel, &now, window, now.timestamp_millis())
}

/// Every category (the default row `0` included, the dialog drops it) and the ids the entry is linked to.
pub fn categories(repo: &CategoryRepo, id: i64) -> DataResult<(Vec<(i64, String)>, Vec<i64>)> {
    let all = repo.all()?.into_iter().map(|cat| (cat.id, cat.name)).collect();
    let membership = repo.by_entry(id)?.into_iter().map(|cat| cat.id).collect();
    Ok((all, membership))
}

/// The user categories (the default row `0` is left out), for the default-category routing.
pub fn user_categories(repo: &CategoryRepo) -> DataResult<Vec<Category>> {
    Ok(repo.all()?.into_iter().filter(|c| c.id != 0).collect())
}

/// The tracking count and the logged-in flag of the entry, through the same filter as the sheet (T-TRK-12, D-10).
pub fn track_summary(tracks: &TrackRepo, kind: Kind, id: i64, source_id: i64) -> DataResult<Summary> {
    let bound: Vec<i64> = tracks.by_entry(id)?.iter().map(|t| t.tracker_id).collect();
    Ok(session::summary(kind, source_id, &bound))
}

/// The user's own score of the entry on the 10-point scale (`kuro_track::model::score_10`): the mean over its tracks on
/// the trackers `logged_in` that carry a score, as the library's "Tracker score" sort reads it. `None` when none does.
pub fn own_score(tracks: &TrackRepo, id: i64, logged_in: &[i64]) -> DataResult<Option<f64>> {
    let scores: Vec<f64> = tracks
        .by_entry(id)?
        .iter()
        .filter(|t| logged_in.contains(&t.tracker_id) && t.score > 0.0)
        .map(|t| kuro_track::model::score_10(kuro_track::model::TrackerId(t.tracker_id), t.score))
        .collect();
    Ok((!scores.is_empty()).then(|| scores.iter().sum::<f64>() / scores.len() as f64))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kuro_db::flags::item as flag;
    use kuro_db::handle::Db;
    use kuro_domain::entries::defaults::set_item_settings_default;

    use super::*;

    fn db() -> (tempfile::TempDir, Arc<Db>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("schema");
        (dir, Arc::new(Db::new(&path, writer).expect("db")))
    }

    fn entry(repo: &EntryRepo, kind: Kind) -> i64 {
        repo.insert(&Entry { source: 7, url: "/e".into(), title: "T".into(), ..Entry::create(kind) }).expect("insert")
    }

    #[test]
    fn a_library_entry_with_the_same_title_is_a_duplicate_and_the_entry_itself_is_not() {
        let (_dir, db) = db();
        let repo = EntryRepo::new(db);
        for kind in [Kind::Manga, Kind::Anime] {
            let id = repo.insert(&Entry { source: 7, url: format!("/{kind}/a"), title: "\u{c9}cole".into(), ..Entry::create(kind) }).unwrap();
            assert!(duplicates(&repo, id).unwrap().is_empty());
            let twin = repo
                .insert(&Entry { source: 8, url: format!("/{kind}/b"), title: "\u{e9}COLE".into(), favorite: true, ..Entry::create(kind) })
                .unwrap();
            assert_eq!(duplicates(&repo, id).unwrap().iter().map(|e| e.id).collect::<Vec<_>>(), vec![twin], "{kind}: Unicode case folds");
            assert!(duplicates(&repo, twin).unwrap().is_empty(), "{kind}: an entry is never its own duplicate");
        }
        assert!(duplicates(&repo, 4242).unwrap().is_empty());
    }

    #[test]
    fn a_favorite_write_sets_the_date_and_a_missing_row_is_false() {
        let (_dir, db) = db();
        let repo = EntryRepo::new(db);
        let id = entry(&repo, Kind::Manga);
        assert!(set_favorite(&repo, id, true, 5_000).unwrap());
        let row = repo.by_id(id).unwrap();
        assert!(row.favorite && row.date_added == 5_000);
        assert!(set_favorite(&repo, id, false, 6_000).unwrap());
        let row = repo.by_id(id).unwrap();
        assert!(!row.favorite && row.date_added == 0, "E-FAV-7");
        assert!(!set_favorite(&repo, 4242, true, 1).unwrap());
    }

    #[test]
    fn adding_writes_the_defaults_of_the_kind_and_only_then() {
        let (_dir, db) = db();
        let repo = EntryRepo::new(db);
        let prefs = PrefStore::in_memory();
        set_item_settings_default(&prefs, Kind::Manga, flag::SHOW_UNDONE | flag::SORT_ASC | flag::SORTING_NUMBER);
        set_item_settings_default(&prefs, Kind::Anime, flag::SHOW_DONE | flag::SORT_ASC | flag::SORTING_NUMBER);
        for (kind, shown) in [(Kind::Manga, flag::SHOW_UNDONE), (Kind::Anime, flag::SHOW_DONE)] {
            let id = entry(&repo, kind);
            assert_eq!(repo.by_id(id).unwrap().item_flags, 0, "opening writes nothing");
            assert!(add_to_library(&repo, &prefs, id, &[], 9).unwrap());
            let row = repo.by_id(id).unwrap();
            assert!(row.favorite);
            assert_eq!(row.item_flags & flag::UNDONE_MASK, shown, "{kind}: the library default reached the row");
        }
        assert!(!add_to_library(&repo, &prefs, 4242, &[], 9).unwrap());
    }

    #[test]
    fn a_failed_category_link_leaves_the_entry_out_of_the_library() {
        let (_dir, db) = db();
        let repo = EntryRepo::new(db);
        let id = entry(&repo, Kind::Manga);
        assert!(add_to_library(&repo, &PrefStore::in_memory(), id, &[4242], 1_000).is_err(), "unknown category");
        assert!(!repo.by_id(id).unwrap().favorite);
    }

    #[test]
    fn marking_done_writes_only_the_rows_that_change_and_undone_resets_the_progress() {
        let (_dir, db) = db();
        let (repo, items) = (EntryRepo::new(Arc::clone(&db)), ItemRepo::new(db));
        for kind in [Kind::Manga, Kind::Anime] {
            let id = entry(&repo, kind);
            let rows: Vec<Item> = [(true, 0), (false, 5), (false, 0)]
                .iter()
                .enumerate()
                .map(|(n, &(done, progress))| Item { entry_id: id, url: format!("/{kind}/{n}"), done, progress, ..Item::create(kind) })
                .collect();
            let stored = items.add_all(&rows).unwrap();
            assert_eq!(mark_done(&items, &stored, true).unwrap().len(), 2, "{kind}: the done row is left alone");
            assert!(items.by_entry_id(id, false).unwrap().iter().all(|i| i.done));
            let back = mark_done(&items, &items.by_entry_id(id, false).unwrap(), false).unwrap();
            assert_eq!(back.len(), 3);
            assert!(items.by_entry_id(id, false).unwrap().iter().all(|i| !i.done && i.progress == 0));
            assert!(mark_done(&items, &items.by_entry_id(id, false).unwrap(), false).unwrap().is_empty());
        }
    }

    #[test]
    fn the_own_score_is_the_mean_over_the_logged_in_trackers_that_carry_one() {
        let (_dir, db) = db();
        let id = entry(&EntryRepo::new(db.clone()), Kind::Anime);
        let tracks = TrackRepo::new(db);
        assert_eq!(own_score(&tracks, id, &[1, 2, 3]).unwrap(), None, "not tracked");
        let track = |tracker: i64, score: f64| kuro_data::track::Track { entry_id: id, remote_id: tracker * 10, score, ..kuro_data::track::Track::new(Kind::Anime, tracker) };
        // MyAnimeList (1) holds 9, AniList (2) the same scale times ten, Kitsu (3) no score
        tracks.insert_all(&[track(1, 9.0), track(2, 80.0), track(3, 0.0)]).unwrap();
        assert_eq!(own_score(&tracks, id, &[1, 2, 3]).unwrap(), Some(8.5));
        assert_eq!(own_score(&tracks, id, &[2]).unwrap(), Some(8.0), "a logged-out tracker is left out");
        assert_eq!(own_score(&tracks, id, &[3]).unwrap(), None, "bound, with no score given");
    }
}
