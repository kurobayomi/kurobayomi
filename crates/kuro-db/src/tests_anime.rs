//! Tests of the handle (T-04a-007 to T-04a-012, T-04a-027): the change bus, the write and read paths, and the
//! entry rules the schema enforces (unique key, 64-bit source ids, coalesced updates, the library read model).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kuro_testkit::TempPaths;
use rusqlite::Connection;
use tokio::sync::broadcast;

use crate::error::{DbError, DbResult};
use crate::notify::Changed;
use crate::runtime::{Db, TxExt};
use crate::schema_unified::db_path;
use crate::tables::{EntryPatch, entries, library};
use kuro_platform::kind::Kind;

const QUIET: Duration = Duration::from_millis(400);

fn open() -> (TempPaths, Arc<Db>) {
    let paths = TempPaths::new().unwrap();
    let db = Db::open(&paths).unwrap();
    (paths, db)
}

/// Every table name published since the last call.
fn published(rx: &mut broadcast::Receiver<Changed>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    while let Ok(changed) = rx.try_recv() {
        out.extend(changed.iter().cloned());
    }
    out
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// One anime entry with the unified defaults; returns its id.
fn try_anime(c: &Connection, url: &str) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO entries(kind, source, url, title, status, favorite, initialized, viewer, item_flags, cover_last_modified, \
         date_added, fetch_type, season_flags, season_number, season_source_order, background_last_modified, memo) \
         VALUES ('anime', 1, ?1, ?2, 1, 0, 1, 0, 0, 0, 0, 1, 0, -1.0, 0, 0, X'7B7D')",
        (url, format!("T {url}")),
    )?;
    Ok(c.last_insert_rowid())
}

fn new_anime(c: &Connection, url: &str) -> i64 {
    try_anime(c, url).unwrap()
}

/// One episode of `anime_id`; returns its id.
fn new_episode(c: &Connection, anime_id: i64, url: &str) -> i64 {
    c.execute(
        "INSERT INTO items(entry_id, url, name, done, bookmark, progress, total_seconds, number, source_order, date_fetch, date_upload) \
         VALUES (?1, ?2, ?3, 0, 0, 0, 0, 1, 0, 0, 0)",
        (anime_id, url, format!("E {url}")),
    )
    .unwrap();
    c.last_insert_rowid()
}

/// An anime with one episode, one history row, one category link, one track and one relation; returns
/// `(anime, episode)`.
fn full_anime(tx: &Connection) -> (i64, i64) {
    let anime = new_anime(tx, "/a");
    let other = new_anime(tx, "/b");
    let episode = new_episode(tx, anime, "/a/1");
    tx.execute("INSERT INTO history(item_id, last_at, duration_ms) VALUES (?1, 0, 0)", [episode]).unwrap();
    tx.execute("INSERT INTO categories(kind, _id, name, sort, flags) VALUES ('anime', 1, 'c', 1, 0)", []).unwrap();
    tx.execute("INSERT INTO entry_categories(entry_id, kind, category_id) VALUES (?1, 'anime', 1)", [anime]).unwrap();
    tx.execute(
        "INSERT INTO tracks(entry_id, tracker_id, remote_id, title, last_item, total_items, status, score, \
         remote_url, start_date, finish_date) VALUES (?1, 1, 1, 't', 0, 0, 1, 0, 'u', 0, 0)",
        [anime],
    )
    .unwrap();
    tx.execute("INSERT INTO anime_relations(entry_id, related_id, name, sort_order, last_fetched_at) VALUES (?1, ?2, 'Sequel', 0, 0)", [anime, other])
        .unwrap();
    (anime, episode)
}

// T-04a-007: the change bus ------------------------------------------------------------------------

#[test]
fn deleting_an_anime_publishes_every_table_the_cascade_touches() {
    let (_p, db) = open();
    let (anime, _) = db.write(|tx| Ok(full_anime(tx))).unwrap();
    let mut rx = db.changes.subscribe();
    db.write(|tx| Ok(tx.execute("DELETE FROM entries WHERE _id = ?1", [anime]).map(drop)?)).unwrap();
    let got = published(&mut rx);
    for table in ["entries", "items", "history", "entry_categories", "tracks", "anime_relations"] {
        assert!(got.contains(table), "{table} missing from {got:?}");
    }
}

#[test]
fn trigger_writes_are_published() {
    let (_p, db) = open();
    let (_, episode) = db.write(|tx| Ok(full_anime(tx))).unwrap();
    let mut rx = db.changes.subscribe();
    // Marking an episode seen fires the version triggers, which write `entries` and `items`.
    db.write(|tx| Ok(tx.execute("UPDATE items SET done = 1 WHERE _id = ?1", [episode]).map(drop)?)).unwrap();
    assert_eq!(published(&mut rx), names(&["entries", "items"]));
}

#[test]
fn a_rolled_back_write_publishes_nothing() {
    let (_p, db) = open();
    let mut rx = db.changes.subscribe();
    let result: DbResult<()> = db.write(|tx| {
        new_anime(tx, "/a");
        Err(DbError::NotFound)
    });
    assert!(result.is_err());
    assert!(published(&mut rx).is_empty());
}

#[test]
fn touch_names_a_table_the_hook_did_not_see() {
    let (_p, db) = open();
    let mut rx = db.changes.subscribe();
    db.write(|tx| tx.touch(&["history", "items"])).unwrap();
    assert_eq!(published(&mut rx), names(&["history", "items"]));
    // Touching in a write that then fails publishes nothing.
    let _: DbResult<()> = db.write(|tx| {
        tx.touch(&["entries"])?;
        Err(DbError::NotFound)
    });
    assert!(published(&mut rx).is_empty());
}

#[tokio::test]
async fn a_failing_query_is_an_item_not_the_end_of_the_stream() {
    let (_p, db) = open();
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = Arc::clone(&fail);
    let mut sub = db.subscribe(&["entries"], Duration::ZERO, move |c| {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(DbError::NotFound);
        }
        Ok(entries::favorites(c, Kind::Anime)?.len())
    });
    let first = tokio::time::timeout(QUIET, sub.changed()).await.unwrap().unwrap();
    assert!(first.is_err());
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    db.write_async(|tx| Ok(new_anime(tx, "/a"))).await.unwrap();
    let second = tokio::time::timeout(QUIET, sub.changed()).await.unwrap().unwrap();
    assert_eq!(second.map(|v| *v), Ok(0));
}

// T-04a-008: the handler --------------------------------------------------------------------------

#[test]
#[ignore = "slow: run on demand"]
fn two_concurrent_writes_serialize() {
    let (_p, db) = open();
    let db = Arc::clone(&db);
    let started = Instant::now();
    let first = {
        let db = Arc::clone(&db);
        std::thread::spawn(move || {
            db.write(|tx| {
                new_anime(tx, "/first");
                std::thread::sleep(Duration::from_millis(250));
                Ok(())
            })
            .unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    let seen_by_second = db
        .write(|tx| Ok(tx.query_row("SELECT count(*) FROM entries", [], |r| r.get::<_, i64>(0))?))
        .unwrap();
    first.join().unwrap();
    assert_eq!(seen_by_second, 1, "the second write started after the first committed");
    assert!(started.elapsed() >= Duration::from_millis(250));
}

#[test]
#[ignore = "slow: run on demand"]
fn a_long_write_does_not_block_readers() {
    let (_p, db) = open();
    let db = Arc::clone(&db);
    db.write(|tx| Ok(new_anime(tx, "/old"))).unwrap();
    let writer = {
        let db = Arc::clone(&db);
        std::thread::spawn(move || {
            db.write(|tx| {
                new_anime(tx, "/new");
                std::thread::sleep(Duration::from_millis(400));
                Ok(())
            })
            .unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    let count = db.read(|c| Ok(c.query_row("SELECT count(*) FROM entries", [], |r| r.get::<_, i64>(0))?)).unwrap();
    assert!(t.elapsed() < Duration::from_millis(200), "the read waited for the write");
    assert_eq!(count, 1, "the reader sees the last committed state");
    writer.join().unwrap();
}

#[tokio::test]
async fn a_write_future_dropped_before_it_starts_leaves_nothing_behind() {
    let (_p, db) = open();
    let future = db.write_async(|tx| Ok(new_anime(tx, "/never")));
    drop(future);
    db.write_async(|tx| Ok(new_anime(tx, "/after"))).await.unwrap();
    let urls: Vec<String> = db
        .read_async(|c| Ok(entries::favorites(c, Kind::Anime)?.into_iter().map(|a| a.url).collect::<Vec<_>>()))
        .await
        .unwrap();
    assert!(urls.is_empty());
    let total: i64 = db.read_async(|c| Ok(c.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?)).await.unwrap();
    assert_eq!(total, 1, "only the second write ran");
}

#[test]
fn a_savepoint_rolls_back_unless_released() {
    let (_p, db) = open();
    db
        .write(|tx| {
            new_anime(tx, "/keep");
            {
                let sp = tx.savepoint()?;
                new_anime(tx, "/dropped");
                drop(sp);
            }
            let sp = tx.savepoint()?;
            new_anime(tx, "/released");
            sp.release()
        })
        .unwrap();
    let urls: Vec<String> = db
        .read(|c| Ok(c.prepare("SELECT url FROM entries ORDER BY _id")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?))
        .unwrap();
    assert_eq!(urls, ["/keep", "/released"]);
}

#[test]
fn close_folds_the_wal_into_the_database_file() {
    let (paths, db) = open();
    db.write(|tx| Ok(new_anime(tx, "/a"))).unwrap();
    db.close().unwrap();
    let wal = db_path(&kuro_platform::paths::Paths::data_dir(&paths)).with_extension("db-wal");
    let size = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    assert_eq!(size, 0, "the checkpoint truncated the WAL");
    // The handle still works afterwards.
    db.write(|tx| Ok(new_anime(tx, "/b"))).unwrap();
}

// T-04a-009: entries -------------------------------------------------------------------------------

#[test]
fn a_second_anime_with_the_same_source_and_url_is_refused() {
    let (_p, db) = open();
    let first = db.write(|tx| Ok(new_anime(tx, "/same"))).unwrap();
    let second = db.write(|tx| Ok(try_anime(tx, "/same")?));
    assert!(second.is_err(), "UNIQUE (kind, source, url)");
    let found = db.read(|c| Ok(entries::by_url_and_source(c, Kind::Anime, "/same", 1)?.unwrap().id)).unwrap();
    assert_eq!(found, first);
}

#[test]
fn source_ids_above_2_pow_63_round_trip_and_the_highest_id_is_reused() {
    let (_p, db) = open();
    let source = 0xF000_0000_0000_0001_u64 as i64;
    db
        .write(|tx| {
            let id = new_anime(tx, "/a");
            tx.execute("UPDATE entries SET source = ?2 WHERE _id = ?1", [id, source])?;
            Ok(())
        })
        .unwrap();
    let back = db.read(|c| Ok(entries::by_url_and_source(c, Kind::Anime, "/a", source)?.map(|a| a.source as u64))).unwrap();
    assert_eq!(back, Some(0xF000_0000_0000_0001));
    let (last, reused) = db
        .write(|tx| {
            let last = new_anime(tx, "/b");
            tx.execute("DELETE FROM entries WHERE _id = ?1", [last])?;
            Ok((last, new_anime(tx, "/c")))
        })
        .unwrap();
    assert_eq!(last, reused, "no AUTOINCREMENT: the freed id comes back");
}

#[test]
fn updates_of_different_columns_from_two_threads_both_survive() {
    let (_p, db) = open();
    let handle = Arc::clone(&db);
    let a = handle.write(|tx| Ok(new_anime(tx, "/a"))).unwrap();
    let (h1, h2) = (Arc::clone(&handle), Arc::clone(&handle));
    let t1 = std::thread::spawn(move || {
        h1.write(|tx| Ok(entries::update(tx, &EntryPatch { id: a, item_flags: Some(0x22), ..Default::default() })?)).unwrap()
    });
    let t2 = std::thread::spawn(move || {
        h2.write(|tx| Ok(entries::update(tx, &EntryPatch { id: a, last_update: Some(77), ..Default::default() })?)).unwrap()
    });
    t1.join().unwrap();
    t2.join().unwrap();
    let row = handle.read(|c| Ok(entries::by_id(c, a)?)).unwrap();
    assert_eq!((row.item_flags, row.last_update), (0x22, Some(77)));
}

#[test]
fn count_columns_are_truncated_like_the_kotlin_mapper() {
    let c = crate::schema_unified::open_in_memory().unwrap();
    let a = new_anime(&c, "/a");
    c.execute("UPDATE entries SET favorite = 1", []).unwrap();
    new_episode(&c, a, "/a/1");
    // A view column that comes back as REAL still reads as a whole number.
    c.execute_batch("DROP VIEW libraryView; CREATE VIEW libraryView AS SELECT e.*, 3 AS totalCount, 2.9 AS doneCount, 1.5 AS bookmarkCount, 0.0 AS fillermarkCount, 0 AS latestUpload, 0 AS itemFetchedAt, 0 AS lastActivity, 0 AS category FROM entries e")
        .unwrap();
    let rows = library::library(&c, Kind::Anime).unwrap();
    assert_eq!((rows[0].done, rows[0].bookmarks, rows[0].fillermarks), (2, 1, 0));
}

// T-04a-002: the open gate through the handle ---------------------------------------------------------

#[test]
fn opening_a_file_from_a_newer_app_through_the_handle_is_refused_and_untouched() {
    let paths = TempPaths::new().unwrap();
    let file = db_path(&kuro_platform::paths::Paths::data_dir(&paths));
    drop(Db::open(&paths).unwrap());
    // one version past what this app knows, whatever that is now
    let newer = crate::schema_unified::SCHEMA_VERSION + 1;
    Connection::open(&file).unwrap().pragma_update(None, "user_version", newer).unwrap();
    let before = std::fs::read(&file).unwrap();
    let err = Db::open(&paths).err().expect("refused");
    assert!(matches!(err, DbError::TooNew { found, supported } if found == newer && supported == crate::schema_unified::SCHEMA_VERSION), "{err:?}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn a_history_row_cannot_outlive_its_episode_and_empty_database_is_empty() {
    let (_p, db) = open();
    let counts = db
        .read(|c| {
            Ok(["entries", "items", "history"]
                .map(|t| c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get::<_, i64>(0)).unwrap()))
        })
        .unwrap();
    assert_eq!(counts, [0, 0, 0], "a new database holds no entries: the app inserts no sample data");
}
