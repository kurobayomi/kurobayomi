//! Domain error types. `DomainError` is the failure of a domain write or read. `NoChapters`,
//! `NoEpisodes` and `NoSeasons` are the empty-result errors of the entry interactors (`Exception`
//! classes upstream). Each carries an i18n key; the text is looked up by the UI, never written here.
//!
//! ## Error and logging policy (T-09a-059)
//!
//! Three patterns, chosen per interactor.
//!
//! | Interactor (module) | Pattern | Result to the caller |
//! |---|---|---|
//! | Track reads and writes (`track::service`, the manga and anime `GetMangaTracks`, `InsertMangaTrack`, `DeleteMangaTrack` and twins) | swallow and log, via `log_and_default` | `*_or_default`: `None`, empty list or nothing, one `error` log line. `try_*`: `Result`, for new UI |
//! | Storage (`storage::StorageManager`) | propagate | `StorageUnavailable` for a missing or refused base, `Io` for a folder that cannot be created, `Invalid` for a non-directory |
//! | Backup preferences (`backup`) | no failure path | values are plain preferences |
//! | Custom buttons (`InternalError` cases) | propagate, carrying the source error and logged at `error` | not in this chunk: custom buttons land with T-09a-036 to T-09a-040 |
//! | Release check (`release`) | propagate, `Err` leaves the last-check time unchanged | not in this chunk: lands with T-09a-042 to T-09a-046 |
//!
//! Cancellation: the app layer runs a write that must finish under `app::rx::non_cancellable` (EC-29); a reader that is
//! dropped stops reading and writes nothing.

use kuro_db::open::DbError;

/// Failure of a category, library or history operation.
#[derive(thiserror::Error, Debug)]
pub enum DomainError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("invalid category name")]
    InvalidName,
    #[error("a category with this name already exists")]
    DuplicateName,
    #[error("category not found")]
    NotFound,
    #[error("the default category cannot be changed")]
    SystemCategory,
    #[error("background task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    /// A store-flow failure whose text the dialog shows as is (`StoreError`).
    #[error("{0}")]
    Store(&'static str),
    #[error("source not installed: {0}")]
    SourceNotInstalled(String),
    /// A non-local source returned no seasons for a season parent (`NoSeasonsException`; message key `no_seasons_error`).
    #[error("no seasons found")]
    NoSeasons,
    #[error("storage base directory unavailable")]
    StorageUnavailable,
    /// An HTTP request failed (no response, or a non-success status). The text is the client's message.
    #[error("network error: {0}")]
    Net(String),
    /// A folder under the base could not be created or written (read-only filesystem, permissions).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// A source call failed (for example a related-anime fetch). The text is the source error's message.
    #[error("source error: {0}")]
    Source(#[from] kuro_source::SourceError),
}

/// Failure of an extension repository or store write (`SaveExtensionRepoException` upstream). `Save` is the only
/// variant a constraint or write failure produces; everything else is a [`DomainError`].
#[derive(thiserror::Error, Debug)]
pub enum RepoStoreError {
    #[error("error saving repository to database")]
    Save(#[source] DbError),
    #[error(transparent)]
    Other(#[from] DomainError),
}

/// `NoChaptersException`: a manga source returned no chapters.
#[derive(thiserror::Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("no chapters found")]
pub struct NoChapters;

/// `NoEpisodesException`: an anime source returned no episodes.
#[derive(thiserror::Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("no episodes found")]
pub struct NoEpisodes;

/// `NoSeasonsException`: a non-local source returned no seasons for a season parent.
#[derive(thiserror::Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("no seasons found")]
pub struct NoSeasons;

/// User-visible text of a domain error, as an i18n key.
pub mod message_key {
    /// Details-screen snackbar for `NoChapters`.
    pub const NO_CHAPTERS_FOUND: &str = "no_chapters_found";
    /// Details-screen snackbar for `NoEpisodes`.
    pub const NO_EPISODES_FOUND: &str = "no_episodes_found";
    /// Details-screen snackbar for `NoSeasons`.
    pub const NO_SEASONS_FOUND: &str = "no_seasons_found";
    /// Library-update log and generic error dialog for `NoChapters` and `NoEpisodes`.
    pub const NO_RESULTS_FOUND: &str = "no_results_found";
    /// Generic error dialog for `NoSeasons`.
    pub const NO_SEASONS_FOUND_GENERIC: &str = "no_seasons_found";
    /// Toast for any failed database write.
    pub const INTERNAL_ERROR: &str = "internal_error";
    /// Inline field error for `DuplicateName`.
    pub const ERROR_CATEGORY_EXISTS: &str = "error_category_exists";
    /// Display name of the system category.
    pub const LABEL_DEFAULT: &str = "label_default";
}

impl NoChapters {
    /// Key of the details-screen snackbar.
    pub fn snackbar_key(self) -> &'static str {
        message_key::NO_CHAPTERS_FOUND
    }

    /// Key of the library-update log and generic error dialog.
    pub fn generic_key(self) -> &'static str {
        message_key::NO_RESULTS_FOUND
    }
}

impl NoEpisodes {
    /// Key of the details-screen snackbar.
    pub fn snackbar_key(self) -> &'static str {
        message_key::NO_EPISODES_FOUND
    }

    /// Key of the library-update log and generic error dialog.
    pub fn generic_key(self) -> &'static str {
        message_key::NO_RESULTS_FOUND
    }
}

impl NoSeasons {
    /// Key of the details-screen snackbar.
    pub fn snackbar_key(self) -> &'static str {
        message_key::NO_SEASONS_FOUND
    }

    /// Key of the generic error dialog (its own text, not the "No results" one).
    pub fn generic_key(self) -> &'static str {
        message_key::NO_SEASONS_FOUND_GENERIC
    }
}

impl DomainError {
    /// Key of the toast or inline text shown for this error.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::DuplicateName => message_key::ERROR_CATEGORY_EXISTS,
            _ => message_key::INTERNAL_ERROR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_result_errors_map_to_their_own_keys() {
        assert_eq!(NoChapters.snackbar_key(), "no_chapters_found");
        assert_eq!(NoEpisodes.snackbar_key(), "no_episodes_found");
        assert_eq!(NoSeasons.snackbar_key(), "no_seasons_found");
        assert_eq!(NoChapters.generic_key(), "no_results_found");
        assert_eq!(NoEpisodes.generic_key(), "no_results_found");
        assert_eq!(NoSeasons.generic_key(), "no_seasons_found");
    }

    #[test]
    fn domain_errors_have_english_messages_and_keys() {
        assert_eq!(DomainError::InvalidName.to_string(), "invalid category name");
        assert_eq!(DomainError::DuplicateName.to_string(), "a category with this name already exists");
        assert_eq!(DomainError::SystemCategory.to_string(), "the default category cannot be changed");
        assert_eq!(DomainError::NotFound.message_key(), "internal_error");
        assert_eq!(DomainError::DuplicateName.message_key(), "error_category_exists");
    }
}
