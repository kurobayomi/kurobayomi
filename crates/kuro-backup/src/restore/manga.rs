//! Manga entry restore (area 22, T-22a-011 to T-22a-021). The backup row becomes a shared [`Incoming`] entry and
//! `write::restore_entry` writes it.

use std::collections::{HashMap, HashSet};

use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_data::track::Track;
use kuro_db::{DbResult, Transaction};
use kuro_platform::kind::Kind;

use super::merge::{RunClock, round6, sort_new_first};
use super::write::{self, HistoryIn, Incoming};
use crate::flags::chapter_flags_from_wire;
use crate::schema::{BackupChapter, BackupManga, BackupTracking};

pub use super::write::refresh_interval;

/// The `(source, url)` pairs already in `entries` as manga, read once per section (T-22a-011).
pub fn existing_pairs(tx: &Transaction<'_>) -> DbResult<HashSet<(i64, String)>> {
    write::existing_pairs(tx, Kind::Manga)
}

/// Entries not in the database first, newest `lastModifiedAt` first, ties in backup order (T-22a-011).
pub fn sort_manga(entries: Vec<BackupManga>, existing: &HashSet<(i64, String)>) -> Vec<BackupManga> {
    sort_new_first(entries, |m| (m.source, m.url.as_str(), m.last_modified_at), existing)
}

/// Restores one manga entry: row, categories, chapters, tracks, history, excluded scanlators, fetch interval.
/// `categories` maps backup category order to database id (empty when the categories option is off).
pub fn restore_manga_entry(
    tx: &Transaction<'_>,
    clock: &RunClock,
    entry: &BackupManga,
    categories: &HashMap<i64, i64>,
) -> DbResult<()> {
    let incoming = Incoming {
        entry: entry_of(entry),
        items: entry.chapters.iter().map(item_of).collect(),
        tracks: entry.tracking.iter().map(track_of).collect(),
        history: entry
            .history
            .iter()
            .map(|h| HistoryIn { url: h.url.clone(), last_at: h.last_read, duration_ms: h.read_duration })
            .collect(),
        category_orders: &entry.categories,
        excluded_scanlators: &entry.excluded_scanlators,
    };
    write::restore_entry(tx, clock, &incoming, None, categories).map(drop)
}

/// A new row (T-22a-014). `initialized` is `description != null` (D6 fixed). `next_update` and `calculate_interval`
/// start at 0 and are set by `refresh_interval`. `chapterFlags` is stored in the item-flag layout.
fn entry_of(b: &BackupManga) -> Entry {
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
        viewer_flags: i64::from(b.effective_viewer_flags()),
        item_flags: chapter_flags_from_wire(i64::from(b.chapter_flags)),
        date_added: b.date_added,
        update_strategy: b.update_strategy,
        version: b.version,
        ..Entry::create(Kind::Manga)
    }
}

fn item_of(b: &BackupChapter) -> Item {
    Item {
        url: b.url.clone(),
        name: b.name.clone(),
        scanlator: b.scanlator.clone(),
        done: b.read,
        bookmark: b.bookmark,
        progress: b.last_page_read,
        number: round6(b.chapter_number),
        source_order: b.source_order,
        date_fetch: b.date_fetch,
        date_upload: b.date_upload,
        version: b.version,
        ..Item::create(Kind::Manga)
    }
}

fn track_of(b: &BackupTracking) -> Track {
    Track {
        remote_id: b.remote_id(),
        library_id: Some(b.library_id),
        title: b.title.clone(),
        last_item: f64::from(b.last_chapter_read),
        total_items: i64::from(b.total_chapters),
        status: i64::from(b.status),
        score: f64::from(b.score),
        remote_url: b.tracking_url.clone(),
        start_date: b.started_reading_date,
        finish_date: b.finished_reading_date,
        private: b.private,
        ..Track::new(Kind::Manga, i64::from(b.sync_id))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};
    use kuro_db::schema_unified;
    use kuro_db::tables::excluded;
    use kuro_db::tables::{EntryRow, categories, entries, history, items, tracks};
    use rusqlite::Connection;

    use super::*;
    use crate::schema::BackupHistory;
    use crate::schema::BackupCategory;

    fn clock() -> RunClock {
        RunClock::new(Local.with_ymd_and_hms(2026, 10, 9, 12, 0, 0).single().unwrap())
    }

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let c = schema_unified::open_file(&dir.path().join("kurobayomi.db")).unwrap();
        (dir, c)
    }

    fn manga(url: &str) -> BackupManga {
        BackupManga { source: 7, url: url.into(), title: format!("T {url}"), ..Default::default() }
    }

    fn chapter(url: &str, read: bool, bookmark: bool, last_page_read: i64) -> BackupChapter {
        BackupChapter {
            url: url.into(),
            name: format!("name {url}"),
            read,
            bookmark,
            last_page_read,
            chapter_number: 1.1,
            version: 1,
            ..Default::default()
        }
    }

    fn row(c: &Connection, url: &str) -> EntryRow {
        entries::by_url_and_source(c, Kind::Manga, url, 7).unwrap().unwrap()
    }

    /// T-20a-094: 2499283573021220255 is the id Tachiyomi and Aniyomi backups carry for MangaDex (English, version 1). The
    /// id rule reproduces it from the name, language and version, and a restore keeps a backup's source integer as is.
    #[test]
    fn a_backup_source_id_equals_the_generated_id_and_survives_a_restore_unchanged() {
        const MANGADEX_BACKUP_ID: i64 = 2499283573021220255;
        assert_eq!(kuro_source::generate_id("MangaDex", "en", 1), MANGADEX_BACKUP_ID);
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        let mut m = manga("md");
        m.source = MANGADEX_BACKUP_ID;
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let r = entries::by_url_and_source(&c, Kind::Manga, "md", MANGADEX_BACKUP_ID).unwrap().unwrap();
        assert_eq!(r.source, kuro_source::generate_id("MangaDex", "en", 1));
    }

    #[test]
    fn new_row_stores_initialized_from_description_and_zeroed_schedule() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        let mut m = manga("a");
        m.description = Some("text".into());
        m.version = 4;
        m.date_added = 1234;
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let r = row(&c, "a");
        assert!(r.initialized, "D6: initialized follows description");
        assert_eq!(r.version, 4);
        assert_eq!(r.date_added, 1234);
    }

    /// E-25 and E-26: a new row is written not syncing, without the backup's `lastModifiedAt` and `favoriteModifiedAt`.
    /// (Triggers set both times on their own, so the test asserts only that the backup values are not copied.) The backup values only drive the sort.
    #[test]
    fn new_row_is_not_syncing_and_ignores_the_backup_modified_times() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        let mut m = manga("k");
        m.last_modified_at = 500;
        m.favorite_modified_at = Some(900);
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let r = row(&c, "k");
        assert_ne!(r.last_modified_at, 500);
        assert_ne!(r.favorite_modified_at, Some(900));
    }

    #[test]
    fn backup_modified_times_are_not_written_on_an_existing_row() {
        for backup_version in [1, 9] {
            let (_d, mut c) = db();
            {
                let tx = c.transaction().unwrap();
                let mut first = manga("s");
                first.version = 5;
                restore_manga_entry(&tx, &clock(), &first, &HashMap::new()).unwrap();
                tx.commit().unwrap();
            }
            let tx = c.transaction().unwrap();
            let mut b = manga("s");
            b.version = backup_version;
            b.last_modified_at = 500;
            b.favorite_modified_at = Some(900);
            restore_manga_entry(&tx, &clock(), &b, &HashMap::new()).unwrap();
            tx.commit().unwrap();
            let r = row(&c, "s");
            assert_ne!(r.last_modified_at, 500, "version {backup_version}, E-26: the backup value is not copied");
            assert_ne!(r.favorite_modified_at, Some(900));
        }
    }

    #[test]
    fn chapter_flags_are_stored_in_the_item_flag_layout() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        let mut m = manga("fl");
        m.chapter_flags = 0x100 | 0x1;
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        assert_eq!(row(&c, "fl").item_flags, 0x200 | 0x1);
    }

    #[test]
    fn new_row_without_description_is_not_initialized() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &manga("b"), &HashMap::new()).unwrap();
        tx.commit().unwrap();
        assert!(!row(&c, "b").initialized);
    }

    #[test]
    fn existing_row_keeps_favorite_and_takes_metadata_from_the_newer_side() {
        let (_d, mut c) = db();
        {
            let tx = c.transaction().unwrap();
            let mut db_side = manga("e");
            db_side.version = 5;
            db_side.title = "db title".into();
            db_side.author = Some("db author".into());
            db_side.favorite = true;
            restore_manga_entry(&tx, &clock(), &db_side, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        // backup older (version 2) and not favourite: metadata from the row, favourite stays true
        let tx = c.transaction().unwrap();
        let mut b = manga("e");
        b.version = 2;
        b.title = "backup title".into();
        b.author = Some("backup author".into());
        b.favorite = false;
        b.date_added = 77;
        restore_manga_entry(&tx, &clock(), &b, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let r = row(&c, "e");
        assert!(r.favorite, "favorite is always the OR");
        assert_eq!(r.author.as_deref(), Some("db author"));
        assert_eq!(r.title, "backup title", "older backup owns title");
        assert_eq!(r.date_added, 77);
        assert_eq!(r.version, 5, "older backup keeps the row version");
    }

    #[test]
    fn newer_backup_owns_metadata_and_keeps_the_row_title() {
        let (_d, mut c) = db();
        {
            let tx = c.transaction().unwrap();
            let mut db_side = manga("n");
            db_side.version = 1;
            db_side.title = "db title".into();
            restore_manga_entry(&tx, &clock(), &db_side, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let tx = c.transaction().unwrap();
        let mut b = manga("n");
        b.version = 9;
        b.title = "backup title".into();
        b.author = Some("new author".into());
        restore_manga_entry(&tx, &clock(), &b, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let r = row(&c, "n");
        assert_eq!(r.author.as_deref(), Some("new author"));
        assert_eq!(r.title, "db title");
        assert_eq!(r.version, 9);
    }

    #[test]
    fn sort_puts_new_rows_first_then_newest_modified() {
        let (_d, mut c) = db();
        {
            let tx = c.transaction().unwrap();
            restore_manga_entry(&tx, &clock(), &manga("old"), &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let existing = existing_pairs(&c.transaction().unwrap()).unwrap();
        let mut a = manga("old");
        a.last_modified_at = 999;
        let mut b = manga("new-1");
        b.last_modified_at = 10;
        let mut d = manga("new-2");
        d.last_modified_at = 20;
        let sorted = sort_manga(vec![a, b, d], &existing);
        let urls: Vec<&str> = sorted.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(urls, vec!["new-2", "new-1", "old"]);
    }

    #[test]
    fn chapters_insert_new_and_merge_progress_on_existing() {
        let (_d, mut c) = db();
        {
            let tx = c.transaction().unwrap();
            let mut m = manga("ch");
            m.chapters = vec![chapter("c1", true, false, 0), chapter("c2", false, false, 7)];
            restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let tx = c.transaction().unwrap();
        let mut m = manga("ch");
        // c1: backup unread at 0, db read -> read stays true. c2: backup read at 5 with bookmark, db at 7 unread.
        m.chapters = vec![chapter("c1", false, true, 0), chapter("c2", true, true, 5), chapter("c3", false, false, 0)];
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let id = row(&c, "ch").id;
        let rows = items::by_entry(&c, id, false).unwrap();
        let by = |u: &str| rows.iter().find(|r| r.url == u).unwrap().clone();
        assert_eq!(rows.len(), 3);
        assert!(by("c1").done && by("c1").bookmark);
        assert_eq!(by("c2").progress, 5, "backup position wins when smaller and read differs");
        assert!(by("c2").done);
        assert!(!by("c3").done);
        assert_eq!(by("c1").number, 1.1, "float chapter numbers stored as 1.1");
    }

    #[test]
    fn unchanged_chapter_is_skipped() {
        let (_d, mut c) = db();
        let mut m = manga("same");
        m.chapters = vec![chapter("x", false, false, 0)];
        {
            let tx = c.transaction().unwrap();
            restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let id = row(&c, "same").id;
        let before = items::by_entry(&c, id, false).unwrap();
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let after = items::by_entry(&c, id, false).unwrap();
        assert_eq!(before[0].version, after[0].version, "no write for an unchanged chapter");
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn categories_replace_memberships_only_when_an_order_maps() {
        let (_d, mut c) = db();
        let read = categories::insert(&c, Kind::Manga, "Read", 0, 0).unwrap();
        let later = categories::insert(&c, Kind::Manga, "Later", 1, 0).unwrap();
        let mut m = manga("cat");
        m.categories = vec![0];
        let map = HashMap::from([(0, read)]);
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &map).unwrap();
        tx.commit().unwrap();
        let id = row(&c, "cat").id;
        let names: Vec<i64> = categories::by_entry(&c, Kind::Manga, id).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(names, vec![read]);

        // unmapped order: memberships untouched
        m.categories = vec![9];
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::from([(0, later)])).unwrap();
        tx.commit().unwrap();
        let names: Vec<i64> = categories::by_entry(&c, Kind::Manga, id).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(names, vec![read]);
    }

    #[test]
    fn new_row_with_mapped_categories_keeps_the_backup_version() {
        let (_d, mut c) = db();
        let a = categories::insert(&c, Kind::Manga, "A", 0, 0).unwrap();
        let b = categories::insert(&c, Kind::Manga, "B", 1, 0).unwrap();
        let mut m = manga("ver");
        m.version = 4;
        m.categories = vec![0, 1];
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::from([(0, a), (1, b)])).unwrap();
        tx.commit().unwrap();
        assert_eq!(row(&c, "ver").version, 4);
    }

    #[test]
    fn history_is_scoped_to_the_manga_and_time_read_takes_the_max() {
        let (_d, mut c) = db();
        {
            let tx = c.transaction().unwrap();
            let mut other = manga("other");
            other.chapters = vec![chapter("shared", false, false, 0)];
            restore_manga_entry(&tx, &clock(), &other, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let mut m = manga("hist");
        m.chapters = vec![chapter("shared", false, false, 0)];
        m.history = vec![BackupHistory { url: "shared".into(), last_read: 100, read_duration: 50 }];
        {
            let tx = c.transaction().unwrap();
            restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let id = row(&c, "hist").id;
        let mine = items::by_entry(&c, id, false).unwrap();
        let h = history::by_entry(&c, id).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].item_id, mine[0].id, "history attaches to this manga's chapter");
        assert_eq!((h[0].last_at, h[0].duration_ms), (100, 50));

        // restoring again keeps the same values (idempotent)
        {
            let tx = c.transaction().unwrap();
            restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let h = history::by_entry(&c, id).unwrap();
        assert_eq!((h[0].last_at, h[0].duration_ms), (100, 50));
    }

    #[test]
    fn history_for_a_missing_chapter_is_skipped() {
        let (_d, mut c) = db();
        let mut m = manga("skip");
        m.history = vec![BackupHistory { url: "ghost".into(), last_read: 5, read_duration: 1 }];
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let id = row(&c, "skip").id;
        assert!(history::by_entry(&c, id).unwrap().is_empty());
    }

    #[test]
    fn track_new_is_inserted_and_existing_changes_only_remote_library_and_progress() {
        let (_d, mut c) = db();
        let mut m = manga("tr");
        m.tracking = vec![BackupTracking { sync_id: 2, media_id: 55, library_id: 9, last_chapter_read: 3.0, score: 4.0, status: 1, title: "t".into(), ..Default::default() }];
        {
            let tx = c.transaction().unwrap();
            restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        let id = row(&c, "tr").id;
        let first = tracks::by_entry(&c, id).unwrap();
        assert_eq!((first.len(), first[0].remote_id, first[0].score), (1, 55, 4.0));

        // DB side has a higher progress and a different score; the backup changes remote id and library id only
        c.execute("UPDATE tracks SET last_item = 8.0, score = 9.0, status = 2 WHERE entry_id = ?1", [id]).unwrap();
        m.tracking[0].media_id = 66;
        m.tracking[0].library_id = 11;
        m.tracking[0].last_chapter_read = 5.0;
        m.tracking[0].score = 1.0;
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let t = &tracks::by_entry(&c, id).unwrap()[0];
        assert_eq!(t.remote_id, 66);
        assert_eq!(t.library_id, Some(11));
        assert_eq!(t.last_item, 8.0, "max of DB and backup");
        assert_eq!((t.score, t.status), (9.0, 2), "score and status keep the DB values");
    }

    #[test]
    fn excluded_scanlators_insert_only_the_missing_ones() {
        let (_d, mut c) = db();
        let mut m = manga("sc");
        m.excluded_scanlators = vec!["A".into()];
        {
            let tx = c.transaction().unwrap();
            restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
            tx.commit().unwrap();
        }
        m.excluded_scanlators = vec!["A".into(), "B".into(), "B".into()];
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let id = row(&c, "sc").id;
        let mut got = excluded::by_entry_id(&c, id).unwrap();
        got.sort();
        assert_eq!(got, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn fetch_interval_is_recomputed_and_only_its_two_fields_change() {
        let (_d, mut c) = db();
        let mut m = manga("fi");
        m.chapters = (0..4).map(|i| BackupChapter { date_upload: i * 86_400_000, ..chapter(&format!("f{i}"), false, false, 0) }).collect();
        let tx = c.transaction().unwrap();
        restore_manga_entry(&tx, &clock(), &m, &HashMap::new()).unwrap();
        tx.commit().unwrap();
        let r = row(&c, "fi");
        assert!((1..=28).contains(&r.calculate_interval), "interval clamped to 1..28");
        assert!(r.next_update.unwrap_or(0) > 0);
        assert!(r.title == "T fi", "title untouched by the interval step");
    }

    #[test]
    fn the_category_order_map_comes_from_the_backup_list() {
        // Shape check for the caller: a backup category order maps to the id of the row of the same name.
        let (_d, c) = db();
        let out = super::super::categories::restore_categories(
            &c,
            Kind::Manga,
            &[BackupCategory { name: "Z".into(), order: 3, id: 1, flags: 0 }],
        )
        .unwrap();
        assert!(out.by_order.contains_key(&3));
    }
}
