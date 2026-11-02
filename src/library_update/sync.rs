//! The database side of the item sync of a pass (behaviour 5.4 and 5.5): the `ItemSync` the pass calls for each
//! entry. The diff is the domain's (`kuro_domain::items::sync::SyncItems`, one for both kinds). This file turns the
//! pass's `SourceItem`s into the domain's source items (the scanlator rule stays per kind in the two adapters), runs
//! the shared sync on the entry's row and maps the new rows back to `SourceItem`.

use std::sync::Arc;

use kuro_data::entry::{Entry, EntryRepo};
use kuro_data::item::Item;
use kuro_db::handle::Db;
use kuro_domain::items::source_sync::EntrySyncLocks;
use kuro_domain::items::sync::{
    ItemRenames, NoRenames, SourceItem as DomainItem, SyncItems, SyncItemsError, SyncOptions, source_item_from_chapter,
    source_item_from_episode,
};
use kuro_platform::kind::Kind;
use kuro_source::anime::SEpisode;
use kuro_source::local::LOCAL_SOURCE_ID;
use kuro_source::model::chapter::SChapter;

use crate::library_update::error::UpdateError;
use crate::library_update::fetch_interval::Window;
use crate::library_update::pass::ItemSync;
use crate::library_update::queue::LibraryEntry;
use crate::library_update::source::{Fetched, SourceItem};

/// The item sync over the app database. One lock set serves both kinds (the key is the entry id, unique across them);
/// the caller shares it with other writers of the same rows, so a pass and a manual refresh never interleave.
pub struct DbItemSync {
    db: Arc<Db>,
    locks: Arc<EntrySyncLocks>,
    sync: SyncItems,
    mark_duplicate_read: bool,
    mark_duplicate_seen: bool,
}

impl DbItemSync {
    pub fn new(db: Arc<Db>, locks: Arc<EntrySyncLocks>, mark_duplicate_read: bool, mark_duplicate_seen: bool) -> Self {
        let sync = SyncItems::new(db.clone(), locks.clone(), Arc::new(NoRenames));
        Self { db, locks, sync, mark_duplicate_read, mark_duplicate_seen }
    }

    /// Takes the folder renames of downloaded items instead of the no-op one (T-26a-086).
    pub fn with_renames(mut self, renames: Arc<dyn ItemRenames>) -> Self {
        self.sync = SyncItems::new(self.db.clone(), self.locks.clone(), renames);
        self
    }

    fn mark_duplicate(&self, kind: Kind) -> bool {
        match kind {
            Kind::Manga => self.mark_duplicate_read,
            Kind::Anime => self.mark_duplicate_seen,
        }
    }
}

/// The source chapter a pass item stands for.
pub(crate) fn to_source_chapter(item: &SourceItem) -> SChapter {
    SChapter {
        url: item.url.clone(),
        name: item.name.clone(),
        date_upload: item.date_upload_ms,
        chapter_number: item.number as f32,
        scanlator: item.scanlator.clone(),
    }
}

/// The source episode a pass item stands for. Fields the pass does not carry keep the source defaults.
pub(crate) fn to_source_episode(item: &SourceItem) -> SEpisode {
    SEpisode {
        url: item.url.clone(),
        name: item.name.clone(),
        episode_number: item.number as f32,
        date_upload: item.date_upload_ms,
        scanlator: item.scanlator.clone(),
        ..SEpisode::default()
    }
}

/// The domain source item of a pass item: the per-kind adapter keeps the kind's scanlator rule.
fn domain_item(kind: Kind, item: &SourceItem) -> DomainItem {
    match kind {
        Kind::Manga => source_item_from_chapter(&to_source_chapter(item)),
        Kind::Anime => source_item_from_episode(&to_source_episode(item)),
    }
}

/// The pass item of a stored item (the rows the domain returns as new).
pub(crate) fn item_of(i: &Item) -> SourceItem {
    SourceItem {
        url: i.url.clone(),
        name: i.name.clone(),
        number: i.number,
        date_upload_ms: i.date_upload,
        source_order: i.source_order,
        scanlator: i.scanlator.clone(),
    }
}

fn window_pair(window: Window) -> (i64, i64) {
    (window.lower_ms, window.upper_ms)
}

fn sync_error(kind: Kind, error: SyncItemsError) -> UpdateError {
    match error {
        SyncItemsError::NoItems => match kind {
            Kind::Manga => UpdateError::NoChapters,
            Kind::Anime => UpdateError::NoEpisodes,
        },
        SyncItemsError::Db(e) => UpdateError::from(e),
    }
}

/// Reads the entry row off the async worker: the repository call blocks.
async fn load_entry(db: Arc<Db>, id: i64) -> Result<Entry, UpdateError> {
    tokio::task::spawn_blocking(move || EntryRepo::new(db).by_id(id))
        .await
        .map_err(|e| UpdateError::Other(e.to_string()))?
        .map_err(|e| UpdateError::Other(e.to_string()))
}

impl ItemSync for DbItemSync {
    async fn sync(
        &self,
        entry: &LibraryEntry,
        fetched: &Fetched,
        window: Window,
        now_ms: i64,
    ) -> Result<Vec<SourceItem>, UpdateError> {
        // The pass does not say whether the refresh was manual; an automatic pass is the default (see DECISIONS).
        let kind = entry.kind;
        let stored = load_entry(self.db.clone(), entry.id).await?;
        let raw: Vec<DomainItem> = fetched.items.iter().map(|i| domain_item(kind, i)).collect();
        let opts = SyncOptions {
            now_ms,
            manual: false,
            window: window_pair(window),
            mark_duplicate_done: self.mark_duplicate(kind),
            is_local: entry.source == LOCAL_SOURCE_ID,
        };
        let new = self.sync.run(raw, &stored, opts, None).await.map_err(|e| sync_error(kind, e))?;
        Ok(new.iter().map(item_of).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_data::item::Item;

    fn item(url: &str, number: f64) -> SourceItem {
        SourceItem {
            url: url.into(),
            name: format!("Chapter {number}"),
            number,
            date_upload_ms: 42,
            source_order: 3,
            scanlator: Some("group".into()),
        }
    }

    #[test]
    fn a_pass_item_becomes_a_source_chapter_with_its_fields() {
        let chapter = to_source_chapter(&item("/c/1", 1.5));
        assert_eq!(chapter.url, "/c/1");
        assert_eq!(chapter.name, "Chapter 1.5");
        assert_eq!(chapter.chapter_number, 1.5);
        assert_eq!(chapter.date_upload, 42);
        assert_eq!(chapter.scanlator.as_deref(), Some("group"));
    }

    #[test]
    fn a_pass_item_becomes_a_source_episode_and_keeps_source_defaults_for_the_rest() {
        let episode = to_source_episode(&item("/e/2", -1.0));
        assert_eq!(episode.url, "/e/2");
        assert_eq!(episode.episode_number, -1.0, "an unrecognised number stays negative");
        assert_eq!(episode.date_upload, 42);
        assert!(!episode.fillermark);
        assert_eq!(episode.summary, None);
        assert_eq!(episode.preview_url, None);
        assert!(episode.memo.is_empty());
    }

    #[test]
    fn stored_rows_map_back_to_pass_items() {
        let stored = Item {
            url: "/c/9".into(),
            name: "Nine".into(),
            number: 9.0,
            date_upload: 7,
            source_order: 4,
            scanlator: None,
            ..Item::create(Kind::Manga)
        };
        let back = item_of(&stored);
        assert_eq!(back, SourceItem { url: "/c/9".into(), name: "Nine".into(), number: 9.0, date_upload_ms: 7, source_order: 4, scanlator: None });
    }

    #[test]
    fn the_domain_errors_map_to_the_pass_errors() {
        assert_eq!(sync_error(Kind::Manga, SyncItemsError::NoItems), UpdateError::NoChapters);
        assert_eq!(sync_error(Kind::Anime, SyncItemsError::NoItems), UpdateError::NoEpisodes);
    }

    fn test_db(dir: &std::path::Path) -> Arc<Db> {
        let path = kuro_db::schema_unified::db_path(dir);
        let writer = kuro_db::schema_unified::open_file(&path).expect("open manga db");
        Arc::new(Db::new(&path, writer).expect("manga db handle"))
    }

    fn sync_for(manga: Arc<Db>) -> DbItemSync {
        DbItemSync::new(manga, Arc::new(EntrySyncLocks::default()), false, false)
    }

    fn entry(id: i64) -> LibraryEntry {
        crate::library_update::fake::entry(id, 7, "Berserk")
    }

    fn window() -> Window {
        Window { lower_ms: 0, upper_ms: 0 }
    }

    fn fetched(items: Vec<SourceItem>) -> Fetched {
        Fetched { details: None, items }
    }

    #[tokio::test]
    async fn the_second_sync_of_the_same_list_finds_nothing_new() {
        let dir = tempfile::tempdir().expect("temp dir");
        let manga = test_db(dir.path());
        let id = EntryRepo::new(manga.clone())
            .insert(&Entry { favorite: true, source: 7, url: "/m".into(), title: "Berserk".into(), ..Entry::create(Kind::Manga) })
            .expect("insert manga");
        let sync = sync_for(manga);
        let list = fetched(vec![item("/c/1", 1.0), item("/c/2", 2.0)]);
        let first = sync.sync(&entry(id), &list, window(), 1_000).await.expect("first sync");
        assert_eq!(first.len(), 2, "both chapters are new on the first sync");
        let second = sync.sync(&entry(id), &list, window(), 2_000).await.expect("second sync");
        assert!(second.is_empty(), "nothing is new the second time");
    }

    #[derive(Default)]
    struct RecordingRenames(std::sync::Mutex<Vec<(String, String)>>);

    impl ItemRenames for RecordingRenames {
        fn rename(&self, _e: &Entry, stored: &Item, prepared: &Item) {
            self.0.lock().unwrap().push((stored.name.clone(), prepared.name.clone()));
        }
    }

    #[tokio::test]
    async fn a_pass_sync_hands_a_renamed_chapter_to_the_rename_port_it_was_given() {
        // T-26a-086: the pass takes the real port in place of `NoRenames`.
        let dir = tempfile::tempdir().expect("temp dir");
        let manga = test_db(dir.path());
        let id = EntryRepo::new(manga.clone())
            .insert(&Entry { favorite: true, source: 7, url: "/m".into(), title: "Berserk".into(), ..Entry::create(Kind::Manga) })
            .expect("insert manga");
        let renames = Arc::new(RecordingRenames::default());
        let sync = sync_for(manga).with_renames(renames.clone());
        sync.sync(&entry(id), &fetched(vec![item("/c/1", 1.0)]), window(), 1_000).await.expect("first sync");
        assert!(renames.0.lock().unwrap().is_empty(), "a new chapter has nothing to rename");
        let renamed = SourceItem { name: "Chapter 1 v2".into(), ..item("/c/1", 1.0) };
        sync.sync(&entry(id), &fetched(vec![renamed]), window(), 2_000).await.expect("second sync");
        assert_eq!(renames.0.lock().unwrap().as_slice(), &[("Chapter 1".to_owned(), "Chapter 1 v2".to_owned())]);
    }

    #[tokio::test]
    async fn an_empty_list_from_a_remote_source_is_no_chapters_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let manga = test_db(dir.path());
        let id = EntryRepo::new(manga.clone())
            .insert(&Entry { favorite: true, source: 7, url: "/m".into(), title: "Berserk".into(), ..Entry::create(Kind::Manga) })
            .expect("insert manga");
        let sync = sync_for(manga);
        let result = sync.sync(&entry(id), &fetched(Vec::new()), window(), 1_000).await;
        assert_eq!(result, Err(UpdateError::NoChapters));
    }
}
