//! Facts about the running Windows for crash reports: its name and build from the registry, the user's locale.

use std::ffi::OsStr;

use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};

use super::wide;

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// A text value under `CurrentVersion`, or `None` when it is missing or empty.
fn text(name: &str) -> Option<String> {
    let (key, value) = (wide(OsStr::new(CURRENT_VERSION)), wide(OsStr::new(name)));
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the key and value names are NUL-terminated; `size` is the buffer's length in bytes, and the call writes at
    // most that many, NUL included.
    let status = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
    };
    let chars = (size as usize / 2).saturating_sub(1);
    (status == 0 && chars > 0).then(|| String::from_utf16_lossy(&buffer[..chars]))
}

/// A number value under `CurrentVersion`.
fn number(name: &str) -> Option<u32> {
    let (key, value) = (wide(OsStr::new(CURRENT_VERSION)), wide(OsStr::new(name)));
    let (mut data, mut size) = (0u32, 4u32);
    // SAFETY: as in `text`; the data is one 32-bit number.
    let status = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&raw mut data).cast(), &mut size)
    };
    (status == 0).then_some(data)
}

/// The build number, as `22631`.
fn build() -> Option<u32> {
    text("CurrentBuild")?.parse().ok()
}

/// `Windows 11 Pro 23H2`. Windows 11 still names itself "Windows 10" in the registry; builds from 22000 on are 11.
pub fn os_name() -> Option<String> {
    let mut name = text("ProductName")?;
    if build().is_some_and(|b| b >= 22000) {
        name = name.replacen("Windows 10", "Windows 11", 1);
    }
    Some(match text("DisplayVersion") {
        Some(version) => format!("{name} {version}"),
        None => name,
    })
}

/// `Windows NT 10.0.22631.4317`: the version and build with its update revision.
pub fn kernel() -> Option<String> {
    let (major, minor, build) = (number("CurrentMajorVersionNumber")?, number("CurrentMinorVersionNumber")?, build()?);
    let revision = number("UBR").map(|ubr| format!(".{ubr}")).unwrap_or_default();
    Some(format!("Windows NT {major}.{minor}.{build}{revision}"))
}

/// The user's locale, as `en-US`.
pub fn locale() -> Option<String> {
    // LOCALE_NAME_MAX_LENGTH
    let mut buffer = [0u16; 85];
    // SAFETY: the buffer holds the 85 characters the call may write, NUL included.
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    (length > 1).then(|| String::from_utf16_lossy(&buffer[..length as usize - 1]))
}
