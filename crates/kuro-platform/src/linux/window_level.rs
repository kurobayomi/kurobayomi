//! Keeping a window above others. On X11: the EWMH `_NET_WM_STATE_ABOVE` request sent to the window manager. Wayland has
//! no such request for apps, so there the window manager is asked directly where it offers a way: KWin (KDE Plasma) runs a
//! short script, loaded over its D-Bus scripting interface, that sets "keep above" on this process's own windows.

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
    surface.display().is::<X11Display>() || kwin::running()
}

pub fn set(surface: &gdk::Surface, above: bool) -> bool {
    let Some(x11) = surface.downcast_ref::<X11Surface>() else {
        return match kwin::keep_above(above) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "KWin could not keep the window above others");
                false
            }
        };
    };
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

/// KWin's scripting interface on the session bus (Plasma 5 and 6).
mod kwin {
    use zbus::blocking::{Connection, Proxy};

    const SERVICE: &str = "org.kde.KWin";
    const PLUGIN: &str = "kurobayomi-keep-above";

    /// Whether KWin owns its name on the session bus.
    pub fn running() -> bool {
        let Ok(conn) = Connection::session() else { return false };
        let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
        SERVICE.try_into().ok().is_some_and(|name| bus.name_has_owner(name).unwrap_or(false))
    }

    /// The script: every normal window of this process gets the state (dialogs follow their parent).
    fn script(pid: u32, above: bool) -> String {
        format!(
            "const all = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
             for (const w of all) {{ if (w.pid === {pid} && w.normalWindow) {{ w.keepAbove = {above}; }} }}\n"
        )
    }

    pub fn keep_above(above: bool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = Connection::session()?;
        let path = std::env::temp_dir().join(format!("{PLUGIN}-{}.js", std::process::id()));
        std::fs::write(&path, script(std::process::id(), above))?;
        let scripting = Proxy::new(&conn, SERVICE, "/Scripting", "org.kde.kwin.Scripting")?;
        // a script left loaded by an earlier call would make the load below refuse the name
        let _: bool = scripting.call("unloadScript", &(PLUGIN,)).unwrap_or(false);
        let id: i32 = scripting.call("loadScript", &(path.to_string_lossy().as_ref(), PLUGIN))?;
        if id < 0 {
            return Err("KWin refused the script".into());
        }
        // Plasma 6 puts the loaded script at /Scripting/Script<id>, Plasma 5 at /<id>
        let ran = [format!("/Scripting/Script{id}"), format!("/{id}")].into_iter().any(|object| {
            Proxy::new(&conn, SERVICE, object.as_str(), "org.kde.kwin.Script").and_then(|script| script.call::<_, _, ()>("run", &())).is_ok()
        });
        let _: bool = scripting.call("unloadScript", &(PLUGIN,)).unwrap_or(false);
        let _ = std::fs::remove_file(&path);
        if ran { Ok(()) } else { Err("KWin did not run the script".into()) }
    }

    #[cfg(test)]
    mod tests {
        use super::script;

        #[test]
        fn the_script_targets_only_this_process_and_sets_the_state() {
            let on = script(42, true);
            assert!(on.contains("w.pid === 42") && on.contains("w.keepAbove = true"));
            assert!(script(42, false).contains("w.keepAbove = false"));
        }
    }
}

