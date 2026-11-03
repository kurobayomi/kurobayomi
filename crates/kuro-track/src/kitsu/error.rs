//! Kitsu errors. The display texts are the texts the user sees.

use kuro_platform::secrets::SecretError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KitsuError {
    #[error("Not authenticated with Kitsu")]
    NotAuthenticated,
    #[error("Kitsu session has expired, log in again")]
    ReauthRequired,
    #[error("HTTP error {0}")]
    Http(u16),
    #[error("{0}")]
    Transport(String),
    #[error("Failed to decode Kitsu response: {0}")]
    Decode(String),
    #[error("Unknown status")]
    UnknownStatus(String),
    #[error("Missing {0} data from Kitsu")]
    MissingMediaData(&'static str),
    #[error("Could not find {0}")]
    NotFound(&'static str),
    #[error("Invalid Kitsu timestamp: {0}")]
    BadDate(String),
    #[error("Missing User data from Kitsu")]
    NoUser,
    #[error("Secret storage is unavailable")]
    SecretStorage,
}

impl From<reqwest::Error> for KitsuError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(code) => KitsuError::Http(code.as_u16()),
            None => KitsuError::Transport(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for KitsuError {
    fn from(e: serde_json::Error) -> Self {
        KitsuError::Decode(e.to_string())
    }
}

impl From<SecretError> for KitsuError {
    fn from(_: SecretError) -> Self {
        KitsuError::SecretStorage
    }
}

impl KitsuError {
    /// The stable key of this message, for localisation later (T-30a-024). The English text stays in `Display`.
    pub fn key(&self) -> &'static str {
        match self {
            KitsuError::NotAuthenticated => "kitsu.not_authenticated",
            KitsuError::ReauthRequired => "kitsu.reauth_required",
            KitsuError::Http(_) => "kitsu.http_error",
            KitsuError::Transport(_) => "kitsu.transport",
            KitsuError::Decode(_) => "kitsu.decode",
            KitsuError::UnknownStatus(_) => "kitsu.unknown_status",
            KitsuError::MissingMediaData(_) => "kitsu.missing_media_data",
            KitsuError::NotFound(_) => "kitsu.not_found",
            KitsuError::BadDate(_) => "kitsu.bad_date",
            KitsuError::NoUser => "kitsu.no_user",
            KitsuError::SecretStorage => "kitsu.secret_storage",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_30a_024_user_visible_texts_match_the_source() {
        assert_eq!(
            KitsuError::NotAuthenticated.to_string(),
            "Not authenticated with Kitsu"
        );
        assert_eq!(KitsuError::Http(503).to_string(), "HTTP error 503");
        assert_eq!(
            KitsuError::UnknownStatus("x".into()).to_string(),
            "Unknown status"
        );
        assert_eq!(
            KitsuError::MissingMediaData("Anime").to_string(),
            "Missing Anime data from Kitsu"
        );
        assert_eq!(
            KitsuError::MissingMediaData("Manga").to_string(),
            "Missing Manga data from Kitsu"
        );
        assert_eq!(
            KitsuError::NotFound("anime").to_string(),
            "Could not find anime"
        );
        assert_eq!(
            KitsuError::NoUser.to_string(),
            "Missing User data from Kitsu"
        );
    }

    #[test]
    fn t_30a_024_every_message_has_a_distinct_stable_key() {
        let all = [
            KitsuError::NotAuthenticated,
            KitsuError::ReauthRequired,
            KitsuError::Http(0),
            KitsuError::Transport(String::new()),
            KitsuError::Decode(String::new()),
            KitsuError::UnknownStatus(String::new()),
            KitsuError::MissingMediaData("Manga"),
            KitsuError::NotFound("manga"),
            KitsuError::BadDate(String::new()),
            KitsuError::NoUser,
            KitsuError::SecretStorage,
        ];
        let mut keys: Vec<_> = all.iter().map(KitsuError::key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), all.len());
    }
}
