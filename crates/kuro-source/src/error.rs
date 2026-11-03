//! `SourceError`: the failure kinds of a source call (T-15a-001).

/// A failure of a source call. The UI maps the variant to a message; `Display` is for logs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// Non-2xx response. Display: "HTTP error {status}" (same text as HttpException).
    #[error("HTTP error {0}")]
    Http(u16),
    /// Connection, TLS, timeout or body read failure. The message is the transport error text.
    #[error("{0}")]
    Transport(String),
    /// The host name did not resolve (`UnknownHostException`). The UI shows "No Internet connection" when the device is
    /// offline and "Couldn't reach {host}" otherwise (`exception_unknown_host`).
    #[error("unknown host {host}")]
    UnknownHost { host: String },
    /// HTML or JSON structure did not match what the source expects.
    #[error("parse error: {0}")]
    Parse(String),
    /// A URL could not be built or parsed.
    #[error("invalid URL: {0}")]
    Url(String),
    /// The page has no image URL when the image was requested.
    #[error("page has no image URL")]
    MissingImageUrl,
    /// The source returned an empty page list.
    #[error("page list is empty")]
    EmptyPageList,
    /// The operation is not implemented by this source (for example a source without search).
    #[error("not supported: {0}")]
    Unsupported(&'static str),
    /// The caller cancelled the operation.
    #[error("cancelled")]
    Cancelled,
    /// The local stream server could not start, or was asked for port 0 (E-08, E-37).
    #[error("local server: {0}")]
    LocalServer(String),
    /// No installed source has this id: a stub, or an id nothing knows (`SourceError.NotInstalled`, T-17a-013).
    #[error("source {0} is not installed")]
    NotInstalled(i64),
    /// The id names an installed source that is not a catalogue source, or a stub (T-17a-013). Replaces the Kotlin cast.
    #[error("source {0} is not a catalogue source")]
    NotCatalogue(i64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_display_text() {
        let all = [
            SourceError::LocalServer("bind".into()),
            SourceError::Http(503),
            SourceError::Transport("reset".into()),
            SourceError::UnknownHost { host: "x.example".into() },
            SourceError::Parse("bad html".into()),
            SourceError::Url("no host".into()),
            SourceError::MissingImageUrl,
            SourceError::EmptyPageList,
            SourceError::Unsupported("search"),
            SourceError::Cancelled,
            SourceError::NotInstalled(7),
            SourceError::NotCatalogue(7),
        ];
        for err in &all {
            assert!(!err.to_string().is_empty(), "{err:?}");
        }
        assert_eq!(SourceError::Parse("x".into()).to_string(), "parse error: x");
    }

    #[test]
    fn the_typed_catalogue_errors_name_the_id_and_are_not_panics() {
        assert_eq!(SourceError::NotInstalled(42).to_string(), "source 42 is not installed");
        assert_eq!(SourceError::NotCatalogue(42).to_string(), "source 42 is not a catalogue source");
        assert_ne!(SourceError::NotInstalled(42), SourceError::NotCatalogue(42));
    }
}
