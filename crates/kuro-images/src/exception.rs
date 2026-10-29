//! Exception to user message mapping (area:34 behavior section 28, T-34a-059).
//!
//! `AppError` is the closed set of errors the formatter tells apart; everything else is `Other`, which keeps the
//! Kotlin simple class name and the message so the fallback text matches `ExceptionFormatter`.

use kuro_net::NetError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppError {
    Http {
        code: u16,
    },
    /// `offline` is the device state when the error is shown (`!context.isOnline()`).
    UnknownHost {
        host: String,
        offline: bool,
    },
    /// `NoChaptersException` and `NoEpisodesException`.
    NoResults,
    /// `SourceNotInstalledException` and `AnimeSourceNotInstalledException`.
    SourceNotInstalled,
    /// `kind` is the class simple name, such as `IOException` or `IllegalStateException`.
    Other {
        kind: &'static str,
        message: Option<String>,
    },
}

impl AppError {
    /// The error a network failure shows as. `online` is whether the device has a connection.
    pub fn from_net(error: &NetError, online: bool) -> Self {
        match error {
            NetError::Http { code } => AppError::Http { code: *code },
            NetError::UnknownHost { host } => AppError::UnknownHost {
                host: host.clone(),
                offline: !online,
            },
            other => AppError::Other {
                kind: "IOException",
                message: Some(other.to_string()),
            },
        }
    }
}

/// The text of `Throwable.formattedMessage`, in the branch order of behavior section 28.1.
pub fn user_message(error: &AppError) -> String {
    match error {
        AppError::Http { code } => format!("HTTP {code}, check website in WebView"),
        AppError::UnknownHost { offline: true, .. } => "No Internet connection".to_owned(),
        AppError::UnknownHost { host, .. } => format!("Couldn't reach {host}"),
        AppError::NoResults => "No results found".to_owned(),
        AppError::SourceNotInstalled => "Source not found".to_owned(),
        AppError::Other { kind, message } => match kind {
            // A plain `Exception` or `IOException` shows its message, or its own name when it has none.
            &("Exception" | "IOException") => message.clone().unwrap_or_else(|| (*kind).to_owned()),
            // Any other class prints `Name: null` for a missing message (edge E-IMG-36).
            _ => format!("{kind}: {}", message.as_deref().unwrap_or("null")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn other(kind: &'static str, message: Option<&str>) -> AppError {
        AppError::Other {
            kind,
            message: message.map(str::to_owned),
        }
    }

    #[test]
    fn g_exc_table() {
        let host = |offline| AppError::UnknownHost {
            host: "example.invalid".into(),
            offline,
        };
        let rows: Vec<(AppError, &str)> = vec![
            (
                AppError::Http { code: 404 },
                "HTTP 404, check website in WebView",
            ),
            (host(true), "No Internet connection"),
            (host(false), "Couldn't reach example.invalid"),
            (AppError::NoResults, "No results found"),
            (AppError::SourceNotInstalled, "Source not found"),
            (other("IOException", Some("disk full")), "disk full"),
            (other("IOException", None), "IOException"),
            (other("Exception", Some("boom")), "boom"),
            (other("Exception", None), "Exception"),
            (
                other("IllegalStateException", Some("bad")),
                "IllegalStateException: bad",
            ),
            (
                other("IllegalStateException", None),
                "IllegalStateException: null",
            ),
        ];
        for (error, want) in rows {
            assert_eq!(user_message(&error), want, "{error:?}");
        }
    }

    #[test]
    fn unknown_host_without_a_name_prints_an_empty_host() {
        let error = AppError::UnknownHost {
            host: String::new(),
            offline: false,
        };
        assert_eq!(user_message(&error), "Couldn't reach ");
    }

    #[test]
    fn network_errors_map_to_the_http_and_host_branches() {
        assert_eq!(
            user_message(&AppError::from_net(&NetError::Http { code: 503 }, true)),
            "HTTP 503, check website in WebView"
        );
        let host = NetError::UnknownHost {
            host: "example.invalid".into(),
        };
        assert_eq!(
            user_message(&AppError::from_net(&host, false)),
            "No Internet connection"
        );
        assert_eq!(
            user_message(&AppError::from_net(&host, true)),
            "Couldn't reach example.invalid"
        );
        assert_eq!(
            user_message(&AppError::from_net(&NetError::Timeout, true)),
            "timeout"
        );
    }
}
