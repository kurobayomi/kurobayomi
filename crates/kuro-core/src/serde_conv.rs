//! Serialization conventions (T-01a-016): the Rust equivalent of Aniyomi's `Json { ignoreUnknownKeys = true;
//! explicitNulls = false }`.
//!
//! * `ignoreUnknownKeys = true`: serde ignores unknown keys unless a struct says `deny_unknown_fields`. Never add that.
//! * missing keys: put `#[serde(default)]` on the struct (or on one field) so an absent key takes `Default`.
//! * `explicitNulls = false`: put `#[serde(skip_serializing_if = "serde_conv::is_none")]` on every `Option` field,
//!   so `None` is omitted instead of written as `null`. Reading accepts both an absent key and `null`.
//!
//! ```
//! use kuro_core::serde_conv;
//!
//! #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
//! #[serde(default)]
//! struct Entry {
//!     title: String,
//!     #[serde(skip_serializing_if = "serde_conv::is_none")]
//!     author: Option<String>,
//! }
//!
//! let e: Entry = serde_conv::from_json(r#"{"title":"t","future":1}"#).unwrap();
//! assert_eq!(serde_conv::to_json(&e).unwrap(), r#"{"title":"t"}"#);
//! ```
//!
//! XML is covered by [`crate::xml`]; the protobuf backup container is hand-written in `kuro-backup`.

use serde::{Serialize, de::DeserializeOwned};

/// `skip_serializing_if` predicate for `Option` fields (the `explicitNulls = false` half).
pub fn is_none<T>(value: &Option<T>) -> bool {
    value.is_none()
}

/// Decode JSON text; unknown keys are ignored and missing keys take their serde defaults.
pub fn from_json<T: DeserializeOwned>(text: &str) -> serde_json::Result<T> {
    serde_json::from_str(text)
}

/// Encode compact JSON; fields marked with [`is_none`] are omitted when `None`.
pub fn to_json<T: Serialize>(value: &T) -> serde_json::Result<String> {
    serde_json::to_string(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Sample {
        title: String,
        #[serde(skip_serializing_if = "is_none")]
        author: Option<String>,
        count: u32,
    }

    #[test]
    fn unknown_field_is_ignored() {
        let s: Sample = from_json(r#"{"title":"a","author":"b","count":2,"added_later":{"x":1}}"#).unwrap();
        assert_eq!(s, Sample { title: "a".into(), author: Some("b".into()), count: 2 });
    }

    #[test]
    fn missing_optional_and_defaulted_fields_decode() {
        let s: Sample = from_json(r#"{"title":"a"}"#).unwrap();
        assert_eq!(s, Sample { title: "a".into(), author: None, count: 0 });
    }

    #[test]
    fn explicit_null_decodes_as_none() {
        let s: Sample = from_json(r#"{"title":"a","author":null}"#).unwrap();
        assert_eq!(s.author, None);
    }

    #[test]
    fn encoding_omits_null_fields() {
        let s = Sample { title: "a".into(), author: None, count: 1 };
        assert_eq!(to_json(&s).unwrap(), r#"{"title":"a","count":1}"#);
        let s = Sample { author: Some("b".into()), ..s };
        assert_eq!(to_json(&s).unwrap(), r#"{"title":"a","author":"b","count":1}"#);
    }
}
