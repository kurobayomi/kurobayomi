//! The one item list refresh of both kinds (`SyncChaptersWithSource.await`, `SyncEpisodesWithSource.await`; T-10a-008,
//! T-10a-009): it sanitises the source list, prepares each row, calls the pure planner ([`crate::items::sync_plan`]),
//! asks the download port to rename folders, then writes the plan in ONE transaction: delete, insert (one bulk
//! statement for both kinds), update, fetch interval and last update (the Kotlin runs these as separate calls; one
//! transaction is the deliberate change of area 10 section 5).
//!
//! What differs by kind is data, not code: the scanlator rule of the source adapters ([`source_item_from_chapter`] trims,
//! [`source_item_from_episode`] only blanks), the number recognition, the duplicate-done preference, and the excluded
//! scanlators (a manga rule: the new-item list drops them).
//!
//! A first sync (an entry with no stored items) stamps its items with the time the entry was added, not with now: they
//! are the back catalogue, not updates (docs/design/unified-model.md section 7; see the planner).

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Local};
use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_data::sanitize::sanitize_item_name;
use kuro_db::Db;
use kuro_db::codec::Memo;
use kuro_db::tables::items as item_table;
use kuro_db::tables::{EntryPatch, ItemPatch, ItemRow, entries as entry_table};
use kuro_platform::kind::Kind;
use kuro_source::anime::SEpisode;
use kuro_source::model::SChapter;
use serde_json::{Map, Value};

use crate::entries::fetch_interval::{ItemDates, Stored, interval_update};
use crate::items::model::{blank_to_none, scanlator_of, widen_source_number};
use crate::items::recognition::{parse_chapter_number, parse_episode_number};
use crate::items::should_update::should_update_db_item;
use crate::items::source_sync::EntrySyncLocks;
use crate::items::sync_plan::{PlanInput, SyncRow, plan_sync};

/// One item as a source lists it, for either kind. `number` is the source's `f32`; the row stores its widening.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceItem {
    pub url: String,
    pub name: String,
    pub date_upload: i64,
    pub number: f32,
    pub scanlator: Option<String>,
    pub fillermark: bool,
    pub summary: Option<String>,
    pub preview_url: Option<String>,
    pub memo: Option<Map<String, Value>>,
}

/// The source chapter as a [`SourceItem`]; the scanlator is trimmed.
pub fn source_item_from_chapter(c: &SChapter) -> SourceItem {
    SourceItem {
        url: c.url.clone(),
        name: c.name.clone(),
        date_upload: c.date_upload,
        number: c.chapter_number,
        scanlator: scanlator_of(Kind::Manga, c.scanlator.as_deref()),
        fillermark: false,
        summary: None,
        preview_url: None,
        memo: None,
    }
}

/// The source episode as a [`SourceItem`]; the scanlator is only blanked, not trimmed.
pub fn source_item_from_episode(e: &SEpisode) -> SourceItem {
    SourceItem {
        url: e.url.clone(),
        name: e.name.clone(),
        date_upload: e.date_upload,
        number: e.episode_number,
        scanlator: scanlator_of(Kind::Anime, e.scanlator.as_deref()),
        fillermark: e.fillermark,
        summary: blank_to_none(e.summary.as_deref()),
        preview_url: blank_to_none(e.preview_url.as_deref()),
        memo: Some(e.memo.clone()),
    }
}

/// The download folder rename of a changed item (`downloadManager.renameChapter`, `renameEpisode`). The implementation
/// decides whether a folder exists and whether its name changed. It logs its own failures: a failed rename never fails
/// the sync (EC-11).
pub trait ItemRenames: Send + Sync {
    fn rename(&self, e: &Entry, stored: &Item, prepared: &Item);
}

/// A download port that does nothing (no downloads on disk, or the download layer is not wired yet).
pub struct NoRenames;

impl ItemRenames for NoRenames {
    fn rename(&self, _e: &Entry, _stored: &Item, _prepared: &Item) {}
}

/// Why an item refresh failed.
#[derive(Debug, thiserror::Error)]
pub enum SyncItemsError {
    /// A non-local source returned no items. Nothing was written (EC-01).
    #[error("no items found")]
    NoItems,
    #[error("database error: {0}")]
    Db(#[from] kuro_db::DbError),
}

/// The options of one refresh: the manual flag, the fetch window (`(0, 0)` means "now"), the duplicate-done preference and
/// whether the source is the local one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncOptions {
    /// The refresh time, epoch milliseconds (the clock is an input, so the sync is deterministic under test).
    pub now_ms: i64,
    pub manual: bool,
    pub window: (i64, i64),
    pub mark_duplicate_done: bool,
    pub is_local: bool,
}

/// The row a source item becomes: name sanitised, number widened, the source order its position.
fn item_from_source(e: &Entry, s: &SourceItem, position: usize) -> Item {
    Item {
        entry_id: e.id,
        name: sanitize_item_name(&s.name, &e.title),
        url: s.url.clone(),
        date_upload: s.date_upload,
        number: widen_source_number(s.number),
        scanlator: s.scanlator.clone(),
        fillermark: s.fillermark,
        summary: s.summary.clone(),
        preview_url: s.preview_url.clone(),
        memo: s.memo.clone().map(Memo).or_else(|| e.kind.is_anime().then(Memo::default)),
        source_order: i64::try_from(position).unwrap_or(i64::MAX),
        ..Item::create(e.kind)
    }
}

/// The stored-row shape back to the source shape, for the prepare hook.
fn source_item_of(row: &Item) -> SourceItem {
    SourceItem {
        url: row.url.clone(),
        name: row.name.clone(),
        date_upload: row.date_upload,
        number: row.number as f32,
        scanlator: row.scanlator.clone(),
        fillermark: row.fillermark,
        summary: row.summary.clone(),
        preview_url: row.preview_url.clone(),
        memo: row.memo.as_ref().map(|m| m.0.clone()),
    }
}

/// `copyFromSChapter` of a prepared source item onto the row it came from. Only the source fields change.
fn copy_from_source(base: &Item, s: &SourceItem) -> Item {
    Item {
        name: s.name.clone(),
        url: s.url.clone(),
        date_upload: s.date_upload,
        number: widen_source_number(s.number),
        scanlator: scanlator_of(base.kind, s.scanlator.as_deref()),
        fillermark: s.fillermark,
        summary: blank_to_none(s.summary.as_deref()),
        preview_url: blank_to_none(s.preview_url.as_deref()),
        memo: s.memo.clone().map(Memo).or_else(|| base.memo.clone()),
        ..base.clone()
    }
}

impl SyncRow for Item {
    fn id(&self) -> i64 {
        self.id
    }
    fn url(&self) -> &str {
        &self.url
    }
    fn number(&self) -> f64 {
        self.number
    }
    fn read(&self) -> bool {
        self.done
    }
    fn bookmark(&self) -> bool {
        self.bookmark
    }
    fn date_fetch(&self) -> i64 {
        self.date_fetch
    }
    fn date_upload(&self) -> i64 {
        self.date_upload
    }
    fn set_source_order(&mut self, order: i64) {
        self.source_order = order;
    }
    fn set_date_upload(&mut self, date: i64) {
        self.date_upload = date;
    }
    fn set_date_fetch(&mut self, date: i64) {
        self.date_fetch = date;
    }
    fn set_read(&mut self, done: bool) {
        self.done = done;
    }
    fn set_bookmark(&mut self, bookmark: bool) {
        self.bookmark = bookmark;
    }
    fn needs_update(db: &Self, src: &Self) -> bool {
        should_update_db_item(db, src)
    }
    /// `toChange`: name, number, scanlator, summary, source order and memo from the source; the upload date only when the
    /// source has one; a fillermark is never cleared by the source (only set while stored false); the preview URL is filled
    /// only while stored blank. A manga item has none of the anime fields, so those rules do nothing for it.
    fn merged(db: &Self, src: &Self) -> Self {
        let mut out = Item {
            name: src.name.clone(),
            number: src.number,
            scanlator: src.scanlator.clone(),
            summary: src.summary.clone(),
            source_order: src.source_order,
            memo: src.memo.clone(),
            ..db.clone()
        };
        if src.date_upload != 0 {
            out.date_upload = src.date_upload;
        }
        if !out.fillermark {
            out.fillermark = src.fillermark;
        }
        if out.preview_url.as_deref().is_none_or(|p| p.trim().is_empty()) {
            out.preview_url = src.preview_url.clone();
        }
        out
    }
}

/// The columns a refresh owns. Done, bookmark, progress and the seconds are NOT written, so a read made while the sync
/// runs is not lost (race 1). A `None` scanlator, summary or preview keeps the stored one (coalesce, EC-08).
fn refresh_patch(i: &Item) -> ItemPatch {
    ItemPatch {
        id: i.id,
        name: Some(i.name.clone()),
        scanlator: i.scanlator.clone(),
        number: Some(i.number),
        source_order: Some(i.source_order),
        date_upload: Some(i.date_upload),
        summary: i.summary.clone(),
        preview_url: i.preview_url.clone(),
        fillermark: i.kind.is_anime().then_some(i.fillermark),
        memo: i.memo.clone(),
        ..ItemPatch::default()
    }
}

/// The item sync of one entry. Clones share the database handle and the per-entry locks.
#[derive(Clone)]
pub struct SyncItems {
    db: Arc<Db>,
    locks: Arc<EntrySyncLocks>,
    renames: Arc<dyn ItemRenames>,
}

impl SyncItems {
    pub fn new(db: Arc<Db>, locks: Arc<EntrySyncLocks>, renames: Arc<dyn ItemRenames>) -> Self {
        Self { db, locks, renames }
    }

    /// `await(rawSourceItems, entry, source, manualFetch, fetchWindow)`. Returns the NEW items that should notify and
    /// auto-download. `prepare` is the source's `prepareNewChapter` hook (`None` when the source has none, and for the
    /// local source).
    pub async fn run(
        &self,
        raw: Vec<SourceItem>,
        e: &Entry,
        opts: SyncOptions,
        prepare: Option<&(dyn Fn(&mut SourceItem) + Sync)>,
    ) -> Result<Vec<Item>, SyncItemsError> {
        if raw.is_empty() && !opts.is_local {
            return Err(SyncItemsError::NoItems);
        }
        let now_ms = opts.now_ms;
        let now = DateTime::from_timestamp_millis(now_ms).map(|utc| utc.with_timezone(&Local)).unwrap_or_else(Local::now);
        let _guard = self.locks.lock(e.id).await;

        let stored: Vec<Item> =
            self.db.read(|c| Ok(item_table::by_entry(c, e.id, false)?.into_iter().map(Item::from).collect()))?;

        let source: Vec<Item> = raw.iter().enumerate().map(|(i, s)| item_from_source(e, s, i)).collect();
        // The hook sees the source shape of each row. The date of a new row is judged on the raw row (see the planner).
        let prepare_row = |row: Item| -> Item {
            let prepared = match prepare {
                Some(hook) => {
                    let mut s = source_item_of(&row);
                    hook(&mut s);
                    copy_from_source(&row, &s)
                }
                None => row,
            };
            let known = Some(prepared.number);
            let number = match e.kind {
                Kind::Manga => parse_chapter_number(&e.title, &prepared.name, known),
                Kind::Anime => parse_episode_number(&e.title, &prepared.name, known),
            };
            Item { number, ..prepared }
        };
        let plan = plan_sync(PlanInput {
            source,
            db: &stored,
            now_ms,
            added_ms: if e.favorite { e.date_added } else { 0 },
            mark_duplicate_as_read: opts.mark_duplicate_done,
            prepare: &prepare_row,
        });

        for (db_row, prepared) in &plan.renames {
            self.renames.rename(e, db_row, prepared);
        }

        let window = opts.window;
        let fetch_needed = opts.manual || e.fetch_interval == 0 || e.next_update < window.0;
        let changed = !plan.is_empty();
        let (inserted, excluded) = self.db.write(|tx| {
            if changed && !plan.removed_ids.is_empty() {
                item_table::remove_with_ids(tx, &plan.removed_ids)?;
            }
            let mut inserted: Vec<Item> = Vec::new();
            if changed && !plan.added.is_empty() {
                let rows: Vec<ItemRow> = plan.added.iter().map(ItemRow::from).collect();
                let ids = item_table::insert_all(tx, &rows)?;
                inserted = plan.added.iter().zip(ids).map(|(i, id)| Item { id, ..i.clone() }).collect();
            }
            for updated in &plan.updated {
                item_table::update(tx, &refresh_patch(updated))?;
            }
            if changed || fetch_needed {
                let stored_interval =
                    Stored { fetch_interval: e.fetch_interval, next_update: e.next_update, last_update: e.last_update };
                let mut read_error = None;
                let sample = || match item_table::by_entry(tx, e.id, true) {
                    Ok(rows) => rows.iter().map(|r| ItemDates { date_upload: r.date_upload, date_fetch: r.date_fetch }).collect(),
                    Err(err) => {
                        read_error = Some(err);
                        Vec::new()
                    }
                };
                let out = interval_update(stored_interval, sample, &now, window, now_ms);
                if let Some(err) = read_error {
                    return Err(err.into());
                }
                let patch = EntryPatch {
                    id: e.id,
                    next_update: Some(out.next_update),
                    calculate_interval: Some(i64::from(out.fetch_interval)),
                    last_update: changed.then_some(now_ms),
                    ..EntryPatch::default()
                };
                entry_table::update(tx, &patch)?;
            }
            let excluded: HashSet<String> = if changed && e.kind == Kind::Manga {
                kuro_db::tables::excluded::by_entry_id(tx, e.id)?.into_iter().collect()
            } else {
                HashSet::new()
            };
            Ok((inserted, excluded))
        })?;

        Ok(inserted
            .into_iter()
            .filter(|i| !plan.changed_or_duplicate_urls.contains(&i.url))
            .filter(|i| !i.scanlator.as_ref().is_some_and(|s| excluded.contains(s)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::testdb::test_db;
    use kuro_data::entry::EntryRepo;
    use kuro_data::item::ItemRepo;

    const NOW_MS: i64 = 1_000_000;
    const WINDOW: (i64, i64) = (1_000_000, 2_000_000);

    #[derive(Default)]
    struct RecordingRenames {
        calls: Mutex<Vec<(String, String)>>,
    }

    impl ItemRenames for RecordingRenames {
        fn rename(&self, _e: &Entry, stored: &Item, prepared: &Item) {
            self.calls.lock().unwrap().push((stored.url.clone(), prepared.name.clone()));
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Arc<Db>,
        sync: SyncItems,
        entries: EntryRepo,
        items: ItemRepo,
        id: i64,
        kind: Kind,
    }

    fn fixture(kind: Kind, favorite: bool, date_added: i64, renames: Arc<dyn ItemRenames>) -> Fixture {
        let (dir, db) = test_db();
        let entries = EntryRepo::new(db.clone());
        let id = entries
            .insert(&Entry { title: "Foo".into(), url: "/foo".into(), source: 5, favorite, date_added, ..Entry::create(kind) })
            .unwrap();
        let sync = SyncItems::new(db.clone(), Arc::new(EntrySyncLocks::default()), renames);
        Fixture { _dir: dir, db: db.clone(), sync, entries, items: ItemRepo::new(db), id, kind }
    }

    fn plain(kind: Kind) -> Fixture {
        fixture(kind, false, 0, Arc::new(NoRenames))
    }

    impl Fixture {
        fn entry(&self) -> Entry {
            self.entries.by_id(self.id).unwrap()
        }

        fn stored(&self) -> Vec<Item> {
            self.items.by_entry_id(self.id, false).unwrap()
        }

        fn insert(&self, i: Item) -> i64 {
            self.items.add_all(&[Item { entry_id: self.id, ..i }]).unwrap()[0].id
        }

        fn item(&self, url: &str, number: f64) -> Item {
            Item { url: url.into(), name: format!("Item {number}"), number, ..Item::create(self.kind) }
        }

        fn src(&self, url: &str, name: &str, date: i64) -> SourceItem {
            SourceItem {
                url: url.into(),
                name: name.into(),
                date_upload: date,
                number: -1.0,
                scanlator: None,
                fillermark: false,
                summary: None,
                preview_url: None,
                memo: self.kind.is_anime().then(Map::new),
            }
        }
    }

    fn opts(manual: bool) -> SyncOptions {
        SyncOptions { now_ms: NOW_MS, manual, window: (0, 0), mark_duplicate_done: true, is_local: false }
    }

    #[tokio::test]
    async fn an_empty_remote_list_is_an_error_and_writes_nothing_for_both_kinds() {
        for kind in [Kind::Manga, Kind::Anime] {
            let f = plain(kind);
            let err = f.sync.run(Vec::new(), &f.entry(), opts(false), None).await.expect_err("error");
            assert!(matches!(err, SyncItemsError::NoItems), "EC-01 {kind}");
            assert!(f.stored().is_empty());
            let local = SyncOptions { is_local: true, ..opts(false) };
            assert!(f.sync.run(Vec::new(), &f.entry(), local, None).await.expect("ok").is_empty());
        }
    }

    /// The worked example of `p1-topic-chapter-episode-sync.md` section 3, against the database, for both kinds.
    #[tokio::test]
    async fn worked_example_for_both_kinds() {
        for kind in [Kind::Manga, Kind::Anime] {
            let renames = Arc::new(RecordingRenames::default());
            let f = fixture(kind, false, 0, renames.clone());
            f.insert(Item { done: true, date_fetch: 100, source_order: 4, ..f.item("/c1", 1.0) });
            f.insert(Item { date_fetch: 200, source_order: 1, ..f.item("/c2", 2.0) });
            f.insert(Item { done: true, bookmark: true, date_fetch: 500, source_order: 2, ..f.item("/c3", 3.0) });
            let raw = vec![f.src("/c5", "Item 5", 0), f.src("/c4", "Item 4", 900), f.src("/c3b", "Item 3", 0), f.src("/c2", "Item 2", 0), f.src("/c1", "Item 1", 0)];
            let returned = f.sync.run(raw, &f.entry(), opts(false), None).await.expect("sync");
            let urls: Vec<&str> = returned.iter().map(|c| c.url.as_str()).collect();
            assert_eq!(urls, ["/c5", "/c4"], "{kind}: c3b is a duplicate of a removed read item, not new");
            let all = f.stored();
            let by_url = |u: &str| all.iter().find(|r| r.url == u).expect("row").clone();
            assert_eq!(by_url("/c5").date_fetch, NOW_MS + 3, "newest first: the first listed new item is now + 3");
            assert_eq!(by_url("/c5").date_upload, NOW_MS, "no date and no earlier date: now");
            assert_eq!(by_url("/c4").date_fetch, NOW_MS + 2);
            assert!(by_url("/c3b").done && by_url("/c3b").bookmark);
            assert_eq!(by_url("/c3b").date_fetch, 500);
            assert!(all.iter().all(|r| r.url != "/c3"), "the removed row is gone");
            assert_eq!(renames.calls.lock().unwrap().len(), 1, "only /c2 changes: its stored source order is 1, the source says 3");
        }
    }

    /// Section 7: a first sync is the back catalogue. Adding before or after the first fetch both leave Updates empty;
    /// a later sync's new items show.
    #[tokio::test]
    async fn a_first_sync_is_not_an_update_whether_the_entry_is_added_before_or_after() {
        for kind in [Kind::Manga, Kind::Anime] {
            // Added after its list loaded: not a favourite during the fetch, so the items are stamped at the start of
            // time, and the view's `date_fetch > date_added` cannot hold once the user adds the entry later.
            let after = plain(kind);
            let raw = vec![after.src("/a", "Item 1", 10), after.src("/b", "Item 2", 20)];
            after.sync.run(raw, &after.entry(), opts(false), None).await.unwrap();
            after.db.write(|tx| Ok(tx.execute("UPDATE entries SET favorite = 1, date_added = ?1 WHERE _id = ?2", (NOW_MS + 10, after.id)).map(drop)?)).unwrap();
            let feed = |f: &Fixture| f.db.read(|c| Ok(kuro_db::tables::updates::recent(c, kind, 0, 100)?)).unwrap();
            assert!(feed(&after).is_empty(), "{kind}: added after the first fetch");

            // Added before the list loaded: a favourite with a date, the items are stamped with it.
            let before = fixture(kind, true, 500_000, Arc::new(NoRenames));
            let raw = vec![before.src("/a", "Item 1", 10), before.src("/b", "Item 2", 20)];
            before.sync.run(raw, &before.entry(), opts(false), None).await.unwrap();
            assert!(feed(&before).is_empty(), "{kind}: added before the first fetch");
            assert!(before.stored().iter().all(|i| i.date_fetch <= 500_000));

            // A later sync's new items do show.
            let raw = vec![before.src("/c", "Item 3", 30), before.src("/a", "Item 1", 10), before.src("/b", "Item 2", 20)];
            let new = before.sync.run(raw, &before.entry(), opts(false), None).await.unwrap();
            assert_eq!(new.len(), 1);
            assert_eq!(feed(&before).len(), 1, "{kind}: the later item is an update");
        }
    }

    #[tokio::test]
    async fn an_unchanged_second_sync_writes_nothing_and_keeps_last_update() {
        for kind in [Kind::Manga, Kind::Anime] {
            let f = plain(kind);
            let raw = vec![f.src("/a", "Item 1", 10), f.src("/b", "Item 2", 20)];
            assert_eq!(f.sync.run(raw.clone(), &f.entry(), opts(false), None).await.unwrap().len(), 2);
            let after_first = f.entry();
            assert!(f.sync.run(raw, &after_first, opts(false), None).await.unwrap().is_empty(), "EC-09: nothing new");
            assert_eq!(f.entry().last_update, after_first.last_update, "no last_update bump without changes");
        }
    }

    #[tokio::test]
    async fn a_scanlator_that_disappears_keeps_the_stored_one() {
        let f = plain(Kind::Manga);
        f.insert(Item { scanlator: Some("Team".into()), ..f.item("/c", 1.0) });
        f.sync.run(vec![f.src("/c", "Item 1", 0)], &f.entry(), opts(false), None).await.unwrap();
        assert_eq!(f.stored()[0].scanlator.as_deref(), Some("Team"), "EC-08");
    }

    #[tokio::test]
    async fn duplicate_urls_are_dropped_before_the_order_is_assigned() {
        for kind in [Kind::Manga, Kind::Anime] {
            let f = plain(kind);
            let raw = vec![f.src("/x", "Item 1", 0), f.src("/x", "Item 1 again", 0), f.src("/y", "Item 2", 0)];
            let returned = f.sync.run(raw, &f.entry(), opts(false), None).await.unwrap();
            assert_eq!(returned.iter().map(|c| c.url.as_str()).collect::<Vec<_>>(), ["/x", "/y"]);
            assert_eq!(returned[1].source_order, 1, "EC-03: the second distinct URL is at position 1");
        }
    }

    /// The scanlator rule stays per kind: chapters are trimmed, episodes only blanked.
    #[test]
    fn the_scanlator_rule_is_per_kind() {
        let chapter = SChapter { scanlator: Some("  Team  ".into()), ..SChapter::new("/x", "Ch") };
        assert_eq!(source_item_from_chapter(&chapter).scanlator.as_deref(), Some("Team"));
        let episode = SEpisode { scanlator: Some("  Team  ".into()), ..SEpisode::default() };
        assert_eq!(source_item_from_episode(&episode).scanlator.as_deref(), Some("  Team  "), "episodes do not trim");
        let blank = SEpisode { scanlator: Some("   ".into()), ..SEpisode::default() };
        assert_eq!(source_item_from_episode(&blank).scanlator, None);
    }

    /// The excluded-scanlator filter of the new-item list is a manga rule.
    #[tokio::test]
    async fn a_new_item_of_an_excluded_scanlator_is_not_reported_for_manga() {
        let f = plain(Kind::Manga);
        f.entries.set_excluded_scanlators(f.id, &["B".to_owned()].into_iter().collect()).unwrap();
        let raw = vec![
            SourceItem { scanlator: Some("A".into()), ..f.src("/a", "Item 1", 1) },
            SourceItem { scanlator: Some("B".into()), ..f.src("/b", "Item 2", 2) },
        ];
        let new = f.sync.run(raw, &f.entry(), opts(false), None).await.unwrap();
        assert_eq!(new.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(), ["/a"]);
        assert_eq!(f.stored().len(), 2, "the excluded item is stored, only not reported");
    }

    /// `fillermark_sticky_and_preview_fill`: a user fillermark is never cleared by the source, a stored preview URL is
    /// kept while a blank one is filled, and a source fillermark is taken only while the stored one is false.
    #[tokio::test]
    async fn fillermark_is_sticky_and_the_preview_is_filled_only_when_blank() {
        let f = plain(Kind::Anime);
        f.insert(Item { fillermark: true, ..f.item("/e1", 1.0) });
        f.insert(Item { preview_url: Some("keep".into()), ..f.item("/e2", 2.0) });
        f.insert(f.item("/e3", 3.0));
        let raw = vec![
            SourceItem { preview_url: Some("new".into()), number: 1.0, ..f.src("/e1", "Item 1", 0) },
            SourceItem { preview_url: Some("new".into()), number: 2.0, ..f.src("/e2", "Item 2", 0) },
            SourceItem { fillermark: true, number: 3.0, ..f.src("/e3", "Item 3", 0) },
        ];
        f.sync.run(raw, &f.entry(), opts(false), None).await.unwrap();
        let rows = f.stored();
        let by = |u: &str| rows.iter().find(|r| r.url == u).unwrap().clone();
        assert!(by("/e1").fillermark, "a stored fillermark stays");
        assert_eq!(by("/e1").preview_url.as_deref(), Some("new"), "a blank preview is filled");
        assert_eq!(by("/e2").preview_url.as_deref(), Some("keep"), "a stored preview is kept");
        assert!(by("/e3").fillermark, "false takes the source value true");
    }

    #[test]
    fn the_merge_never_clears_a_fillermark() {
        let db = Item { fillermark: true, ..Item::create(Kind::Anime) };
        let src = Item { fillermark: false, ..Item::create(Kind::Anime) };
        assert!(Item::merged(&db, &src).fillermark);
    }

    /// The delete, insert and the rest run in one transaction: a failing insert rolls back the delete.
    #[tokio::test]
    async fn a_failing_insert_rolls_back_the_delete_for_both_kinds() {
        for kind in [Kind::Manga, Kind::Anime] {
            let f = plain(kind);
            f.insert(f.item("/gone", 1.0));
            f.db.write(|tx| {
                tx.execute_batch("CREATE TRIGGER boom BEFORE INSERT ON items WHEN NEW.url = '/boom' BEGIN SELECT RAISE(ABORT, 'boom'); END;")?;
                Ok(())
            })
            .unwrap();
            let before = f.entry();
            let urls = |f: &Fixture| f.stored().into_iter().map(|r| r.url).collect::<Vec<_>>();
            let err = f.sync.run(vec![f.src("/boom", "Item 2", 0)], &before, opts(false), None).await.expect_err("the insert fails");
            assert!(matches!(err, SyncItemsError::Db(_)));
            assert_eq!(urls(&f), ["/gone"], "the removed row is still stored");
            assert_eq!(f.entry().last_update, before.last_update, "no last_update write survived either");
            f.db.write(|tx| {
                tx.execute_batch("DROP TRIGGER boom;")?;
                Ok(())
            })
            .unwrap();
            f.sync.run(vec![f.src("/boom", "Item 2", 0)], &before, opts(false), None).await.unwrap();
            assert_eq!(urls(&f), ["/boom"], "control: without the failing insert the same plan does delete the row");
        }
    }

    /// RC-08: `run` takes the per-entry lock, so a held lock stops it before any write.
    #[tokio::test]
    async fn run_waits_for_the_per_entry_lock() {
        let f = plain(Kind::Manga);
        let locks = Arc::new(EntrySyncLocks::default());
        let sync = SyncItems::new(f.db.clone(), locks.clone(), Arc::new(NoRenames));
        let held = locks.lock(f.id).await;
        let blocked = tokio::time::timeout(std::time::Duration::from_millis(50), sync.run(vec![f.src("/a", "Item 1", 0)], &f.entry(), opts(false), None)).await;
        assert!(blocked.is_err(), "the sync waits while the lock is held");
        assert!(f.stored().is_empty(), "nothing was written");
        drop(held);
        assert_eq!(sync.run(vec![f.src("/a", "Item 1", 0)], &f.entry(), opts(false), None).await.unwrap().len(), 1);
    }

    const SENTINEL_NEXT: i64 = 7_777_777;

    fn set_interval(f: &Fixture, interval: i64, next_update: i64) {
        f.db.write(|tx| Ok(entry_table::update(tx, &EntryPatch { id: f.id, next_update: Some(next_update), calculate_interval: Some(interval), ..EntryPatch::default() }).map(drop)?)).unwrap();
    }

    /// Syncs once (changes), pins the interval fields, then syncs the same list again with the given options.
    async fn second_sync_row(kind: Kind, interval: i64, next_update: i64, manual: bool) -> (Entry, Entry) {
        let f = plain(kind);
        let raw = vec![f.src("/a", "Item 1", 10), f.src("/b", "Item 2", 20)];
        f.sync.run(raw.clone(), &f.entry(), SyncOptions { window: WINDOW, ..opts(false) }, None).await.unwrap();
        set_interval(&f, interval, next_update);
        let before = f.entry();
        let out = f.sync.run(raw, &before, SyncOptions { window: WINDOW, ..opts(manual) }, None).await.unwrap();
        assert!(out.is_empty());
        (before, f.entry())
    }

    /// S-11: no change, interval set, next update not before the window start, not manual: nothing is written.
    #[tokio::test]
    async fn s11_no_change_and_no_trigger_recomputes_nothing() {
        for kind in [Kind::Manga, Kind::Anime] {
            let (before, after) = second_sync_row(kind, 7, SENTINEL_NEXT, false).await;
            assert_eq!((after.next_update, after.fetch_interval, after.last_update), (SENTINEL_NEXT, 7, before.last_update));
        }
    }

    /// S-12: no change, but a manual refresh, interval 0, or a next update before the window start recomputes.
    #[tokio::test]
    async fn s12_no_change_recomputes_on_manual_interval_zero_or_overdue() {
        for kind in [Kind::Manga, Kind::Anime] {
            for (interval, next_update, manual, why) in [(7, SENTINEL_NEXT, true, "manual"), (0, SENTINEL_NEXT, false, "interval 0"), (7, 5, false, "next update before the window start")] {
                let (before, after) = second_sync_row(kind, interval, next_update, manual).await;
                assert!(after.next_update != next_update || after.fetch_interval != i32::try_from(interval).unwrap(), "{kind} {why}: recomputed");
                assert_eq!(after.last_update, before.last_update, "{why}: last_update is not touched");
            }
        }
    }

    /// S-13: with changes the interval is recomputed even when no trigger holds, and `last_update` becomes now.
    #[tokio::test]
    async fn s13_changes_recompute_and_set_last_update_to_now() {
        for kind in [Kind::Manga, Kind::Anime] {
            let f = plain(kind);
            set_interval(&f, 7, SENTINEL_NEXT);
            let o = SyncOptions { window: WINDOW, ..opts(false) };
            assert_eq!(f.sync.run(vec![f.src("/a", "Item 1", 10)], &f.entry(), o, None).await.unwrap().len(), 1);
            let after = f.entry();
            assert_eq!(after.last_update, NOW_MS);
            assert_ne!(after.next_update, SENTINEL_NEXT, "recomputed");
        }
    }
}
