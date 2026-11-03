//! Restore error log (area 22, T-22a-030). Only the last run's log exists: each run with errors
//! replaces the file, and a run without errors writes nothing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::sections::ErrorLine;

/// File name kept from the Kotlin app.
pub const LOG_FILE: &str = "aniyomi_restore_error.txt";

/// One log line without its newline: `[yyyy-MM-dd HH:mm:ss.SSS] message`, local time, ASCII digits. Backslash, CR and LF
/// in the message are escaped, so every error stays on one line (E-69).
pub fn line(error: &ErrorLine) -> String {
    let message = error.message.replace('\\', "\\\\").replace('\r', "\\r").replace('\n', "\\n");
    format!("[{}] {}", error.at.format("%Y-%m-%d %H:%M:%S%.3f"), message)
}

/// Writes the log into `dir`, creating the directory when missing (E-71). Returns the file path, or `None` when there
/// are no errors and nothing is written.
pub fn write_error_log(dir: &Path, errors: &[ErrorLine]) -> io::Result<Option<PathBuf>> {
    if errors.is_empty() {
        return Ok(None);
    }
    fs::create_dir_all(dir)?;
    let mut text = String::new();
    for error in errors {
        text.push_str(&line(error));
        text.push('\n');
    }
    let path = dir.join(LOG_FILE);
    fs::write(&path, text)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone, Timelike};

    use super::*;

    fn at() -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 9, 8, 5, 3).single().unwrap().with_nanosecond(42_000_000).unwrap()
    }

    fn error(message: &str) -> ErrorLine {
        ErrorLine { at: at(), message: message.into() }
    }

    #[test]
    fn line_has_millisecond_timestamp_and_message() {
        assert_eq!(line(&error("Berserk [Mock]: boom")), "[2026-10-09 08:05:03.042] Berserk [Mock]: boom");
    }

    #[test]
    fn newlines_in_a_message_stay_on_one_line() {
        assert_eq!(line(&error("a\nb\r\\c")), "[2026-10-09 08:05:03.042] a\\nb\\r\\\\c");
    }

    #[test]
    fn no_errors_writes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(write_error_log(dir.path(), &[]).unwrap(), None);
        assert!(!dir.path().join(LOG_FILE).exists());
    }

    #[test]
    fn each_run_overwrites_the_previous_log() {
        let dir = tempfile::tempdir().unwrap();
        write_error_log(dir.path(), &[error("first")]).unwrap();
        let path = write_error_log(dir.path(), &[error("second"), error("third")]).unwrap().unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(!text.contains("first"));
        assert!(text.ends_with(" third\n"));
    }

    #[test]
    fn missing_directory_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("kurobayomi").join("cache");
        let path = write_error_log(&nested, &[error("x")]).unwrap().unwrap();
        assert_eq!(path, nested.join(LOG_FILE));
        assert!(path.is_file());
    }

    #[test]
    fn write_failure_is_an_error_not_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, b"not a directory").unwrap();
        assert!(write_error_log(&blocker.join("sub"), &[error("x")]).is_err());
    }
}
