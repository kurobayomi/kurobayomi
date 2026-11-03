//! Error helpers over the database error (T-06a-004): constraint classification and the one-row mapping.
//! The `Pool`, `NestedWrite`, `Join` and `Migration` variants belong to `kuro-db`, not here.

use rusqlite::ErrorCode;

use crate::{DataError, DataResult};

/// Classification helpers on [`DataError`].
pub trait DataErrorExt {
    /// True for an SQLite constraint violation (unique, foreign key, not null, check, trigger `RAISE`
    /// with `SQLITE_CONSTRAINT`) and for the crate's own constraint error. False for everything else.
    fn is_constraint(&self) -> bool;
}

impl DataErrorExt for DataError {
    fn is_constraint(&self) -> bool {
        match self {
            DataError::Constraint(_) => true,
            DataError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) => f.code == ErrorCode::ConstraintViolation,
            _ => false,
        }
    }
}

/// A single-row query result: `QueryReturnedNoRows` becomes [`DataError::NotFound`], other errors pass through.
pub fn one_row<T>(r: rusqlite::Result<T>) -> DataResult<T> {
    match r {
        Ok(value) => Ok(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(DataError::NotFound),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn a_unique_violation_is_a_constraint_error() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t(v INTEGER UNIQUE); INSERT INTO t VALUES (1);").unwrap();
        let err: DataError = c.execute("INSERT INTO t VALUES (1)", []).unwrap_err().into();
        assert!(err.is_constraint());
    }

    #[test]
    fn a_busy_or_missing_table_error_is_not_a_constraint_error() {
        let c = Connection::open_in_memory().unwrap();
        let err: DataError = c.execute("INSERT INTO missing VALUES (1)", []).unwrap_err().into();
        assert!(!err.is_constraint());
        assert!(!DataError::NotFound.is_constraint());
    }

    #[test]
    fn no_rows_maps_to_not_found_and_values_pass_through() {
        let c = Connection::open_in_memory().unwrap();
        let missing: DataResult<i64> = one_row(c.query_row("SELECT 1 WHERE 0", [], |r| r.get(0)));
        assert!(matches!(missing, Err(DataError::NotFound)));
        let present: DataResult<i64> = one_row(c.query_row("SELECT 7", [], |r| r.get(0)));
        assert_eq!(present.unwrap(), 7);
    }
}
