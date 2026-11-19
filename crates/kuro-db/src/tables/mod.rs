//! The unified SQL statements (docs/design/unified-model.md section 1.2): one function per statement, `kind` is a
//! parameter, rows hold the stored layout (no translation, no old-name aliases). Plain functions over
//! `&Connection` (a `&Transaction` derefs to one).

use kuro_platform::kind::Kind;
use rusqlite::types::{ToSql, Type};
use rusqlite::{Connection, OptionalExtension, Result, Row};

pub mod categories;
pub mod community_scores;
pub mod downloads;
pub mod entries;
pub mod excluded;
pub mod history;
pub mod items;
pub mod library;
pub mod rows;
pub mod stub_sources;
pub mod tracks;
pub mod updates;

pub use rows::*;

/// Named parameters of one statement, as rusqlite takes them.
pub(crate) type Params<'a> = &'a [(&'a str, &'a dyn ToSql)];

/// The `kind` column as a [`Kind`]. `Kind` lives in the lowest crate, which has no rusqlite, so it has no `FromSql`.
pub(crate) fn kind_col(r: &Row<'_>, name: &str) -> Result<Kind> {
    let text: String = r.get(name)?;
    Kind::parse(&text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(0, Type::Text, format!("unknown kind {text:?}").into())
    })
}

pub(crate) fn query_all<T>(c: &Connection, sql: &str, p: Params<'_>, f: fn(&Row<'_>) -> Result<T>) -> Result<Vec<T>> {
    c.prepare_cached(sql)?.query_map(p, f)?.collect()
}

pub(crate) fn query_opt<T>(c: &Connection, sql: &str, p: Params<'_>, f: fn(&Row<'_>) -> Result<T>) -> Result<Option<T>> {
    c.prepare_cached(sql)?.query_row(p, f).optional()
}

pub(crate) fn exec(c: &Connection, sql: &str, p: Params<'_>) -> Result<usize> {
    c.prepare_cached(sql)?.execute(p)
}

/// The `entries` columns, no aliases (`$p` is a table alias with its dot, or empty).
macro_rules! entry_cols {
    ($p:literal) => {
        concat!(
            $p, "_id, ", $p, "kind, ", $p, "source, ", $p, "url, ", $p, "title, ", $p, "artist, ", $p, "author, ",
            $p, "description, ", $p, "genre, ", $p, "status, ", $p, "thumbnail_url, ", $p, "favorite, ",
            $p, "last_update, ", $p, "next_update, ", $p, "initialized, ", $p, "viewer, ", $p, "item_flags, ",
            $p, "cover_last_modified, ", $p, "date_added, ", $p, "update_strategy, ", $p, "calculate_interval, ",
            $p, "last_modified_at, ", $p, "favorite_modified_at, ", $p, "version, ", $p, "fetch_type, ",
            $p, "parent_id, ", $p, "season_flags, ", $p, "season_number, ", $p, "season_source_order, ",
            $p, "background_url, ", $p, "background_last_modified, ", $p, "memo"
        )
    };
}
pub(crate) use entry_cols;

/// The `items` columns plus the joined `kind` of the entry; use with `FROM items i JOIN entries e ON e._id = i.entry_id`.
macro_rules! item_cols {
    () => {
        "i._id, i.entry_id, e.kind, i.url, i.name, i.scanlator, i.done, i.bookmark, i.fillermark, i.progress, i.number, \
         i.source_order, i.date_fetch, i.date_upload, i.last_modified_at, i.version, i.total_seconds, i.summary, \
         i.preview_url, i.memo"
    };
}
pub(crate) use item_cols;

/// Escapes `\`, `%` and `_` so the user's text matches literally under `ESCAPE '\'`.
pub(crate) fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
