//! `items` statements for both kinds (chapters and episodes). The item's `kind` is the joined entry's.

use kuro_platform::kind::Kind;
use rusqlite::{Connection, Result, Transaction};

use super::{exec, item_cols, query_all, query_opt, rows::*};
use crate::codec::ids_json;
use crate::error::{DbError, DbResult};

const BY_ID: &str = concat!("SELECT ", item_cols!(), " FROM items i JOIN entries e ON e._id = i.entry_id WHERE i._id = :id");
const BY_ENTRY: &str = concat!(
    "SELECT ", item_cols!(), " FROM items i JOIN entries e ON e._id = i.entry_id LEFT JOIN excluded_scanlators ES ON i.entry_id = ES.entry_id AND i.scanlator = ES.scanlator WHERE i.entry_id = :entryId AND (:applyScanlatorFilter = 0 OR ES.scanlator IS NULL)"
);
const BOOKMARKED: &str =
    concat!("SELECT ", item_cols!(), " FROM items i JOIN entries e ON e._id = i.entry_id WHERE i.bookmark AND i.entry_id = :entryId");
const BY_URL: &str = concat!("SELECT ", item_cols!(), " FROM items i JOIN entries e ON e._id = i.entry_id WHERE e.kind = :kind AND i.url = :url ORDER BY i._id");
/// Lowest id first: with a duplicate url the first row wins, for both kinds.
const BY_URL_AND_ENTRY: &str =
    concat!("SELECT ", item_cols!(), " FROM items i JOIN entries e ON e._id = i.entry_id WHERE i.url = :url AND i.entry_id = :entryId ORDER BY i._id");
const INSERT: &str = "INSERT INTO items(entry_id, url, name, scanlator, done, bookmark, progress, total_seconds, number, source_order, date_fetch, date_upload, last_modified_at, version, summary, preview_url, fillermark, memo, page_count)
VALUES (:entryId, :url, :name, :scanlator, :done, :bookmark, :progress, :totalSeconds, :number, :sourceOrder, :dateFetch, :dateUpload, 0, :version, :summary, :previewUrl, :fillermark, :memo, :pageCount)";
const UPDATE: &str = "UPDATE items SET
    entry_id = coalesce(:entryId, entry_id), url = coalesce(:url, url), name = coalesce(:name, name),
    scanlator = coalesce(:scanlator, scanlator), done = coalesce(:done, done), bookmark = coalesce(:bookmark, bookmark),
    progress = coalesce(:progress, progress), total_seconds = coalesce(:totalSeconds, total_seconds),
    number = coalesce(:number, number), source_order = coalesce(:sourceOrder, source_order),
    date_fetch = coalesce(:dateFetch, date_fetch), date_upload = coalesce(:dateUpload, date_upload),
    version = coalesce(:version, version), summary = coalesce(:summary, summary),
    preview_url = coalesce(:previewUrl, preview_url), fillermark = coalesce(:fillermark, fillermark),
    memo = coalesce(:memo, memo), page_count = coalesce(:pageCount, page_count)
WHERE _id = :id";

pub fn by_id(c: &Connection, id: i64) -> DbResult<ItemRow> {
    query_opt(c, BY_ID, &[(":id", &id)], ItemRow::from_row)?.ok_or(DbError::NotFound)
}

/// Items of an entry. With `apply_scanlator_filter` the items whose `(entry, scanlator)` is excluded are dropped; a
/// NULL scanlator is never dropped. No order is guaranteed.
pub fn by_entry(c: &Connection, entry_id: i64, apply_scanlator_filter: bool) -> Result<Vec<ItemRow>> {
    query_all(c, BY_ENTRY, &[(":entryId", &entry_id), (":applyScanlatorFilter", &apply_scanlator_filter)], ItemRow::from_row)
}

pub fn bookmarked(c: &Connection, entry_id: i64) -> Result<Vec<ItemRow>> {
    query_all(c, BOOKMARKED, &[(":entryId", &entry_id)], ItemRow::from_row)
}

/// Every row's scanlator, NOT distinct, NULL as `""`.
pub fn scanlators(c: &Connection, entry_id: i64) -> Result<Vec<String>> {
    query_all(c, "SELECT i.scanlator FROM items i WHERE i.entry_id = :entryId", &[(":entryId", &entry_id)], |r| {
        Ok(r.get::<_, Option<String>>(0)?.unwrap_or_default())
    })
}

/// Every item of the kind with this url, lowest id first.
pub fn by_url(c: &Connection, kind: Kind, url: &str) -> Result<Vec<ItemRow>> {
    query_all(c, BY_URL, &[(":kind", &kind.as_str()), (":url", &url)], ItemRow::from_row)
}

/// The rows with this url inside one entry, lowest id first (the caller takes the first and warns on more).
pub fn by_url_and_entry(c: &Connection, url: &str, entry_id: i64) -> Result<Vec<ItemRow>> {
    query_all(c, BY_URL_AND_ENTRY, &[(":url", &url), (":entryId", &entry_id)], ItemRow::from_row)
}

/// Inserts `r` and returns its id. `id`, `kind` and `last_modified_at` are not bound.
pub fn insert(c: &Connection, r: &ItemRow) -> Result<i64> {
    exec(
        c,
        INSERT,
        &[
            (":entryId", &r.entry_id),
            (":url", &r.url),
            (":name", &r.name),
            (":scanlator", &r.scanlator),
            (":done", &r.done),
            (":bookmark", &r.bookmark),
            (":progress", &r.progress),
            (":totalSeconds", &r.total_seconds),
            (":number", &r.number),
            (":sourceOrder", &r.source_order),
            (":dateFetch", &r.date_fetch),
            (":dateUpload", &r.date_upload),
            (":version", &r.version),
            (":summary", &r.summary),
            (":previewUrl", &r.preview_url),
            (":fillermark", &r.fillermark),
            (":memo", &r.memo),
            (":pageCount", &r.page_count),
        ],
    )?;
    Ok(c.last_insert_rowid())
}

/// Inserts every row with one prepared statement; the caller owns the transaction, so an error rolls the batch back.
/// Returns the new ids in input order.
pub fn insert_all(tx: &Transaction<'_>, rows: &[ItemRow]) -> Result<Vec<i64>> {
    rows.iter().map(|r| insert(tx, r)).collect()
}

/// Coalesce update of one item. Returns the rows changed.
pub fn update(c: &Connection, p: &ItemPatch) -> Result<usize> {
    exec(
        c,
        UPDATE,
        &[
            (":entryId", &p.entry_id),
            (":url", &p.url),
            (":name", &p.name),
            (":scanlator", &p.scanlator),
            (":done", &p.done),
            (":bookmark", &p.bookmark),
            (":progress", &p.progress),
            (":totalSeconds", &p.total_seconds),
            (":number", &p.number),
            (":sourceOrder", &p.source_order),
            (":dateFetch", &p.date_fetch),
            (":dateUpload", &p.date_upload),
            (":version", &p.version),
            (":summary", &p.summary),
            (":previewUrl", &p.preview_url),
            (":fillermark", &p.fillermark),
            (":memo", &p.memo),
            (":pageCount", &p.page_count),
            (":id", &p.id),
        ],
    )
}

/// Deletes the items with these ids in one statement. An empty list runs nothing.
pub fn remove_with_ids(c: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    c.execute("DELETE FROM items WHERE _id IN (SELECT value FROM json_each(?1))", [ids_json(ids)])
}

