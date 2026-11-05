//! Whether this run is inside a Flatpak sandbox. The one place that tells: what the sandbox changes (no self-update, a
//! child that must outlive the app, no host programs) asks here.

use std::path::Path;

/// Flatpak sets `FLATPAK_ID` in the sandbox and mounts `/.flatpak-info` into it.
pub fn is_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file()
}
