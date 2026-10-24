//! Disk space through `GetDiskFreeSpaceExW`, for the volume that holds a path.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW,
};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

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

/// The drives to show sizes for: the fixed and removable ones (`C:\`, `D:\`), in letter order. Network drives, CD
/// drives and RAM disks are left out.
pub fn roots() -> Vec<PathBuf> {
    let mut buffer = [0u16; 512];
    // SAFETY: the call writes at most `buffer.len()` characters into `buffer`: the drive roots, each NUL-terminated.
    let length = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) } as usize;
    if length == 0 || length > buffer.len() {
        return Vec::new();
    }
    buffer[..length]
        .split(|&c| c == 0)
        .filter(|drive| !drive.is_empty())
        .filter(|drive| {
            let root: Vec<u16> = drive.iter().copied().chain(std::iter::once(0)).collect();
            // SAFETY: `root` is a NUL-terminated drive root.
            matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, DRIVE_FIXED | DRIVE_REMOVABLE)
        })
        .map(|drive| PathBuf::from(OsString::from_wide(drive)))
        .collect()
}
