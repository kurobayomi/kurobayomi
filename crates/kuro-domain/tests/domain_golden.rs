//! Golden and integration tests of the domain layer (T-10a-036). They use only the public API, `kuro-testkit` and a
//! migrated database in a temporary directory.

use kuro_platform::kind::Kind;
use std::sync::{Arc, Mutex};

use kuro_data::entry::{Entry, EntryRepo};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::Db;
use kuro_domain::entries::update::{CacheFiles, UpdateEntry, remote_from_manga};
use kuro_domain::items::source_sync::EntrySyncLocks;
use kuro_domain::items::status::{ItemDownloads, SetDoneResult, SetDoneStatus};
use kuro_domain::items::sync::{NoRenames, SourceItem, SyncItems, SyncOptions, source_item_from_chapter};
use kuro_domain::track::delayed::{DelayedTrackingStore, DelayedTrackingWorker, QUEUE_FILE, ReplayOutcome, TrackReplay};
use kuro_source::model::SChapter;
use kuro_testkit::{TempDir, TestDb};

const NOW_MS: i64 = 1_000_000;

/// A migrated database and a manga entry titled `title`.
fn manga_fixture(title: &str) -> (TempDir, Arc<Db>, i64) {
    let dir = TempDir::new().expect("temp dir");
    let db = TestDb::handle(&dir);
    let id = EntryRepo::new(db.clone()).insert(&Entry { title: title.into(), ..Entry::create(Kind::Manga) }).expect("insert manga");
    (dir, db, id)
}

fn chapter_row(manga_id: i64, url: &str, name: &str, number: f64) -> Item {
    Item { entry_id: manga_id, url: url.into(), name: name.into(), number, ..Item::create(Kind::Manga) }
}

fn stored_chapters(db: &Arc<Db>, manga_id: i64) -> Vec<Item> {
    ItemRepo::new(db.clone()).by_entry_id(manga_id, false).expect("read chapters")
}

fn sync(db: &Arc<Db>, locks: &Arc<EntrySyncLocks>) -> SyncItems {
    SyncItems::new(db.clone(), locks.clone(), Arc::new(NoRenames))
}

fn options(now_ms: i64) -> SyncOptions {
    SyncOptions { now_ms, manual: false, window: (0, 0), mark_duplicate_done: true, is_local: false }
}

fn source_chapter(url: &str, name: &str, date_upload: i64) -> SChapter {
    SChapter { url: url.into(), name: name.into(), date_upload, chapter_number: -1.0, scanlator: None }
}

fn items_of(raw: &[SChapter]) -> Vec<SourceItem> {
    raw.iter().map(source_item_from_chapter).collect()
}

fn manga_row(db: &Arc<Db>, id: i64) -> Entry {
    EntryRepo::new(db.clone()).by_id(id).expect("read manga")
}

mod sync_items {
    use super::*;

    #[tokio::test]
    async fn worked_example() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("fixtures/sync/worked_example.json")).expect("fixture");
        let (_dir, db, id) = manga_fixture(fixture["manga_title"].as_str().expect("title"));
        for row in fixture["stored"].as_array().expect("stored") {
            let mut r = chapter_row(id, row["url"].as_str().expect("url"), row["name"].as_str().expect("name"), row["number"].as_f64().expect("number"));
            r.done = row["read"].as_bool().expect("read");
            r.bookmark = row["bookmark"].as_bool().expect("bookmark");
            r.date_fetch = row["date_fetch"].as_i64().expect("date_fetch");
            r.source_order = row["source_order"].as_i64().expect("source_order");
            ItemRepo::new(db.clone()).add_all(&[r]).expect("insert");
        }
        let raw: Vec<SChapter> = fixture["source"]
            .as_array()
            .expect("source")
            .iter()
            .map(|c| source_chapter(c["url"].as_str().expect("url"), c["name"].as_str().expect("name"), c["date_upload"].as_i64().expect("date")))
            .collect();
        let locks = Arc::new(EntrySyncLocks::default());
        let manga = manga_row(&db, id);
        let returned = sync(&db, &locks).run(items_of(&raw), &manga, options(fixture["now_ms"].as_i64().expect("now")), None).await.expect("sync");
        let urls: Vec<&str> = returned.iter().map(|c| c.url.as_str()).collect();
        let expected: Vec<&str> = fixture["expected_returned_urls"].as_array().expect("urls").iter().map(|u| u.as_str().expect("url")).collect();
        assert_eq!(urls, expected, "only /c5 and /c4 are reported as new");

        let rows = stored_chapters(&db, id);
        for (url, want) in fixture["expected_date_fetch"].as_object().expect("dates") {
            let row = rows.iter().find(|r| &r.url == url).expect("row");
            assert_eq!(row.date_fetch, want.as_i64().expect("date"), "dateFetch of {url}");
        }
        assert!(rows.iter().all(|r| r.url != "/c3"), "the removed chapter is deleted");
        let c3b = rows.iter().find(|r| r.url == "/c3b").expect("c3b");
        assert!(c3b.done && c3b.bookmark, "c3b inherits read and bookmark from the removed twin");
    }

    #[tokio::test]
    async fn empty_source_errors() {
        let (_dir, db, id) = manga_fixture("Foo");
        let manga = manga_row(&db, id);
        let err = sync(&db, &Arc::new(EntrySyncLocks::default())).run(Vec::new(), &manga, options(NOW_MS), None).await;
        assert!(err.is_err(), "EC-01");
        assert!(stored_chapters(&db, id).is_empty(), "nothing written");
    }

    #[tokio::test]
    async fn local_empty_ok() {
        let (_dir, db, id) = manga_fixture("Foo");
        let manga = manga_row(&db, id);
        let local = SyncOptions { is_local: true, ..options(NOW_MS) };
        let out = sync(&db, &Arc::new(EntrySyncLocks::default())).run(Vec::new(), &manga, local, None).await.expect("ok");
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn no_change_no_write() {
        let (_dir, db, id) = manga_fixture("Foo");
        let locks = Arc::new(EntrySyncLocks::default());
        let raw = vec![source_chapter("/a", "Ch 1", 10), source_chapter("/b", "Ch 2", 20)];
        let first = manga_row(&db, id);
        sync(&db, &locks).run(items_of(&raw), &first, options(NOW_MS), None).await.expect("first");
        let after_first = manga_row(&db, id);
        let second = sync(&db, &locks).run(items_of(&raw), &after_first, options(NOW_MS + 60_000), None).await.expect("second");
        assert!(second.is_empty(), "EC-09");
        assert_eq!(manga_row(&db, id).last_update, after_first.last_update, "no last_update bump on a no-change refresh");
    }

    #[tokio::test]
    async fn duplicate_urls_first_wins() {
        let (_dir, db, id) = manga_fixture("Foo");
        let manga = manga_row(&db, id);
        let raw = vec![source_chapter("/x", "Ch 1", 0), source_chapter("/x", "Ch 1 again", 0), source_chapter("/y", "Ch 2", 0)];
        let out = sync(&db, &Arc::new(EntrySyncLocks::default())).run(items_of(&raw), &manga, options(NOW_MS), None).await.expect("sync");
        assert_eq!(out.len(), 2, "EC-03");
        assert_eq!(out[0].name, "Ch 1", "the first occurrence wins");
    }

    #[tokio::test]
    async fn excluded_scanlators_filter() {
        let (_dir, db, id) = manga_fixture("Foo");
        EntryRepo::new(db.clone()).set_excluded_scanlators(id, &["Bad".to_owned()].into()).expect("exclude");
        let manga = manga_row(&db, id);
        let raw = vec![
            SChapter { scanlator: Some("Bad".into()), ..source_chapter("/a", "Ch 1", 0) },
            SChapter { scanlator: Some("Good".into()), ..source_chapter("/b", "Ch 2", 0) },
        ];
        let out = sync(&db, &Arc::new(EntrySyncLocks::default())).run(items_of(&raw), &manga, options(NOW_MS), None).await.expect("sync");
        assert_eq!(out.iter().map(|c| c.url.as_str()).collect::<Vec<_>>(), ["/b"], "EC-12: not reported as new");
        assert_eq!(stored_chapters(&db, id).len(), 2, "but both rows are inserted");
    }

    #[tokio::test]
    async fn f32_number_not_requeued() {
        let (_dir, db, id) = manga_fixture("Foo");
        let locks = Arc::new(EntrySyncLocks::default());
        let raw = vec![SChapter { chapter_number: 12.1_f32, ..source_chapter("/c12", "Ch 12.1", 0) }];
        let manga = manga_row(&db, id);
        sync(&db, &locks).run(items_of(&raw), &manga, options(NOW_MS), None).await.expect("first");
        assert_eq!(stored_chapters(&db, id)[0].number, 12.1_f64, "EC-CH-07: the decimal form, not 12.100000762939453");
        let after = manga_row(&db, id);
        let second = sync(&db, &locks).run(items_of(&raw), &after, options(NOW_MS), None).await.expect("second");
        assert!(second.is_empty(), "an identical second sync reports nothing");
        assert_eq!(manga_row(&db, id).last_update, after.last_update, "and writes nothing");
    }

    #[tokio::test]
    async fn scanlator_never_cleared() {
        let (_dir, db, id) = manga_fixture("Foo");
        let locks = Arc::new(EntrySyncLocks::default());
        let first = vec![SChapter { scanlator: Some("Team".into()), ..source_chapter("/c", "Ch 1", 0) }];
        let manga = manga_row(&db, id);
        sync(&db, &locks).run(items_of(&first), &manga, options(NOW_MS), None).await.expect("first");
        let manga = manga_row(&db, id);
        sync(&db, &locks).run(items_of(&[source_chapter("/c", "Ch 1", 0)]), &manga, options(NOW_MS), None).await.expect("second");
        assert_eq!(stored_chapters(&db, id)[0].scanlator.as_deref(), Some("Team"), "EC-08");
    }
}

mod update_entry {
    use super::*;
    use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
    use kuro_data::item::ItemRepo;

    #[derive(Default)]
    struct Files {
        deleted: Mutex<usize>,
    }

    impl CacheFiles for Files {
        fn has_custom_cover(&self, _e: &Entry) -> bool {
            false
        }
        fn has_custom_background(&self, _e: &Entry) -> bool {
            false
        }
        fn delete_cover(&self, _e: &Entry) {
            *self.deleted.lock().expect("lock") += 1;
        }
        fn delete_background(&self, _e: &Entry) {}
    }

    #[test]
    fn title_not_overwritten_for_favorite() {
        let (_dir, db, id) = manga_fixture("Kept");
        let repo = EntryRepo::new(db.clone());
        repo.update(&EntryUpdate { favorite: Some(true), ..EntryUpdate::new(id) }).expect("favorite");
        let interactor = UpdateEntry::new(repo.clone(), ItemRepo::new(db));
        let local = repo.by_id(id).expect("read");
        let remote = remote_from_manga(&kuro_source::model::SManga { title: "Remote".into(), ..kuro_source::model::SManga::new("/m", "M") });
        let now = chrono::Local::now();
        interactor.update_from_source(&local, &remote, false, &Files::default(), &now).expect("update");
        assert_eq!(repo.by_id(id).expect("read").title, "Kept", "EC-18");
    }

    #[test]
    fn favorite_sets_date_added() {
        let (_dir, db, id) = manga_fixture("Foo");
        let repo = EntryRepo::new(db.clone());
        let interactor = UpdateEntry::new(repo.clone(), ItemRepo::new(db));
        let now = chrono::DateTime::from_timestamp_millis(1_700_000_000_000).expect("instant").with_timezone(&chrono::Local);
        interactor.update_favorite(id, true, &now).expect("on");
        assert_eq!(repo.by_id(id).expect("read").date_added, 1_700_000_000_000, "EC-25");
        interactor.update_favorite(id, false, &now).expect("off");
        assert_eq!(repo.by_id(id).expect("read").date_added, 0);
    }
}

mod set_read_status {
    use super::*;

    #[derive(Default)]
    struct Downloads {
        calls: Mutex<usize>,
    }

    impl ItemDownloads for Downloads {
        fn delete(&self, _e: &Entry, _items: &[Item]) {
            *self.calls.lock().expect("lock") += 1;
        }
    }


    #[tokio::test]
    async fn results() {
        let (_dir, db, id) = manga_fixture("Foo");
        ItemRepo::new(db.clone()).add_all(&[chapter_row(id, "/a", "Ch 1", 1.0)]).expect("insert");
        let downloads = Arc::new(Downloads::default());
        let set = SetDoneStatus::new(ItemRepo::new(db.clone()), EntryRepo::new(db.clone()), downloads.clone());
        let rows = stored_chapters(&db, id);
        assert_eq!(set.run(true, rows, true).await, SetDoneResult::Success);
        assert_eq!(*downloads.calls.lock().expect("lock"), 1, "downloads deleted once for the manga");
        let read_rows = stored_chapters(&db, id);
        assert_eq!(set.run(true, read_rows, true).await, SetDoneResult::NoItems, "already read: nothing to do");
    }
}

mod set_seen_status {
    use super::*;

    #[derive(Default)]
    struct EpisodeDl;

    impl ItemDownloads for EpisodeDl {
        fn delete(&self, _e: &Entry, _items: &[Item]) {}
    }

    #[tokio::test]
    async fn last_second_reset() {
        let dir = TempDir::new().expect("temp dir");
        let db = TestDb::handle(&dir);
        let anime_id = EntryRepo::new(db.clone()).insert(&Entry::create(Kind::Anime)).expect("anime");
        let items = ItemRepo::new(db.clone());
        items
            .add_all(&[Item {
                entry_id: anime_id,
                url: "/e".into(),
                name: "Ep".into(),
                done: true,
                progress: 40,
                total_seconds: Some(500),
                number: 1.0,
                version: 1,
                ..Item::create(Kind::Anime)
            }])
            .expect("insert");
        let set = SetDoneStatus::new(items.clone(), EntryRepo::new(db.clone()), Arc::new(EpisodeDl));
        let rows = items.by_entry_id(anime_id, false).expect("read");
        assert_eq!(set.run(false, rows, false).await, SetDoneResult::Success);
        let row = items.by_entry_id(anime_id, false).expect("read").remove(0);
        assert!(!row.done);
        assert_eq!(row.progress, 0, "marking unseen resets the position");
        assert_eq!(row.total_seconds, Some(500), "total_seconds is untouched");
    }
}

mod domain_end_to_end {
    use super::*;

    /// Sync new chapters, mark one read, then refresh again: the read state survives and the second refresh reports
    /// nothing. The track and offline-queue stages are in [`sync_mark_read_track_offline_then_replay`].
    #[tokio::test]
    async fn sync_then_mark_read_then_refresh() {
        let (_dir, db, id) = manga_fixture("Foo");
        let locks = Arc::new(EntrySyncLocks::default());
        let raw = vec![source_chapter("/c2", "Ch 2", 0), source_chapter("/c1", "Ch 1", 0)];
        let manga = manga_row(&db, id);
        let added = sync(&db, &locks).run(items_of(&raw), &manga, options(NOW_MS), None).await.expect("first sync");
        assert_eq!(added.len(), 2);

        let read: Vec<Item> = stored_chapters(&db, id).into_iter().filter(|c| c.number == 1.0).collect();
        let set = SetDoneStatus::new(ItemRepo::new(db.clone()), EntryRepo::new(db.clone()), Arc::new(NoDownloads));
        assert_eq!(set.run(true, read, false).await, SetDoneResult::Success);

        let after = manga_row(&db, id);
        let again = sync(&db, &locks).run(items_of(&raw), &after, options(NOW_MS + 1), None).await.expect("second sync");
        assert!(again.is_empty(), "nothing new on the second refresh");
        let c1 = stored_chapters(&db, id).into_iter().find(|c| c.url == "/c1").expect("c1");
        assert!(c1.done, "the read state survives the refresh");
    }

    /// A tracker the test switches online: `replay` pushes the progress and records it.
    struct FakeTracker {
        online: tokio::sync::watch::Sender<bool>,
        pushed: Mutex<Vec<(Kind, i64, f32)>>,
    }

    #[async_trait::async_trait]
    impl TrackReplay for FakeTracker {
        async fn wait_online(&self) {
            let mut rx = self.online.subscribe();
            let _ = rx.wait_for(|up| *up).await;
        }

        async fn replay(&self, kind: Kind, track_id: i64, progress: f32) -> ReplayOutcome {
            self.pushed.lock().expect("lock").push((kind, track_id, progress));
            ReplayOutcome::Done
        }
    }

    /// `domain_end_to_end` of the plan: sync (new chapters) -> mark read -> track update while offline (queued) -> go
    /// online -> the worker replays. Asserts the final database, queue file and tracker state.
    #[tokio::test]
    async fn sync_mark_read_track_offline_then_replay() {
        let (dir, db, id) = manga_fixture("Foo");
        let locks = Arc::new(EntrySyncLocks::default());
        let raw = vec![source_chapter("/c2", "Ch 2", 0), source_chapter("/c1", "Ch 1", 0)];
        let manga = manga_row(&db, id);
        sync(&db, &locks).run(items_of(&raw), &manga, options(NOW_MS), None).await.expect("sync");

        let read: Vec<Item> = stored_chapters(&db, id).into_iter().filter(|c| c.number == 1.0).collect();
        let progress = read.iter().map(|c| c.number).fold(0.0, f64::max);
        let set = SetDoneStatus::new(ItemRepo::new(db.clone()), EntryRepo::new(db.clone()), Arc::new(NoDownloads));
        assert_eq!(set.run(true, read, false).await, SetDoneResult::Success);

        // The tracker is offline: the push cannot happen, so the progress is queued under the track row id.
        let (up, _keep) = tokio::sync::watch::channel(false);
        let tracker = Arc::new(FakeTracker { online: up, pushed: Mutex::default() });
        let queue_path = dir.path().join(QUEUE_FILE);
        let store = Arc::new(DelayedTrackingStore::new(queue_path.clone()));
        store.add(Kind::Manga, 7, progress).expect("queue");
        let worker = DelayedTrackingWorker::new(store.clone(), tracker.clone());
        worker.schedule();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(tracker.pushed.lock().expect("lock").is_empty(), "offline: nothing is pushed");
        assert_eq!(store.items(Kind::Manga), vec![(7, 1.0_f32)], "and the progress waits in the queue");

        // Online again: the worker replays the item and empties the queue.
        tracker.online.send_replace(true);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while worker.is_running() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the worker finishes");

        assert_eq!(*tracker.pushed.lock().expect("lock"), vec![(Kind::Manga, 7, 1.0_f32)], "tracker state");
        assert!(store.is_empty(), "queue state");
        assert!(DelayedTrackingStore::new(queue_path).is_empty(), "the queue file is empty too");
        let c1 = stored_chapters(&db, id).into_iter().find(|c| c.url == "/c1").expect("c1");
        let c2 = stored_chapters(&db, id).into_iter().find(|c| c.url == "/c2").expect("c2");
        assert!(c1.done && !c2.done, "database state");
    }

    struct NoDownloads;

    impl ItemDownloads for NoDownloads {
        fn delete(&self, _e: &Entry, _items: &[Item]) {}
    }
}
