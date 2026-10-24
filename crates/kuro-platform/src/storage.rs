//! Free disk space for the filesystem that holds a path, the storage roots a storage page lists, and the modified time
//! of a file or folder.
//!
//! The engine is per platform: `statvfs` on Linux (`linux::storage`), `GetDiskFreeSpaceExW` on Windows
//! (`windows::storage`). Elsewhere there is none yet, so [`free_bytes`] gives `None`, which callers treat as "unknown"
//! and skip their space checks.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Bytes available to unprivileged users on the filesystem holding `path`. `None` when the path is missing, the call
/// fails or this platform has no engine.
#[allow(unused_variables, unreachable_code)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    return crate::linux::storage::free_bytes(path);
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return crate::windows::storage::free_bytes(path);
    None
}

/// The storage roots to show sizes for, sorted: the mounted disks and the mount holding `data_dir` on Linux
/// (`linux::storage`), the fixed and removable drives on Windows. Empty where the platform has no engine.
#[allow(unused_variables, unreachable_code)]
pub fn roots(data_dir: &Path) -> Vec<PathBuf> {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    return crate::linux::storage::roots(data_dir);
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return crate::windows::storage::roots();
    Vec::new()
}

/// Sets the modified time of a file or a folder. Windows opens a folder only with a flag (`windows::storage`);
/// elsewhere a plain open does.
#[allow(unreachable_code)]
pub fn set_modified(path: &Path, when: SystemTime) -> io::Result<()> {
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return crate::windows::storage::set_modified(path, when);
    std::fs::File::open(path)?.set_modified(when)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_and_a_file_take_a_modified_time() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for path in [dir.path(), file.as_path()] {
            set_modified(path, when).unwrap();
            assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), when, "{}", path.display());
        }
    }
}
