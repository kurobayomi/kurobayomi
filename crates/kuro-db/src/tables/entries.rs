//! `entries` statements for both kinds. Flag writes are one masked statement and bump `version` (decision 11a).

use kuro_platform::kind::Kind;
use rusqlite::{named_params, Connection, Result, Transaction};

use super::{entry_cols, exec, query_all, query_opt, rows::*, Params};
use crate::codec::ids_json;
use crate::error::{DbError, DbResult};
use crate::flags::item::MANGA_MASK;
use crate::codec::FetchType;

/// Which flag word of an entry a flag statement touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagColumn {
    /// `item_flags`, the item layout of `kuro_db::flags::item`. A manga row only carries the manga groups.
    ItemFlags,
    /// `season_flags` (anime only: a manga row is never written).
    SeasonFlags,
}

const BY_ID: &str = concat!("SELECT ", entry_cols!(""), " FROM entries WHERE _id = :id");
const BY_URL_AND_SOURCE: &str =
    concat!("SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND url = :url AND source = :source LIMIT 1");
const FAVORITES: &str = concat!("SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND favorite = 1");
const FAVORITES_BY_SOURCE: &str =
    concat!("SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND favorite = 1 AND source = :source");
const BY_SOURCE: &str = concat!("SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND source = :source");
const WATCHED_NOT_IN_LIBRARY: &str = concat!(
    "SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND favorite = 0 AND _id IN (SELECT DISTINCT entry_id FROM items WHERE done = 1 OR progress != 0)"
);
const DUPLICATES: &str =
    concat!("SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND favorite = 1 AND kuro_lower(title) = :title AND _id != :id");
const UPCOMING: &str = concat!(
    "SELECT ", entry_cols!(""), " FROM entries WHERE kind = :kind AND next_update >= :startOfDay AND next_update > 0 AND favorite = 1 AND status IN (SELECT value FROM json_each(:statuses)) ORDER BY next_update ASC"
);
const CHILDREN_OF: &str = concat!("SELECT ", entry_cols!(""), " FROM entries WHERE parent_id = :parentId");
const INSERT: &str = "INSERT INTO entries(kind, source, url, title, artist, author, description, genre, status, thumbnail_url, favorite, last_update, next_update, initialized, viewer, item_flags, cover_last_modified, date_added, update_strategy, calculate_interval, last_modified_at, version, fetch_type, parent_id, season_flags, season_number, season_source_order, background_url, background_last_modified, memo)
VALUES (:kind, :source, :url, :title, :artist, :author, :description, :genre, :status, :thumbnailUrl, :favorite, :lastUpdate, :nextUpdate, :initialized, :viewer, :itemFlags, :coverLastModified, :dateAdded, :updateStrategy, :calculateInterval, 0, :version, :fetchType, :parentId, :seasonFlags, :seasonNumber, :seasonSourceOrder, :backgroundUrl, :backgroundLastModified, :memo)";
const UPDATE: &str = "UPDATE entries SET
    source = coalesce(:source, source), url = coalesce(:url, url), title = coalesce(:title, title),
    artist = coalesce(:artist, artist), author = coalesce(:author, author), description = coalesce(:description, description),
    genre = coalesce(:genre, genre), status = coalesce(:status, status), thumbnail_url = coalesce(:thumbnailUrl, thumbnail_url),
    favorite = coalesce(:favorite, favorite), last_update = coalesce(:lastUpdate, last_update),
    next_update = coalesce(:nextUpdate, next_update), initialized = coalesce(:initialized, initialized),
    viewer = coalesce(:viewer, viewer), item_flags = coalesce(:itemFlags, item_flags),
    cover_last_modified = coalesce(:coverLastModified, cover_last_modified), date_added = coalesce(:dateAdded, date_added),
    update_strategy = coalesce(:updateStrategy, update_strategy), calculate_interval = coalesce(:calculateInterval, calculate_interval),
    version = coalesce(:version, version), fetch_type = coalesce(:fetchType, fetch_type), parent_id = coalesce(:parentId, parent_id),
    season_flags = coalesce(:seasonFlags, season_flags), season_number = coalesce(:seasonNumber, season_number),
    season_source_order = coalesce(:seasonSourceOrder, season_source_order), background_url = coalesce(:backgroundUrl, background_url),
    background_last_modified = coalesce(:backgroundLastModified, background_last_modified), memo = coalesce(:memo, memo)
WHERE _id = :id";
/// `viewer` bits of `mask` replaced by those of `value`, in one statement (the skip-intro and airing groups, the reader
/// modes, never overwrite each other).
const SET_VIEWER_BITS: &str =
    "UPDATE entries SET viewer = (viewer & ~:mask) | (:value & :mask), version = version + 1 WHERE _id = :id";
const RESET_VIEWER: &str = "UPDATE entries SET viewer = 0 WHERE kind = :kind";
const READ_ITEM_FLAGS: &str = "SELECT item_flags FROM entries WHERE _id = :id";
const READ_SEASON_FLAGS: &str = "SELECT season_flags FROM entries WHERE _id = :id AND kind = 'anime'";
/// A manga row only carries the manga groups, so its mask is ANDed with `:mangaMask` (anime-only bits are ignored).
const ITEM_MASKED: &str = "UPDATE entries SET
    item_flags = (item_flags & ~(CASE WHEN kind = 'manga' THEN :mask & :mangaMask ELSE :mask END))
                 | (:flag & (CASE WHEN kind = 'manga' THEN :mask & :mangaMask ELSE :mask END)),
    version = version + 1
WHERE _id = :id";
const SEASON_MASKED: &str =
    "UPDATE entries SET season_flags = (season_flags & ~:mask) | (:flag & :mask), version = version + 1 WHERE _id = :id AND kind = 'anime'";
const ITEM_WRITE: &str = "UPDATE entries SET item_flags = CASE WHEN kind = 'manga' THEN :word & :mangaMask ELSE :word END,
    version = version + 1 WHERE _id = :id";
const SEASON_WRITE: &str = "UPDATE entries SET season_flags = :word, version = version + 1 WHERE _id = :id AND kind = 'anime'";

pub fn by_id(c: &Connection, id: i64) -> DbResult<EntryRow> {
    let p: Params<'_> = &[(":id", &id)];
    query_opt(c, BY_ID, p, EntryRow::from_row)?.ok_or(DbError::NotFound)
}

pub fn by_url_and_source(c: &Connection, kind: Kind, url: &str, source: i64) -> Result<Option<EntryRow>> {
    query_opt(c, BY_URL_AND_SOURCE, &[(":kind", &kind.as_str()), (":url", &url), (":source", &source)], EntryRow::from_row)
}

pub fn favorites(c: &Connection, kind: Kind) -> Result<Vec<EntryRow>> {
    query_all(c, FAVORITES, &[(":kind", &kind.as_str())], EntryRow::from_row)
}

pub fn favorites_by_source(c: &Connection, kind: Kind, source: i64) -> Result<Vec<EntryRow>> {
    query_all(c, FAVORITES_BY_SOURCE, &[(":kind", &kind.as_str()), (":source", &source)], EntryRow::from_row)
}

/// Every entry of a source, in the library or not (a title's downloads outlive its place in the library).
pub fn by_source(c: &Connection, kind: Kind, source: i64) -> Result<Vec<EntryRow>> {
    query_all(c, BY_SOURCE, &[(":kind", &kind.as_str()), (":source", &source)], EntryRow::from_row)
}

/// Non-favourites with at least one done item or some progress.
pub fn watched_not_in_library(c: &Connection, kind: Kind) -> Result<Vec<EntryRow>> {
    query_all(c, WATCHED_NOT_IN_LIBRARY, &[(":kind", &kind.as_str())], EntryRow::from_row)
}

pub fn all_source_and_url(c: &Connection, kind: Kind) -> Result<Vec<(i64, String)>> {
    let mut st = c.prepare_cached("SELECT source, url FROM entries WHERE kind = :kind")?;
    st.query_map(named_params! {":kind": kind.as_str()}, |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// Inserts `e` and returns its id. `id`, `last_modified_at` (written 0) and `favorite_modified_at` are not bound.
pub fn insert(c: &Connection, e: &EntryRow) -> Result<i64> {
    exec(
        c,
        INSERT,
        &[
            (":kind", &e.kind.as_str()),
            (":source", &e.source),
            (":url", &e.url),
            (":title", &e.title),
            (":artist", &e.artist),
            (":author", &e.author),
            (":description", &e.description),
            (":genre", &e.genre),
            (":status", &e.status),
            (":thumbnailUrl", &e.thumbnail_url),
            (":favorite", &e.favorite),
            (":lastUpdate", &e.last_update),
            (":nextUpdate", &e.next_update),
            (":initialized", &e.initialized),
            (":viewer", &e.viewer),
            (":itemFlags", &e.item_flags),
            (":coverLastModified", &e.cover_last_modified),
            (":dateAdded", &e.date_added),
            (":updateStrategy", &e.update_strategy),
            (":calculateInterval", &e.calculate_interval),
            (":version", &e.version),
            (":fetchType", &e.fetch_type),
            (":parentId", &e.parent_id),
            (":seasonFlags", &e.season_flags),
            (":seasonNumber", &e.season_number),
            (":seasonSourceOrder", &e.season_source_order),
            (":backgroundUrl", &e.background_url),
            (":backgroundLastModified", &e.background_last_modified),
            (":memo", &e.memo),
        ],
    )?;
    Ok(c.last_insert_rowid())
}

/// Coalesce update: every `None` keeps the stored column. Returns the rows changed.
pub fn update(c: &Connection, p: &EntryPatch) -> Result<usize> {
    exec(
        c,
        UPDATE,
        &[
            (":source", &p.source),
            (":url", &p.url),
            (":title", &p.title),
            (":artist", &p.artist),
            (":author", &p.author),
            (":description", &p.description),
            (":genre", &p.genre),
            (":status", &p.status),
            (":thumbnailUrl", &p.thumbnail_url),
            (":favorite", &p.favorite),
            (":lastUpdate", &p.last_update),
            (":nextUpdate", &p.next_update),
            (":initialized", &p.initialized),
            (":viewer", &p.viewer),
            (":itemFlags", &p.item_flags),
            (":coverLastModified", &p.cover_last_modified),
            (":dateAdded", &p.date_added),
            (":updateStrategy", &p.update_strategy),
            (":calculateInterval", &p.calculate_interval),
            (":version", &p.version),
            (":fetchType", &p.fetch_type),
            (":parentId", &p.parent_id),
            (":seasonFlags", &p.season_flags),
            (":seasonNumber", &p.season_number),
            (":seasonSourceOrder", &p.season_source_order),
            (":backgroundUrl", &p.background_url),
            (":backgroundLastModified", &p.background_last_modified),
            (":memo", &p.memo),
            (":id", &p.id),
        ],
    )
}

/// Writes `value` under `mask` into `viewer` of one entry and bumps `version`. Returns the rows changed (0 for a
/// deleted entry).
pub fn set_viewer_bits(c: &Connection, id: i64, value: i64, mask: i64) -> Result<usize> {
    exec(c, SET_VIEWER_BITS, &[(":id", &id), (":value", &value), (":mask", &mask)])
}

/// `viewer = 0` on every entry of the kind.
pub fn reset_viewer(c: &Connection, kind: Kind) -> Result<usize> {
    exec(c, RESET_VIEWER, &[(":kind", &kind.as_str())])
}

/// One flag word, `None` for a missing row (and for `SeasonFlags` of a manga row).
pub fn read_flags(c: &Connection, col: FlagColumn, id: i64) -> Result<Option<i64>> {
    let sql = match col {
        FlagColumn::ItemFlags => READ_ITEM_FLAGS,
        FlagColumn::SeasonFlags => READ_SEASON_FLAGS,
    };
    Ok(query_opt(c, sql, &[(":id", &id)], |r| r.get::<_, Option<i64>>(0))?.flatten())
}

/// Replaces the bits of `mask` by those of `flag` in one statement and bumps `version` (both kinds). For a manga row
/// and `ItemFlags` the mask is ANDed with [`MANGA_MASK`] in SQL, so an anime-only group is ignored. `SeasonFlags`
/// changes anime rows only.
pub fn set_flags_masked(c: &Connection, col: FlagColumn, id: i64, flag: i64, mask: i64) -> Result<usize> {
    match col {
        FlagColumn::ItemFlags => exec(
            c,
            ITEM_MASKED,
            &[(":id", &id), (":flag", &flag), (":mask", &mask), (":mangaMask", &MANGA_MASK)],
        ),
        FlagColumn::SeasonFlags => exec(c, SEASON_MASKED, &[(":id", &id), (":flag", &flag), (":mask", &mask)]),
    }
}

/// Stores a whole flag word and bumps `version`. A manga row keeps only the manga groups of `word`.
pub fn write_flags(c: &Connection, col: FlagColumn, id: i64, word: i64) -> Result<usize> {
    match col {
        FlagColumn::ItemFlags => exec(c, ITEM_WRITE, &[(":id", &id), (":word", &word), (":mangaMask", &MANGA_MASK)]),
        FlagColumn::SeasonFlags => exec(c, SEASON_WRITE, &[(":id", &id), (":word", &word)]),
    }
}

/// The other library entries of the kind whose title, lowercased with `kuro_lower`, equals `title_lower`.
pub fn duplicates(c: &Connection, kind: Kind, id: i64, title_lower: &str) -> Result<Vec<EntryRow>> {
    let title = title_lower.to_lowercase();
    query_all(c, DUPLICATES, &[(":kind", &kind.as_str()), (":id", &id), (":title", &title)], EntryRow::from_row)
}

/// Favourites with `next_update >= start_of_day_ms` and a status in `statuses`, soonest first. An empty list matches
/// nothing.
pub fn upcoming(c: &Connection, kind: Kind, start_of_day_ms: i64, statuses: &[i64]) -> Result<Vec<EntryRow>> {
    if statuses.is_empty() {
        return Ok(Vec::new());
    }
    let json = ids_json(statuses);
    query_all(
        c,
        UPCOMING,
        &[(":kind", &kind.as_str()), (":startOfDay", &start_of_day_ms), (":statuses", &json)],
        EntryRow::from_row,
    )
}

/// `(source, favourite count)` of the kind, ordered by source.
pub fn favorite_counts(c: &Connection, kind: Kind) -> Result<Vec<(i64, i64)>> {
    let mut st = c.prepare_cached(
        "SELECT source, count(*) FROM entries WHERE kind = :kind AND favorite = 1 GROUP BY source ORDER BY source",
    )?;
    st.query_map(named_params! {":kind": kind.as_str()}, |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// `(source, non-favourite count)` of the kind, ordered by source.
pub fn non_library_counts(c: &Connection, kind: Kind) -> Result<Vec<(i64, i64)>> {
    let mut st = c.prepare_cached(
        "SELECT source, count(*) FROM entries WHERE kind = :kind AND favorite = 0 GROUP BY source ORDER BY source",
    )?;
    st.query_map(named_params! {":kind": kind.as_str()}, |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// Clears the non-library entries of `source_ids` inside the caller's transaction: deletes the non-favourites (items,
/// tracks and links go by cascade), orphans the favourites whose parent went (`parent_id` NULL, no foreign key),
/// then deletes the kind's reset history. Returns the history rows removed. An empty list changes nothing.
pub fn clear_non_library(tx: &Transaction<'_>, kind: Kind, source_ids: &[i64]) -> Result<usize> {
    if source_ids.is_empty() {
        return Ok(0);
    }
    let sources = ids_json(source_ids);
    let ids: Vec<i64> = {
        let mut st = tx.prepare(
            "SELECT _id FROM entries WHERE kind = :kind AND favorite = 0 AND source IN (SELECT value FROM json_each(:sources))",
        )?;
        let rows = st.query_map(named_params! {":kind": kind.as_str(), ":sources": sources}, |r| r.get(0))?;
        rows.collect::<Result<_>>()?
    };
    let json = ids_json(&ids);
    tx.execute("DELETE FROM entries WHERE favorite = 0 AND _id IN (SELECT value FROM json_each(?1))", [&json])?;
    tx.execute(
        "UPDATE entries SET parent_id = NULL WHERE kind = ?1 AND favorite = 1 AND parent_id IN (SELECT value FROM json_each(?2))",
        [kind.as_str(), json.as_str()],
    )?;
    super::history::remove_resetted(tx, kind)
}

/// The clear screen's tree path: deletes the non-favourites in `ids`, orphans the `orphaned` ids that were not
/// deleted, then deletes the reset history of the kind(s) of the deleted entries.
pub fn clear_ids(tx: &Transaction<'_>, ids: &[i64], orphaned: &[i64]) -> Result<()> {
    let json = ids_json(ids);
    let kinds: Vec<String> = {
        let mut st =
            tx.prepare("SELECT DISTINCT kind FROM entries WHERE favorite = 0 AND _id IN (SELECT value FROM json_each(?1))")?;
        let rows = st.query_map([&json], |r| r.get(0))?;
        rows.collect::<Result<_>>()?
    };
    tx.execute("DELETE FROM entries WHERE favorite = 0 AND _id IN (SELECT value FROM json_each(?1))", [&json])?;
    let kept: Vec<i64> = orphaned.iter().copied().filter(|id| !ids.contains(id)).collect();
    remove_parent_ids(tx, &kept)?;
    for kind in kinds.iter().filter_map(|k| Kind::parse(k)) {
        super::history::remove_resetted(tx, kind)?;
    }
    Ok(())
}

/// The seasons of one parent, in `_id` order: the two stats CTEs of the library view over the children of `:parentId`
/// and their children. A browsed parent that is not a favourite has seasons too, which the view's favourites prefilter
/// would zero out, so this is a statement and not a view.
const SEASONS_OF: &str = concat!(
    "WITH scope(entry_id) AS (SELECT _id FROM entries WHERE parent_id = :parentId UNION SELECT _id FROM entries WHERE parent_id IN (SELECT _id FROM entries WHERE parent_id = :parentId)), ",
    "item_stats AS (SELECT i.entry_id, count(*) AS total, sum(i.done) AS doneCount, ",
    "coalesce(max(i.date_upload), 0) AS latestUpload, coalesce(max(i.date_fetch), 0) AS fetchedAt, ",
    "sum(i.bookmark) AS bookmarkCount, sum(i.fillermark) AS fillermarkCount, coalesce(max(h.last_at), 0) AS lastActivity ",
    "FROM items i LEFT JOIN excluded_scanlators x ON x.entry_id = i.entry_id AND x.scanlator = i.scanlator ",
    "LEFT JOIN history h ON h.item_id = i._id ",
    "WHERE x.scanlator IS NULL AND i.entry_id IN (SELECT entry_id FROM scope) GROUP BY i.entry_id), ",
    "season_stats AS (SELECT s.parent_id, count(*) AS child_count, ",
    "sum(CASE WHEN s.fetch_type = 1 AND coalesce(t.total, 0) = coalesce(t.doneCount, 0) THEN 1 ELSE 0 END) AS fully_done, ",
    "max(coalesce(t.latestUpload, 0)) AS latestUpload, max(coalesce(t.fetchedAt, 0)) AS fetchedAt, ",
    "max(coalesce(t.lastActivity, 0)) AS lastActivity, sum(coalesce(t.bookmarkCount, 0)) AS bookmarkCount, ",
    "sum(coalesce(t.fillermarkCount, 0)) AS fillermarkCount ",
    "FROM entries s LEFT JOIN item_stats t ON t.entry_id = s._id ",
    "WHERE s.parent_id IN (SELECT _id FROM entries WHERE parent_id = :parentId) GROUP BY s.parent_id) ",
    "SELECT ", entry_cols!("e."), ", ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.child_count, 0) ELSE coalesce(st.total, 0) END AS totalCount, ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.fully_done, 0) ELSE coalesce(st.doneCount, 0) END AS doneCount, ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.latestUpload, 0) ELSE coalesce(st.latestUpload, 0) END AS latestUpload, ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.fetchedAt, 0) ELSE coalesce(st.fetchedAt, 0) END AS fetchedAt, ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.lastActivity, 0) ELSE coalesce(st.lastActivity, 0) END AS lastActivity, ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.bookmarkCount, 0) ELSE coalesce(st.bookmarkCount, 0) END AS bookmarkCount, ",
    "CASE WHEN e.fetch_type = 0 THEN coalesce(ss.fillermarkCount, 0) ELSE coalesce(st.fillermarkCount, 0) END AS fillermarkCount ",
    "FROM entries e LEFT JOIN item_stats st ON st.entry_id = e._id LEFT JOIN season_stats ss ON ss.parent_id = e._id ",
    "WHERE e.parent_id = :parentId ORDER BY e._id"
);

pub fn seasons_of(c: &Connection, parent_id: i64) -> Result<Vec<SeasonRow>> {
    query_all(c, SEASONS_OF, &[(":parentId", &parent_id)], |r| {
        Ok(SeasonRow {
            entry: EntryRow::from_row(r)?,
            total: r.get("totalCount")?,
            done: count_col(r, "doneCount")?,
            bookmarks: count_col(r, "bookmarkCount")?,
            fillermarks: count_col(r, "fillermarkCount")?,
            latest_upload: r.get("latestUpload")?,
            fetched_at: r.get("fetchedAt")?,
            last_activity: r.get("lastActivity")?,
        })
    })
}

/// Every row whose `parent_id` is `parent_id`, favourite or not.
pub fn children_of(c: &Connection, parent_id: i64) -> Result<Vec<EntryRow>> {
    query_all(c, CHILDREN_OF, &[(":parentId", &parent_id)], EntryRow::from_row)
}

/// Non-favourite top-level entries (the candidates of the clear screen).
pub fn deletable_parents(c: &Connection) -> Result<Vec<DeletableRow>> {
    query_all(
        c,
        "SELECT _id, source, kind, coalesce(fetch_type, 1) AS fetch_type FROM entries WHERE favorite = 0 AND parent_id IS NULL",
        &[],
        |r| {
            Ok(DeletableRow {
                entry_id: r.get("_id")?,
                source_id: r.get("source")?,
                kind: super::kind_col(r, "kind")?,
                fetch_type: r.get("fetch_type")?,
            })
        },
    )
}

/// The ids a clear removes for one non-library entry `root`, and the ids it orphans. The root is always removed. Below
/// it a favourite child is orphaned, a non-favourite child is removed, and a non-favourite child that is a container
/// of seasons is searched for its own children.
pub fn clearable_tree(c: &Connection, root: i64) -> Result<(Vec<i64>, Vec<i64>)> {
    let (children, orphaned) = clearable_children(c, root)?;
    let mut ids = vec![root];
    ids.extend(children);
    Ok((ids, orphaned))
}

fn clearable_children(c: &Connection, parent: i64) -> Result<(Vec<i64>, Vec<i64>)> {
    let (mut ids, mut orphaned) = (Vec::new(), Vec::new());
    for child in children_of(c, parent)? {
        if child.favorite {
            orphaned.push(child.id);
            continue;
        }
        ids.push(child.id);
        if child.fetch_type == Some(FetchType::Seasons) {
            let (sub_ids, sub_orphaned) = clearable_children(c, child.id)?;
            ids.extend(sub_ids);
            orphaned.extend(sub_orphaned);
        }
    }
    Ok((ids, orphaned))
}

/// Clears `parent_id` on the entries named by `ids` (`parent_id` has no foreign key, so a deleted parent leaves it).
pub fn remove_parent_ids(c: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    c.execute("UPDATE entries SET parent_id = NULL WHERE _id IN (SELECT value FROM json_each(?1))", [ids_json(ids)])
}

/// Replaces the category links of one entry (the kind comes from the entry row). A repeated id is linked once; each
/// inserted link bumps `entries.version` through its trigger. Ids the kind does not have are refused by the foreign key.
pub fn set_categories(tx: &Transaction<'_>, entry_id: i64, ids: &[i64]) -> Result<()> {
    tx.execute("DELETE FROM entry_categories WHERE entry_id = ?1", [entry_id])?;
    let mut unique = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut st = tx.prepare_cached(
        "INSERT OR IGNORE INTO entry_categories(entry_id, kind, category_id) SELECT _id, kind, ?2 FROM entries WHERE _id = ?1",
    )?;
    for id in unique {
        st.execute((entry_id, id))?;
    }
    Ok(())
}
