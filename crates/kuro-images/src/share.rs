//! The share staging folder and the share fallback (behavior sections 21.7 and 23.4, design section 8.3).
//!
//! Linux has no system share sheet, so sharing hands the staged file to the caller's `share` action. When that
//! action fails, the file's folder opens in the file manager instead, and the caller shows no error (T-34a-047).

use std::io;
use std::path::{Path, PathBuf};

use kuro_platform::paths::Paths;

/// The folder under the cache dir that holds the latest shared picture (behavior section 21.3).
pub const STAGING_FOLDER: &str = "shared_image";

/// `<cache>/shared_image`.
pub fn staging_dir(paths: &dyn Paths) -> PathBuf {
    paths.share_staging_dir()
}

/// Runs `share` on the staged file. When it fails, `reveal` runs on the same file instead, and the share error is
/// not returned. The caller passes [`reveal`] for the real desktop, and a closure in tests.
pub fn share_or_reveal(
    path: &Path,
    share: impl FnOnce(&Path) -> io::Result<()>,
    reveal: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    match share(path) {
        Ok(()) => Ok(()),
        Err(_) => reveal(path),
    }
}

/// Opens the folder that holds `path` with the desktop's default handler, which is the file manager for folders.
pub fn reveal(path: &Path) -> io::Result<()> {
    kuro_platform::opener::Opener::reveal(&kuro_platform::opener::SystemOpener, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestPaths;

    #[test]
    fn staging_is_under_the_cache_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert_eq!(staging_dir(&paths), tmp.path().join("cache/shared_image"));
    }

    #[test]
    fn a_successful_share_does_not_reveal() {
        let mut shared = Vec::new();
        let mut revealed = false;
        let result = share_or_reveal(
            Path::new("/x/a.png"),
            |p| {
                shared.push(p.to_path_buf());
                Ok(())
            },
            |_| {
                revealed = true;
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(shared, vec![PathBuf::from("/x/a.png")]);
        assert!(!revealed);
    }

    #[test]
    fn a_failed_share_reveals_the_file_and_shows_no_error() {
        let mut revealed = Vec::new();
        let result = share_or_reveal(
            Path::new("/x/a.png"),
            |_| Err(io::Error::other("no sharing service")),
            |p| {
                revealed.push(p.to_path_buf());
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(revealed, vec![PathBuf::from("/x/a.png")]);
    }

    #[test]
    fn a_failed_reveal_is_the_error_the_caller_sees() {
        let result = share_or_reveal(
            Path::new("/x/a.png"),
            |_| Err(io::Error::other("no sharing service")),
            |_| Err(io::Error::other("no file manager")),
        );
        assert_eq!(result.unwrap_err().to_string(), "no file manager");
    }
}
