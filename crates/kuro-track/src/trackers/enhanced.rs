//! Shared contract of the enhanced trackers (`EnhancedMangaTracker.kt`, `EnhancedAnimeTracker.kt`): the entry
//! reference the bind path passes in, the error type, the source-identity helpers, and the login flag that replaces
//! the dummy credentials (T-33a-005, T-33a-007, T-33a-008).

use std::sync::Arc;
#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

use kuro_prefs::store::PrefStore;
use tokio::sync::watch;

use crate::error::{TrackError, redact};
use crate::messages::MsgKey;
use crate::model::TrackerId;

/// The entry a tracker matches. `url` is the source's `manga.url` or `anime.url`; trackers match by it, never by
/// title (P-12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRef {
    /// `manga_id` or `anime_id`, set by the bind path.
    pub id: i64,
    /// Kurobayomi source id (area 17).
    pub source_id: i64,
    /// The source's `manga.url` or `anime.url`.
    pub url: String,
    /// For logs only.
    pub title: String,
}

/// Failure of an enhanced tracker. The English text is for logs; the UI shows the key from [`message_key`].
///
/// [`message_key`]: EnhancedError::message_key
#[derive(Debug, thiserror::Error)]
pub enum EnhancedError {
    #[error("Unauthorized / api key not valid")]
    Unauthorized,
    #[error("Error fetching JWT token")]
    ServerError,
    #[error("source not installed")]
    SourceNotInstalled,
    #[error("source not configured")]
    SourceNotConfigured,
    #[error("search is not supported for this tracker")]
    SearchUnsupported,
    #[error("tracking URL has no fragment")]
    MissingFragment,
    #[error("no chapter {0} on the server")]
    ChapterNotOnServer(String),
    #[error("no episode {0} on the server")]
    EpisodeNotFound(String),
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl EnhancedError {
    /// The `track.enhanced.*` message key shown to the user. `None` for the
    /// variants that only reach the log.
    pub fn message_key(&self) -> Option<&'static str> {
        match self {
            EnhancedError::Unauthorized => Some("track.enhanced.unauthorized"),
            EnhancedError::ServerError => Some("track.enhanced.server_error"),
            EnhancedError::SourceNotConfigured => Some("track.enhanced.login_required"),
            EnhancedError::SearchUnsupported => Some("track.enhanced.search_unsupported"),
            EnhancedError::MissingFragment => Some("track.enhanced.missing_fragment"),
            EnhancedError::ChapterNotOnServer(_) => Some("track.enhanced.chapter_missing"),
            EnhancedError::SourceNotInstalled
            | EnhancedError::EpisodeNotFound(_)
            | EnhancedError::Decode(_)
            | EnhancedError::Http(_) => None,
        }
    }
}

impl From<EnhancedError> for TrackError {
    /// `Unauthorized` and `Http` have their own variants. A variant with a message key becomes `TrackError::Key`, so
    /// the toast shows the localised text; the rest (log-only variants) stay `Message` (T-33a-005, T-33a-022).
    fn from(e: EnhancedError) -> Self {
        match e {
            EnhancedError::Unauthorized => TrackError::Unauthorised,
            EnhancedError::Http(h) => TrackError::Http(redact(&h.to_string())),
            other => match other.message_key() {
                Some(key) => {
                    let arg = match &other {
                        EnhancedError::ChapterNotOnServer(n) => n.as_str(),
                        _ => "",
                    };
                    TrackError::Key(MsgKey::enhanced(key, arg))
                }
                None => TrackError::Message(other.to_string()),
            },
        }
    }
}

/// True when `source_identity` is one of the accepted identities (exact string equality, E-96).
pub fn accepts(accepted: &[&str], source_identity: &str) -> bool {
    accepted.contains(&source_identity)
}

/// Kotlin `isTrackFrom`: the remote URL is the entry URL and the entry's source is accepted.
pub fn is_track_from(
    track_remote_url: &str,
    entry_url: &str,
    source_identity: Option<&str>,
    accepted: &[&str],
) -> bool {
    track_remote_url == entry_url && source_identity.is_some_and(|s| accepts(accepted, s))
}

/// Kotlin `migrateTrack`: the new entry URL becomes the remote URL only when the new source is accepted.
pub fn migrate_remote_url(
    new_source_identity: &str,
    accepted: &[&str],
    new_entry_url: &str,
) -> Option<String> {
    accepts(accepted, new_source_identity).then(|| new_entry_url.to_owned())
}

/// The preference key of the login flag (Proposal).
pub fn enabled_key(id: TrackerId) -> String {
    format!("enhanced_tracker_enabled_{}", id.0)
}

/// Boolean preference storage for the login flags.
pub trait FlagStore: Send + Sync {
    fn read_flag(&self, key: &str) -> bool;
    fn write_flag(&self, key: &str, value: bool) -> Result<(), TrackError>;
}

impl FlagStore for PrefStore {
    fn read_flag(&self, key: &str) -> bool {
        self.pref(key.to_owned(), false).get()
    }

    fn write_flag(&self, key: &str, value: bool) -> Result<(), TrackError> {
        Ok(self.pref(key.to_owned(), false).set(value)?)
    }
}

/// Login state of an enhanced tracker. No secret is stored; the flag alone says the tracker is logged in.
pub struct LoginFlag {
    key: String,
    store: Arc<dyn FlagStore>,
    tx: watch::Sender<bool>,
}

impl LoginFlag {
    pub fn new(id: TrackerId, store: Arc<dyn FlagStore>) -> Self {
        let key = enabled_key(id);
        let (tx, _) = watch::channel(store.read_flag(&key));
        Self { key, store, tx }
    }

    /// Reads the flag on every call (no cache, P-7).
    pub fn is_logged_in(&self) -> bool {
        self.store.read_flag(&self.key)
    }

    pub fn login_state(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }

    /// Replaces the dummy `"user"`/`"pass"` credentials of the Kotlin `loginNoop()`.
    pub fn login_noop(&self) -> Result<(), TrackError> {
        self.write(true)
    }

    pub fn logout(&self) -> Result<(), TrackError> {
        self.write(false)
    }

    fn write(&self, value: bool) -> Result<(), TrackError> {
        self.store.write_flag(&self.key, value)?;
        self.tx.send_if_modified(|state| {
            let changed = *state != value;
            *state = value;
            changed
        });
        Ok(())
    }
}

/// In-memory [`FlagStore`] for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemFlags {
    pub values: Mutex<HashMap<String, bool>>,
}

#[cfg(test)]
impl FlagStore for MemFlags {
    fn read_flag(&self, key: &str) -> bool {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .copied()
            .unwrap_or(false)
    }

    fn write_flag(&self, key: &str, value: bool) -> Result<(), TrackError> {
        self.values.lock().unwrap().insert(key.to_owned(), value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KAVITA_ACCEPTED: &[&str] = &["kavita/all"];
    const KOMGA_ACCEPTED: &[&str] = &["komga/all"];

    #[test]
    fn t_33a_007_a_different_source_identity_is_not_accepted() {
        assert!(!accepts(KOMGA_ACCEPTED, "kavita/all"));
        assert!(accepts(KAVITA_ACCEPTED, "kavita/all"));
    }

    #[test]
    fn t_33a_007_is_track_from_needs_the_same_url_and_an_accepted_source() {
        assert!(is_track_from(
            "https://h/a",
            "https://h/a",
            Some("kavita/all"),
            KAVITA_ACCEPTED
        ));
        assert!(!is_track_from(
            "https://h/a",
            "https://h/b",
            Some("kavita/all"),
            KAVITA_ACCEPTED
        ));
        assert!(!is_track_from(
            "https://h/a",
            "https://h/a",
            Some("komga/all"),
            KAVITA_ACCEPTED
        ));
        assert!(!is_track_from(
            "https://h/a",
            "https://h/a",
            None,
            KAVITA_ACCEPTED
        ));
    }

    #[test]
    fn t_33a_007_migration_copies_the_url_only_for_an_accepted_source() {
        let copied = migrate_remote_url("kavita/all", KAVITA_ACCEPTED, "https://h/new");
        assert_eq!(copied.as_deref(), Some("https://h/new"));
        assert_eq!(
            migrate_remote_url("komga/all", KAVITA_ACCEPTED, "https://h/new"),
            None
        );
    }

    #[test]
    fn t_33a_005_enhanced_errors_map_to_track_errors_without_secrets() {
        assert_eq!(
            EnhancedError::SourceNotConfigured.message_key(),
            Some("track.enhanced.login_required")
        );
        assert_eq!(
            EnhancedError::EpisodeNotFound("3".into()).message_key(),
            None
        );
        let log_only: TrackError = EnhancedError::EpisodeNotFound("3".into()).into();
        assert!(
            matches!(log_only, TrackError::Message(ref m) if m == "no episode 3 on the server")
        );
    }

    #[test]
    fn t_33a_005_unauthorized_maps_to_unauthorised_and_its_toast_is_the_key_text() {
        let e: TrackError = EnhancedError::Unauthorized.into();
        assert!(matches!(e, TrackError::Unauthorised));
        assert_eq!(
            e.toast("Jellyfin"),
            MsgKey::enhanced("track.enhanced.unauthorized", "")
        );
    }

    #[tokio::test]
    async fn t_33a_005_http_maps_to_the_http_variant_with_the_query_redacted() {
        let err = reqwest::Client::new()
            .get("http://127.0.0.1:1/x?api_key=SECRET9")
            .send()
            .await
            .unwrap_err();
        let e: TrackError = EnhancedError::Http(err).into();
        let TrackError::Http(text) = e else {
            panic!("expected the Http variant");
        };
        assert!(!text.contains("SECRET9"), "{text}");
    }

    #[test]
    fn t_33a_022_a_missing_login_reaches_the_toast_as_the_key_never_as_raw_english() {
        let e: TrackError = EnhancedError::SourceNotConfigured.into();
        assert!(
            matches!(e, TrackError::Key(MsgKey::Enhanced { key, .. }) if key == "track.enhanced.login_required")
        );
        let toast = e.toast("Jellyfin");
        assert_eq!(toast, MsgKey::enhanced("track.enhanced.login_required", ""));
        assert!(!matches!(
            toast,
            MsgKey::TrackerError { .. } | MsgKey::Raw(_)
        ));
    }

    #[test]
    fn t_33a_022_search_unsupported_and_chapter_missing_use_their_keys() {
        let e: TrackError = EnhancedError::SearchUnsupported.into();
        assert_eq!(
            e.toast("Kavita"),
            MsgKey::enhanced("track.enhanced.search_unsupported", "")
        );
        let c: TrackError = EnhancedError::ChapterNotOnServer("12".into()).into();
        assert_eq!(
            crate::messages::english(&c.toast("Kavita")),
            "Chapter 12 is not on the server"
        );
    }

    #[test]
    fn t_33a_008_logout_clears_the_login_flag() {
        let store = Arc::new(MemFlags::default());
        let flag = LoginFlag::new(TrackerId::JELLYFIN, store.clone());
        flag.login_noop().unwrap();
        assert!(flag.is_logged_in());
        flag.logout().unwrap();
        assert!(!flag.is_logged_in());
    }

    #[test]
    fn t_33a_008_noop_login_sets_the_flag_and_stores_no_secret() {
        let store = Arc::new(MemFlags::default());
        let flag = LoginFlag::new(TrackerId::JELLYFIN, store.clone());
        let rx = flag.login_state();
        flag.login_noop().unwrap();
        assert!(*rx.borrow());
        let values = store.values.lock().unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values.get("enhanced_tracker_enabled_102"), Some(&true));
    }

    #[test]
    fn t_33a_008_login_state_is_read_from_the_store_on_start() {
        let store = Arc::new(MemFlags::default());
        store
            .write_flag("enhanced_tracker_enabled_102", true)
            .unwrap();
        let flag = LoginFlag::new(TrackerId::JELLYFIN, store);
        assert!(*flag.login_state().borrow());
    }
}
