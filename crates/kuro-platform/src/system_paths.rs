//! The app's directories on this system, from the platform's base directories (`directories` crate): the XDG
//! directories on Linux, the roaming and local application data folders on Windows.

use std::path::{Path, PathBuf};

use directories::{ProjectDirs, UserDirs};

use crate::paths::Paths;

/// The app's folder name under each base directory (`~/.config/kurobayomi` and so on), lowercase.
const APP_DIR: &str = if crate::channel::NIGHTLY { "kurobayomi-nightly" } else { "kurobayomi" };

/// Base directories that [`SystemPaths`] derives every other path from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    /// The user's home folder, which holds the app's default storage folder.
    pub home: PathBuf,
    /// The user's Downloads folder, where app updates are saved.
    pub downloads: PathBuf,
}

/// The user's Downloads folder: the platform's download directory when it names one, else `Downloads` in the home
/// folder.
pub fn downloads_folder(configured: Option<&Path>, home: &Path) -> PathBuf {
    configured.map_or_else(|| home.join("Downloads"), Path::to_path_buf)
}

/// Failure to resolve the base directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("no home directory: cannot resolve the app directories")]
    NoHome,
}

/// Paths derived from the platform's base directories under the app name `kurobayomi`.
#[derive(Debug, Clone)]
pub struct SystemPaths {
    roots: Roots,
}

impl SystemPaths {
    /// Builds paths from explicit roots. Tests pass directories under a temp dir.
    pub fn new(roots: Roots) -> Self {
        Self { roots }
    }

    /// Builds paths from the real environment. Computes paths only; creates nothing. Only Linux has a state
    /// directory; elsewhere the local (not roaming) data directory holds the state.
    pub fn system() -> Result<Self, PathError> {
        let project =
            ProjectDirs::from_path(PathBuf::from(APP_DIR)).ok_or(PathError::NoHome)?;
        let user = UserDirs::new().ok_or(PathError::NoHome)?;
        let downloads = downloads_folder(user.download_dir(), user.home_dir());
        let state = project.state_dir().unwrap_or(project.data_local_dir()).to_path_buf();
        Ok(Self::new(Roots {
            data: project.data_dir().to_path_buf(),
            config: project.config_dir().to_path_buf(),
            cache: project.cache_dir().to_path_buf(),
            state,
            home: user.home_dir().to_path_buf(),
            downloads,
        }))
    }
}

impl Paths for SystemPaths {
    fn data_dir(&self) -> PathBuf {
        self.roots.data.clone()
    }

    fn config_dir(&self) -> PathBuf {
        self.roots.config.clone()
    }

    fn state_dir(&self) -> PathBuf {
        self.roots.state.clone()
    }

    fn cache_dir(&self) -> PathBuf {
        self.roots.cache.clone()
    }

    fn log_dir(&self) -> PathBuf {
        self.roots.state.join("logs")
    }

    fn temp_dir(&self) -> PathBuf {
        self.roots.cache.join("tmp")
    }

    fn crash_dir(&self) -> PathBuf {
        self.roots.state.join("crashes")
    }

    fn user_home_dir(&self) -> PathBuf {
        self.roots.home.clone()
    }

    fn user_downloads_dir(&self) -> PathBuf {
        self.roots.downloads.clone()
    }

    fn covers_dir(&self) -> PathBuf {
        self.roots.cache.join("covers")
    }

    fn plugin_dir(&self) -> PathBuf {
        self.roots.data.join("plugins")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots_under(base: &Path) -> Roots {
        Roots {
            data: base.join("data"),
            config: base.join("config"),
            cache: base.join("cache"),
            state: base.join("state"),
            home: base.join("home"),
            downloads: base.join("home/Downloads"),
        }
    }

    #[test]
    fn the_configured_downloads_folder_wins_over_the_home_downloads_folder() {
        let home = Path::new("/home/u");
        assert_eq!(downloads_folder(Some(Path::new("/media/Get")), home), PathBuf::from("/media/Get"));
        assert_eq!(downloads_folder(None, home), PathBuf::from("/home/u/Downloads"));
    }

    #[test]
    fn overridden_roots_drive_every_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let paths = SystemPaths::new(roots_under(base));
        assert_eq!(paths.data_dir(), base.join("data"));
        assert_eq!(paths.config_dir(), base.join("config"));
        assert_eq!(paths.cache_dir(), base.join("cache"));
        assert_eq!(paths.state_dir(), base.join("state"));
        assert_eq!(paths.log_dir(), base.join("state/logs"));
        assert_eq!(paths.temp_dir(), base.join("cache/tmp"));
        assert_eq!(paths.crash_dir(), base.join("state/crashes"));
        assert_eq!(paths.user_home_dir(), base.join("home"));
        assert_eq!(paths.user_downloads_dir(), base.join("home/Downloads"));
        assert_eq!(paths.default_storage_dir(), base.join("home/Kurobayomi"));
        assert_eq!(paths.covers_dir(), base.join("cache/covers"));
        assert_eq!(paths.plugin_dir(), base.join("data/plugins"));
    }

    #[test]
    fn resolving_paths_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = SystemPaths::new(roots_under(tmp.path()));
        let _ = (
            paths.log_dir(),
            paths.temp_dir(),
            paths.crash_dir(),
            paths.covers_dir(),
            paths.plugin_dir(),
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn first_run_creates_the_four_base_directories_private() {
        use crate::paths::create_app_dirs;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let paths = SystemPaths::new(roots_under(tmp.path()));
        create_app_dirs(&paths).unwrap();
        for dir in [paths.config_dir(), paths.state_dir(), paths.data_dir(), paths.cache_dir()] {
            assert!(dir.is_dir(), "{}", dir.display());
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", dir.display());
        }
        assert!(!paths.log_dir().exists(), "log dir is created by the logger, not here");
    }

    #[cfg(unix)]
    #[test]
    fn first_run_keeps_an_existing_directory_and_its_mode() {
        use crate::paths::create_app_dirs;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let paths = SystemPaths::new(roots_under(tmp.path()));
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        std::fs::set_permissions(paths.data_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
        create_app_dirs(&paths).unwrap();
        let mode = std::fs::metadata(paths.data_dir()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn first_run_failure_names_the_blocking_path() {
        use crate::paths::create_app_dirs;
        let tmp = tempfile::tempdir().unwrap();
        let paths = SystemPaths::new(roots_under(tmp.path()));
        std::fs::write(paths.cache_dir(), b"not a directory").unwrap();
        let err = create_app_dirs(&paths).unwrap_err();
        assert_eq!(err.path, paths.cache_dir());
        assert!(err.to_string().starts_with(&format!("cannot create {}", paths.cache_dir().display())));
    }

    // the layout of the XDG directories; Windows nests them as `kurobayomi\data`, `kurobayomi\config` and so on
    #[cfg(target_os = "linux")]
    #[test]
    fn system_paths_end_in_app_name() {
        // Reads the real environment but writes nothing.
        let paths = SystemPaths::system().unwrap();
        assert!(paths.data_dir().ends_with("kurobayomi"));
        assert!(paths.config_dir().ends_with("kurobayomi"));
        assert!(paths.cache_dir().ends_with("kurobayomi"));
        assert!(paths.state_dir().ends_with("kurobayomi"));
        assert!(paths.log_dir().ends_with("kurobayomi/logs"));
    }
}
