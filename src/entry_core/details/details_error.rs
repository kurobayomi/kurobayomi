//! Typed errors of the anime details page (T-39a-060, D-5). Each place the Kotlin used `!!` or a failing lookup now
//! returns one of these, shows its text, and logs it with `tracing::error!`.

use kuro_i18n::{MsgKey, tr};
use std::fmt;

use kuro_data::DataError;

#[derive(Debug)]
pub enum DetailsError {
    EpisodeNotFound(i64),
    AnimeNotFound(i64),
    /// The quality (hoster and video) options of an episode failed; the text is the cause.
    /// A read or write of the anime database failed.
    Data(DataError),
    Db(rusqlite::Error),
    Io(std::io::Error),
}

impl DetailsError {
    /// The text a user sees in the error state or the snackbar.
    pub fn user_message(&self) -> String {
        match self {
            Self::EpisodeNotFound(_) => tr(MsgKey::ui_this_episode_is_no_longer_in_the_library).to_owned(),
            Self::AnimeNotFound(_) => tr(MsgKey::ui_this_anime_is_no_longer_in_the_library).to_owned(),
            Self::Data(_) | Self::Db(_) | Self::Io(_) => tr(MsgKey::ui_something_went_wrong_while_reading_this_entr).to_owned(),
        }
    }

    /// Logs the error with its detail and returns it, so a caller can log and show in one line.
    pub fn record(self) -> Self {
        tracing::error!("anime details: {self}");
        self
    }
}

impl fmt::Display for DetailsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EpisodeNotFound(id) => write!(f, "episode {id} not found"),
            Self::AnimeNotFound(id) => write!(f, "anime {id} not found"),
            Self::Data(error) => write!(f, "data: {error}"),
            Self::Db(error) => write!(f, "db: {error}"),
            Self::Io(error) => write!(f, "io: {error}"),
        }
    }
}

impl std::error::Error for DetailsError {}

impl From<DataError> for DetailsError {
    fn from(error: DataError) -> Self {
        Self::Data(error)
    }
}

impl From<rusqlite::Error> for DetailsError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(error)
    }
}

impl From<std::io::Error> for DetailsError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Replaces `!!` and `get(...)!!`: a missing value becomes the typed error, never a panic.
pub fn required<T>(value: Option<T>, missing: impl FnOnce() -> DetailsError) -> Result<T, DetailsError> {
    value.ok_or_else(missing)
}

/// A blocking task that did not finish (it panicked or the runtime is shutting down) is an I/O failure of the page.
pub fn join_failed(error: &tokio::task::JoinError) -> DetailsError {
    DetailsError::Io(std::io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_episode_or_anime_is_an_error_not_a_panic() {
        // the two lookups of EpisodeOptionsDialogScreen.kt:172-173
        assert!(matches!(required(None::<i64>, || DetailsError::EpisodeNotFound(4)), Err(DetailsError::EpisodeNotFound(4))));
        assert!(matches!(required(Some(1), || DetailsError::AnimeNotFound(4)), Ok(1)));
    }

    #[test]
    fn each_variant_has_a_message_and_a_detail_line() {
        let all = [
            DetailsError::EpisodeNotFound(1),
            DetailsError::AnimeNotFound(2),
            DetailsError::Data(DataError::NotFound),
            DetailsError::Db(rusqlite::Error::QueryReturnedNoRows),
            DetailsError::Io(std::io::Error::other("disk")),
        ];
        for error in &all {
            assert!(!error.user_message().is_empty());
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(DetailsError::EpisodeNotFound(7).to_string(), "episode 7 not found");
    }

    #[test]
    fn the_source_errors_convert_and_keep_their_variant() {
        assert!(matches!(DetailsError::from(rusqlite::Error::QueryReturnedNoRows), DetailsError::Db(_)));
        assert!(matches!(DetailsError::from(std::io::Error::other("x")), DetailsError::Io(_)));
        assert!(matches!(DetailsError::from(DataError::NotFound), DetailsError::Data(_)));
    }

    #[test]
    fn record_returns_the_same_error() {
        assert!(matches!(DetailsError::AnimeNotFound(9).record(), DetailsError::AnimeNotFound(9)));
    }
}
