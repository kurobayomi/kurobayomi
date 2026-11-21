//! The one database file `<data_dir>/kurobayomi.db` (docs/design/unified-db.md section 4): manga and anime in
//! the same tables, told apart by `kind`. Version 1 is the baseline, and `MIGRATIONS` lists the steps after it; the old `manga.db` and `anime.db` are
//! never opened, only listed by [`legacy_files`].

use std::path::{Path, PathBuf};

use kuro_platform::paths::Paths;
use rusqlite::Connection;

use crate::error::{DbError, DbResult};
use crate::notify::TABLES;
use crate::open::{create_baseline, open_and_migrate, register_functions, SchemaBundle};

/// File name of the database inside the data directory.
pub const DB_FILE: &str = "kurobayomi.db";

/// `PRAGMA application_id`: ASCII "KUR2".
pub const APPLICATION_ID: i64 = 0x4B55_5232;

/// The version the baseline script creates.
pub const BASELINE_VERSION: i64 = 1;

/// `PRAGMA user_version` of a fresh file and of an upgraded one: the target of the last migration.
pub const SCHEMA_VERSION: i64 = 3;

/// The steps after the baseline, each to the version it names. 2: `community_scores`. 3: `items.page_count`.
const MIGRATIONS: &[(i64, &str)] = &[(2, include_str!("../sql/unified/migrate_2.sql")), (3, include_str!("../sql/unified/migrate_3.sql"))];

/// The old per-kind files, only for [`legacy_files`].
pub const LEGACY_FILES: [&str; 2] = ["manga.db", "anime.db"];

/// Tables, indexes, triggers and seed rows.
pub const SCHEMA_SQL: &str = include_str!("../sql/unified/schema.sql");

/// The three download tables, with the text the baseline uses. Exported so a store can create just these tables
/// on a plain in-memory connection for its own tests (foreign keys are off there).
pub const DOWNLOAD_DDL: &str = include_str!("../sql/unified/downloads.sql");

/// The three views.
pub const VIEWS_SQL: &str = include_str!("../sql/unified/views.sql");

/// The unified file: the baseline at version 1, then [`MIGRATIONS`]. A fresh file runs both, so it has the shape of an
/// upgraded one.
pub const UNIFIED_SCHEMA: SchemaBundle = SchemaBundle {
    baseline_sql: concat!(
        include_str!("../sql/unified/schema.sql"),
        include_str!("../sql/unified/downloads.sql"),
        include_str!("../sql/unified/views.sql"),
    ),
    baseline_version: BASELINE_VERSION,
    migrations: MIGRATIONS,
    application_id: APPLICATION_ID,
    tables: TABLES,
    refuse_foreign: |found| DbError::ForeignFile { found },
};

/// `<data_dir>/kurobayomi.db`.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DB_FILE)
}

/// `db_path` of the app's platform paths.
pub fn open_in_data_dir(paths: &dyn Paths) -> DbResult<Connection> {
    open_file(&db_path(&paths.data_dir()))
}

/// Opens the unified file at `path`: creates it in an empty file, refuses a newer or foreign one before any
/// write, then runs [`heal`].
pub fn open_file(path: &Path) -> DbResult<Connection> {
    let c = open_and_migrate(path, &UNIFIED_SCHEMA)?;
    heal(&c)?;
    Ok(c)
}

/// The one heal: the two system categories exist (a restore or a user must never leave a kind without its
/// "Default" row).
pub fn heal(c: &Connection) -> DbResult<()> {
    c.execute_batch(
        "INSERT OR IGNORE INTO categories(kind, _id, name, sort, flags) VALUES ('manga', 0, '', -1, 64), ('anime', 0, '', -1, 64);",
    )?;
    Ok(())
}

/// A fresh unified database in memory (tests and the test kit). WAL does not apply to memory databases, so only
/// `foreign_keys` is set.
pub fn open_in_memory() -> DbResult<Connection> {
    let mut c = Connection::open_in_memory()?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    register_functions(&c)?;
    create_baseline(&mut c, &UNIFIED_SCHEMA)?;
    Ok(c)
}

/// The old `manga.db` and `anime.db` in `data_dir` that exist and are not empty. They are never opened.
pub fn legacy_files(data_dir: &Path) -> Vec<PathBuf> {
    LEGACY_FILES
        .iter()
        .map(|name| data_dir.join(name))
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
        .collect()
}

/// Golden text of the object inventory: one line per `sqlite_master` object, sorted by type and name, then one
/// line per column of every table and view in `PRAGMA table_info` order.
pub fn object_dump(c: &Connection) -> rusqlite::Result<String> {
    let objects: Vec<(String, String, String)> = c
        .prepare("SELECT type, name, tbl_name FROM sqlite_master ORDER BY type, name")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = String::new();
    for (kind, name, table) in objects {
        out.push_str(&format!("{kind} {name} on {table}\n"));
        if kind == "table" || kind == "view" {
            let mut st = c.prepare("SELECT cid, name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")?;
            let columns = st
                .query_map([&name], |r| {
                    Ok(format!(
                        "  {}|{}|{}|{}|{}|{}\n",
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "-".into()),
                        r.get::<_, i64>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            columns.iter().for_each(|line| out.push_str(line));
        }
    }
    Ok(out)
}
