//! Windows-specific integration: the GL entry points, disk space and the default program opener.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub mod gl;
pub mod opener;
pub mod storage;

/// `text` as the NUL-terminated UTF-16 the Windows API takes.
pub(crate) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}
