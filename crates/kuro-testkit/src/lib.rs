//! Shared test helpers for Kurobayomi crates: temporary directories, fixture
//! loading and golden-file comparison. Not a dependency of any production crate.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use kuro_platform::paths::Paths;

mod db;
mod settings;
mod tracker;

pub mod logs;
pub mod track_samples;
pub use db::{TestDb, insert_category};
pub use settings::InMemorySettings;
pub use tracker::FakeTracker;

/// Environment variable that makes [`assert_golden`] rewrite golden files.
pub const UPDATE_GOLDEN_ENV: &str = "KURO_UPDATE_GOLDEN";

/// A temporary directory, deleted when dropped.
#[derive(Debug)]
pub struct TempDir(tempfile::TempDir);

impl TempDir {
    pub fn new() -> io::Result<Self> {
        tempfile::TempDir::new().map(Self)
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

/// [`Paths`] rooted in a temporary directory. Each role is a subdirectory, created on first use.
#[derive(Debug)]
pub struct TempPaths(TempDir);

impl TempPaths {
    pub fn new() -> io::Result<Self> {
        TempDir::new().map(Self)
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.path().join(name);
        fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("cannot create {}: {e}", path.display()));
        path
    }
}

impl Paths for TempPaths {
    fn data_dir(&self) -> PathBuf {
        self.dir("data")
    }

    fn config_dir(&self) -> PathBuf {
        self.dir("config")
    }

    fn state_dir(&self) -> PathBuf {
        self.dir("state")
    }

    fn cache_dir(&self) -> PathBuf {
        self.dir("cache")
    }

    fn log_dir(&self) -> PathBuf {
        self.dir("log")
    }

    fn temp_dir(&self) -> PathBuf {
        self.dir("temp")
    }

    fn crash_dir(&self) -> PathBuf {
        self.dir("crash")
    }

    fn covers_dir(&self) -> PathBuf {
        self.dir("covers")
    }

    fn plugin_dir(&self) -> PathBuf {
        self.dir("plugins")
    }
}

/// Reads `<manifest_dir>/tests/fixtures/<name>` as raw bytes.
///
/// `manifest_dir` is normally `env!("CARGO_MANIFEST_DIR")` of the calling crate.
pub fn load_fixture(manifest_dir: impl AsRef<Path>, name: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = manifest_dir.as_ref().join("tests/fixtures").join(name);
    fs::read(&path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

/// `tests/fixtures/settings/<name>`, e.g. `valid.toml`, `corrupt.toml`, `unknown_keys.toml`.
pub fn settings_fixture(name: &str) -> io::Result<Vec<u8>> {
    load_fixture(env!("CARGO_MANIFEST_DIR"), Path::new("settings").join(name))
}

/// `tests/fixtures/os-release/<name>`, e.g. `cachyos` or `missing` (no `PRETTY_NAME`).
pub fn os_release_fixture(name: &str) -> io::Result<Vec<u8>> {
    load_fixture(
        env!("CARGO_MANIFEST_DIR"),
        Path::new("os-release").join(name),
    )
}

/// `tests/fixtures/crash/<name>`, e.g. `valid.txt` or `unknown_magic.txt`.
pub fn crash_fixture(name: &str) -> io::Result<Vec<u8>> {
    load_fixture(env!("CARGO_MANIFEST_DIR"), Path::new("crash").join(name))
}

/// `tests/fixtures/db/<name>`, e.g. `too_new.db` (`PRAGMA user_version = 9999`).
pub fn db_fixture(name: &str) -> io::Result<Vec<u8>> {
    load_fixture(env!("CARGO_MANIFEST_DIR"), Path::new("db").join(name))
}

/// Writes `v0_empty.db` into `dir` and returns its path: a zero-byte file, which SQLite reads as an
/// empty database at `PRAGMA user_version = 0`.
pub fn v0_empty_db(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join("v0_empty.db");
    fs::write(&path, [])?;
    Ok(path)
}

/// A crash report of exactly 2 MiB in report format version 1 (the `huge.txt` fixture, generated).
pub fn huge_crash_report() -> String {
    const SIZE: usize = 2 * 1024 * 1024;
    let mut text = String::from("kurobayomi-crash-report 1\n");
    let line =
        "0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789\n";
    while text.len() + line.len() <= SIZE {
        text.push_str(line);
    }
    text.extend(std::iter::repeat_n('.', SIZE - text.len()));
    text
}

/// `lines` log lines with exactly `errors` ERROR lines spread evenly among INFO lines (the
/// `logs/mixed.log` fixture, generated).
pub fn mixed_log(lines: usize, errors: usize) -> String {
    assert!(errors <= lines, "more error lines than lines");
    let mut text = String::new();
    for i in 0..lines {
        // Bresenham spread: exactly `errors` of the `lines` indices are ERROR.
        let is_error = ((i + 1) * errors) / lines > (i * errors) / lines;
        let level = if is_error { "ERROR" } else { "INFO" };
        text.push_str(&format!(
            "2026-10-08T14:03:{:02}Z {level} kuro_core: event {i}\n",
            i % 60
        ));
    }
    text
}

/// Compares `actual` with the golden file at `path`.
///
/// With `KURO_UPDATE_GOLDEN=1` the file is (re)written instead of compared.
/// Panics on mismatch or when the golden file is missing.
pub fn assert_golden(path: impl AsRef<Path>, actual: impl AsRef<[u8]>) {
    let update = std::env::var(UPDATE_GOLDEN_ENV).as_deref() == Ok("1");
    check_golden(path.as_ref(), actual.as_ref(), update);
}

fn check_golden(path: &Path, actual: &[u8], update: bool) {
    if update {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("cannot create {}: {e}", parent.display()));
        }
        fs::write(path, actual)
            .unwrap_or_else(|e| panic!("cannot write golden {}: {e}", path.display()));
        return;
    }
    let expected = fs::read(path).unwrap_or_else(|e| {
        panic!(
            "cannot read golden {}: {e} (run with {UPDATE_GOLDEN_ENV}=1 to create it)",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "golden mismatch for {}: expected {} bytes, got {} (run with {UPDATE_GOLDEN_ENV}=1 to update)",
        path.display(),
        expected.len(),
        actual.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dir_exists_while_alive_and_is_removed_on_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        fs::write(path.join("f.txt"), b"x").unwrap();
        assert!(path.join("f.txt").is_file());
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn load_fixture_reads_bytes_from_tests_fixtures() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("tests/fixtures/sub")).unwrap();
        fs::write(root.path().join("tests/fixtures/sub/a.bin"), [0u8, 1, 255]).unwrap();
        let bytes = load_fixture(root.path(), "sub/a.bin").unwrap();
        assert_eq!(bytes, vec![0u8, 1, 255]);
    }

    #[test]
    fn load_fixture_error_names_the_missing_path() {
        let root = TempDir::new().unwrap();
        let err = load_fixture(root.path(), "missing.json").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("missing.json"));
    }

    #[test]
    fn golden_update_writes_file_and_creates_parents() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("nested/dir/out.txt");
        check_golden(&path, b"hello", true);
        assert_eq!(fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn golden_compare_passes_on_equal_bytes() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("g.txt");
        fs::write(&path, b"same").unwrap();
        check_golden(&path, b"same", false);
    }

    #[test]
    #[should_panic(expected = "golden mismatch")]
    fn golden_compare_panics_on_different_bytes() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("g.txt");
        fs::write(&path, b"old").unwrap();
        check_golden(&path, b"new", false);
    }

    #[test]
    #[should_panic(expected = "cannot read golden")]
    fn golden_compare_panics_when_file_is_missing() {
        let root = TempDir::new().unwrap();
        check_golden(&root.path().join("absent.txt"), b"x", false);
    }

    #[test]
    fn temp_paths_are_created_under_one_root() {
        let paths = TempPaths::new().unwrap();
        let root = paths.0.path().to_path_buf();
        let all = [
            paths.data_dir(),
            paths.config_dir(),
            paths.cache_dir(),
            paths.log_dir(),
            paths.temp_dir(),
            paths.crash_dir(),
            paths.covers_dir(),
            paths.plugin_dir(),
        ];
        for dir in &all {
            assert!(dir.starts_with(&root), "{}", dir.display());
            assert!(dir.is_dir(), "{}", dir.display());
        }
    }

    #[test]
    fn settings_fixtures_load() {
        let valid = String::from_utf8(settings_fixture("valid.toml").unwrap()).unwrap();
        assert!(valid.contains("verbose_logging = true"));
        let corrupt = settings_fixture("corrupt.toml").unwrap();
        assert!(!corrupt.is_empty() && !corrupt.ends_with(b"\n"));
        assert!(!settings_fixture("unknown_keys.toml").unwrap().is_empty());
    }

    #[test]
    fn os_release_fixtures_load() {
        let cachy = String::from_utf8(os_release_fixture("cachyos").unwrap()).unwrap();
        assert!(cachy.contains("PRETTY_NAME=\"CachyOS Linux\""));
        let missing = String::from_utf8(os_release_fixture("missing").unwrap()).unwrap();
        assert!(!missing.contains("PRETTY_NAME"));
    }

    #[test]
    fn crash_and_db_fixtures_load() {
        let valid = String::from_utf8(crash_fixture("valid.txt").unwrap()).unwrap();
        assert!(valid.starts_with("kurobayomi-crash-report 1\n"));
        let unknown = String::from_utf8(crash_fixture("unknown_magic.txt").unwrap()).unwrap();
        assert!(unknown.starts_with("kurobayomi-crash-report 2\n"));
        let db = db_fixture("too_new.db").unwrap();
        assert!(db.starts_with(b"SQLite format 3\0"));
        // user_version is the big-endian u32 at byte offset 60
        assert_eq!(u32::from_be_bytes(db[60..64].try_into().unwrap()), 9999);
    }

    #[test]
    fn huge_crash_report_is_two_mib_and_keeps_the_magic_line() {
        let text = huge_crash_report();
        assert_eq!(text.len(), 2 * 1024 * 1024);
        assert!(text.starts_with("kurobayomi-crash-report 1\n"));
    }

    #[test]
    fn v0_empty_db_is_an_empty_database_at_version_zero() {
        let root = TempDir::new().unwrap();
        let path = v0_empty_db(root.path()).unwrap();
        assert_eq!(path.parent(), Some(root.path()));
        let c = rusqlite::Connection::open(&path).unwrap();
        let version: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let objects: i64 = c
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        assert_eq!((version, objects), (0, 0));
    }

    #[test]
    fn mixed_log_has_the_requested_lines_and_error_count() {
        let text = mixed_log(1000, 600);
        assert_eq!(text.lines().count(), 1000);
        assert_eq!(text.lines().filter(|l| l.contains(" ERROR ")).count(), 600);
    }
}
