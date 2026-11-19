//! Test databases (the unified schema, in memory or in a file). The schema and the statements are `kuro-db`'s; rows
//! are built with `kuro_data::entry::Entry::create(kind)` and `kuro_data::item::Item::create(kind)`.

use std::sync::Arc;

use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use kuro_db::open::BUSY_TIMEOUT;
use kuro_db::schema_unified;
use rusqlite::Connection;

use crate::TempDir;

/// The unified database for tests.
pub struct TestDb;

impl TestDb {
    /// A fresh unified database in memory at `user_version` 1 with every table, index, trigger and view. It has
    /// the foreign keys, busy timeout and synchronous mode of a file database. WAL does not apply to memory
    /// databases, so `journal_mode` stays `memory`.
    pub fn anime() -> Connection {
        let c = schema_unified::open_in_memory().expect("fresh unified schema");
        c.busy_timeout(BUSY_TIMEOUT).expect("busy timeout");
        c.pragma_update(None, "synchronous", "NORMAL")
            .expect("synchronous");
        c
    }

    /// The unified database behind the shared handle `kuro_db::handle::Db`, in one file under `dir`. A `Db` reads
    /// through its own connections, so unlike [`TestDb::anime`] it cannot be a memory database. Keep `dir` alive.
    pub fn handle(dir: &TempDir) -> Arc<Db> {
        Self::at(&schema_unified::db_path(dir.path()))
    }

    fn at(path: &std::path::Path) -> Arc<Db> {
        let writer = schema_unified::open_file(path).expect("unified schema");
        Arc::new(Db::new(path, writer).expect("handle"))
    }
}

/// Inserts a category of `kind` at the end of the order (system row excluded, so the first one is order 0) and returns
/// its id. A fixture helper: the production write path is `CategoryService`.
pub fn insert_category(db: &Db, kind: Kind, name: &str) -> i64 {
    db.write(|tx| {
        let order = kuro_db::tables::categories::all(tx, kind)?.iter().map(|c| c.order).max().map_or(0, |max| max + 1);
        Ok(kuro_db::tables::categories::insert(tx, kind, name, order, 0)?)
    })
    .expect("insert category")
}

#[cfg(test)]
mod tests {
    use kuro_db::schema_unified::{APPLICATION_ID, SCHEMA_VERSION, UNIFIED_SCHEMA};

    use super::*;

    /// The names after each `keyword` at the start of a line, e.g. `CREATE TABLE ` in the schema.
    fn names_after(sql: &str, keyword: &str) -> Vec<String> {
        sql.lines()
            .filter_map(|line| line.strip_prefix(keyword))
            .map(|rest| {
                rest.chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                    .collect()
            })
            .collect()
    }

    /// The unified inventory: 14 tables (13 of the baseline, 1 of the migrations) and 3 views, each present in a fresh
    /// file.
    #[test]
    fn every_table_and_view_of_the_unified_schema_exists() {
        let mut tables = names_after(UNIFIED_SCHEMA.baseline_sql, "CREATE TABLE ");
        assert_eq!(tables.len(), 13, "tables in schema.sql and downloads.sql: {tables:?}");
        for (_, sql) in UNIFIED_SCHEMA.migrations {
            tables.extend(names_after(sql, "CREATE TABLE "));
        }
        let views = names_after(UNIFIED_SCHEMA.baseline_sql, "CREATE VIEW ");
        assert_eq!(tables.len(), 14, "with community_scores of migration 2: {tables:?}");
        assert_eq!(views.len(), 3, "views in views.sql: {views:?}");
        let mut named: Vec<&str> = UNIFIED_SCHEMA.tables.to_vec();
        named.sort_unstable();
        let mut created: Vec<&str> = tables.iter().map(String::as_str).collect();
        created.sort_unstable();
        assert_eq!(named, created, "the notification bit list names the created tables");

        let c = TestDb::anime();
        for (kind, names) in [("table", &tables), ("view", &views)] {
            for name in names {
                let found: i64 = c
                    .query_row(
                        "SELECT count(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
                        [kind, name.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(found, 1, "{kind} {name} is missing");
            }
        }
    }

    #[test]
    fn test_db_has_the_file_pragmas_and_the_fresh_version() {
        let c = TestDb::anime();
        let pragma =
            |name: &str| -> i64 { c.pragma_query_value(None, name, |r| r.get(0)).unwrap() };
        assert_eq!(pragma("foreign_keys"), 1);
        assert_eq!(pragma("synchronous"), 1, "NORMAL");
        assert_eq!(pragma("busy_timeout"), 5000);
        assert_eq!(pragma("user_version"), SCHEMA_VERSION);
        assert_eq!(pragma("application_id"), APPLICATION_ID);
    }

    #[test]
    fn the_file_handle_is_one_unified_file() {
        let dir = crate::TempDir::new().unwrap();
        let db = TestDb::handle(&dir);
        let categories: i64 =
            db.read(|c| Ok(c.query_row("SELECT count(*) FROM categories", [], |r| r.get(0))?)).unwrap();
        assert_eq!(categories, 2, "the two system categories");
        let files: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".db"))
            .collect();
        assert_eq!(files, ["kurobayomi.db"]);
    }
}
