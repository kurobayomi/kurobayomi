//! Keeping a window above others on X11: the EWMH `_NET_WM_STATE_ABOVE` request sent to the window manager. Wayland has
//! no such request for apps.

use gdk::prelude::*;
use gdk_x11::{X11Display, X11Surface};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

/// `_NET_WM_STATE` actions.
const REMOVE: u32 = 0;
const ADD: u32 = 1;
/// The request comes from a normal application.
const SOURCE_APPLICATION: u32 = 1;

pub fn supported(surface: &gdk::Surface) -> bool {
    surface.display().is::<X11Display>()
}

pub fn set(surface: &gdk::Surface, above: bool) -> bool {
    let Some(x11) = surface.downcast_ref::<X11Surface>() else { return false };
    let display = surface.display().name();
    match send(display.as_str(), x11.xid(), above) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "the window could not be kept above others");
            false
        }
    }
}

/// Asks the window manager, over a connection of our own to the same display, to add or remove the window's "above" state.
fn send(display: &str, window: std::os::raw::c_ulong, above: bool) -> Result<(), Box<dyn std::error::Error>> {
    let window = u32::try_from(window)?;
    let (conn, screen) = x11rb::connect(Some(display))?;
    let root = conn.setup().roots[screen].root;
    let state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
    let state_above = conn.intern_atom(false, b"_NET_WM_STATE_ABOVE")?.reply()?.atom;
    let action = if above { ADD } else { REMOVE };
    let event = ClientMessageEvent::new(32, window, state, [action, state_above, 0, SOURCE_APPLICATION, 0]);
    conn.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)?;
    conn.flush()?;
    Ok(())
}
