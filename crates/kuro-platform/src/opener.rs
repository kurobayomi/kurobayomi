//! Hands URLs and paths to the desktop's default handler. Every caller goes through the [`Opener`] trait, so a test can
//! record the calls. A folder opens in the file manager. The engine is per platform: `xdg-open` on Linux, the shell's
//! `ShellExecuteW` on Windows (`windows::opener`).

use std::io;
use std::path::Path;

use crate::process::spawn_detached;

/// Opens a URL or a path with the desktop handler. Callers hold an `Opener` so tests can record the calls.
pub trait Opener: Send + Sync {
    /// Starts the handler for `target` and does not wait for it.
    fn open_target(&self, target: &str) -> io::Result<()>;

    /// Shows `path` in the file manager. The default opens its folder, which every desktop can do; an implementation
    /// that can select the file itself overrides it.
    fn reveal(&self, path: &Path) -> io::Result<()> {
        let folder = path.parent().unwrap_or(path);
        self.open_target(&folder.to_string_lossy())
    }
}

/// The system's [`Opener`]: `xdg-open`, or the shell on Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    #[allow(unreachable_code)]
    fn open_target(&self, target: &str) -> io::Result<()> {
        #[cfg(all(target_os = "windows", feature = "platform-windows"))]
        return crate::windows::opener::open(target);
        spawn_detached(Path::new("xdg-open"), &[std::ffi::OsStr::new(target)])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl Opener for Recorder {
        fn open_target(&self, target: &str) -> io::Result<()> {
            self.0.lock().unwrap().push(target.to_owned());
            Ok(())
        }
    }

    #[test]
    fn reveal_opens_the_folder_of_the_file() {
        let rec = Recorder::default();
        rec.reveal(Path::new("/home/u/Downloads/Naruto/12/001.cbz")).unwrap();
        assert_eq!(*rec.0.lock().unwrap(), vec!["/home/u/Downloads/Naruto/12".to_owned()]);
    }

    #[test]
    fn reveal_of_a_root_path_opens_the_path_itself() {
        let rec = Recorder::default();
        rec.reveal(Path::new("/")).unwrap();
        assert_eq!(*rec.0.lock().unwrap(), vec!["/".to_owned()]);
    }
}
