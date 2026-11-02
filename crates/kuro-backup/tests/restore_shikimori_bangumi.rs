//! T-31a-048: an Aniyomi backup with one Shikimori (`syncId` 4) and one Bangumi (`syncId` 5) track, for a manga and for
//! an anime, restores with `remote_id`, `library_id`, `status`, `score`, dates and `private` unchanged. The fixture is
//! built here and written as a gzip `.tachibk`, so nothing depends on a personal backup.

use std::io::Write;
use std::path::Path;

use chrono::{Local, TimeZone};
use kuro_backup::format::decode_file;
use kuro_backup::restore::driver::{Targets, restore_backup};
use kuro_backup::restore::options::RestoreOptions;
use kuro_backup::restore::prefs::IntervalScheduler;
use kuro_backup::schema::{Backup, BackupAnime, BackupAnimeTracking, BackupManga, BackupTracking};
use kuro_backup::wire::Message;
use kuro_db::tables::tracks;
use kuro_platform::kind::Kind;
use kuro_db::schema_unified;
use kuro_prefs::store::PrefStore;
use rusqlite::Connection;

const SHIKIMORI: i32 = 4;
const BANGUMI: i32 = 5;
const STARTED: i64 = 1_600_000_000_000;
const FINISHED: i64 = 1_700_000_000_000;

fn fixture() -> Backup {
    let manga = BackupManga {
        source: 7,
        url: "/m/1".into(),
        title: "Manga 1".into(),
        tracking: vec![
            // Shikimori: status 6 is rereading, `library_id` is the user-rate id.
            BackupTracking {
                sync_id: SHIKIMORI,
                library_id: 8123,
                media_id: 55,
                title: "Shiki manga".into(),
                tracking_url: "https://shikimori.one/mangas/55".into(),
                last_chapter_read: 12.5,
                total_chapters: 40,
                score: 8.0,
                status: 6,
                started_reading_date: STARTED,
                finished_reading_date: FINISHED,
                ..Default::default()
            },
            // Bangumi: status 3 is reading, no library id, private collection.
            BackupTracking {
                sync_id: BANGUMI,
                library_id: 0,
                media_id: 777,
                title: "Bgm manga".into(),
                tracking_url: "https://bgm.tv/subject/777".into(),
                last_chapter_read: 3.0,
                total_chapters: 10,
                score: 7.0,
                status: 3,
                started_reading_date: STARTED,
                private: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let anime = BackupAnime {
        source: 8,
        url: "/a/1".into(),
        title: "Anime 1".into(),
        tracking: vec![
            BackupAnimeTracking {
                sync_id: SHIKIMORI,
                library_id: 9001,
                media_id: 56,
                title: "Shiki anime".into(),
                tracking_url: "https://shikimori.one/animes/56".into(),
                last_episode_seen: 5.0,
                total_episodes: 12,
                score: 9.0,
                status: 2,
                started_watching_date: STARTED,
                finished_watching_date: FINISHED,
                ..Default::default()
            },
            BackupAnimeTracking {
                sync_id: BANGUMI,
                library_id: 0,
                media_id: 778,
                title: "Bgm anime".into(),
                tracking_url: "https://bgm.tv/subject/778".into(),
                last_episode_seen: 2.0,
                total_episodes: 24,
                score: 6.0,
                status: 5,
                started_watching_date: STARTED,
                private: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    Backup { manga: vec![manga], anime: vec![anime], ..Default::default() }
}

fn write_gzip(path: &Path, backup: &Backup) {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&backup.to_bytes().unwrap()).unwrap();
    std::fs::write(path, enc.finish().unwrap()).unwrap();
}

struct Dbs {
    db: Connection,
    app: PrefStore,
}

fn fresh() -> Dbs {
    Dbs { db: schema_unified::open_in_memory().unwrap(), app: PrefStore::in_memory() }
}

fn restore(dbs: &mut Dbs, path: &Path) {
    let backup = decode_file(path).unwrap();
    let off = |_: Option<u32>| {};
    let sched = IntervalScheduler { store: &dbs.app, backup_timer: &off };
    let open = |_: &str| PrefStore::in_memory();
    let now = Local.with_ymd_and_hms(2026, 10, 9, 12, 0, 0).single().unwrap();
    let targets = Targets { db: &mut dbs.db, app_prefs: &dbs.app, source_prefs: &open, scheduler: &sched };
    let opts = RestoreOptions { extensions: false, ..RestoreOptions::from_bits([true; 7]) };
    let run = restore_backup(backup, opts, now, targets, &|| false, &mut |_| {});
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

#[test]
fn t_31a_048_shikimori_and_bangumi_manga_tracks_restore_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shikimori_bangumi.tachibk");
    write_gzip(&file, &fixture());
    let mut dbs = fresh();
    restore(&mut dbs, &file);

    let mut rows = tracks::all(&dbs.db, Kind::Manga).unwrap();
    rows.sort_by_key(|t| t.tracker_id);
    assert_eq!(rows.len(), 2);

    let shiki = &rows[0];
    assert_eq!((shiki.tracker_id, shiki.remote_id, shiki.library_id), (4, 55, Some(8123)));
    assert_eq!((shiki.status, shiki.score), (6, 8.0));
    assert_eq!((shiki.start_date, shiki.finish_date, shiki.private), (STARTED, FINISHED, false));
    assert_eq!((shiki.last_item, shiki.total_items), (12.5, 40));

    let bgm = &rows[1];
    assert_eq!((bgm.tracker_id, bgm.remote_id, bgm.library_id), (5, 777, Some(0)));
    assert_eq!((bgm.status, bgm.score), (3, 7.0));
    assert_eq!((bgm.start_date, bgm.finish_date, bgm.private), (STARTED, 0, true));
}

#[test]
fn t_31a_048_shikimori_and_bangumi_anime_tracks_restore_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shikimori_bangumi.tachibk");
    write_gzip(&file, &fixture());
    let mut dbs = fresh();
    restore(&mut dbs, &file);

    let mut rows = tracks::all(&dbs.db, Kind::Anime).unwrap();
    rows.sort_by_key(|t| t.tracker_id);
    assert_eq!(rows.len(), 2);

    let shiki = &rows[0];
    assert_eq!((shiki.tracker_id, shiki.remote_id, shiki.library_id), (4, 56, Some(9001)));
    assert_eq!((shiki.status, shiki.score), (2, 9.0));
    assert_eq!((shiki.start_date, shiki.finish_date, shiki.private), (STARTED, FINISHED, false));
    assert_eq!((shiki.last_item, shiki.total_items), (5.0, 12));

    let bgm = &rows[1];
    assert_eq!((bgm.tracker_id, bgm.remote_id, bgm.library_id), (5, 778, Some(0)));
    assert_eq!((bgm.status, bgm.score), (5, 6.0));
    assert_eq!((bgm.start_date, bgm.finish_date, bgm.private), (STARTED, 0, true));
}

#[test]
fn t_31a_048_the_fixture_decodes_with_both_tracker_ids() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shikimori_bangumi.tachibk");
    write_gzip(&file, &fixture());
    let decoded = decode_file(&file).unwrap();
    let ids: Vec<i32> = decoded.manga[0].tracking.iter().map(|t| t.sync_id).collect();
    assert_eq!(ids, [SHIKIMORI, BANGUMI]);
    let anime_ids: Vec<i32> = decoded.anime[0].tracking.iter().map(|t| t.sync_id).collect();
    assert_eq!(anime_ids, [SHIKIMORI, BANGUMI]);
}
