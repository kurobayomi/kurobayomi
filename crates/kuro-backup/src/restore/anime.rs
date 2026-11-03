//! Anime entry restore (area 22, T-22a-022 to T-22a-025). Seasons are restored under their parent (D7 and D18 fixed):
//! `plan_anime` splits the sorted list into top-level rows and seasons, and each season takes the database id of its
//! parent. The backup row becomes a shared [`Incoming`] entry and `write::restore_entry` writes it.

use std::collections::{HashMap, HashSet};

use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_data::track::Track;
use kuro_db::codec::Memo;
use kuro_db::{DbResult, Transaction};
use kuro_platform::kind::Kind;

use super::merge::{RunClock, round6, sort_new_first};
use super::write::{self, HistoryIn, Incoming};
use crate::schema::{BackupAnime, BackupAnimeTracking, BackupEpisode};

pub use super::write::refresh_interval;

/// A top-level anime row with the seasons that restore under it.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimeRoot {
    pub entry: BackupAnime,
    pub seasons: Vec<BackupAnime>,
}

/// The `(source, url)` pairs of the anime already in `entries`, read once per section (T-22a-022).
pub fn existing_pairs(tx: &Transaction<'_>) -> DbResult<HashSet<(i64, String)>> {
    write::existing_pairs(tx, Kind::Anime)
}

/// Entries not in the database first, newest `lastModifiedAt` first, ties in backup order (T-22a-022).
pub fn sort_anime(entries: Vec<BackupAnime>, existing: &HashSet<(i64, String)>) -> Vec<BackupAnime> {
    sort_new_first(entries, |a| (a.source, a.url.as_str(), a.last_modified_at), existing)
}

/// Splits the sorted list into roots and seasons (T-22a-023). A season is a row whose `parentId` names a row in this
/// file. Every other row is a root, including a season whose parent is missing: it restores as a top-level row (X-43).
pub fn plan_anime(sorted: Vec<BackupAnime>) -> Vec<AnimeRoot> {
    let ids: HashSet<i64> = sorted.iter().filter_map(|a| a.id).collect();
    let is_season = |a: &BackupAnime| a.parent_id.is_some_and(|p| ids.contains(&p));
    let (seasons, roots): (Vec<BackupAnime>, Vec<BackupAnime>) = sorted.into_iter().partition(|a| is_season(a));
    roots
        .into_iter()
        .map(|entry| {
            let seasons = seasons.iter().filter(|s| s.parent_id.is_some() && s.parent_id == entry.id).cloned().collect();
            AnimeRoot { entry, seasons }
        })
        .collect()
}

/// Restores one root and its seasons inside the caller's transaction. Each season takes the database id of the root.
pub fn restore_anime_root(
    tx: &Transaction<'_>,
    clock: &RunClock,
    root: &AnimeRoot,
    categories: &HashMap<i64, i64>,
) -> DbResult<()> {
    let parent = restore_anime_entry(tx, clock, &root.entry, None, categories)?;
    for season in &root.seasons {
        restore_anime_entry(tx, clock, season, Some(parent), categories)?;
    }
    Ok(())
}

/// Decodes the backup memo bytes. Empty or invalid JSON becomes `{}`.
pub fn memo_of(bytes: &[u8]) -> Memo {
    serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(bytes).map(Memo).unwrap_or_default()
}

/// Restores one row (a root, or a season under `parent`) inside the caller's transaction. Returns its database id.
/// History keeps `readDuration` as the larger of the stored and the backup value, like manga.
pub fn restore_anime_entry(
    tx: &Transaction<'_>,
    clock: &RunClock,
    b: &BackupAnime,
    parent: Option<i64>,
    categories: &HashMap<i64, i64>,
) -> DbResult<i64> {
    let incoming = Incoming {
        entry: entry_of(b),
        items: b.episodes.iter().map(item_of).collect(),
        tracks: b.tracking.iter().map(track_of).collect(),
        history: b
            .history
            .iter()
            .map(|h| HistoryIn { url: h.url.clone(), last_at: h.last_read, duration_ms: h.read_duration })
            .collect(),
        category_orders: &b.categories,
        excluded_scanlators: &[],
    };
    write::restore_entry(tx, clock, &incoming, parent, categories)
}

/// A new row (T-22a-022). `initialized` is `description != null`. The parent is set by the writer: the database id of
/// the root for a season, and none for a root (the backup's raw `parentId` is never written, D7).
fn entry_of(b: &BackupAnime) -> Entry {
    Entry {
        source: b.source,
        url: b.url.clone(),
        artist: b.artist.clone(),
        author: b.author.clone(),
        description: b.description.clone(),
        genre: Some(b.genre.clone()),
        title: b.title.clone(),
        status: i64::from(b.status),
        thumbnail_url: b.thumbnail_url.clone(),
        favorite: b.favorite,
        initialized: b.description.is_some(),
        viewer_flags: i64::from(b.viewer_flags),
        item_flags: i64::from(b.episode_flags),
        date_added: b.date_added,
        update_strategy: b.update_strategy,
        version: b.version,
        fetch_type: Some(b.fetch_type),
        season_flags: Some(b.season_flags),
        season_number: Some(b.season_number),
        season_source_order: Some(b.season_source_order),
        background_url: b.background_url.clone(),
        memo: Some(memo_of(&b.memo)),
        ..Entry::create(Kind::Anime)
    }
}

fn item_of(b: &BackupEpisode) -> Item {
    Item {
        url: b.url.clone(),
        name: b.name.clone(),
        scanlator: b.scanlator.clone(),
        done: b.seen,
        bookmark: b.bookmark,
        fillermark: b.fillermark,
        progress: b.last_second_seen,
        number: round6(b.episode_number),
        source_order: b.source_order,
        date_fetch: b.date_fetch,
        date_upload: b.date_upload,
        version: b.version,
        total_seconds: Some(b.total_seconds),
        summary: b.summary.clone(),
        preview_url: b.preview_url.clone(),
        memo: Some(memo_of(&b.memo)),
        ..Item::create(Kind::Anime)
    }
}

fn track_of(b: &BackupAnimeTracking) -> Track {
    Track {
        remote_id: b.remote_id(),
        library_id: Some(b.library_id),
        title: b.title.clone(),
        last_item: f64::from(b.last_episode_seen),
        total_items: i64::from(b.total_episodes),
        status: i64::from(b.status),
        score: f64::from(b.score),
        remote_url: b.tracking_url.clone(),
        start_date: b.started_watching_date,
        finish_date: b.finished_watching_date,
        private: b.private,
        ..Track::new(Kind::Anime, i64::from(b.sync_id))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};
    use kuro_db::schema_unified;
    use kuro_db::tables::{EntryRow, categories, entries, history, items, tracks};
    use rusqlite::Connection;

    use super::*;
    use crate::schema::BackupAnimeHistory;

    fn clock() -> RunClock {
        RunClock::new(Local.with_ymd_and_hms(2026, 10, 9, 12, 0, 0).single().unwrap())
    }

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let c = schema_unified::open_file(&dir.path().join("kurobayomi.db")).unwrap();
        (dir, c)
    }

    fn anime(url: &str) -> BackupAnime {
        BackupAnime { source: 3, url: url.into(), title: format!("A {url}"), ..Default::default() }
    }

    fn episode(url: &str, seen: bool, last: i64) -> BackupEpisode {
        BackupEpisode { url: url.into(), name: format!("ep {url}"), seen, last_second_seen: last, episode_number: 2.0, version: 1, ..Default::default() }
    }

    fn by_url(c: &Connection, url: &str) -> EntryRow {
        entries::by_url_and_source(c, Kind::Anime, url, 3).unwrap().unwrap()
    }

    fn restore(c: &mut Connection, root: &AnimeRoot) {
        let tx = c.transaction().unwrap();
        restore_anime_root(&tx, &clock(), root, &HashMap::new()).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn new_row_with_mapped_categories_keeps_the_backup_version() {
        let (_d, mut c) = db();
        let cat = categories::insert(&c, Kind::Anime, "A", 0, 0).unwrap();
        let mut a = anime("ver");
        a.version = 6;
        a.categories = vec![0];
        let tx = c.transaction().unwrap();
        restore_anime_root(&tx, &clock(), &AnimeRoot { entry: a, seasons: vec![] }, &HashMap::from([(0, cat)])).unwrap();
        tx.commit().unwrap();
        assert_eq!(by_url(&c, "ver").version, 6);
    }

    #[test]
    fn season_rows_are_not_top_level_and_take_the_root_db_id() {
        let (_d, mut c) = db();
        let mut root = anime("root");
        root.id = Some(10);
        let mut season = anime("season-1");
        season.id = Some(11);
        season.parent_id = Some(10);
        season.season_number = 1.0;
        let roots = plan_anime(vec![root.clone(), season.clone()]);
        assert_eq!(roots.len(), 1, "season is not a root");
        assert_eq!(roots[0].seasons.len(), 1);
        restore(&mut c, &roots[0]);
        let parent = by_url(&c, "root");
        let child = by_url(&c, "season-1");
        assert_eq!(child.parent_id, Some(parent.id), "D7: parent is the db id, not the backup id");
        assert!(parent.parent_id.is_none());
    }

    #[test]
    fn season_with_missing_parent_becomes_a_top_level_row() {
        let (_d, mut c) = db();
        let mut orphan = anime("orphan");
        orphan.id = Some(5);
        orphan.parent_id = Some(999);
        let roots = plan_anime(vec![orphan]);
        assert_eq!(roots.len(), 1);
        assert!(roots[0].seasons.is_empty());
        restore(&mut c, &roots[0]);
        assert!(by_url(&c, "orphan").parent_id.is_none(), "X-43: parent_id stays NULL");
    }

    #[test]
    fn existing_root_keeps_its_parent_link() {
        let (_d, mut c) = db();
        let mut root = anime("keep");
        root.id = Some(1);
        restore(&mut c, &AnimeRoot { entry: root.clone(), seasons: vec![] });
        let db_id = by_url(&c, "keep").id;
        c.execute("UPDATE entries SET parent_id = 42 WHERE _id = ?1", [db_id]).unwrap();
        root.parent_id = Some(7);
        restore(&mut c, &AnimeRoot { entry: root, seasons: vec![] });
        assert_eq!(by_url(&c, "keep").parent_id, Some(42), "a root never writes the raw backup parent");
    }

    #[test]
    fn episodes_merge_seen_and_position_and_bookmark_or() {
        let (_d, mut c) = db();
        let mut a = anime("eps");
        a.episodes = vec![episode("e1", true, 0), episode("e2", false, 300)];
        restore(&mut c, &AnimeRoot { entry: a.clone(), seasons: vec![] });
        let id = by_url(&c, "eps").id;

        let mut a2 = anime("eps");
        a2.episodes = vec![
            BackupEpisode { bookmark: true, ..episode("e1", false, 0) },
            BackupEpisode { seen: true, last_second_seen: 120, ..episode("e2", true, 120) },
            episode("e3", false, 0),
        ];
        restore(&mut c, &AnimeRoot { entry: a2, seasons: vec![] });
        let rows = items::by_entry(&c, id, false).unwrap();
        let get = |u: &str| rows.iter().find(|r| r.url == u).unwrap().clone();
        assert_eq!(rows.len(), 3);
        assert!(get("e1").done, "seen never reverts");
        assert!(get("e1").bookmark, "bookmark is OR");
        assert_eq!(get("e2").progress, 120);
        assert!((get("e2").number - 2.0).abs() < f64::EPSILON);
        assert!(!get("e3").done);
    }

    #[test]
    fn season_episodes_are_restored_under_the_season() {
        let (_d, mut c) = db();
        let mut root = anime("r2");
        root.id = Some(1);
        let mut season = anime("s2");
        season.id = Some(2);
        season.parent_id = Some(1);
        season.episodes = vec![episode("only-in-season", false, 0)];
        let roots = plan_anime(vec![root, season]);
        restore(&mut c, &roots[0]);
        let s = by_url(&c, "s2");
        let eps = items::by_url_and_entry(&c, "only-in-season", s.id).unwrap();
        assert_eq!(eps.len(), 1);
    }

    #[test]
    fn history_uses_the_later_timestamp_and_keeps_the_larger_duration() {
        let (_d, mut c) = db();
        let mut a = anime("hist");
        a.episodes = vec![episode("h1", true, 0)];
        a.history = vec![BackupAnimeHistory { url: "h1".into(), last_read: 500, read_duration: 77 }];
        restore(&mut c, &AnimeRoot { entry: a.clone(), seasons: vec![] });
        let id = by_url(&c, "hist").id;
        let h = |c: &Connection| history::by_entry(c, id).unwrap()[0];
        assert_eq!((h(&c).last_at, h(&c).duration_ms), (500, 77), "the backup duration is stored, not dropped");

        a.history = vec![BackupAnimeHistory { url: "h1".into(), last_read: 100, read_duration: 20 }];
        restore(&mut c, &AnimeRoot { entry: a.clone(), seasons: vec![] });
        assert_eq!((h(&c).last_at, h(&c).duration_ms), (500, 77), "max keeps the later time and the larger duration");

        a.history = vec![BackupAnimeHistory { url: "h1".into(), last_read: 100, read_duration: 90 }];
        restore(&mut c, &AnimeRoot { entry: a, seasons: vec![] });
        assert_eq!((h(&c).last_at, h(&c).duration_ms), (500, 90));
    }

    #[test]
    fn categories_are_replaced_when_an_order_maps() {
        let (_d, mut c) = db();
        let cat = categories::insert(&c, Kind::Anime, "Watching", 0, 0).unwrap();
        let mut a = anime("cat-a");
        a.categories = vec![0];
        let tx = c.transaction().unwrap();
        restore_anime_root(&tx, &clock(), &AnimeRoot { entry: a, seasons: vec![] }, &HashMap::from([(0, cat)])).unwrap();
        tx.commit().unwrap();
        let id = by_url(&c, "cat-a").id;
        let got: Vec<i64> = categories::by_entry(&c, Kind::Anime, id).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(got, vec![cat]);
    }

    #[test]
    fn sort_and_tracks_follow_the_manga_rules() {
        let (_d, mut c) = db();
        let existing: HashSet<(i64, String)> = [(3, "old".to_string())].into_iter().collect();
        let mut fresh = anime("fresh");
        fresh.last_modified_at = 1;
        let sorted = sort_anime(vec![anime("old"), fresh], &existing);
        assert_eq!(sorted[0].url, "fresh");

        let mut a = anime("trk");
        a.tracking = vec![BackupAnimeTracking { sync_id: 4, media_id: 8, library_id: 1, last_episode_seen: 2.0, ..Default::default() }];
        restore(&mut c, &AnimeRoot { entry: a.clone(), seasons: vec![] });
        a.tracking[0].last_episode_seen = 1.0;
        a.tracking[0].media_id = 9;
        restore(&mut c, &AnimeRoot { entry: a, seasons: vec![] });
        let id = by_url(&c, "trk").id;
        let t = &tracks::by_entry(&c, id).unwrap()[0];
        assert_eq!((t.remote_id, t.last_item), (9, 2.0));
    }

    #[test]
    fn memo_bytes_decode_and_empty_is_an_empty_object() {
        assert!(memo_of(b"").0.is_empty());
        assert_eq!(memo_of(br#"{"k":1}"#).0.get("k"), Some(&serde_json::json!(1)));
    }
}
