//! Column adapters (`DatabaseAdapter.kt`, T-05a-003): the named decode and encode functions for the columns
//! that are not plain integers or text. The codec types live in `kuro-db`; this module gives them the names of
//! the Kotlin adapters, so repository code reads the same way.

use kuro_db::codecs::GenreList;
pub use kuro_db::codecs::UpdateStrategy;
pub use kuro_db::codec::{FetchType, Memo};
use rusqlite::types::{FromSql, ValueRef};

/// `StringListColumnAdapter.decode`: `""` is an empty list, otherwise the text splits on the two-character
/// separator `", "`. A genre that contains `", "` therefore splits on read (EC-19).
pub fn decode_string_list(column: &str) -> Vec<String> {
    GenreList::column_result(ValueRef::Text(column.as_bytes())).map(|g| g.0).unwrap_or_default()
}

/// `StringListColumnAdapter.encode`: the values joined with `", "`.
pub fn encode_string_list(values: &[String]) -> String {
    values.join(", ")
}

/// `memo` column bytes: the compact JSON of the object, keys in insertion order.
pub fn encode_memo(memo: &Memo) -> Vec<u8> {
    serde_json::to_vec(&memo.0).expect("a JSON object always serialises")
}

/// `memo` column bytes to a map. Invalid JSON and non-object JSON read as `{}` with a warning (EC-18).
pub fn decode_memo(bytes: &[u8]) -> Memo {
    Memo::column_result(ValueRef::Blob(bytes)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn string_list_decodes_empty_and_splits_on_comma_space() {
        assert!(decode_string_list("").is_empty());
        assert_eq!(decode_string_list("Action, Drama"), strings(&["Action", "Drama"]));
        assert_eq!(decode_string_list("Action"), strings(&["Action"]));
    }

    #[test]
    fn a_genre_containing_the_separator_splits_on_read() {
        let stored = encode_string_list(&strings(&["Slice, of life"]));
        assert_eq!(stored, "Slice, of life");
        assert_eq!(decode_string_list(&stored), strings(&["Slice", "of life"]));
    }

    #[test]
    fn string_list_encodes_with_the_two_character_separator() {
        assert_eq!(encode_string_list(&[]), "");
        assert_eq!(encode_string_list(&strings(&["a", "b", "c"])), "a, b, c");
    }

    #[test]
    fn update_strategy_decodes_0_and_1_and_maps_everything_else_to_always() {
        assert_eq!(UpdateStrategy::from_db(0), UpdateStrategy::AlwaysUpdate);
        assert_eq!(UpdateStrategy::from_db(1), UpdateStrategy::OnlyFetchOnce);
        for other in [2, 99, -1] {
            assert_eq!(UpdateStrategy::from_db(other), UpdateStrategy::AlwaysUpdate);
        }
    }

    #[test]
    fn fetch_type_decodes_0_to_seasons_and_everything_else_to_episodes() {
        assert_eq!(FetchType::from_db(0), FetchType::Seasons);
        assert_eq!(FetchType::from_db(1), FetchType::Episodes);
        for other in [2, -5] {
            assert_eq!(FetchType::from_db(other), FetchType::Episodes);
        }
    }

    #[test]
    fn memo_round_trips_with_key_order_and_invalid_input_reads_empty() {
        let mut memo = Memo::default();
        memo.0.insert("zeta".into(), serde_json::json!(1));
        memo.0.insert("alpha".into(), serde_json::json!("x"));
        let bytes = encode_memo(&memo);
        assert_eq!(bytes, br#"{"zeta":1,"alpha":"x"}"#);
        assert_eq!(decode_memo(&bytes), memo);
        assert_eq!(decode_memo(b"not json"), Memo::default());
        assert_eq!(decode_memo(b"[1,2]"), Memo::default());
    }
}
