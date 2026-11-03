//! Typed repositories over `kuro-db`: one `Entry`, `Item`, `Track` and `History` for both kinds (the kind is a field
//! or a parameter) and their repositories over `kuro_db::tables`. No GTK and no OS code in this crate.
//!
//! Repositories are concrete structs that share one `kuro_db::handle::Db` through an `Arc`. Reads and
//! writes are synchronous. A `subscribe_*` method returns a live query from `kuro-db`, which must be
//! created inside a tokio runtime.

use std::time::Duration;

use rusqlite::{Connection, ErrorCode};

pub mod anime_relation;
pub mod category;
pub mod columns;
pub mod compat;
pub mod dropped;
pub mod entry;
pub mod error;
pub mod history;
pub mod item;
pub mod sanitize;
pub mod source;
pub mod time;
pub mod track;

pub use kuro_db::open::{DbError as DataError, DbResult as DataResult};

/// Ids per statement chunk in batch deletes (`remove_with_ids`, EC-17).
pub(crate) const ID_CHUNK: usize = 500;

/// Quiet time before an entry subscription re-queries after a change.
pub(crate) const ENTRY_DEBOUNCE: Duration = Duration::from_millis(50);

/// Quiet time before a library subscription re-queries after a change.
pub(crate) const LIBRARY_DEBOUNCE: Duration = Duration::from_millis(100);

/// What a lookup-or-insert found: the stored row, or the id of the row this call inserted.
#[derive(Clone, Debug, PartialEq)]
pub enum LookupOrInsert<T> {
    Found(T),
    Inserted(i64),
}

/// What a library subscription publishes: the rows, or the error text once transient retries ran out.
pub type LibraryState<T> = Result<Vec<T>, String>;

/// Retries a library query makes after a transient SQLite lock (`GetLibraryManga.kt` retries after 0.5 s).
pub const LIBRARY_RETRIES: u32 = 20;

/// Wait before each library retry.
pub(crate) const LIBRARY_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Wraps a library query: a transient lock (`SQLITE_BUSY`, `SQLITE_LOCKED`) is retried after `delay`, up to
/// `retries` times, then the error text is returned as the value. Any other error is returned as `Err`, so the
/// subscription publishes it as an error state and the next change runs the query again.
// ponytail: the wait runs while the reader slot is held; move the loop into the subscription task if readers contend.
pub(crate) fn retrying<T>(
    retries: u32,
    delay: Duration,
    query: impl Fn(&Connection) -> DataResult<T>,
) -> impl Fn(&Connection) -> DataResult<Result<T, String>> {
    move |c: &Connection| {
        let mut done = 0;
        loop {
            match query(c) {
                Ok(value) => return Ok(Ok(value)),
                Err(e) if is_transient(&e) && done < retries => {
                    done += 1;
                    std::thread::sleep(delay);
                }
                Err(e) if is_transient(&e) => return Ok(Err(e.to_string())),
                Err(e) => return Err(e),
            }
        }
    }
}

fn is_transient(e: &DataError) -> bool {
    matches!(
        e,
        DataError::Sqlite(rusqlite::Error::SqliteFailure(f, _))
            if matches!(f.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// `None` for a missing row, which is how a subscription reports a deleted entry (EC-01).
pub(crate) fn none_if_not_found<T>(r: DataResult<T>) -> DataResult<Option<T>> {
    match r {
        Ok(value) => Ok(Some(value)),
        Err(DataError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
pub(crate) mod support;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod shared_tests;

#[cfg(test)]
mod open_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use kuro_db::handle::{Db, READERS};
    use kuro_platform::kind::Kind;
    use rusqlite::Connection;

    use crate::support::test_db;

    fn assert_pragmas(c: &Connection) {
        let fk: i64 = c.pragma_query_value(None, "foreign_keys", |r| r.get(0)).unwrap();
        let journal: String = c.pragma_query_value(None, "journal_mode", |r| r.get(0)).unwrap();
        let sync: i64 = c.pragma_query_value(None, "synchronous", |r| r.get(0)).unwrap();
        let recursive: i64 = c.pragma_query_value(None, "recursive_triggers", |r| r.get(0)).unwrap();
        let busy: i64 = c.pragma_query_value(None, "busy_timeout", |r| r.get(0)).unwrap();
        assert_eq!((fk, journal.as_str(), sync, recursive, busy), (1, "wal", 1, 0, 5000));
    }

    fn check_every_connection(db: &Arc<Db>) {
        db.write(|tx| {
            assert_pragmas(tx);
            Ok(())
        })
        .unwrap();
        for _ in 0..=READERS {
            db.read(|c| {
                assert_pragmas(c);
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn the_database_opens_with_the_required_pragmas_on_the_writer_and_every_reader() {
        let (_d, db) = test_db();
        check_every_connection(&db);
    }

    #[test]
    fn a_newer_schema_version_is_refused_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tachiyomi.db");
        drop(kuro_db::schema_unified::open_file(&path).unwrap());
        {
            let c = Connection::open(&path).unwrap();
            c.pragma_update(None, "user_version", 999_999i64).unwrap();
        }
        let err = kuro_db::schema_unified::open_file(&path).expect_err("a newer file is refused");
        assert!(matches!(err, kuro_db::DbError::TooNew { .. }), "{err}");
        let c = Connection::open(&path).unwrap();
        let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 999_999);
    }

    #[test]
    fn a_thousand_subscriptions_created_and_dropped_leave_no_task_behind() {
        let (_dir, db) = test_db();
        let repo = crate::category::CategoryRepo::new(db.clone(), Kind::Manga);
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async {
            for _ in 0..1000 {
                let sub = repo.subscribe_all();
                drop(sub);
            }
        });
        rt.shutdown_timeout(Duration::from_secs(10));
    }

    /// T-06a-003 lint: every delete statement in this crate's sources has a WHERE clause.
    #[test]
    fn no_unconditional_delete_in_this_crate() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let needle = concat!("DELETE", " FROM ");
            for (at, _) in text.match_indices(needle) {
                let statement = &text[at..];
                let end = statement.find(';').unwrap_or(statement.len());
                assert!(statement[..end].contains("WHERE"), "{}: unconditional {}", path.display(), &statement[..end]);
            }
        }
    }
}
