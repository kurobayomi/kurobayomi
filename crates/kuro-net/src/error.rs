//! `NetError` (T-12a-001) and the user-facing message mapping.

use std::error::Error as _;

/// Every failure the HTTP stack reports. `Display` matches the Kotlin messages the user sees.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("HTTP error {code}")]
    Http { code: u16 },
    #[error("{0}")]
    InvalidUrl(String),
    #[error("Canceled")]
    Canceled,
    #[error("timeout")]
    Timeout,
    #[error("{host}")]
    UnknownHost { host: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Tls(String),
    #[error("Failed to bypass Cloudflare")]
    CloudflareBypassFailed,
    #[error("{0}")]
    Decode(String),
    #[error("{0}")]
    Other(String),
}

/// What the UI shows for an error. `Key` and `KeyArg` are message keys for the i18n layer; `Raw` is shown as is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserMessage {
    Key(&'static str),
    KeyArg(&'static str, String),
    Raw(String),
    /// Cancellation is not an error the user sees.
    Hidden,
}

impl NetError {
    /// The message for this error. `online` says whether the device has a connection (`exception_offline`).
    pub fn user_message(&self, online: bool) -> UserMessage {
        match self {
            NetError::Http { code } => UserMessage::KeyArg("exception_http", code.to_string()),
            NetError::UnknownHost { .. } if !online => UserMessage::Key("exception_offline"),
            NetError::UnknownHost { host } => UserMessage::KeyArg("exception_unknown_host", host.clone()),
            NetError::CloudflareBypassFailed => UserMessage::Key("information_cloudflare_bypass_failure"),
            NetError::Canceled => UserMessage::Hidden,
            other => UserMessage::Raw(other.to_string()),
        }
    }
}

impl From<reqwest::Error> for NetError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return NetError::Timeout;
        }
        if err.is_builder() {
            return NetError::InvalidUrl(err.to_string());
        }
        // an address the app refuses for sources (`public_only`) says so, rather than looking like a missing host
        if chain_mentions(&err, crate::public_only::REFUSED) {
            return NetError::Other(crate::public_only::REFUSED.to_owned());
        }
        if let Some(kind) = find_source::<std::io::Error>(&err).map(std::io::Error::kind) {
            // The reqwest error stays inside the io error, so the original io error is still in the source chain.
            return NetError::Io(std::io::Error::new(kind, err));
        }
        if err.is_connect() && is_dns_failure(&err) {
            let host = err.url().and_then(|u| u.host_str()).unwrap_or_default().to_owned();
            return NetError::UnknownHost { host };
        }
        if err.is_connect() && is_tls_failure(&err) {
            return NetError::Tls(err.to_string());
        }
        if err.is_decode() {
            return NetError::Decode(err.to_string());
        }
        NetError::Other(err.to_string())
    }
}

/// Whether any error in the source chain of `err` has `text` in its message.
fn chain_mentions(err: &(dyn std::error::Error + 'static), text: &str) -> bool {
    let mut next = Some(err);
    while let Some(e) = next {
        if e.to_string().contains(text) {
            return true;
        }
        next = e.source();
    }
    false
}

/// The first error of type `T` in the source chain of `err`.
fn find_source<'a, T: std::error::Error + 'static>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a T> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(found) = e.downcast_ref::<T>() {
            return Some(found);
        }
        current = e.source();
    }
    None
}

/// A name lookup failed. reqwest does not type this, so the resolver's text is matched.
fn is_dns_failure(err: &reqwest::Error) -> bool {
    chain_text(err).iter().any(|t| t.contains("dns error") || t.contains("failed to lookup address"))
}

fn is_tls_failure(err: &reqwest::Error) -> bool {
    chain_text(err).iter().any(|t| {
        let t = t.to_ascii_lowercase();
        t.contains("certificate") || t.contains("tls") || t.contains("handshake")
    })
}

fn chain_text(err: &reqwest::Error) -> Vec<String> {
    let mut out = vec![err.to_string()];
    let mut current = err.source();
    while let Some(e) = current {
        out.push(e.to_string());
        current = e.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_user_message() {
        let all = [
            NetError::Http { code: 503 },
            NetError::InvalidUrl("bad".into()),
            NetError::Canceled,
            NetError::Timeout,
            NetError::UnknownHost { host: "x.test".into() },
            NetError::Io(std::io::Error::other("io")),
            NetError::Tls("tls".into()),
            NetError::CloudflareBypassFailed,
            NetError::Decode("json".into()),
            NetError::Other("other".into()),
        ];
        for err in &all {
            let _ = err.user_message(true);
            let _ = err.user_message(false);
        }
    }

    #[test]
    fn http_503_names_the_key_and_the_code() {
        assert_eq!(
            NetError::Http { code: 503 }.user_message(true),
            UserMessage::KeyArg("exception_http", "503".into())
        );
    }

    #[test]
    fn unknown_host_is_offline_or_named_depending_on_connection() {
        let err = NetError::UnknownHost { host: "x.test".into() };
        assert_eq!(err.user_message(false), UserMessage::Key("exception_offline"));
        assert_eq!(
            err.user_message(true),
            UserMessage::KeyArg("exception_unknown_host", "x.test".into())
        );
    }

    #[test]
    fn cloudflare_failure_uses_its_key_and_cancel_is_hidden() {
        assert_eq!(
            NetError::CloudflareBypassFailed.user_message(true),
            UserMessage::Key("information_cloudflare_bypass_failure")
        );
        assert_eq!(NetError::Canceled.user_message(true), UserMessage::Hidden);
    }

    #[test]
    fn other_variants_show_their_display_text() {
        assert_eq!(
            NetError::Timeout.user_message(true),
            UserMessage::Raw("timeout".into())
        );
    }

    #[test]
    fn io_errors_keep_their_source() {
        let err: NetError = std::io::Error::other("disk").into();
        assert!(matches!(&err, NetError::Io(e) if e.to_string() == "disk"));
    }
}
