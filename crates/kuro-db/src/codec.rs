//! Conversions between Rust values and SQLite column values, and the JSON text bound to `json_each`.

use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use crate::error::DbError;

/// TEXT column holding genres joined by ", " (Aniyomi `StringListColumnAdapter`).
/// NULL is `None` through the `Option` impls; `''` is an empty list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenreList(pub Vec<String>);

impl FromSql for GenreList {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        Ok(Self(if s.is_empty() {
            Vec::new()
        } else {
            s.split(", ").map(str::to_owned).collect()
        }))
    }
}

impl ToSql for GenreList {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.0.join(", ").into())
    }
}

/// INTEGER column of `mangas.update_strategy`. Unknown values read as `AlwaysUpdate`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateStrategy {
    #[default]
    AlwaysUpdate = 0,
    OnlyFetchOnce = 1,
}

impl UpdateStrategy {
    pub fn from_db(v: i64) -> Self {
        if v == 1 { Self::OnlyFetchOnce } else { Self::AlwaysUpdate }
    }

    pub fn to_db(self) -> i64 {
        self as i64
    }
}

impl FromSql for UpdateStrategy {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_i64().map(Self::from_db)
    }
}

impl ToSql for UpdateStrategy {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.to_db().into())
    }
}

/// `animes.fetch_type`: 0 Seasons, 1 Episodes. Any other value reads as Episodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FetchType {
    Seasons = 0,
    #[default]
    Episodes = 1,
}

impl FetchType {
    pub fn from_db(v: i64) -> Self {
        if v == 0 { Self::Seasons } else { Self::Episodes }
    }
}

impl FromSql for FetchType {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Self::from_db)
    }
}

impl ToSql for FetchType {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok((*self as i64).into())
    }
}

/// `memo`: a JSON object stored as BLOB (or TEXT in the fresh default `'{}'`). A cell that is not a
/// JSON object reads as the empty object with a warning. `FromSql` has no table or row to report, so
/// the read is tolerant rather than an error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Memo(pub serde_json::Map<String, serde_json::Value>);

impl FromSql for Memo {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let bytes: &[u8] = match value {
            ValueRef::Text(t) | ValueRef::Blob(t) => t,
            _ => &[],
        };
        match serde_json::from_slice::<serde_json::Value>(bytes) {
            Ok(serde_json::Value::Object(map)) => Ok(Self(map)),
            _ => {
                tracing::warn!("memo is not a JSON object; reading it as {{}}");
                Ok(Self::default())
            }
        }
    }
}

impl Memo {
    /// Strict read of a `memo` cell: a TEXT or BLOB holding a JSON object, or `DbError::BadMemo` naming the
    /// table and row. The row readers are tolerant (`FromSql` above) so one corrupt cell never fails a list
    /// query; a caller that must know about the cell, such as a backup, uses this.
    pub fn decode(value: ValueRef<'_>, table: &'static str, id: i64) -> Result<Self, DbError> {
        let bytes = match value {
            ValueRef::Text(t) | ValueRef::Blob(t) => t,
            _ => return Err(DbError::BadMemo { table, id }),
        };
        match serde_json::from_slice::<serde_json::Value>(bytes) {
            Ok(serde_json::Value::Object(map)) => Ok(Self(map)),
            _ => Err(DbError::BadMemo { table, id }),
        }
    }
}

impl ToSql for Memo {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let bytes = serde_json::to_vec(&self.0)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(ToSqlOutput::Owned(rusqlite::types::Value::Blob(bytes)))
    }
}

/// JSON array text for `IN (SELECT value FROM json_each(:ids))`. Replaces the `IN :list` expansion
/// of SQLDelight, which hits the bound-variable limit on large lists.
pub fn ids_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).expect("a slice of integers always serialises")
}

/// JSON array text of strings for `json_each`; quotes, backslashes and non-ASCII are escaped.
pub fn strings_json(values: &[String]) -> String {
    serde_json::to_string(values).expect("a slice of strings always serialises")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn roundtrip_genres(input: Option<GenreList>) -> (Option<String>, Option<GenreList>) {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE g(v TEXT)").unwrap();
        c.execute("INSERT INTO g(v) VALUES (?1)", [&input]).unwrap();
        let stored: Option<String> = c.query_row("SELECT v FROM g", [], |r| r.get(0)).unwrap();
        let back: Option<GenreList> = c.query_row("SELECT v FROM g", [], |r| r.get(0)).unwrap();
        (stored, back)
    }

    #[test]
    fn genre_list_encoding_matches_upstream() {
        let s = |v: &[&str]| Some(GenreList(v.iter().map(|x| (*x).to_owned()).collect()));
        assert_eq!(roundtrip_genres(None), (None, None));
        assert_eq!(roundtrip_genres(s(&[])), (Some(String::new()), Some(GenreList(vec![]))));
        assert_eq!(
            roundtrip_genres(s(&["a", "b"])),
            (Some("a, b".into()), s(&["a", "b"]))
        );
        // A genre containing ", " splits on read: the documented sharp edge.
        assert_eq!(roundtrip_genres(s(&["a, b"])), (Some("a, b".into()), s(&["a", "b"])));
    }

    #[test]
    fn genre_list_with_no_separator_reads_as_one_item() {
        let c = Connection::open_in_memory().unwrap();
        let v: GenreList = c.query_row("SELECT 'Action'", [], |r| r.get(0)).unwrap();
        assert_eq!(v, GenreList(vec!["Action".into()]));
    }

    #[test]
    fn update_strategy_maps_known_and_unknown_values() {
        assert_eq!(UpdateStrategy::from_db(7), UpdateStrategy::AlwaysUpdate);
        assert_eq!(UpdateStrategy::from_db(2), UpdateStrategy::AlwaysUpdate);
        assert_eq!(UpdateStrategy::from_db(-1), UpdateStrategy::AlwaysUpdate);
        assert_eq!(UpdateStrategy::from_db(0), UpdateStrategy::AlwaysUpdate);
        assert_eq!(UpdateStrategy::from_db(1), UpdateStrategy::OnlyFetchOnce);
        assert_eq!(UpdateStrategy::AlwaysUpdate.to_db(), 0);
        assert_eq!(UpdateStrategy::OnlyFetchOnce.to_db(), 1);
    }

    #[test]
    fn update_strategy_round_trips_through_sqlite() {
        let c = Connection::open_in_memory().unwrap();
        let v: UpdateStrategy = c.query_row("SELECT 7", [], |r| r.get(0)).unwrap();
        assert_eq!(v, UpdateStrategy::AlwaysUpdate);
        let stored: i64 = c
            .query_row("SELECT ?1", [UpdateStrategy::OnlyFetchOnce], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, 1);
    }

    #[test]
    fn ids_json_drives_json_each() {
        let c = Connection::open_in_memory().unwrap();
        let ids = ids_json(&[3, 1, 2]);
        assert_eq!(ids, "[3,1,2]");
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM json_each(?1)",
                [ids],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn string_lists_round_trip_through_json_each() {
        let c = Connection::open_in_memory().unwrap();
        let input = vec![
            "quote \"x\"".to_owned(),
            "back\\slash".to_owned(),
            "Ärger 漫画".to_owned(),
            String::new(),
        ];
        let mut st = c.prepare("SELECT value FROM json_each(?1)").unwrap();
        let out: Vec<String> = st
            .query_map([strings_json(&input)], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(out, input);
    }
}
