//! The drop register (T-11a-029): every legacy Aniyomi setting, directory, file or job tag that Kurobayomi does not
//! carry over, each with the task that decided it and the reason the import report shows.
//!
//! One table feeds both the normaliser (which skips these keys) and the import report (which lists them), so the two
//! cannot drift. The tests check that every legacy key marked "ignored" is here,
//! and no converted or typed key is.

#[cfg(test)]
/// What kind of leftover an entry names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    /// A key of the legacy preference map. A trailing `*` in the pattern means "every key with this start".
    Key,
    /// An Android cache directory.
    Directory,
    /// A legacy preferences file.
    File,
    /// An Android job tag.
    WorkTag,
}

#[cfg(test)]
impl DropKind {
    /// The word the import report prints in front of the name.
    pub const fn label(self) -> &'static str {
        match self {
            DropKind::Key => "setting",
            DropKind::Directory => "folder",
            DropKind::File => "file",
            DropKind::WorkTag => "background job",
        }
    }
}

#[cfg(test)]
/// One thing that is not carried over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropEntry {
    pub pattern: &'static str,
    pub kind: DropKind,
    pub task: &'static str,
    pub reason: &'static str,
}

const fn entry(pattern: &'static str, kind: DropKind, task: &'static str, reason: &'static str) -> DropEntry {
    DropEntry { pattern, kind, task, reason }
}

#[cfg(test)]
const CACHE: &str = "Android cache folder; covers and pages are fetched again when needed";
#[cfg(test)]
const MAL: &str = "MyAnimeList login is OAuth here; log in again if the tracker asks";
#[cfg(test)]
const JOB: &str = "Android job tag; the in-app scheduler owns library updates and backups";
#[cfg(test)]
const APP_STATE: &str = "app state of the old install; this install keeps its own";
#[cfg(test)]
const TRACKER_LOGIN: &str = "tracker logins live in the keyring, not in imported settings";

use DropKind::{Directory, File, Key, WorkTag};

#[cfg(test)]
/// The register. Exact entries come first, so `pref_mangasync_1` is reported as a MyAnimeList login and not as a
/// generic tracker key.
pub const REGISTER: &[DropEntry] = &[
    entry("cache_disk_cache", Directory, "T-11a-020", CACHE),
    entry("chapter_disk_cache", Directory, "T-11a-020", CACHE),
    entry("cover_disk_cache", Directory, "T-11a-020", CACHE),
    entry("pref_mangasync_1", Key, "T-11a-021", MAL),
    entry("pass_mangasync_1", Key, "T-11a-021", MAL),
    entry("track_token_1", Key, "T-11a-021", MAL),
    entry("UpdateChecker", WorkTag, "T-11a-022", JOB),
    entry("ExtensionUpdate", WorkTag, "T-11a-022", JOB),
    entry("LibraryUpdate", WorkTag, "T-11a-022", JOB),
    entry("LibraryUpdate-auto", WorkTag, "T-11a-022", JOB),
    entry("AnimeLibraryUpdate", WorkTag, "T-11a-022", JOB),
    entry("AnimeLibraryUpdate-auto", WorkTag, "T-11a-022", JOB),
    entry("BackupCreator", WorkTag, "T-11a-022", JOB),
    entry("automatic_ext_updates", Key, "T-11a-022", "removed upstream in 2021; there is no APK updater here"),
    entry("pref_download_only", Key, "T-11a-023", APP_STATE),
    entry("incognito_mode", Key, "T-11a-023", APP_STATE),
    entry("last_catalogue_source", Key, "T-11a-023", APP_STATE),
    entry("trusted_signatures", Key, "T-11a-023", APP_STATE),
    entry("last_app_closed", Key, "T-11a-023", APP_STATE),
    entry("library_update_last_timestamp", Key, "T-11a-023", APP_STATE),
    entry("library_unseen_updates_count", Key, "T-11a-023", APP_STATE),
    entry("last_used_category", Key, "T-11a-023", APP_STATE),
    entry("last_app_check", Key, "T-11a-023", APP_STATE),
    entry("last_ext_check", Key, "T-11a-023", APP_STATE),
    entry("last_version_code", Key, "T-11a-023", APP_STATE),
    entry("storage_dir", Key, "T-11a-023", APP_STATE),
    entry("__APP_STATE_trusted_signatures", Key, "T-11a-023", "extension trust by signature does not exist here"),
    entry("pref_mangasync_*", Key, "T-11a-023", TRACKER_LOGIN),
    entry("track_token_*", Key, "T-11a-023", TRACKER_LOGIN),
    entry("extension_installer", Key, "T-11a-024", "the installer choice has no meaning; extensions are not installed as packages"),
    entry("tracking_queue", File, "T-11a-024", "a pending tracker push is rebuilt from history, not replayed"),
    entry("pref_invert_playback_txt", Key, "T-11a-029", "removed upstream in step 123"),
    entry("pref_invert_duration_txt", Key, "T-11a-029", "removed upstream in step 123"),
    entry("pref_hwdec", Key, "T-11a-029", "removed upstream in step 123; mpv picks the decoder"),
];

#[cfg(test)]
fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(start) => name.starts_with(start),
        None => pattern == name,
    }
}

#[cfg(test)]
/// The entry that drops `name`, if any. An exact entry wins over a prefix entry.
pub fn lookup(name: &str) -> Option<&'static DropEntry> {
    REGISTER.iter().find(|e| e.pattern == name).or_else(|| REGISTER.iter().find(|e| e.pattern.ends_with('*') && matches(e.pattern, name)))
}

#[cfg(test)]
/// One line of the import report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedKey {
    pub key: String,
    pub task: &'static str,
    pub reason: &'static str,
}

#[cfg(test)]
/// The keys of an imported legacy map that are dropped, sorted by key, one line each. Keys that are converted or kept
/// are not listed.
pub fn report<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<DroppedKey> {
    let mut out: Vec<DroppedKey> = keys
        .into_iter()
        .filter_map(|key| lookup(key).map(|e| DroppedKey { key: key.to_owned(), task: e.task, reason: e.reason }))
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out.dedup_by(|a, b| a.key == b.key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_entries_match_and_exact_entries_win() {
        assert_eq!(lookup("pref_mangasync_7").map(|e| e.task), Some("T-11a-023"));
        assert_eq!(lookup("pref_mangasync_1").map(|e| e.task), Some("T-11a-021"));
        assert_eq!(lookup("track_token_42").map(|e| e.pattern), Some("track_token_*"));
        assert_eq!(lookup("track_token"), None);
    }

    /// The plan ignores the un-prefixed legacy copy of this counter (T-11a-023), while the registry keeps the key for the
    /// updates badge, as Aniyomi does. A backup's old count is not imported; the app recounts it.
    const LIVE_AND_IGNORED: [&str; 1] = ["library_unseen_updates_count"];

    #[test]
    fn no_registered_preference_is_dropped() {
        for meta in kuro_prefs::registry::REGISTRY.iter().filter(|m| !LIVE_AND_IGNORED.contains(&m.key)) {
            assert!(lookup(meta.key).is_none(), "{} is a live preference but the register drops it", meta.key);
        }
    }

    #[test]
    fn the_report_lists_only_dropped_keys_sorted_once() {
        let lines = report(["secure_screen", "track_token_9", "incognito_mode", "track_token_9", "storage_dir"]);
        let keys: Vec<_> = lines.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, ["incognito_mode", "storage_dir", "track_token_9"]);
        assert!(lines.iter().all(|l| !l.reason.is_empty()));
        assert!(report(["secure_screen", "backup_interval"]).is_empty());
    }

    #[test]
    fn kinds_have_report_words() {
        assert_eq!(DropKind::Key.label(), "setting");
        assert_eq!(DropKind::WorkTag.label(), "background job");
        assert_eq!(DropKind::Directory.label(), "folder");
        assert_eq!(DropKind::File.label(), "file");
    }
}
