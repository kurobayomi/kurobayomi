//! Child processes: finding a program on `PATH`, starting one without a console window, and starting one that outlives
//! the caller.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The program `name` in the folders of a `PATH` value: the first file there with the system's program suffix (`.exe`
/// on Windows), so callers name a program the same way on every system.
pub fn find_program(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path_var).map(|dir| dir.join(&file)).find(|path| path.is_file())
}

/// Opens no console window for a console program (ffmpeg, a torrent server): on Windows a program started from a
/// windowed app otherwise gets one. Elsewhere nothing changes.
#[allow(unused_variables)]
pub fn no_console(cmd: &mut Command) -> &mut Command {
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    cmd
}

/// Starts `program` with `args` in its own process group, with stdio closed. The child is not waited for.
pub fn spawn_detached(program: &Path, args: &[&OsStr]) -> io::Result<()> {
    let mut cmd = Command::new(program);
    no_console(cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(drop)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn spawns_and_returns_without_waiting() {
        spawn_detached(Path::new("true"), &[]).unwrap();
    }

    #[test]
    fn missing_program_is_an_error() {
        assert!(spawn_detached(Path::new("/nonexistent/kuro-missing"), &[]).is_err());
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn a_program_is_found_by_its_name_with_the_system_suffix() {
        let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let file = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(second.path().join(&file), b"x").unwrap();
        std::fs::create_dir(first.path().join(&file)).unwrap();
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        assert_eq!(find_program("ffmpeg", &path), Some(second.path().join(&file)), "a folder of that name is skipped");
        assert_eq!(find_program("ffprobe", &path), None);
    }
}
