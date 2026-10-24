//! Windows-specific integration: the GL entry points, disks and their space, memory, the default program opener and the
//! system facts of crash reports.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub mod device;
pub mod gl;
pub mod opener;
pub mod storage;
pub mod system;

/// `text` as the NUL-terminated UTF-16 the Windows API takes.
pub(crate) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}
