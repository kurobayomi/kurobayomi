//! Windows-specific integration: the GL entry points, disks and their space, memory, the default program opener, the
//! hidden pages that pass a site's browser check (WebView2) and the system facts of crash reports, and keeping the window above others.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub mod device;
pub mod gl;
pub mod hidden_pages;
pub mod opener;
pub mod storage;
pub mod system;
pub mod window_level;

/// `text` as the NUL-terminated UTF-16 the Windows API takes.
pub(crate) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}
