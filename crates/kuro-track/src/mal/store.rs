//! Persistence. The token lives in the
//! platform secret store (which falls back to its own encrypted file). The username and the auth-expired flag
//! live in a small TOML file in the config directory. The token never goes into that file.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kuro_platform::secrets::SecretStore;
use serde::{Deserialize, Serialize};

use super::error::MalError;
use super::token::MalToken;

/// Secret store service name. The item is the one account for the MAL token.
pub const TOKEN_SERVICE: &str = "kurobayomi";
pub const TOKEN_ACCOUNT: &str = "myanimelist/oauth-token";

/// Where the MAL token is kept. Clearing removes the item, and nothing ever writes the text `null`.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<MalToken>, MalError>;
    fn save(&self, token: &MalToken) -> Result<(), MalError>;
    fn clear(&self) -> Result<(), MalError>;
}

/// The token store over the platform secret store.
pub struct SecretTokenStore {
    secrets: Arc<dyn SecretStore>,
}

impl SecretTokenStore {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }
}

impl TokenStore for SecretTokenStore {
    fn load(&self) -> Result<Option<MalToken>, MalError> {
        let text = self
            .secrets
            .get(TOKEN_SERVICE, TOKEN_ACCOUNT)
            .map_err(|e| MalError::Store(e.to_string()))?;
        let Some(text) = text else { return Ok(None) };
        match serde_json::from_str(&text) {
            Ok(token) => Ok(Some(token)),
            Err(e) => {
                tracing::error!(error = %e, "stored MAL token is corrupt, treating as logged out");
                Ok(None)
            }
        }
    }

    fn save(&self, token: &MalToken) -> Result<(), MalError> {
        let text = serde_json::to_string(token)?;
        self.secrets
            .set(TOKEN_SERVICE, TOKEN_ACCOUNT, &text)
            .map_err(|e| MalError::Store(e.to_string()))
    }

    fn clear(&self) -> Result<(), MalError> {
        self.secrets
            .delete(TOKEN_SERVICE, TOKEN_ACCOUNT)
            .map_err(|e| MalError::Store(e.to_string()))
    }
}

/// Environment variable that holds the MAL client id (T-29a-040). Wins over the config file.
pub const CLIENT_ID_ENV: &str = "KUROBAYOMI_MAL_CLIENT_ID";

/// The non-secret settings: the username, the auth-expired flag and the user's client id.
/// The client id is configuration, not a secret.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct SettingsFile {
    #[serde(default)]
    username: String,
    #[serde(default)]
    auth_expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redirect_port: Option<u16>,
    /// The PKCE verifier of a login that started and has not finished (T-29a-038).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_verifier: Option<String>,
}

/// The client id: the environment value when set and not blank, else the config file value. Blank values count
/// as unset, so a login without an id is refused with `NoClientId`.
pub fn resolve_client_id(from_file: Option<&str>, from_env: Option<&str>) -> Option<String> {
    [from_env, from_file]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|id| !id.is_empty())
        .map(str::to_owned)
}

/// The MAL settings file, `tracking/myanimelist.toml` under the config directory.
pub struct MalSettings {
    path: PathBuf,
    file: Mutex<SettingsFile>,
}

impl MalSettings {
    /// Reads the file. A missing file is the logged-out default. A file that does not parse is logged and
    /// replaced by the default on the next write.
    pub fn open(config_dir: &Path) -> Result<Self, MalError> {
        let path = config_dir.join("tracking").join("myanimelist.toml");
        let file = match fs::read_to_string(&path) {
            Ok(text) => toml_edit::de::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "MAL settings file does not parse, using defaults");
                SettingsFile::default()
            }),
            Err(e) if e.kind() == ErrorKind::NotFound => SettingsFile::default(),
            Err(e) => return Err(MalError::Store(e.to_string())),
        };
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn username(&self) -> String {
        self.lock().username.clone()
    }

    pub fn set_username(&self, name: &str) -> Result<(), MalError> {
        self.update(|f| f.username = name.to_owned())
    }

    /// The client id stored in the file, if any. The environment variable is checked by the caller.
    pub fn client_id(&self) -> Option<String> {
        self.lock().client_id.clone()
    }

    /// The loopback port stored in the file (`redirect_port`), if any. The default is applied by the caller.
    pub fn redirect_port(&self) -> Option<u16> {
        self.lock().redirect_port
    }

    pub fn auth_expired(&self) -> bool {
        self.lock().auth_expired
    }

    /// The verifier of the login in progress. It is kept in the file, so a restart between the browser step and the
    /// code exchange still completes the login (T-29a-038).
    pub fn pending_verifier(&self) -> Option<String> {
        self.lock().pending_verifier.clone()
    }

    pub fn set_pending_verifier(&self, verifier: Option<&str>) -> Result<(), MalError> {
        self.update(|f| f.pending_verifier = verifier.map(str::to_owned))
    }

    pub fn set_auth_expired(&self, value: bool) -> Result<(), MalError> {
        self.update(|f| f.auth_expired = value)
    }

    fn lock(&self) -> MutexGuard<'_, SettingsFile> {
        self.file.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the changed file first, and only then changes memory, so a failed write is not hidden.
    fn update(&self, change: impl FnOnce(&mut SettingsFile)) -> Result<(), MalError> {
        let mut guard = self.lock();
        let mut next = guard.clone();
        change(&mut next);
        let text =
            toml_edit::ser::to_string_pretty(&next).map_err(|e| MalError::Store(e.to_string()))?;
        write_private(&self.path, &text)?;
        *guard = next;
        Ok(())
    }
}

/// Writes `text` with the directory at mode 0700 and the file at mode 0600 on Unix.
fn write_private(path: &Path, text: &str) -> Result<(), MalError> {
    let store_err = |e: std::io::Error| MalError::Store(e.to_string());
    if let Some(dir) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir).map_err(store_err)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(store_err)?;
    file.write_all(text.as_bytes()).map_err(store_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_29a_039_the_redirect_port_comes_from_the_file_or_the_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("tracking").join("myanimelist.toml");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(&file, "redirect_port = 9001\n").expect("write");
        let settings = MalSettings::open(dir.path()).expect("opens");
        assert_eq!(settings.redirect_port(), Some(9001));
        let empty = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            MalSettings::open(empty.path())
                .expect("opens")
                .redirect_port(),
            None
        );
    }

    #[test]
    fn t_29a_040_the_environment_wins_and_blank_values_are_unset() {
        assert_eq!(
            resolve_client_id(Some("file"), Some("env")),
            Some("env".into())
        );
        assert_eq!(
            resolve_client_id(Some("file"), Some("  ")),
            Some("file".into())
        );
        assert_eq!(resolve_client_id(Some(" "), None), None);
        assert_eq!(resolve_client_id(None, None), None);
    }

    #[test]
    fn t_29a_040_the_client_id_survives_a_username_write_and_is_not_a_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("tracking").join("myanimelist.toml");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(&file, "client_id = \"from-config\"\n").expect("write");

        let settings = MalSettings::open(dir.path()).expect("opens");
        assert_eq!(settings.client_id().as_deref(), Some("from-config"));
        settings.set_username("ann").expect("writes");

        let reopened = MalSettings::open(dir.path()).expect("reopens");
        assert_eq!(reopened.client_id().as_deref(), Some("from-config"));
        assert_eq!(reopened.username(), "ann");
        let text = std::fs::read_to_string(&file).expect("reads");
        assert!(!text.contains("access_token"));
    }
}
