//! Items of both kinds (chapters and episodes): one `Item`, one `ItemUpdate` and one `ItemRepo` over
//! `kuro_db::tables::items`. `kind` is not a column; it is the kind of the entry the item belongs to.

use std::collections::HashMap;
use std::sync::Arc;

use kuro_db::handle::Db;
use kuro_db::codec::Memo;
use kuro_db::subscribe::Subscription;
use kuro_db::tables::items as t;
use kuro_db::tables::{ItemPatch, ItemRow};
use kuro_platform::kind::Kind;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value};

use crate::{DataError, DataResult, ENTRY_DEBOUNCE, ID_CHUNK, none_if_not_found};

/// Tables an item list reads: the rows and the scanlator exclusions of the filter.
const ITEM_TABLES: &[&str] = &["items", "excluded_scanlators"];

/// One `items` row. The anime-only fields are `None` for manga; `fillermark` is false for manga.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `-1` until stored.
    pub id: i64,
    pub entry_id: i64,
    pub kind: Kind,
    pub url: String,
    pub name: String,
    /// `None` for a blank scanlator.
    pub scanlator: Option<String>,
    pub done: bool,
    pub bookmark: bool,
    pub fillermark: bool,
    /// `last_page_read` for manga, `last_second_seen` for anime.
    pub progress: i64,
    /// `-1.0` when unrecognised, `-2.0` for the intentionally numberless sentinel.
    pub number: f64,
    pub source_order: i64,
    pub date_fetch: i64,
    pub date_upload: i64,
    pub last_modified_at: i64,
    pub version: i64,
    pub total_seconds: Option<i64>,
    pub summary: Option<String>,
    pub preview_url: Option<String>,
    pub memo: Option<Memo>,
    /// How many pages a chapter has, as the reader last saw it; `None` for an episode and a chapter not read since the
    /// app stores it. With `progress` (the last page read) it says how far through the chapter one is.
    pub page_count: Option<i64>,
}

impl Item {
    /// `Chapter.create()` / `Episode.create()`: id and entry id -1, date upload -1, number -1.0, version 1. Anime also
    /// gets total seconds 0 and an empty memo; manga keeps them `None`.
    pub fn create(kind: Kind) -> Self {
        let anime = kind.is_anime();
        Self {
            id: -1,
            entry_id: -1,
            kind,
            url: String::new(),
            name: String::new(),
            scanlator: None,
            done: false,
            bookmark: false,
            fillermark: false,
            progress: 0,
            number: -1.0,
            source_order: 0,
            date_fetch: 0,
            date_upload: -1,
            last_modified_at: 0,
            version: 1,
            total_seconds: anime.then_some(0),
            summary: None,
            preview_url: None,
            memo: anime.then(Memo::default),
            page_count: None,
        }
    }

    /// 0 for manga.
    pub fn total_seconds(&self) -> i64 {
        self.total_seconds.unwrap_or(0)
    }

    /// Empty for manga.
    pub fn memo(&self) -> Map<String, Value> {
        self.memo.as_ref().map(|m| m.0.clone()).unwrap_or_default()
    }
}

impl From<ItemRow> for Item {
    fn from(r: ItemRow) -> Self {
        Self {
            id: r.id,
            entry_id: r.entry_id,
            kind: r.kind,
            url: r.url,
            name: r.name,
            scanlator: r.scanlator,
            done: r.done,
            bookmark: r.bookmark,
            fillermark: r.fillermark,
            progress: r.progress,
            number: r.number,
            source_order: r.source_order,
            date_fetch: r.date_fetch,
            date_upload: r.date_upload,
            last_modified_at: r.last_modified_at,
            version: r.version,
            total_seconds: r.total_seconds,
            summary: r.summary,
            preview_url: r.preview_url,
            memo: r.memo,
            page_count: r.page_count,
        }
    }
}

/// The insert row; the statement binds neither `id`, `kind` nor `last_modified_at`.
impl From<&Item> for ItemRow {
    fn from(i: &Item) -> Self {
        Self {
            id: i.id,
            entry_id: i.entry_id,
            kind: i.kind,
            url: i.url.clone(),
            name: i.name.clone(),
            scanlator: i.scanlator.clone(),
            done: i.done,
            bookmark: i.bookmark,
            fillermark: i.fillermark,
            progress: i.progress,
            number: i.number,
            source_order: i.source_order,
            date_fetch: i.date_fetch,
            date_upload: i.date_upload,
            last_modified_at: i.last_modified_at,
            version: i.version,
            total_seconds: i.total_seconds,
            summary: i.summary.clone(),
            preview_url: i.preview_url.clone(),
            memo: i.memo.clone(),
            page_count: i.page_count,
        }
    }
}

/// Partial update. `None` keeps the stored value; `id` is required.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemUpdate {
    pub id: i64,
    pub entry_id: Option<i64>,
    pub done: Option<bool>,
    pub bookmark: Option<bool>,
    pub fillermark: Option<bool>,
    pub progress: Option<i64>,
    pub total_seconds: Option<i64>,
    pub date_fetch: Option<i64>,
    pub source_order: Option<i64>,
    pub url: Option<String>,
    pub name: Option<String>,
    pub date_upload: Option<i64>,
    pub number: Option<f64>,
    pub scanlator: Option<String>,
    pub summary: Option<String>,
    pub preview_url: Option<String>,
    pub version: Option<i64>,
    pub memo: Option<Memo>,
    pub page_count: Option<i64>,
}

impl ItemUpdate {
    pub fn new(id: i64) -> Self {
        Self { id, ..Default::default() }
    }
}

impl From<&ItemUpdate> for ItemPatch {
    fn from(u: &ItemUpdate) -> Self {
        Self {
            id: u.id,
            entry_id: u.entry_id,
            url: u.url.clone(),
            name: u.name.clone(),
            scanlator: u.scanlator.clone(),
            done: u.done,
            bookmark: u.bookmark,
            fillermark: u.fillermark,
            progress: u.progress,
            number: u.number,
            source_order: u.source_order,
            date_fetch: u.date_fetch,
            date_upload: u.date_upload,
            version: u.version,
            total_seconds: u.total_seconds,
            summary: u.summary.clone(),
            preview_url: u.preview_url.clone(),
            memo: u.memo.clone(),
            page_count: u.page_count,
        }
    }
}

/// Identity of an item as Kotlin's `ChapterImpl.equals` / `EpisodeImpl.equals` define it: the pair (id, url).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ItemKey {
    pub id: i64,
    pub url: String,
}

impl From<&Item> for ItemKey {
    fn from(i: &Item) -> Self {
        Self { id: i.id, url: i.url.clone() }
    }
}

/// An item's kind must be its entry's. A missing entry is left to the foreign key.
fn check_kinds(c: &Connection, new: &[Item]) -> DataResult<()> {
    let mut stored: HashMap<i64, Option<String>> = HashMap::new();
    for item in new {
        if !stored.contains_key(&item.entry_id) {
            let kind = c.query_row("SELECT kind FROM entries WHERE _id = ?1", [item.entry_id], |r| r.get(0)).optional()?;
            stored.insert(item.entry_id, kind);
        }
        if let Some(Some(kind)) = stored.get(&item.entry_id) {
            if kind != item.kind.as_str() {
                return Err(DataError::Constraint(format!("item {:?} is {} but entry {} is {kind}", item.url, item.kind, item.entry_id)));
            }
        }
    }
    Ok(())
}

/// Item access for both kinds.
#[derive(Clone)]
pub struct ItemRepo {
    db: Arc<Db>,
}

impl ItemRepo {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// The row with `id`, or `None`.
    pub fn by_id(&self, id: i64) -> DataResult<Option<Item>> {
        Ok(self.db.read(|c| none_if_not_found(t::by_id(c, id)))?.map(Item::from))
    }

    /// Every item of the kind with this url, in any entry, lowest id first.
    pub fn by_url(&self, kind: Kind, url: &str) -> DataResult<Vec<Item>> {
        let rows = self.db.read(|c| Ok(t::by_url(c, kind, url)?))?;
        Ok(rows.into_iter().map(Item::from).collect())
    }

    /// The lowest-id row with this url in the entry, or `None`. A duplicate url logs a warning.
    pub fn by_url_and_entry_id(&self, url: &str, entry_id: i64) -> DataResult<Option<Item>> {
        let rows = self.db.read(|c| Ok(t::by_url_and_entry(c, url, entry_id)?))?;
        if rows.len() > 1 {
            tracing::warn!(entry_id, url, count = rows.len(), "duplicate item url in one entry; using the lowest id");
        }
        Ok(rows.into_iter().next().map(Item::from))
    }

    /// Items of an entry. With `apply_scanlator_filter`, items of excluded scanlators are dropped. No order is guaranteed.
    pub fn by_entry_id(&self, entry_id: i64, apply_scanlator_filter: bool) -> DataResult<Vec<Item>> {
        let rows = self.db.read(|c| Ok(t::by_entry(c, entry_id, apply_scanlator_filter)?))?;
        Ok(rows.into_iter().map(Item::from).collect())
    }

    pub fn bookmarked(&self, entry_id: i64) -> DataResult<Vec<Item>> {
        let rows = self.db.read(|c| Ok(t::bookmarked(c, entry_id)?))?;
        Ok(rows.into_iter().map(Item::from).collect())
    }

    /// The scanlators of an entry's items, one per item, a missing one as `""`.
    pub fn scanlators(&self, entry_id: i64) -> DataResult<Vec<String>> {
        self.db.read(|c| Ok(t::scanlators(c, entry_id)?))
    }

    /// The item list of an entry, published again after a change to the items or to the exclusions.
    pub fn subscribe_by_entry_id(&self, entry_id: i64, apply_scanlator_filter: bool) -> Subscription<Vec<Item>> {
        self.db.subscribe(ITEM_TABLES, ENTRY_DEBOUNCE, move |c| {
            Ok(t::by_entry(c, entry_id, apply_scanlator_filter)?.into_iter().map(Item::from).collect::<Vec<_>>())
        })
    }

    /// Inserts the items in one transaction, in order. Returns them with their new ids, in the same order. A failure
    /// on any row (a missing entry, a kind that is not the entry's) leaves the table unchanged.
    pub fn add_all(&self, new: &[Item]) -> DataResult<Vec<Item>> {
        let rows: Vec<ItemRow> = new.iter().map(ItemRow::from).collect();
        let ids = self.db.write(|tx| {
            check_kinds(tx, new)?;
            Ok(t::insert_all(tx, &rows)?)
        })?;
        Ok(new.iter().zip(ids).map(|(i, id)| Item { id, ..i.clone() }).collect())
    }

    /// Deletes the items with these ids, in chunks of [`ID_CHUNK`], in one transaction.
    pub fn remove_with_ids(&self, ids: &[i64]) -> DataResult<()> {
        self.db.write(|tx| {
            for chunk in ids.chunks(ID_CHUNK) {
                t::remove_with_ids(tx, chunk)?;
            }
            Ok(())
        })
    }

    /// Updates one item. A missing id matches no row and is not an error.
    pub fn update(&self, u: &ItemUpdate) -> DataResult<()> {
        let patch = ItemPatch::from(u);
        self.db.write(|tx| Ok(t::update(tx, &patch).map(drop)?))
    }

    /// Every update in one transaction: one failure rolls back all of them.
    pub fn update_all(&self, updates: &[ItemUpdate]) -> DataResult<()> {
        self.db.write(|tx| {
            for u in updates {
                t::update(tx, &ItemPatch::from(u))?;
            }
            Ok(())
        })
    }
}
