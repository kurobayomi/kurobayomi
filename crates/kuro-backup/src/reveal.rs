//! "Show in folder" for a finished backup (area 21, T-21a-042). The D-Bus call is behind `FileManager` so this module
//! stays free of D-Bus code (that lives in kuro-platform). The caller runs this off the GTK thread.

use std::io;
use std::path::{Path, PathBuf};

/// `org.freedesktop.FileManager1.ShowItems` for one URI.
pub trait FileManager {
    fn show_items(&self, uri: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reveal {
    Shown,
    /// The file manager did not answer; the parent folder opened instead.
    OpenedFolder,
    Failed(String),
}

/// `file://` URI of an absolute path.
pub fn file_uri(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

pub fn show_in_folder(path: &Path, files: &dyn FileManager, open_dir: &dyn Fn(&Path) -> io::Result<()>) -> Reveal {
    let Some(uri) = file_uri(path) else {
        return Reveal::Failed("backup path is not absolute".into());
    };
    if files.show_items(&uri).is_ok() {
        return Reveal::Shown;
    }
    let Some(dir) = path.parent() else {
        return Reveal::Failed("backup path has no folder".into());
    };
    match open_dir(dir) {
        Ok(()) => Reveal::OpenedFolder,
        Err(e) => Reveal::Failed(e.to_string()),
    }
}

/// Runs `show_in_folder` on a worker thread so the GTK thread never blocks on D-Bus. The result arrives on the
/// returned channel (one message, then the channel closes).
pub fn spawn_show_in_folder(
    path: PathBuf,
    files: Box<dyn FileManager + Send>,
    open_dir: Box<dyn Fn(&Path) -> io::Result<()> + Send>,
) -> async_channel::Receiver<Reveal> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(show_in_folder(&path, files.as_ref(), open_dir.as_ref()));
    });
    rx
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;

    struct Bus {
        answer: Result<(), String>,
        asked: RefCell<Vec<String>>,
    }

    impl FileManager for Bus {
        fn show_items(&self, uri: &str) -> Result<(), String> {
            self.asked.borrow_mut().push(uri.to_owned());
            self.answer.clone()
        }
    }

    /// An absolute backup path on this system (`/d` would not be absolute on Windows).
    fn backup() -> PathBuf {
        std::env::temp_dir().join("d").join("x.tachibk")
    }

    #[test]
    fn uri_is_file_scheme_with_percent_encoding() {
        let uri = file_uri(&std::env::temp_dir().join("my backups").join("a b.tachibk")).unwrap();
        assert!(uri.starts_with("file:///") && uri.ends_with("/my%20backups/a%20b.tachibk"), "{uri}");
        assert_eq!(file_uri(Path::new("relative.tachibk")), None);
    }

    #[test]
    fn file_manager_answer_wins() {
        let bus = Bus { answer: Ok(()), asked: RefCell::new(Vec::new()) };
        let opened = RefCell::new(None::<PathBuf>);
        let open = |p: &Path| {
            *opened.borrow_mut() = Some(p.to_path_buf());
            Ok(())
        };
        let out = show_in_folder(&backup(), &bus, &open);
        assert_eq!(out, Reveal::Shown);
        assert_eq!(bus.asked.borrow().len(), 1);
        assert!(opened.borrow().is_none());
    }

    #[test]
    fn falls_back_to_parent_folder() {
        let bus = Bus { answer: Err("no service".into()), asked: RefCell::new(Vec::new()) };
        let opened = RefCell::new(None::<PathBuf>);
        let open = |p: &Path| {
            *opened.borrow_mut() = Some(p.to_path_buf());
            Ok(())
        };
        let out = show_in_folder(&backup(), &bus, &open);
        assert_eq!(out, Reveal::OpenedFolder);
        assert_eq!(opened.borrow().as_deref(), backup().parent());
    }

    #[test]
    fn both_paths_failing_reports_failure() {
        let bus = Bus { answer: Err("no service".into()), asked: RefCell::new(Vec::new()) };
        let open = |_: &Path| Err(io::Error::other("no opener"));
        assert!(matches!(show_in_folder(&backup(), &bus, &open), Reveal::Failed(_)));
        assert!(matches!(show_in_folder(Path::new("x.tachibk"), &bus, &open), Reveal::Failed(_)));
    }

    struct Instant(Result<(), String>);
    impl FileManager for Instant {
        fn show_items(&self, _: &str) -> Result<(), String> {
            self.0.clone()
        }
    }

    #[test]
    fn worker_thread_sends_the_result_through_the_channel() {
        let rx = spawn_show_in_folder(backup(), Box::new(Instant(Err("no".into()))), Box::new(|_| Ok(())));
        assert_eq!(rx.recv_blocking().unwrap(), Reveal::OpenedFolder);
        assert!(rx.recv_blocking().is_err(), "the channel closes after one result");
        let rx = spawn_show_in_folder(backup(), Box::new(Instant(Ok(()))), Box::new(|_| Ok(())));
        assert_eq!(rx.recv_blocking().unwrap(), Reveal::Shown);
    }
}
