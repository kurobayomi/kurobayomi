//! Hands a URL or a path to its default program through `ShellExecuteW`, as Explorer does.

use std::ffi::OsStr;
use std::io;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::wide;

/// Opens `target` (a web address, a file or a folder) with its default program and does not wait for it.
pub fn open(target: &str) -> io::Result<()> {
    let (verb, file) = (wide(OsStr::new("open")), wide(OsStr::new(target)));
    // SAFETY: both strings are NUL-terminated and outlive the call; the window, parameters and folder may be null.
    let code = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    } as isize;
    // above 32 is success; anything else is the shell's error code
    if code > 32 { Ok(()) } else { Err(io::Error::other(format!("the shell could not open it (code {code})"))) }
}
