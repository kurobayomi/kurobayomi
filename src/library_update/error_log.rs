//! The update error log (T-26a-095, T-26a-096; behaviour 7, `writeErrorFile`): the text of the failures of one pass in
//! the Kotlin format, and where it is written. It is written only when a pass had failures, and each pass overwrites the
//! previous file. The failure notice opens it with the default text viewer (`misc::notify`).
//!
//! Format: the help line and a blank line, then for each error message in order of first appearance `\n! message\n`;
//! under it, for each source in order of first appearance `  # source\n`, then `    - title\n` for each entry.
//!
//! Decisions:
//! - The file is `update_errors.txt` in the platform cache directory (decision D14), one file for both kinds as in Kotlin.
//! - The help line keeps the two Aniyomi troubleshooting pages (manga and anime), as the Kotlin strings do.

use std::io;
use std::path::{Path, PathBuf};

use kuro_platform::kind::Kind;
use kuro_platform::paths::Paths;

use crate::library_update::pass::Failure;

#[cfg(test)]
/// The file name inside the cache directory.
pub const FILE_NAME: &str = "update_errors.txt";

/// The help page of the header (`ERROR_LOG_HELP_URL` of each job): one page for both kinds.
pub fn help_url(_kind: Kind) -> &'static str {
    kuro_platform::links::URL_UPDATE_ERRORS
}

/// Where the log of a pass is written: `FILE_NAME` in the cache directory of the app.
pub fn path(paths: &dyn Paths) -> PathBuf {
    paths.library_update_log_file()
}

/// The text of the log. `source_name` gives the display name of a source id (an uninstalled source shows its id).
pub fn render(kind: Kind, failures: &[Failure], source_name: &dyn Fn(i64) -> String) -> String {
    let mut out = format!("{}\n\n", kuro_i18n::trf(kuro_i18n::MsgKey::library_errors_help, &[("arg1", kuro_i18n::Arg::Str(&help_url(kind).to_string()))]));
    // (message, sources of that message, entries of each source), each in order of first appearance.
    let mut messages: Vec<(&str, Vec<(i64, Vec<&str>)>)> = Vec::new();
    for failure in failures {
        let slot = match messages.iter().position(|(message, _)| *message == failure.message) {
            Some(index) => index,
            None => {
                messages.push((&failure.message, Vec::new()));
                messages.len() - 1
            }
        };
        let sources = &mut messages[slot].1;
        match sources.iter_mut().find(|(id, _)| *id == failure.source_id) {
            Some((_, titles)) => titles.push(&failure.title),
            None => sources.push((failure.source_id, vec![&failure.title])),
        }
    }
    for (message, sources) in messages {
        out.push_str(&format!("\n! {message}\n"));
        for (source_id, titles) in sources {
            out.push_str(&format!("  # {}\n", source_name(source_id)));
            for title in titles {
                out.push_str(&format!("    - {title}\n"));
            }
        }
    }
    out
}

/// Writes the log to `file`, replacing the one of the last pass. Nothing is written, and `None` comes back, when there
/// are no failures (P-16): an old file stays as it was.
pub fn write(
    file: &Path,
    kind: Kind,
    failures: &[Failure],
    source_name: &dyn Fn(i64) -> String,
) -> io::Result<Option<PathBuf>> {
    if failures.is_empty() {
        return Ok(None);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, render(kind, failures, source_name))?;
    Ok(Some(file.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(source_id: i64, title: &str, message: &str) -> Failure {
        Failure { source_id, title: title.into(), message: message.into() }
    }

    fn names(id: i64) -> String {
        match id {
            1 => "Alpha".into(),
            2 => "Beta".into(),
            3 => "Gamma".into(),
            other => other.to_string(),
        }
    }

    fn fixture_failures() -> Vec<Failure> {
        vec![
            failure(1, "A1", "No chapters found"),
            failure(3, "G1", "timeout"),
            failure(2, "B1", "No chapters found"),
            failure(1, "A2", "No chapters found"),
            failure(1, "A3", "timeout"),
        ]
    }

    #[test]
    fn the_output_equals_the_expected_fixture() {
        // P-17: two messages, three sources, grouped by message then source, in order of first appearance.
        let expected = include_str!("fixtures/errorlog_expected.txt");
        assert_eq!(render(Kind::Manga, &fixture_failures(), &names), expected);
    }

    #[test]
    fn the_anime_log_names_the_anime_help_page() {
        let text = render(Kind::Anime, &[failure(9, "T", "x")], &names);
        assert!(text.starts_with("For help on how to fix library update errors, see https://kurobayomi.github.io/docs/guides/troubleshooting#library-update-errors\n\n"));
        assert!(text.contains("\n! x\n  # 9\n    - T\n"), "an unknown source shows its id");
    }

    #[test]
    fn no_failures_write_nothing_and_leave_the_old_file() {
        // P-16
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        assert_eq!(write(&file, Kind::Manga, &[], &names).expect("no write"), None);
        assert!(!file.exists(), "no file without failures");
        std::fs::write(&file, "old").expect("old log");
        assert_eq!(write(&file, Kind::Manga, &[], &names).expect("no write"), None);
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "old", "an old log is not touched");
    }

    #[test]
    fn each_pass_overwrites_the_file_in_the_cache_directory() {
        // T-26a-096
        struct Fixed(PathBuf);
        impl Paths for Fixed {
            fn data_dir(&self) -> PathBuf { self.0.join("data") }
            fn config_dir(&self) -> PathBuf { self.0.join("config") }
            fn state_dir(&self) -> PathBuf { self.0.join("state") }
            fn cache_dir(&self) -> PathBuf { self.0.join("cache") }
            fn log_dir(&self) -> PathBuf { self.0.join("log") }
            fn temp_dir(&self) -> PathBuf { self.0.join("tmp") }
            fn crash_dir(&self) -> PathBuf { self.0.join("crash") }
            fn covers_dir(&self) -> PathBuf { self.0.join("covers") }
            fn plugin_dir(&self) -> PathBuf { self.0.join("plugins") }
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let paths = Fixed(dir.path().to_path_buf());
        let file = path(&paths);
        assert_eq!(file, dir.path().join("cache").join("update_errors.txt"));
        let first = write(&file, Kind::Manga, &fixture_failures(), &names).expect("write").expect("path");
        assert_eq!(first, file);
        write(&file, Kind::Anime, &[failure(2, "Only", "boom")], &names).expect("second pass");
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.contains("- Only") && !text.contains("A1"), "the second pass replaced the first log");
    }
}
