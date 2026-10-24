//! Disk space through `GetDiskFreeSpaceExW`, for the volume that holds a path.

use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::SystemTime;

use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES, GetDiskFreeSpaceExW};

/// Bytes available to this user (quotas included) on the volume holding `path`. `None` when the path is missing or
/// the call fails.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let wide = super::wide(path.as_os_str());
    let mut free = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the two totals this does not need may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

/// Sets the modified time of a file or a folder: a folder opens only with the backup flag, and the time needs only the
/// right to write attributes.
pub fn set_modified(path: &Path, when: SystemTime) -> io::Result<()> {
    std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?
        .set_modified(when)
}
