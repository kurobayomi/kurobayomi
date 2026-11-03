//! History of both kinds: one `History`, one `HistoryItem` and one `HistoryRepo` over `kuro_db::tables::history`.

use std::sync::Arc;

use kuro_db::handle::Db;
use kuro_db::subscribe::Subscription;
use kuro_db::tables::history as t;
use kuro_db::tables::{HistoryRow, HistoryViewRow};
use kuro_platform::kind::Kind;
use rusqlite::Connection;

use crate::{DataResult, ENTRY_DEBOUNCE};

/// Tables a history list reads.
const HISTORY_TABLES: &[&str] = &["history", "items", "entries"];

/// One `history` row of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct History {
    pub id: i64,
    pub item_id: i64,
    /// Epoch milliseconds of the last read or view; 0 after a reset (the row and its duration stay).
    pub last_at: i64,
    /// Accumulated time in milliseconds.
    pub duration_ms: i64,
}

impl From<HistoryRow> for History {
    fn from(r: HistoryRow) -> Self {
        Self { id: r.id, item_id: r.item_id, last_at: r.last_at, duration_ms: r.duration_ms }
    }
}

/// One line of the history list: the newest history row of an entry, with the entry's cover data.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryItem {
    pub id: i64,
    pub kind: Kind,
    pub entry_id: i64,
    pub item_id: i64,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub source: i64,
    pub favorite: bool,
    pub cover_last_modified: i64,
    pub item_number: f64,
    pub last_at: i64,
    pub duration_ms: i64,
}

impl From<HistoryViewRow> for HistoryItem {
    fn from(r: HistoryViewRow) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            entry_id: r.entry_id,
            item_id: r.item_id,
            title: r.title,
            thumbnail_url: r.thumbnail_url,
            source: r.source,
            favorite: r.favorite,
            cover_last_modified: r.cover_last_modified,
            item_number: r.item_number,
            last_at: r.last_at,
            duration_ms: r.duration_ms,
        }
    }
}

/// History access for both kinds.
#[derive(Clone)]
pub struct HistoryRepo {
    db: Arc<Db>,
}

impl HistoryRepo {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// One row per entry of the kind (its newest history row) with `last_at > 0`, newest first, ties by the higher id.
    /// `query` is a case-insensitive substring of the title (Unicode fold); `%`, `_` and `\` are literal.
    pub fn list(&self, kind: Kind, query: &str) -> DataResult<Vec<HistoryItem>> {
        let rows = self.db.read(|c| Ok(t::list(c, kind, query)?))?;
        Ok(rows.into_iter().map(HistoryItem::from).collect())
    }

    /// The list, published again after a change to history, items or entries.
    pub fn watch(&self, kind: Kind, query: &str) -> Subscription<Vec<HistoryItem>> {
        let query = query.to_owned();
        self.db.subscribe(HISTORY_TABLES, ENTRY_DEBOUNCE, move |c| {
            Ok(t::list(c, kind, &query)?.into_iter().map(HistoryItem::from).collect::<Vec<_>>())
        })
    }

    /// The newest history row of the kind with `last_at > 0`; equals the first row of [`Self::list`] with no query.
    pub fn latest(&self, kind: Kind) -> DataResult<Option<HistoryItem>> {
        Ok(self.db.read(|c| Ok(t::latest(c, kind)?))?.map(HistoryItem::from))
    }

    /// Accumulated time over every history row of the kind, reset rows included.
    pub fn total_duration(&self, kind: Kind) -> DataResult<i64> {
        self.db.read(|c| Ok(t::total_duration(c, kind)?))
    }

    /// Every history row of one entry, reset rows included.
    pub fn by_entry_id(&self, entry_id: i64) -> DataResult<Vec<History>> {
        let rows = self.db.read(|c| Ok(t::by_entry(c, entry_id)?))?;
        Ok(rows.into_iter().map(History::from).collect())
    }

    /// The history row of an item of the kind with this url, lowest id first (the backup restore reads it).
    pub fn by_item_url(c: &Connection, kind: Kind, url: &str) -> DataResult<Option<History>> {
        Ok(t::by_item_url(c, kind, url)?.map(History::from))
    }

    /// Sets the time of the last read to `at_ms` and ADDS `delta_ms` to the duration (a new row starts at `delta_ms`).
    pub fn upsert(&self, item_id: i64, at_ms: i64, delta_ms: i64) -> DataResult<()> {
        self.db.write(|tx| Ok(t::upsert(tx, item_id, at_ms, delta_ms).map(drop)?))
    }

    /// `last_at = 0` on one row; the row and its duration stay.
    pub fn reset_by_id(&self, history_id: i64) -> DataResult<()> {
        self.db.write(|tx| Ok(t::reset_by_id(tx, history_id).map(drop)?))
    }

    pub fn reset_by_entry_id(&self, entry_id: i64) -> DataResult<()> {
        self.db.write(|tx| Ok(t::reset_by_entry(tx, entry_id).map(drop)?))
    }

    /// Deletes every history row of the kind, time included.
    pub fn delete_all(&self, kind: Kind) -> DataResult<()> {
        self.db.write(|tx| Ok(t::remove_all(tx, kind).map(drop)?))
    }

    /// Deletes the reset rows of the kind inside the caller's transaction (restore). Returns the rows removed.
    pub fn remove_resetted(c: &Connection, kind: Kind) -> DataResult<usize> {
        Ok(t::remove_resetted(c, kind)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::{insert_chapter, insert_episode, insert_manga, insert_anime, test_db};

    /// One favourite manga per title, with one chapter per url; returns `(manga id, chapter id)` pairs.
    fn repo_with_chapters(titles: &[(&str, &[&str])]) -> (tempfile::TempDir, Arc<Db>, Vec<(i64, i64)>) {
        let (dir, db) = test_db();
        let mut ids = Vec::new();
        for (title, urls) in titles {
            let m = insert_manga(&db, title, true);
            for url in urls.iter() {
                ids.push((m, insert_chapter(&db, m, url)));
            }
        }
        (dir, db, ids)
    }

    /// A five-row scenario with two manga.
    #[test]
    fn list_shows_the_latest_item_per_entry_and_follows_resets() {
        let (_d, db, ids) = repo_with_chapters(&[("Alpha", &["a1", "a2"]), ("Beta", &["b1"])]);
        let (a1, a2, b1) = (ids[0].1, ids[1].1, ids[2].1);
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(a1, 100, 0).unwrap();
        repo.upsert(a2, 300, 0).unwrap();
        repo.upsert(b1, 200, 0).unwrap();
        let listed = repo.list(Kind::Manga, "").unwrap();
        assert_eq!(listed.iter().map(|h| (h.title.as_str(), h.last_at)).collect::<Vec<_>>(), [("Alpha", 300), ("Beta", 200)]);
        assert_eq!(listed[0].item_id, a2);

        // Resetting a2 reveals a1 for Alpha.
        let a2_history = repo.by_entry_id(ids[0].0).unwrap().into_iter().find(|h| h.item_id == a2).unwrap();
        repo.reset_by_id(a2_history.id).unwrap();
        let after = repo.list(Kind::Manga, "").unwrap();
        assert_eq!(after.iter().find(|h| h.title == "Alpha").unwrap().item_id, a1);

        // Resetting every row of Beta hides it, but keeps its time.
        repo.reset_by_entry_id(ids[2].0).unwrap();
        let hidden = repo.list(Kind::Manga, "").unwrap();
        assert!(hidden.iter().all(|h| h.title != "Beta"));
        assert_eq!(repo.by_entry_id(ids[2].0).unwrap().len(), 1);
    }

    #[test]
    fn latest_equals_the_first_list_row_and_is_none_when_empty() {
        let (_d, db, ids) = repo_with_chapters(&[("Alpha", &["a1"]), ("Beta", &["b1"])]);
        let repo = HistoryRepo::new(db.clone());
        assert_eq!(repo.latest(Kind::Manga).unwrap(), None);
        repo.upsert(ids[0].1, 10, 0).unwrap();
        repo.upsert(ids[1].1, 20, 0).unwrap();
        assert_eq!(repo.latest(Kind::Manga).unwrap(), repo.list(Kind::Manga, "").unwrap().into_iter().next());
    }

    #[test]
    fn equal_times_are_ordered_by_higher_id_first() {
        let (_d, db, ids) = repo_with_chapters(&[("Alpha", &["a1"]), ("Beta", &["b1"])]);
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(ids[0].1, 50, 0).unwrap();
        repo.upsert(ids[1].1, 50, 0).unwrap();
        let listed = repo.list(Kind::Manga, "").unwrap();
        assert!(listed[0].id > listed[1].id);
    }

    #[test]
    fn search_folds_unicode_case_and_matches_percent_literally() {
        let (_d, db, ids) = repo_with_chapters(&[("Éclair", &["e1"]), ("Plain", &["p1"]), ("100% Pure", &["x1"])]);
        let repo = HistoryRepo::new(db.clone());
        for (_, c) in &ids {
            repo.upsert(*c, 10, 0).unwrap();
        }
        let found: Vec<String> = repo.list(Kind::Manga, "éclair").unwrap().into_iter().map(|h| h.title).collect();
        assert_eq!(found, ["Éclair"]);
        let literal: Vec<String> = repo.list(Kind::Manga, "100%").unwrap().into_iter().map(|h| h.title).collect();
        assert_eq!(literal, ["100% Pure"]);
        assert_eq!(repo.list(Kind::Manga, "").unwrap().len(), 3, "an empty query returns every entry");
    }

    #[test]
    fn list_rows_carry_the_entry_favorite_flag() {
        let (_d, db) = test_db();
        let fav = insert_manga(&db, "Fav", true);
        let not = insert_manga(&db, "Not", false);
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(insert_chapter(&db, fav, "f"), 10, 0).unwrap();
        repo.upsert(insert_chapter(&db, not, "n"), 20, 0).unwrap();
        let listed = repo.list(Kind::Manga, "").unwrap();
        assert_eq!(listed.iter().map(|h| (h.title.as_str(), h.favorite)).collect::<Vec<_>>(), [("Not", false), ("Fav", true)]);
    }

    /// The worked example of section 3: upserts accumulate time, resets keep it, a new read adds to it.
    #[test]
    fn upsert_accumulates_time_across_reset_and_reread() {
        let (_d, db) = test_db();
        let m = insert_manga(&db, "m", false);
        let c10 = insert_chapter(&db, m, "c10");
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(c10, 61_000, 60_000).unwrap();
        repo.upsert(c10, 200_000, 30_000).unwrap();
        let row = repo.by_entry_id(m).unwrap().remove(0);
        assert_eq!((row.duration_ms, row.last_at), (90_000, 200_000));
        repo.reset_by_id(row.id).unwrap();
        let reset = repo.by_entry_id(m).unwrap().remove(0);
        assert_eq!((reset.last_at, reset.duration_ms), (0, 90_000));
        assert_eq!(repo.total_duration(Kind::Manga).unwrap(), 90_000);
        repo.upsert(c10, 300_000, 5_000).unwrap();
        let reread = repo.by_entry_id(m).unwrap().remove(0);
        assert_eq!((reread.last_at, reread.duration_ms), (300_000, 95_000));
    }

    #[test]
    fn an_upsert_sets_the_time_even_when_older_for_anime() {
        let (_d, db) = test_db();
        let a = insert_anime(&db, "a", false);
        let e = insert_episode(&db, a, "e1");
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(e, 500, 0).unwrap();
        repo.upsert(e, 200, 0).unwrap();
        let rows = repo.by_entry_id(a).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].last_at, 200);
        repo.reset_by_id(rows[0].id).unwrap();
        assert_eq!(repo.by_entry_id(a).unwrap()[0].last_at, 0, "a reset keeps the row");
        repo.delete_all(Kind::Anime).unwrap();
        assert!(repo.by_entry_id(a).unwrap().is_empty());
    }

    #[test]
    fn reset_by_entry_id_touches_only_that_entry_and_delete_all_only_its_kind() {
        let (_d, db) = test_db();
        let a = insert_manga(&db, "a", false);
        let b = insert_manga(&db, "b", false);
        let anime = insert_anime(&db, "z", false);
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(insert_chapter(&db, a, "a1"), 10, 100).unwrap();
        repo.upsert(insert_chapter(&db, b, "b1"), 20, 200).unwrap();
        repo.upsert(insert_episode(&db, anime, "z1"), 30, 400).unwrap();
        repo.reset_by_entry_id(a).unwrap();
        assert_eq!(repo.by_entry_id(a).unwrap()[0].last_at, 0);
        assert_eq!(repo.by_entry_id(b).unwrap()[0].last_at, 20);
        assert_eq!(repo.total_duration(Kind::Manga).unwrap(), 300, "reset rows keep their time");
        repo.delete_all(Kind::Manga).unwrap();
        assert_eq!(repo.total_duration(Kind::Manga).unwrap(), 0);
        assert!(repo.by_entry_id(a).unwrap().is_empty());
        assert_eq!(repo.total_duration(Kind::Anime).unwrap(), 400, "the other kind is untouched");
    }

    #[test]
    fn a_failing_upsert_returns_the_error() {
        let (_d, db) = test_db();
        let repo = HistoryRepo::new(db.clone());
        // Item 999 does not exist: the foreign key fails.
        assert!(repo.upsert(999, 10, 0).is_err());
        assert!(repo.list(Kind::Manga, "").unwrap().is_empty());
    }

    #[test]
    fn a_history_watch_re_emits_after_upsert_reset_and_item_delete() {
        let (_d, db) = test_db();
        let m = insert_manga(&db, "m", false);
        let chapter = insert_chapter(&db, m, "c");
        let repo = HistoryRepo::new(db.clone());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut sub = repo.watch(Kind::Manga, "");
            assert!(sub.changed().await.unwrap().unwrap().is_empty());
            repo.upsert(chapter, 10, 0).unwrap();
            assert_eq!(sub.changed().await.unwrap().unwrap().len(), 1);
            repo.reset_by_entry_id(m).unwrap();
            assert!(sub.changed().await.unwrap().unwrap().is_empty());
            repo.upsert(chapter, 20, 0).unwrap();
            assert_eq!(sub.changed().await.unwrap().unwrap().len(), 1);
            db.write(|tx| Ok(tx.execute("DELETE FROM items WHERE _id = ?1", [chapter]).map(drop)?)).unwrap();
            assert!(sub.changed().await.unwrap().unwrap().is_empty(), "cascade removes the history row");
        });
    }

    #[test]
    fn delete_all_notifies_a_history_watch() {
        let (_d, db) = test_db();
        let m = insert_manga(&db, "m", false);
        let chapter = insert_chapter(&db, m, "c");
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(chapter, 10, 0).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut sub = repo.watch(Kind::Manga, "");
            assert_eq!(sub.changed().await.unwrap().unwrap().len(), 1);
            repo.delete_all(Kind::Manga).unwrap();
            assert!(sub.changed().await.unwrap().unwrap().is_empty(), "truncate-style delete still notifies");
        });
    }

    #[test]
    fn remove_resetted_deletes_only_reset_rows_of_the_kind_and_by_item_url_finds_a_row() {
        let (_d, db) = test_db();
        let m = insert_manga(&db, "m", false);
        let (a, b) = (insert_chapter(&db, m, "a"), insert_chapter(&db, m, "b"));
        let anime = insert_anime(&db, "z", false);
        let e = insert_episode(&db, anime, "e1");
        let repo = HistoryRepo::new(db.clone());
        repo.upsert(a, 100, 5).unwrap();
        repo.upsert(b, 200, 5).unwrap();
        repo.upsert(e, 300, 5).unwrap();
        let a_id = repo.by_entry_id(m).unwrap().into_iter().find(|h| h.item_id == a).unwrap().id;
        repo.reset_by_id(a_id).unwrap();
        repo.reset_by_entry_id(anime).unwrap();
        let found = db.read(|c| Ok(HistoryRepo::by_item_url(c, Kind::Manga, "a")?)).unwrap().expect("row for a");
        assert_eq!((found.item_id, found.last_at), (a, 0));
        assert!(db.read(|c| Ok(HistoryRepo::by_item_url(c, Kind::Manga, "missing")?)).unwrap().is_none());
        assert_eq!(db.write(|tx| Ok(HistoryRepo::remove_resetted(tx, Kind::Manga)?)).unwrap(), 1);
        assert_eq!(repo.by_entry_id(m).unwrap().iter().map(|h| h.item_id).collect::<Vec<_>>(), [b]);
        assert_eq!(db.write(|tx| Ok(HistoryRepo::remove_resetted(tx, Kind::Manga)?)).unwrap(), 0);
        assert_eq!(repo.by_entry_id(anime).unwrap().len(), 1, "the anime reset row is kept");
    }
}
