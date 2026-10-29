//! Keeping a window above others on Windows: the topmost window order.

use gdk::prelude::*;
use gdk_win32::Win32Surface;
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};

pub fn set(surface: &gdk::Surface, above: bool) -> bool {
    let Some(win32) = surface.downcast_ref::<Win32Surface>() else { return false };
    let window = win32.handle().0;
    let order = if above { HWND_TOPMOST } else { HWND_NOTOPMOST };
    // SAFETY: `window` is the live handle GDK keeps for this surface; only the z-order changes
    let done = unsafe { SetWindowPos(window, order, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    if done == 0 {
        tracing::warn!("the window could not be kept above others");
    }
    done != 0
}
