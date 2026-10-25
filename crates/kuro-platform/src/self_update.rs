//! What the app's own update (`appimage_update` in the GTK crate) needs from the system: how this run was installed,
//! replacing a running AppImage, and running the Windows setup that updates an installed copy.

use std::io;
use std::path::{Path, PathBuf};

/// How this run was installed, when it can update itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Package {
    /// An AppImage (Linux): its file. An update replaces it in place.
    AppImage(PathBuf),
    /// A copy the Windows setup installed. An update runs a newer setup over it.
    Installer,
}

/// The package of this run, or `None` for any other build (a local one, a distribution package, an unpacked zip).
pub fn package() -> Option<Package> {
    appimage_file().map(Package::AppImage).or_else(|| installed_by_setup().then_some(Package::Installer))
}

/// Whether the Windows setup installed this copy: it puts its uninstaller (`unins000.exe`) in the folder above `bin`.
fn installed_by_setup() -> bool {
    cfg!(windows)
        && std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.parent()?.join("unins000.exe")))
            .is_some_and(|uninstaller| uninstaller.is_file())
}

/// Runs a downloaded Windows setup over the installed copy, asking nothing. It closes a running Kurobayomi itself;
/// `relaunch` (Install now) shows the setup's progress window, as the person waits for the app to come back, and starts
/// the app again when it is done (the setup reads `/RELAUNCH=1`, `dist/windows/kurobayomi.iss`); at a plain quit it shows
/// nothing. The setup writes its log next to itself (`<setup>.log`), the one trace of a silent install that failed.
pub fn run_installer(setup: &Path, relaunch: bool) -> io::Result<()> {
    let mut log = std::ffi::OsString::from("/LOG=");
    log.push(setup.with_extension("log"));
    let quiet = if relaunch { "/SILENT" } else { "/VERYSILENT" };
    let mut args = [quiet, "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"].map(std::ffi::OsStr::new).to_vec();
    args.push(&log);
    if relaunch {
        args.push("/RELAUNCH=1".as_ref());
    }
    crate::process::spawn_detached(setup, &args)
}

/// The AppImage file this process runs from: the AppImage runtime sets `APPIMAGE` to its path. `None` for any other build.
pub fn appimage_file() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|path| path.is_file())
}

/// Starts `appimage` again a moment after this process exits (so the new copy is not handed to the running one as a second
/// window). The caller then quits.
pub fn relaunch_after_exit(appimage: &Path) -> io::Result<()> {
    crate::process::spawn_detached(Path::new("sh"), &["-c".as_ref(), "sleep 1; exec \"$0\"".as_ref(), appimage.as_os_str()])
}

/// Makes `new` executable and moves it over `target`. Keep both in one folder so the move is atomic; the running copy keeps
/// its old file until it exits.
pub fn replace_file(new: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(new, target)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_new_file_replaces_the_old_one_and_is_executable() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("App.AppImage"), dir.path().join("App.AppImage.update"));
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        replace_file(&new, &old).unwrap();
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "new");
        assert!(!new.exists());
        assert_eq!(std::fs::metadata(&old).unwrap().permissions().mode() & 0o111, 0o111);
    }
}
