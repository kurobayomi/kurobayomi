//! The OAuth clients of Shikimori and Bangumi from the user's configuration (`[track.clients]`, T-31a-039, D-OPEN-1).
//! Shikimori falls back to Kurobayomi's own application when the build carries it, so every user of a released build
//! can log in; a configured table replaces it. Bangumi has no built-in client.

use std::path::Path;

use serde::Deserialize;

use super::api::OAuthClient;

/// Kurobayomi's Shikimori application (redirect `kurobayomi://shikimori-auth`, scope `user_rates`), the way AniList and
/// MyAnimeList have a built-in client id. Shikimori's code exchange needs the secret too, so neither value is in the
/// source: the release workflows bake them in from the repository secrets `SHIKIMORI_CLIENT_ID` and
/// `SHIKIMORI_CLIENT_SECRET`. A build without them (a local or forked build) has no built-in client.
const BUILT_IN_SHIKIMORI_CLIENT_ID: Option<&str> = option_env!("KB_SHIKIMORI_CLIENT_ID");
const BUILT_IN_SHIKIMORI_CLIENT_SECRET: Option<&str> = option_env!("KB_SHIKIMORI_CLIENT_SECRET");

/// The client of an id and a secret baked into the build. `None` unless both are there and non-empty (a workflow
/// without the repository secrets sets them to an empty string).
fn baked_client(id: Option<&str>, secret: Option<&str>) -> Option<OAuthClient> {
    RawClient {
        client_id: id.unwrap_or_default().trim().to_owned(),
        client_secret: secret.unwrap_or_default().trim().to_owned(),
    }
    .into_client()
}

/// The built-in Shikimori client of this build, if it carries one.
pub fn built_in_shikimori() -> Option<OAuthClient> {
    baked_client(BUILT_IN_SHIKIMORI_CLIENT_ID, BUILT_IN_SHIKIMORI_CLIENT_SECRET)
}

#[derive(Debug, Default, Deserialize)]
struct RawClient {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
}

impl RawClient {
    /// A client needs both values. A missing or empty one leaves the tracker unconfigured, so login is refused.
    fn into_client(self) -> Option<OAuthClient> {
        (!self.client_id.is_empty() && !self.client_secret.is_empty()).then(|| OAuthClient {
            client_id: self.client_id,
            client_secret: self.client_secret,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawClients {
    #[serde(default)]
    shikimori: Option<RawClient>,
    #[serde(default)]
    bangumi: Option<RawClient>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTrack {
    #[serde(default)]
    clients: RawClients,
}

#[derive(Debug, Default, Deserialize)]
struct RawRoot {
    #[serde(default)]
    track: RawTrack,
}

/// `[track.clients]`: one table per tracker, each with `client_id` and `client_secret`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackClientConfig {
    pub shikimori: Option<OAuthClient>,
    pub bangumi: Option<OAuthClient>,
}

impl TrackClientConfig {
    /// The Shikimori client the app logs in with: the configured one, else the build's built-in one. `None` leaves
    /// Shikimori unconfigured, and its Settings row says so.
    pub fn shikimori_or_built_in(&self) -> Option<OAuthClient> {
        self.shikimori.clone().or_else(built_in_shikimori)
    }

    /// Reads `[track.clients]` from the text of `config.toml`. A file without the section has no clients, and unknown
    /// keys are ignored.
    pub fn from_toml(text: &str) -> Result<Self, toml_edit::de::Error> {
        let root: RawRoot = toml_edit::de::from_str(text)?;
        let clients = root.track.clients;
        Ok(Self {
            shikimori: clients.shikimori.and_then(RawClient::into_client),
            bangumi: clients.bangumi.and_then(RawClient::into_client),
        })
    }

    /// Reads `[track.clients]` from `config.toml` in `config_dir` (the file the preference store writes). A missing
    /// file has no clients. An unreadable or malformed file is logged and also has none, so both trackers stay
    /// unconfigured and their login rows say so (T-31a-037), instead of the app failing to start.
    pub fn load(config_dir: &Path) -> Self {
        let file = kuro_prefs::toml_backend::config_file(config_dir);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("cannot read {}: {e}", file.display());
                return Self::default();
            }
        };
        Self::from_toml(&text).unwrap_or_else(|e| {
            tracing::warn!("cannot read [track.clients] from {}: {e}", file.display());
            Self::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_31a_039_a_file_without_the_section_configures_no_client() {
        let c = TrackClientConfig::from_toml("[network]\nverbose_logging = false\n").unwrap();
        assert_eq!(c, TrackClientConfig::default());
    }

    #[test]
    fn t_31a_039_both_clients_load_from_their_tables() {
        let c = TrackClientConfig::from_toml(
            "[track.clients.shikimori]\nclient_id = \"s-id\"\nclient_secret = \"s-sec\"\n\
             [track.clients.bangumi]\nclient_id = \"b-id\"\nclient_secret = \"b-sec\"\n",
        )
        .unwrap();
        assert_eq!(
            c.shikimori,
            Some(OAuthClient {
                client_id: "s-id".into(),
                client_secret: "s-sec".into()
            })
        );
        assert_eq!(c.bangumi.unwrap().client_secret, "b-sec");
    }

    #[test]
    fn t_31a_039_an_empty_or_half_table_leaves_that_tracker_unconfigured() {
        let c = TrackClientConfig::from_toml(
            "[track.clients.shikimori]\nclient_id = \"s-id\"\nclient_secret = \"\"\n\
             [track.clients.bangumi]\nclient_id = \"b-id\"\n",
        )
        .unwrap();
        assert_eq!(c, TrackClientConfig::default());
    }

    #[test]
    fn t_31a_039_load_reads_the_file_in_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            TrackClientConfig::load(dir.path()),
            TrackClientConfig::default(),
            "no file"
        );
        let file = kuro_prefs::toml_backend::config_file(dir.path());
        std::fs::write(
            &file,
            "[track.clients.bangumi]\nclient_id = \"b\"\nclient_secret = \"s\"\n",
        )
        .unwrap();
        let c = TrackClientConfig::load(dir.path());
        assert_eq!(c.bangumi.unwrap().client_id, "b");
        assert!(c.shikimori.is_none());
        std::fs::write(&file, "[track\n").unwrap();
        assert_eq!(
            TrackClientConfig::load(dir.path()),
            TrackClientConfig::default(),
            "malformed file"
        );
    }

    #[test]
    fn t_31a_039_the_preference_store_keeps_the_clients_when_it_saves() {
        use kuro_prefs::pref::Pref;
        use kuro_prefs::store::PrefStore;
        use kuro_prefs::toml_backend::{TomlBackend, config_file};

        let config = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::write(
            config_file(config.path()),
            "[track.clients.shikimori]\nclient_id = \"s-id\"\nclient_secret = \"s-sec\"\n",
        )
        .unwrap();
        let store = PrefStore::new(Box::new(TomlBackend::open(config.path(), state.path()).unwrap())).unwrap();
        // A write from the app rewrites config.toml.
        let flag: Pref<bool> = Pref::new(store.clone(), "pref_auto_update_manga_sync_key", false);
        flag.set(true).unwrap();
        let c = TrackClientConfig::load(config.path());
        assert_eq!(c.shikimori.unwrap().client_id, "s-id");
    }

    #[test]
    fn without_a_table_shikimori_uses_the_built_in_client_and_a_table_replaces_it() {
        let none = TrackClientConfig::default();
        assert_eq!(none.shikimori_or_built_in(), built_in_shikimori(), "whatever this build carries");
        let own = TrackClientConfig::from_toml(
            "[track.clients.shikimori]\nclient_id = \"s-id\"\nclient_secret = \"s-sec\"\n",
        )
        .unwrap();
        assert_eq!(own.shikimori_or_built_in().unwrap().client_id, "s-id");
        assert!(own.bangumi.is_none(), "Bangumi has no built-in client");
    }

    #[test]
    fn a_built_in_client_needs_both_values_baked_in() {
        assert_eq!(baked_client(None, None), None, "a local build");
        assert_eq!(baked_client(Some(""), Some("")), None, "a workflow without the repository secrets");
        assert_eq!(baked_client(Some("id"), Some(" ")), None);
        assert_eq!(
            baked_client(Some("id"), Some("secret")),
            Some(OAuthClient {
                client_id: "id".into(),
                client_secret: "secret".into()
            })
        );
    }

    #[test]
    fn t_31a_039_a_malformed_file_is_an_error() {
        assert!(TrackClientConfig::from_toml("[track\n").is_err());
    }
}
