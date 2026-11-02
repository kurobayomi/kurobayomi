//! Crash restore of the chapter and the page (T-43a-039, E-RESTORE-1). The reader writes `(manga, chapter, page)` on
//! every page change, and a reader that opens the same manga without an explicit chapter within 24 hours starts
//! from it, unless `restore_after_crash` is off. An explicit chapter always wins. An incognito session never saves.
//!
//! The record is one small file in the platform data directory. The write goes through the reader's writer thread
//! (`writer::Command::Restore`), which keeps only the last record of a batch, and the file is replaced atomically, so a
//! crash between a page change and the write loses at most the last page change (T-43a-091).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The record's file name in the data directory.
pub const FILE_NAME: &str = "reader_session.json";

/// How long a record stays usable: 24 hours (a PROPOSAL of E-RESTORE-1; the Kotlin app has no window).
pub const WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// What was on screen when the record was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    pub manga_id: i64,
    pub chapter_id: i64,
    /// The 0-based page index of the chapter.
    pub page_index: i64,
    /// When the record was written, in ms since the epoch.
    pub saved_at: i64,
}

/// Where the reader starts: the chapter to open and the page to show in it when a record restored it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    pub chapter_id: i64,
    pub page_index: Option<i64>,
}

/// The record's path inside `data_dir`.
pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// A chapter id that names a chapter. The entry screens pass `0` when they could not resolve one, and `-1` is the
/// Kotlin "none".
fn is_explicit(chapter_id: i64) -> bool {
    chapter_id > 0
}

/// The start for a request. An explicit chapter wins over any record. Without one, a record of the same manga that is
/// at most 24 hours old gives its chapter and page, when restoring is on. A record from the future (a clock that went
/// back) is not used.
pub fn resolve(requested_manga: i64, requested_chapter: i64, saved: Option<SavedSession>, now_ms: i64, enabled: bool) -> Start {
    let plain = Start { chapter_id: requested_chapter, page_index: None };
    if is_explicit(requested_chapter) || !enabled {
        return plain;
    }
    match saved {
        Some(s) if s.manga_id == requested_manga && s.chapter_id > 0 && s.page_index >= 0 => {
            let age = now_ms - s.saved_at;
            if (0..=WINDOW_MS).contains(&age) {
                Start { chapter_id: s.chapter_id, page_index: Some(s.page_index) }
            } else {
                plain
            }
        }
        _ => plain,
    }
}

/// Reads the record. A missing or unreadable file is no record.
pub fn load(path: &Path) -> Option<SavedSession> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Writes the record through a temporary file and a rename, so a crash never leaves half a record.
pub fn save(path: &Path, session: &SavedSession) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(session).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10 * WINDOW_MS;

    fn saved(manga_id: i64, age: i64) -> SavedSession {
        SavedSession { manga_id, chapter_id: 7, page_index: 3, saved_at: NOW - age }
    }

    #[test]
    fn a_fresh_record_of_the_same_manga_restores_its_chapter_and_page() {
        assert_eq!(resolve(1, 0, Some(saved(1, 1_000)), NOW, true), Start { chapter_id: 7, page_index: Some(3) });
        assert_eq!(resolve(1, -1, Some(saved(1, WINDOW_MS)), NOW, true).chapter_id, 7, "the window is inclusive");
    }

    #[test]
    fn an_explicit_chapter_always_wins() {
        assert_eq!(resolve(1, 9, Some(saved(1, 1_000)), NOW, true), Start { chapter_id: 9, page_index: None });
    }

    #[test]
    fn the_setting_off_an_old_record_another_manga_and_a_future_record_restore_nothing() {
        let none = Start { chapter_id: 0, page_index: None };
        assert_eq!(resolve(1, 0, Some(saved(1, 1_000)), NOW, false), none);
        assert_eq!(resolve(1, 0, Some(saved(1, WINDOW_MS + 1)), NOW, true), none);
        assert_eq!(resolve(1, 0, Some(saved(2, 1_000)), NOW, true), none);
        assert_eq!(resolve(1, 0, Some(saved(1, -5)), NOW, true), none);
        assert_eq!(resolve(1, 0, None, NOW, true), none);
    }

    #[test]
    fn a_record_round_trips_through_its_file_and_a_bad_file_is_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir.path().join("data"));
        assert_eq!(load(&path), None);
        let record = saved(1, 0);
        save(&path, &record).unwrap();
        assert_eq!(load(&path), Some(record));
        let newer = SavedSession { page_index: 8, ..record };
        save(&path, &newer).unwrap();
        assert_eq!(load(&path), Some(newer), "a later write replaces the record");
        std::fs::write(&path, b"{").unwrap();
        assert_eq!(load(&path), None);
    }
}
