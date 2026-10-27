//! Platform directory resolution. Callers ask for a role and never build a path from a literal.

use std::path::{Path, PathBuf};
use std::{fs, io};

/// The folder in the home folder that holds the app's files when the user has not picked another one.
pub const STORAGE_DIR_NAME: &str = if crate::channel::NIGHTLY { "Kurobayomi-Nightly" } else { "Kurobayomi" };

/// Directories the app reads and writes, resolved for the current platform.
pub trait Paths {
    /// Downloaded data, databases and other durable app state.
    fn data_dir(&self) -> PathBuf;
    /// User-editable settings.
    fn config_dir(&self) -> PathBuf;
    /// App state that survives restarts but is not user-editable (`state.toml`, logs).
    fn state_dir(&self) -> PathBuf;
    /// Disposable cache data.
    fn cache_dir(&self) -> PathBuf;
    /// Log files.
    fn log_dir(&self) -> PathBuf;
    /// Scratch space for in-progress work.
    fn temp_dir(&self) -> PathBuf;
    /// Crash reports written by the panic hook.
    fn crash_dir(&self) -> PathBuf;
    /// The user's home folder. Platforms that know it override this. The default is the data directory, so a test
    /// double without a home folder still resolves to a path.
    fn user_home_dir(&self) -> PathBuf {
        self.data_dir()
    }
    /// The platform's usual folder for downloaded files (app updates go there, not into the storage folder). The
    /// default is the data directory, for test doubles.
    fn user_downloads_dir(&self) -> PathBuf {
        self.data_dir()
    }
    /// Cached cover images.
    fn covers_dir(&self) -> PathBuf;
    /// Installed plugins (extensions).
    fn plugin_dir(&self) -> PathBuf;

    // The roles below have defaults built from the base directories, so an implementation only overrides one to
    // relocate it. They exist so no other crate joins a literal onto a base directory (T-req-platform-001).

    /// The storage folder used until the user picks one: `<home>/Kurobayomi`. The folders inside it (downloads per
    /// kind, backups, local folders) are named by the storage manager (`kuro_domain::storage`), which resolves them under
    /// this folder or the one the user picked. Nothing is created here.
    fn default_storage_dir(&self) -> PathBuf {
        self.user_home_dir().join(STORAGE_DIR_NAME)
    }

    /// Decoded page and image cache (Aniyomi's `image_cache`).
    fn image_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("image_cache")
    }
    /// Chapter page cache (Aniyomi's `chapter_disk_cache`).
    fn chapter_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("chapter_disk_cache")
    }
    /// The last results of each source listing and search, shown at once while the site is asked again.
    fn browse_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("browse_results")
    }
    /// Side subtitle files (`.vtt`, `.srt`, `.ass`) of streamed episodes, kept by their address.
    fn subtitle_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("subtitles")
    }
    /// Where a shared image is staged before it is handed to another app.
    fn share_staging_dir(&self) -> PathBuf {
        self.cache_dir().join("shared_image")
    }
    /// Installed manga extensions.
    fn extensions_dir(&self) -> PathBuf {
        self.data_dir().join("extensions")
    }
    /// Source recipes: one `<id>.toml` per declarative source (`docs/source-recipes.md`).
    fn scrapers_dir(&self) -> PathBuf {
        self.data_dir().join("scrapers")
    }
    /// Source files fetched from repository links (recipes and plug-ins), one checked copy per source, and each
    /// repository's saved index. They live with the app's data, not in its cache: a cleaner that empties the cache must not
    /// leave the app without sources while the repository cannot be reached (user, 2026-10-27).
    fn repo_sources_dir(&self) -> PathBuf {
        self.data_dir().join("repo-sources")
    }
    /// Where versions up to 0.2.5 kept [`Paths::repo_sources_dir`], in the cache; the app moves it once at start.
    fn old_repo_sources_dir(&self) -> PathBuf {
        self.cache_dir().join("repo-sources")
    }
    /// The cookies and storage of the hidden web pages that pass a site's browser check (`hidden_pages`), kept so a
    /// later run usually starts past the check.
    fn web_pages_data_dir(&self) -> PathBuf {
        self.data_dir().join("webkit")
    }
    /// The disk cache of those hidden pages.
    fn web_pages_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("webkit")
    }
    /// Compiled WASM plug-ins, one file per plug-in build. Deleting the folder only makes the next start compile again.
    fn wasm_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("wasm-components")
    }
    /// Installed anime extensions.
    fn anime_extensions_dir(&self) -> PathBuf {
        self.extensions_dir().join("anime")
    }
    /// Cached manga extension icons.
    fn extension_icons_dir(&self) -> PathBuf {
        self.cache_dir().join("extensions").join("icons")
    }
    /// Cached anime extension icons.
    fn anime_extension_icons_dir(&self) -> PathBuf {
        self.cache_dir().join("ext-icons")
    }
    /// Extension packages while they download.
    fn extension_download_dir(&self) -> PathBuf {
        self.cache_dir().join("ext-download")
    }
    /// Snapshot of the download index, kept in the cache beside the downloads folder.
    fn download_index_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("downloads")
    }
    /// The text log of the last library update pass.
    fn library_update_log_file(&self) -> PathBuf {
        self.cache_dir().join("update_errors.txt")
    }
    /// The cookie jar of the HTTP client.
    fn cookie_jar_file(&self) -> PathBuf {
        self.data_dir().join("cookies.json")
    }
    /// The build the app ran last time; after an update the app compares it with the running one (What's new).
    fn last_build_file(&self) -> PathBuf {
        self.state_dir().join("last-build")
    }
    /// A cover or background folder under the data directory (`covers`, `animecovers`, ...).
    fn cover_family_dir(&self, folder: &str) -> PathBuf {
        self.data_dir().join(folder)
    }
    /// Scratch folder for page bytes written for another reader. It lives under the system temporary directory and
    /// carries the process id, so two running copies do not share it.
    fn page_bytes_dir(&self) -> PathBuf {
        std::env::temp_dir().join(format!("kuro-page-bytes-{}", std::process::id()))
    }
    /// Rounded cover thumbnails the desktop widgets read (written by the app, read-only for `--updates-json`).
    fn widget_covers_dir(&self) -> PathBuf {
        self.cache_dir().join("widget-covers")
    }
    /// The `cookies.txt` export the user drops in for a source that needs a browser challenge solved by hand.
    fn cookies_import_file(&self) -> PathBuf {
        self.config_dir().join("cookies.txt")
    }
    /// The desktop's own configuration folder, which holds files the app only reads (the KDE colour scheme).
    /// `None` when no home directory exists.
    fn host_config_dir(&self) -> Option<PathBuf> {
        directories::BaseDirs::new().map(|base| base.config_dir().to_path_buf())
    }
}

/// A first-run directory could not be created. The message names the path so the crash window explains it.
#[derive(Debug, thiserror::Error)]
#[error("cannot create {}: {source}", path.display())]
pub struct DirError {
    pub path: PathBuf,
    #[source]
    pub source: io::Error,
}

/// Creates the config, state, data and cache directories when they are missing, with mode 0700 on Unix.
/// Existing directories keep their mode. Stops at the first failure.
pub fn create_app_dirs(paths: &dyn Paths) -> Result<(), DirError> {
    for dir in [paths.config_dir(), paths.state_dir(), paths.data_dir(), paths.cache_dir()] {
        create_private_dir(&dir).map_err(|source| DirError { path: dir, source })?;
    }
    Ok(())
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// The raw bytes of a path, so a stored preference can round-trip any OS path, including non-UTF-8 ones (E-STO-12).
#[cfg(unix)]
pub fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

/// The path with exactly these bytes. Off Unix the bytes must be UTF-8, the port's fallback, else `None`.
#[cfg(unix)]
pub fn path_from_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// The raw bytes of a path. Off Unix the path is stored as UTF-8 (lossy for non-UTF-8 paths).
#[cfg(not(unix))]
pub fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// The path with these bytes, or `None` when they are not UTF-8 (off Unix only).
#[cfg(not(unix))]
pub fn path_from_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(all(test, unix))]
mod byte_path_tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn a_non_utf8_path_round_trips_through_its_bytes() {
        let original = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', b'x', 0xff, b'/', b'y']));
        assert_eq!(path_from_bytes(path_bytes(&original)), Some(original));
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;

    struct Fixed;

    impl Paths for Fixed {
        fn data_dir(&self) -> PathBuf { "/d".into() }
        fn config_dir(&self) -> PathBuf { "/c".into() }
        fn state_dir(&self) -> PathBuf { "/s".into() }
        fn cache_dir(&self) -> PathBuf { "/k".into() }
        fn log_dir(&self) -> PathBuf { "/l".into() }
        fn temp_dir(&self) -> PathBuf { "/t".into() }
        fn crash_dir(&self) -> PathBuf { "/cr".into() }
        fn covers_dir(&self) -> PathBuf { "/k/covers".into() }
        fn plugin_dir(&self) -> PathBuf { "/d/plugins".into() }
    }

    #[test]
    fn default_roles_hang_off_the_base_directories() {
        let p = Fixed;
        assert_eq!(p.image_cache_dir(), PathBuf::from("/k/image_cache"));
        assert_eq!(p.chapter_cache_dir(), PathBuf::from("/k/chapter_disk_cache"));
        assert_eq!(p.share_staging_dir(), PathBuf::from("/k/shared_image"));
        assert_eq!(p.browse_cache_dir(), PathBuf::from("/k/browse_results"));
        assert_eq!(p.subtitle_cache_dir(), PathBuf::from("/k/subtitles"));
        assert_eq!(p.extensions_dir(), PathBuf::from("/d/extensions"));
        assert_eq!(p.anime_extensions_dir(), PathBuf::from("/d/extensions/anime"));
        assert_eq!(p.scrapers_dir(), PathBuf::from("/d/scrapers"));
        assert_eq!(p.repo_sources_dir(), PathBuf::from("/d/repo-sources"));
        assert_eq!(p.old_repo_sources_dir(), PathBuf::from("/k/repo-sources"));
        assert_eq!(p.wasm_cache_dir(), PathBuf::from("/k/wasm-components"));
        assert_eq!(p.web_pages_data_dir(), PathBuf::from("/d/webkit"));
        assert_eq!(p.web_pages_cache_dir(), PathBuf::from("/k/webkit"));
        assert_eq!(p.extension_icons_dir(), PathBuf::from("/k/extensions/icons"));
        assert_eq!(p.anime_extension_icons_dir(), PathBuf::from("/k/ext-icons"));
        assert_eq!(p.extension_download_dir(), PathBuf::from("/k/ext-download"));
        assert_eq!(p.download_index_cache_dir(), PathBuf::from("/k/downloads"));
        assert_eq!(p.library_update_log_file(), PathBuf::from("/k/update_errors.txt"));
        assert_eq!(p.cookie_jar_file(), PathBuf::from("/d/cookies.json"));
        assert_eq!(p.last_build_file(), PathBuf::from("/s/last-build"));
        assert_eq!(p.cover_family_dir("animecovers"), PathBuf::from("/d/animecovers"));
        assert_eq!(p.cookies_import_file(), PathBuf::from("/c/cookies.txt"));
    }

    /// A fake that knows only its home folder, so the storage default resolves through the trait alone.
    struct Home(&'static str);

    impl Paths for Home {
        fn data_dir(&self) -> PathBuf { "/d".into() }
        fn config_dir(&self) -> PathBuf { "/c".into() }
        fn state_dir(&self) -> PathBuf { "/s".into() }
        fn cache_dir(&self) -> PathBuf { "/k".into() }
        fn log_dir(&self) -> PathBuf { "/l".into() }
        fn temp_dir(&self) -> PathBuf { "/t".into() }
        fn crash_dir(&self) -> PathBuf { "/cr".into() }
        fn covers_dir(&self) -> PathBuf { "/k/covers".into() }
        fn plugin_dir(&self) -> PathBuf { "/d/plugins".into() }
        fn user_home_dir(&self) -> PathBuf { self.0.into() }
    }

    #[test]
    fn the_default_storage_folder_is_kurobayomi_in_the_home_folder() {
        assert_eq!(Home("/home/u").default_storage_dir(), PathBuf::from("/home/u/Kurobayomi"));
    }

    #[test]
    fn a_double_without_a_home_folder_defaults_to_the_data_directory() {
        assert_eq!(Fixed.default_storage_dir(), PathBuf::from("/d/Kurobayomi"));
        assert_eq!(Fixed.user_downloads_dir(), PathBuf::from("/d"));
    }

    #[test]
    fn page_bytes_dir_is_under_the_system_temp_dir_and_per_process() {
        let dir = Fixed.page_bytes_dir();
        assert!(dir.starts_with(std::env::temp_dir()));
        assert!(dir.to_string_lossy().ends_with(&std::process::id().to_string()));
    }
}
