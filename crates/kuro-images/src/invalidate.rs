//! Decision logic for `prep_update_cover` and `remove_covers` and the
//! timestamp writes that follow them.
//!
//! The functions return the entry the caller should store. The write itself goes through `TimestampStore`,
//! which the database layer implements, so this crate does not depend on the database.

use kuro_platform::kind::Kind;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use kuro_platform::paths::Paths;

use crate::art::{ArtKind, ArtRef};
use crate::cover_file::{delete_custom_cover, delete_from_cache, set_custom_from_file};
use crate::cover_paths;
use crate::error::ImageError;
use crate::messages::Message;

/// Epoch milliseconds now (`Instant.now().toEpochMilli()` in Kotlin).
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Applies a fresh thumbnail URL from the source to a cover.
///
/// A missing or empty remote URL changes nothing. The same URL changes nothing unless `refresh_same_url`.
/// A local entry only gets a new timestamp. An entry with a custom cover keeps its timestamp, and only its
/// thumbnail file is deleted. Any other entry loses its thumbnail file and gets a new timestamp.
pub fn prep_update_cover(
    paths: &dyn Paths,
    entry: &ArtRef,
    remote_url: Option<&str>,
    refresh_same_url: bool,
    now_ms: i64,
) -> ArtRef {
    let Some(new_url) = remote_url.filter(|url| !url.is_empty()) else {
        return entry.clone();
    };
    if !refresh_same_url && entry.url.as_deref() == Some(new_url) {
        return entry.clone();
    }
    if entry.is_local() {
        return ArtRef {
            last_modified: now_ms,
            ..entry.clone()
        };
    }
    delete_from_cache(paths, entry, false);
    if cover_paths::has_custom(paths, entry) {
        return entry.clone();
    }
    ArtRef {
        last_modified: now_ms,
        ..entry.clone()
    }
}

/// Deletes the thumbnail and custom cover of an entry leaving the library. The
/// timestamp moves only when at least one file was deleted. Local entries are never touched.
pub fn remove_covers(paths: &dyn Paths, entry: &ArtRef, now_ms: i64) -> ArtRef {
    if entry.is_local() {
        return entry.clone();
    }
    if delete_from_cache(paths, entry, true) > 0 {
        ArtRef {
            last_modified: now_ms,
            ..entry.clone()
        }
    } else {
        entry.clone()
    }
}

/// The removal branch of `toggleFavorite` for covers. Deletes the covers, then writes
/// the new cover timestamp only when the returned entry differs from the input.
pub fn remove_from_library(
    paths: &dyn Paths,
    store: &dyn TimestampStore,
    entry: &ArtRef,
    now_ms: i64,
) -> Result<ArtRef, ImageError> {
    let removed = remove_covers(paths, entry, now_ms);
    if removed != *entry {
        await_update_cover_last_modified(store, entry.owner, entry.entry_id, now_ms)?;
    }
    Ok(removed)
}

/// Which timestamp column of an entry to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    CoverLastModified,
    BackgroundLastModified,
}

/// Writes one timestamp column of one entry and nothing else. The database layer
/// implements this over `cover_last_modified` and `background_last_modified`.
pub trait TimestampStore: Send + Sync {
    fn set_last_modified(
        &self,
        owner: Kind,
        entry_id: i64,
        column: Column,
        ms: i64,
    ) -> Result<(), ImageError>;
}

/// `awaitUpdateCoverLastModified`: writes `now_ms` to the cover column.
pub fn await_update_cover_last_modified(
    store: &dyn TimestampStore,
    owner: Kind,
    entry_id: i64,
    now_ms: i64,
) -> Result<(), ImageError> {
    store.set_last_modified(owner, entry_id, Column::CoverLastModified, now_ms)
}

/// `awaitUpdateBackgroundLastModified` (anime only): writes `now_ms` to the background column.
pub fn await_update_background_last_modified(
    store: &dyn TimestampStore,
    entry_id: i64,
    now_ms: i64,
) -> Result<(), ImageError> {
    store.set_last_modified(
        Kind::Anime,
        entry_id,
        Column::BackgroundLastModified,
        now_ms,
    )
}

/// Writes the timestamp column that belongs to this image: the background column for an anime background, the
/// cover column otherwise.
fn bump(store: &dyn TimestampStore, art: &ArtRef, now_ms: i64) -> Result<(), ImageError> {
    match (art.owner, art.kind) {
        (Kind::Anime, ArtKind::Background) => {
            await_update_background_last_modified(store, art.entry_id, now_ms)
        }
        _ => await_update_cover_last_modified(store, art.owner, art.entry_id, now_ms),
    }
}

fn outcome(art: &ArtRef, ok: bool) -> Message {
    match (
        art.kind == ArtKind::Background && art.owner == Kind::Anime,
        ok,
    ) {
        (false, true) => Message::CoverUpdated,
        (false, false) => Message::FailedToUpdateCover,
        (true, true) => Message::BackgroundUpdated,
        (true, false) => Message::FailedToUpdateBackground,
    }
}

/// The Edit action: copies the picked file into the custom image, then bumps the timestamp so the keyer changes.
/// Returns the snackbar message. An entry that is not in the library is refused for
/// both kinds (edge E-IMG-14, decision 11c; the same rule as `kuro_local::images::art_route`). Local entries are not
/// handled here: their own folder is written by area:14, so they report a failure rather than writing a stray custom
/// file.
pub async fn edit_custom(
    paths: &dyn Paths,
    store: &dyn TimestampStore,
    art: &ArtRef,
    picked: &Path,
    now_ms: i64,
) -> Message {
    let allowed = !art.is_local() && art.favorite;
    let ok = allowed
        && set_custom_from_file(paths, art.family(), art.entry_id, picked)
            .await
            .is_ok()
        && bump(store, art, now_ms).is_ok();
    outcome(art, ok)
}

/// The Delete action: removes the custom file, then always bumps the timestamp, so a
/// refresh follows even when the file was already gone.
pub fn delete_custom(
    paths: &dyn Paths,
    store: &dyn TimestampStore,
    art: &ArtRef,
    now_ms: i64,
) -> Message {
    let ok = !art.is_local() && {
        delete_custom_cover(paths, art.family(), art.entry_id);
        bump(store, art, now_ms).is_ok()
    };
    outcome(art, ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{ArtKind, LOCAL_SOURCE_ID};
    use crate::cover_paths::{self, Family};
    use crate::testing::TestPaths;
    use std::sync::Mutex;

    const NOW: i64 = 1_700_000_000_000;
    const OLD: i64 = 100;
    const A: &str = "https://example.invalid/a.jpg";
    const B: &str = "https://example.invalid/b.jpg";

    fn entry(source_id: i64) -> ArtRef {
        ArtRef {
            owner: Kind::Manga,
            entry_id: 7,
            source_id,
            kind: ArtKind::Cover,
            url: Some(A.to_owned()),
            favorite: true,
            last_modified: OLD,
        }
    }

    /// Writes a thumbnail file for the entry's current URL and returns its path.
    fn thumbnail(paths: &TestPaths, e: &ArtRef) -> std::path::PathBuf {
        let file = cover_paths::cover_file_for(paths, e).unwrap();
        std::fs::write(&file, b"thumb").unwrap();
        file
    }

    fn custom(paths: &TestPaths, e: &ArtRef) -> std::path::PathBuf {
        let file = cover_paths::custom_file_for(paths, e);
        std::fs::write(&file, b"custom").unwrap();
        file
    }

    #[test]
    fn c1_null_remote_url_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let file = thumbnail(&paths, &e);
        assert_eq!(prep_update_cover(&paths, &e, None, false, NOW), e);
        assert!(file.exists());
    }

    #[test]
    fn c2_empty_remote_url_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let file = thumbnail(&paths, &e);
        assert_eq!(prep_update_cover(&paths, &e, Some(""), false, NOW), e);
        assert!(file.exists());
    }

    #[test]
    fn c3_same_url_without_refresh_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let file = thumbnail(&paths, &e);
        assert_eq!(prep_update_cover(&paths, &e, Some(A), false, NOW), e);
        assert!(file.exists());
    }

    #[test]
    fn c4_same_url_with_refresh_deletes_and_bumps() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let file = thumbnail(&paths, &e);
        let out = prep_update_cover(&paths, &e, Some(A), true, NOW);
        assert_eq!(out.last_modified, NOW);
        assert!(!file.exists());
    }

    #[test]
    fn c5_local_entry_only_bumps() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(LOCAL_SOURCE_ID);
        let file = thumbnail(&paths, &e);
        let out = prep_update_cover(&paths, &e, Some(B), false, NOW);
        assert_eq!(out.last_modified, NOW);
        assert!(file.exists());
    }

    #[test]
    fn c6_custom_cover_keeps_timestamp_and_custom_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let thumb = thumbnail(&paths, &e);
        let custom_file = custom(&paths, &e);
        let out = prep_update_cover(&paths, &e, Some(B), false, NOW);
        assert_eq!(out.last_modified, OLD);
        assert!(!thumb.exists());
        assert!(custom_file.exists());
    }

    #[test]
    fn c7_remote_entry_without_custom_deletes_and_bumps() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let thumb = thumbnail(&paths, &e);
        let out = prep_update_cover(&paths, &e, Some(B), false, NOW);
        assert_eq!(out.last_modified, NOW);
        assert!(!thumb.exists());
    }

    #[test]
    fn c8_remove_covers_deletes_both_and_bumps() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        let thumb = thumbnail(&paths, &e);
        let custom_file = custom(&paths, &e);
        let out = remove_covers(&paths, &e, NOW);
        assert_eq!(out.last_modified, NOW);
        assert!(!thumb.exists() && !custom_file.exists());
        assert_eq!(delete_from_cache(&paths, &e, true), 0);
    }

    #[test]
    fn c9_remove_covers_without_files_returns_the_same_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        assert_eq!(remove_covers(&paths, &e, NOW), e);
    }

    #[test]
    fn c10_remove_covers_on_local_entry_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(LOCAL_SOURCE_ID);
        let thumb = thumbnail(&paths, &e);
        assert_eq!(remove_covers(&paths, &e, NOW), e);
        assert!(thumb.exists());
    }

    #[test]
    fn c11_delete_custom_cover_when_absent_is_false() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert!(!crate::cover_file::delete_custom_cover(
            &paths,
            Family::MangaCover,
            7
        ));
    }

    #[test]
    fn remove_covers_bumps_only_when_a_file_was_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = entry(1);
        thumbnail(&paths, &e);
        assert_eq!(remove_covers(&paths, &e, NOW).last_modified, NOW);
        assert_eq!(remove_covers(&paths, &e, NOW), e);
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(Kind, i64, Column, i64)>>);

    impl TimestampStore for Recorder {
        fn set_last_modified(
            &self,
            owner: Kind,
            entry_id: i64,
            column: Column,
            ms: i64,
        ) -> Result<(), ImageError> {
            self.0.lock().unwrap().push((owner, entry_id, column, ms));
            Ok(())
        }
    }

    #[test]
    fn await_update_writes_only_the_cover_column() {
        let store = Recorder::default();
        await_update_cover_last_modified(&store, Kind::Manga, 3, NOW).unwrap();
        assert_eq!(
            *store.0.lock().unwrap(),
            vec![(Kind::Manga, 3, Column::CoverLastModified, NOW)]
        );
    }

    #[test]
    fn await_update_background_writes_only_the_background_column() {
        let store = Recorder::default();
        await_update_background_last_modified(&store, 4, NOW).unwrap();
        assert_eq!(
            *store.0.lock().unwrap(),
            vec![(Kind::Anime, 4, Column::BackgroundLastModified, NOW)]
        );
    }

    #[test]
    fn removal_from_library_writes_the_bump_only_when_something_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let store = Recorder::default();
        let e = entry(1);
        thumbnail(&paths, &e);
        let out = remove_from_library(&paths, &store, &e, NOW).unwrap();
        assert_eq!(out.last_modified, NOW);
        assert_eq!(
            *store.0.lock().unwrap(),
            vec![(Kind::Manga, 7, Column::CoverLastModified, NOW)]
        );

        let untouched = Recorder::default();
        let same = remove_from_library(&paths, &untouched, &e, NOW).unwrap();
        assert_eq!(same, e);
        assert!(untouched.0.lock().unwrap().is_empty());
    }

    #[test]
    fn now_millis_is_after_2020() {
        assert!(now_millis() > 1_577_836_800_000);
    }

    #[tokio::test]
    async fn edit_copies_the_file_and_bumps_the_cover_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let picked = tmp.path().join("pick.png");
        std::fs::write(&picked, b"new").unwrap();
        let store = Recorder::default();
        let e = entry(1);
        let got = edit_custom(&paths, &store, &e, &picked, NOW).await;
        assert_eq!(got, Message::CoverUpdated);
        assert_eq!(
            std::fs::read(cover_paths::custom_file_for(&paths, &e)).unwrap(),
            b"new"
        );
        assert_eq!(
            *store.0.lock().unwrap(),
            vec![(Kind::Manga, 7, Column::CoverLastModified, NOW)]
        );
    }

    #[tokio::test]
    async fn a_failed_edit_writes_no_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let store = Recorder::default();
        let got = edit_custom(&paths, &store, &entry(1), &tmp.path().join("gone"), NOW).await;
        assert_eq!(got, Message::FailedToUpdateCover);
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_entry_outside_the_library_or_a_local_entry_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let picked = tmp.path().join("pick.png");
        std::fs::write(&picked, b"new").unwrap();
        let store = Recorder::default();
        let anime = ArtRef {
            owner: Kind::Anime,
            favorite: false,
            kind: ArtKind::Background,
            ..entry(1)
        };
        assert_eq!(
            edit_custom(&paths, &store, &anime, &picked, NOW).await,
            Message::FailedToUpdateBackground
        );
        let manga = ArtRef { favorite: false, ..entry(1) };
        assert_eq!(edit_custom(&paths, &store, &manga, &picked, NOW).await, Message::FailedToUpdateCover, "decision 11c: both kinds");
        assert_eq!(
            edit_custom(&paths, &store, &entry(LOCAL_SOURCE_ID), &picked, NOW).await,
            Message::FailedToUpdateCover
        );
        assert!(store.0.lock().unwrap().is_empty());
        assert!(!cover_paths::custom_file_for(&paths, &anime).exists());
    }

    #[test]
    fn delete_removes_the_file_and_bumps_the_background_column() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let e = ArtRef {
            owner: Kind::Anime,
            kind: ArtKind::Background,
            ..entry(1)
        };
        let file = custom(&paths, &e);
        let store = Recorder::default();
        assert_eq!(
            delete_custom(&paths, &store, &e, NOW),
            Message::BackgroundUpdated
        );
        assert!(!file.exists());
        // A second delete finds no file and still bumps, as Kotlin does.
        assert_eq!(
            delete_custom(&paths, &store, &e, NOW + 1),
            Message::BackgroundUpdated
        );
        assert_eq!(
            *store.0.lock().unwrap(),
            vec![
                (Kind::Anime, 7, Column::BackgroundLastModified, NOW),
                (Kind::Anime, 7, Column::BackgroundLastModified, NOW + 1),
            ]
        );
    }
}
