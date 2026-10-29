//! Keeping the app window above every other window (the pin button).
//!
//! GTK 4 has no call for it, so the engine is per platform: the window manager's "above" state on X11
//! (`linux::window_level`), the topmost window order on Windows (`windows::window_level`). Wayland lets no app ask for
//! it, and elsewhere there is no engine yet: [`can_keep_above`] says false and the app offers no pin.

/// Whether `surface` can be kept above other windows on this platform and display.
#[allow(unused_variables, unreachable_code)]
pub fn can_keep_above(surface: &gdk::Surface) -> bool {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    return crate::linux::window_level::supported(surface);
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return true;
    false
}

/// Keeps `surface` above other windows, or lets it go back among them. False when that did not reach the system.
#[allow(unused_variables, unreachable_code)]
pub fn set_keep_above(surface: &gdk::Surface, above: bool) -> bool {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    return crate::linux::window_level::set(surface, above);
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return crate::windows::window_level::set(surface, above);
    false
}
