//! `StorageManager` (`StorageManager.kt`): the base directory, the folders under it, and the change counter that
//! replaces the Kotlin shared flow (T-09a-049).

use kuro_platform::kind::Kind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use kuro_platform::paths::{path_bytes, path_from_bytes};
use kuro_prefs::pref::Pref;
use kuro_prefs::store::PrefStore;
use tokio::sync::watch;

use super::{BACKUPS_PATH, FONTS_PATH, LOCAL_PATH, MPV_CONFIG_PATH, SCRIPTS_PATH, SCRIPT_OPTS_PATH, SHADERS_PATH, TREE};
use super::kind_folder;
use crate::error::DomainError;

/// The preference that holds the base directory (`Preference.appStateKey("storage_dir")`).
pub const STORAGE_DIR_KEY: &str = "__APP_STATE_storage_dir";

/// Owns the base directory. The stored value is the base64 of the path bytes, so any OS path round-trips (E-STO-12).
/// An empty value means the default base given at `open`.
pub struct StorageManager {
    pref: Pref<String>,
    default_base: PathBuf,
    changes: watch::Sender<u64>,
    set_lock: Mutex<()>,
}

impl StorageManager {
    /// Opens the manager over the stored base. Creates nothing: the tree is made by `ensure_tree` or by `set_base`
    /// (Q-STO-2), and by an accessor on first use.
    pub fn open(store: &PrefStore, default_base: PathBuf) -> Self {
        Self {
            pref: store.pref(STORAGE_DIR_KEY, String::new()),
            default_base,
            changes: watch::channel(0).0,
            set_lock: Mutex::new(()),
        }
    }

    /// The configured base path, whether or not it exists. A stored value that does not decode gives the default.
    fn configured(&self) -> PathBuf {
        let raw = self.pref.get();
        if raw.is_empty() {
            return self.default_base.clone();
        }
        match STANDARD.decode(&raw).ok().and_then(path_from_bytes) {
            Some(path) => path,
            None => {
                tracing::warn!("stored base storage directory does not decode; using the default");
                self.default_base.clone()
            }
        }
    }

    /// Receives the change counter. Each real base change adds one; a late subscriber sees the current count.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// The base directory. A base that is missing or not a directory is `StorageUnavailable` (Q-STO-3). The default
    /// base is created on first use, so the user is never asked for a folder.
    pub fn base(&self) -> Result<PathBuf, DomainError> {
        let base = self.configured();
        if base == self.default_base {
            std::fs::create_dir_all(&base)?;
        }
        if base.is_dir() {
            Ok(base)
        } else {
            Err(DomainError::StorageUnavailable)
        }
    }

    /// Points the base at `path`: checks it, creates the folder tree, then stores it and notifies once. Setting the
    /// path that is already in use changes nothing (TC-STO-12). Calls are serialised (E-STO-14).
    pub fn set_base(&self, path: &Path) -> Result<(), DomainError> {
        let _guard = self.set_lock.lock().unwrap_or_else(PoisonError::into_inner);
        if path.to_string_lossy().starts_with("content://") {
            return Err(DomainError::StorageUnavailable);
        }
        if !path.is_dir() {
            return Err(DomainError::Invalid("not a directory"));
        }
        if self.configured() == path {
            return Ok(());
        }
        create_tree(path)?;
        let _ = self.pref.set(STANDARD.encode(path_bytes(path)));
        self.changes.send_modify(|count| *count += 1);
        Ok(())
    }

    /// Creates the folder tree under the base. The first-run path calls this so the tree exists at startup (Q-STO-2).
    /// The default base is created here when it is missing; a configured base that is gone stays unavailable.
    pub fn ensure_tree(&self) -> Result<(), DomainError> {
        create_tree(&self.base()?)
    }

    /// The folder `rel` under the base, created if missing.
    fn dir(&self, rel: &[&str]) -> Result<PathBuf, DomainError> {
        let path = join_all(self.base()?, rel);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// The backups folder under the base, created if missing.
    pub fn backups_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[BACKUPS_PATH])
    }

    /// The downloads folder of one kind (`Anime` or `Manga`), created if missing.
    pub fn downloads_dir(&self, kind: Kind) -> Result<PathBuf, DomainError> {
        self.dir(&[kind_folder(kind)])
    }

    /// The same folder as [`Self::downloads_dir`], under the configured base, without touching the disk. The download
    /// indexes and readers name it this way, so a chosen base that is unplugged reads as empty instead of as another
    /// folder.
    pub fn downloads_path(&self, kind: Kind) -> PathBuf {
        join_all(self.configured(), &[kind_folder(kind)])
    }

    /// The local manga source folder, `Local/Manga`.
    pub fn local_manga_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[LOCAL_PATH, kind_folder(Kind::Manga)])
    }

    /// The local anime source folder, `Local/Anime`.
    pub fn local_anime_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[LOCAL_PATH, kind_folder(Kind::Anime)])
    }

    pub fn mpv_config_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[MPV_CONFIG_PATH])
    }

    pub fn fonts_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[MPV_CONFIG_PATH, FONTS_PATH])
    }

    pub fn scripts_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[MPV_CONFIG_PATH, SCRIPTS_PATH])
    }

    pub fn script_opts_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[MPV_CONFIG_PATH, SCRIPT_OPTS_PATH])
    }

    pub fn shaders_dir(&self) -> Result<PathBuf, DomainError> {
        self.dir(&[MPV_CONFIG_PATH, SHADERS_PATH])
    }
}

fn join_all(mut path: PathBuf, parts: &[&str]) -> PathBuf {
    for part in parts {
        path.push(part);
    }
    path
}

/// Creates every folder of `TREE` under `base`. A read-only or unwritable base fails with `Io`.
fn create_tree(base: &Path) -> Result<(), DomainError> {
    for parts in TREE {
        std::fs::create_dir_all(join_all(base.to_path_buf(), parts))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn manager(default: &Path) -> (PrefStore, StorageManager) {
        let store = PrefStore::in_memory();
        let manager = StorageManager::open(&store, default.to_path_buf());
        (store, manager)
    }

    fn tree_exists(base: &Path) -> bool {
        TREE.iter().all(|parts| join_all(base.to_path_buf(), parts).is_dir())
    }

    #[test]
    fn set_base_creates_the_exact_folder_tree() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        let base = dir.path().join("base");
        std::fs::create_dir(&base).expect("base");
        m.set_base(&base).expect("set base");
        assert!(tree_exists(&base));
        let mut top: Vec<String> = std::fs::read_dir(&base)
            .expect("read base")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        top.sort();
        assert_eq!(top, ["Anime", "Backups", "Local", "Manga", "mpv-config"]);
        assert!(base.join("Local/Anime").is_dir() && base.join("Local/Manga").is_dir());
    }

    #[test]
    fn open_creates_nothing_and_ensure_tree_creates_the_default_base() {
        let dir = tempfile::tempdir().expect("temp dir");
        let default = dir.path().join("kurobayomi");
        let (_store, m) = manager(&default);
        assert!(!default.exists(), "open does not create the default base");
        m.ensure_tree().expect("first run");
        assert!(tree_exists(&default));
    }

    #[test]
    fn the_default_base_is_created_on_first_use_without_asking() {
        let dir = tempfile::tempdir().expect("temp dir");
        let default = dir.path().join("Kurobayomi");
        let (_store, m) = manager(&default);
        let downloads = m.downloads_dir(Kind::Manga).expect("first use creates the default base");
        assert!(downloads.is_dir() && downloads.starts_with(&default));
        assert_eq!(m.downloads_path(Kind::Manga), downloads);
        assert_eq!(m.downloads_path(Kind::Anime), default.join("Anime"));
        assert_eq!(m.backups_dir().expect("backups"), default.join("Backups"));
    }

    #[test]
    fn an_accessor_creates_a_folder_that_went_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        m.ensure_tree().expect("tree");
        std::fs::remove_dir(dir.path().join("Anime")).expect("remove");
        let downloads = m.downloads_dir(Kind::Anime).expect("accessor");
        assert!(downloads.is_dir());
        assert!(m.local_anime_dir().expect("anime").ends_with("Local/Anime"));
        assert!(m.local_manga_dir().expect("manga").ends_with("Local/Manga"));
        assert!(m.fonts_dir().expect("fonts").ends_with("mpv-config/fonts"));
    }

    #[test]
    fn a_base_that_disappeared_is_storage_unavailable_everywhere() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        let base = dir.path().join("usb");
        std::fs::create_dir(&base).expect("base");
        m.set_base(&base).expect("set");
        std::fs::remove_dir_all(&base).expect("unplug");
        assert!(matches!(m.base(), Err(DomainError::StorageUnavailable)));
        assert!(matches!(m.downloads_dir(Kind::Manga), Err(DomainError::StorageUnavailable)));
        assert_eq!(m.downloads_path(Kind::Manga), base.join("Manga"), "the path still names the chosen folder");
        assert!(matches!(m.ensure_tree(), Err(DomainError::StorageUnavailable)));
        assert!(!base.exists(), "no folder is created for a missing base");
    }

    #[test]
    fn setting_the_same_path_notifies_nothing_and_each_change_bumps_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).expect("a");
        std::fs::create_dir_all(&b).expect("b");
        let mut rx = m.subscribe();
        assert_eq!(*rx.borrow(), 0);
        m.set_base(&a).expect("a");
        assert!(rx.has_changed().expect("open"));
        rx.mark_unchanged();
        m.set_base(&a).expect("a again");
        assert!(!rx.has_changed().expect("open"), "same path is not a change");
        m.set_base(&b).expect("b");
        assert!(rx.has_changed().expect("open"));
        assert_eq!(*rx.borrow_and_update(), 2);
    }

    #[test]
    fn a_late_subscriber_sees_the_current_count() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        let a = dir.path().join("a");
        std::fs::create_dir(&a).expect("a");
        m.set_base(&a).expect("set");
        assert_eq!(*m.subscribe().borrow(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_base_round_trips_through_the_preference() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let (store, m) = manager(dir.path());
        let odd = dir.path().join(std::ffi::OsString::from_vec(b"odd\xff".to_vec()));
        std::fs::create_dir(&odd).expect("odd");
        m.set_base(&odd).expect("set");
        assert_eq!(StorageManager::open(&store, dir.path().to_path_buf()).base().expect("base"), odd);
    }

    #[test]
    fn a_file_or_a_content_uri_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        let file = dir.path().join("plain");
        std::fs::write(&file, b"x").expect("file");
        assert!(matches!(m.set_base(&file), Err(DomainError::Invalid("not a directory"))));
        assert!(matches!(
            m.set_base(Path::new("content://com.android.externalstorage/tree/primary")),
            Err(DomainError::StorageUnavailable)
        ));
    }

    #[test]
    fn a_blocked_folder_is_an_io_error_not_a_panic() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        m.ensure_tree().expect("tree");
        let downloads = dir.path().join("Manga");
        std::fs::remove_dir(&downloads).expect("remove");
        std::fs::write(&downloads, b"in the way").expect("file");
        assert!(matches!(m.downloads_dir(Kind::Manga), Err(DomainError::Io(_))));
    }

    #[test]
    fn concurrent_set_base_calls_are_all_applied_once_each() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (_store, m) = manager(dir.path());
        let m = Arc::new(m);
        let paths: Vec<PathBuf> = (0..8).map(|i| dir.path().join(format!("p{i}"))).collect();
        for p in &paths {
            std::fs::create_dir(p).expect("dir");
        }
        let handles: Vec<_> = paths
            .iter()
            .cloned()
            .map(|p| {
                let m = Arc::clone(&m);
                std::thread::spawn(move || m.set_base(&p))
            })
            .collect();
        for h in handles {
            h.join().expect("no panic").expect("set base");
        }
        assert_eq!(*m.subscribe().borrow(), 8);
    }
}
