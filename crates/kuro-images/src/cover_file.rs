//! Library cover file cache: writes the source to the cover file and promotes it,
//! and the custom-cover write and delete paths.
//!
//! Cover files are plain files under `<data>`, so they survive a cache clear. `write_atomic` is the
//! `write_source_to_cover_file` of Kotlin with a rename at the end, so a reader never sees a partial file.

use kuro_platform::kind::Kind;
use std::io;
use std::path::{Path, PathBuf};

use kuro_platform::paths::Paths;
use tokio::io::{AsyncRead, AsyncWriteExt};

use crate::art::ArtRef;
use crate::cover_paths::{self, Family};
use crate::disk_image_cache::DiskImageCache;

/// Removes a temporary file on drop, unless it was disarmed after the rename. A failed or cancelled write
/// therefore leaves no `.part` file behind (section 9.1 step 6).
struct PartFile(Option<PathBuf>);

impl PartFile {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Writes `bytes` to `target`: creates the parent folder, writes `<name>.part`, then renames it over
/// `target`. A reader that opened the old file keeps it, and a failure leaves `target` untouched.
pub async fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "target has no file name"))?;
    let part = target.with_file_name(format!("{}.part", name.to_string_lossy()));
    let guard = PartFile(Some(part.clone()));
    tokio::fs::write(&part, bytes).await?;
    tokio::fs::rename(&part, target).await?;
    guard.disarm();
    Ok(())
}

/// Copies a disk-cache snapshot into the library cover file, then removes the disk entry, so a library
/// entry's image lives in one place.
pub async fn promote_snapshot(
    disk: &DiskImageCache,
    key: &str,
    snapshot: &Path,
    cover: &Path,
) -> io::Result<()> {
    let bytes = tokio::fs::read(snapshot).await?;
    write_atomic(cover, &bytes).await?;
    disk.remove(key).await?;
    Ok(())
}

/// Copies the whole stream into the custom file, truncating an existing one. No validation of the bytes.
/// Any I/O error is returned and nothing else is changed.
pub async fn set_custom<R>(
    paths: &dyn Paths,
    family: Family,
    entry_id: i64,
    mut reader: R,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let target = cover_paths::custom_file(paths, family, entry_id);
    let mut file = tokio::fs::File::create(&target).await?;
    tokio::io::copy(&mut reader, &mut file).await?;
    file.flush().await?;
    Ok(())
}

/// Copies a picked local image into the custom file of the entry, replacing the old one.
/// The bytes are not checked. A missing source is an error and changes nothing.
pub async fn set_custom_from_file(
    paths: &dyn Paths,
    family: Family,
    entry_id: i64,
    source: &Path,
) -> io::Result<()> {
    let file = tokio::fs::File::open(source).await?;
    set_custom(paths, family, entry_id, file).await
}

/// Deletes the custom file of an entry. `true` only when the file existed and was removed.
pub fn delete_custom_cover(paths: &dyn Paths, family: Family, entry_id: i64) -> bool {
    remove_if_exists(&cover_paths::custom_file(paths, family, entry_id))
}

/// Deletes the thumbnail or background file of the image and, when asked, its custom file. Returns how many
/// files were actually deleted, so a thumbnail plus a custom cover counts 2.
pub fn delete_from_cache(paths: &dyn Paths, art: &ArtRef, delete_custom: bool) -> usize {
    let mut deleted = 0;
    if let Some(file) = cover_paths::cover_file_for(paths, art) {
        if remove_if_exists(&file) {
            deleted += 1;
        }
    }
    if delete_custom && delete_custom_cover(paths, art.family(), art.entry_id) {
        deleted += 1;
    }
    deleted
}

/// Copies the custom file of `from_id` to `to_id` during a migration (T-34a-051). The manga
/// migration copies its cover. The anime migration calls it for the cover and for the background. A missing source is
/// not an error. A failed copy is logged, the partial target is removed, and `false` is returned, so the migration
/// carries on.
pub fn copy_custom(paths: &dyn Paths, family: Family, from_id: i64, to_id: i64) -> bool {
    let from = cover_paths::custom_file(paths, family, from_id);
    if !from.is_file() {
        return false;
    }
    let to = cover_paths::custom_file(paths, family, to_id);
    match std::fs::copy(&from, &to) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(path = %to.display(), error = %e, "cannot copy custom cover during migration");
            let _ = std::fs::remove_file(&to);
            false
        }
    }
}

/// The migration step (T-34a-051): copies the custom cover of `from_id` to `to_id`, and for an anime entry its custom
/// background too. Nothing here can fail the migration: a missing source or a failed copy is skipped (and logged by
/// `copy_custom`). Returns how many files were copied.
pub fn migrate_custom(paths: &dyn Paths, owner: Kind, from_id: i64, to_id: i64) -> usize {
    let families: &[Family] = match owner {
        Kind::Manga => &[Family::MangaCover],
        Kind::Anime => &[Family::AnimeCover, Family::AnimeBackground],
    };
    families.iter().filter(|family| copy_custom(paths, **family, from_id, to_id)).count()
}

fn remove_if_exists(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "cannot delete cached image");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestPaths;

    #[tokio::test]
    async fn write_atomic_creates_parents_and_replaces_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a/b/cover");
        write_atomic(&target, b"first").await.unwrap();
        write_atomic(&target, b"second").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second");
        assert!(!tmp.path().join("a/b/cover.part").exists());
    }

    #[tokio::test]
    async fn failed_write_leaves_no_part_file() {
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"file, not a folder").unwrap();
        let target = blocker.join("cover");
        assert!(write_atomic(&target, b"x").await.is_err());
        assert_eq!(std::fs::read(&blocker).unwrap(), b"file, not a folder");
    }

    #[tokio::test]
    async fn set_custom_truncates_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        set_custom(
            &paths,
            Family::MangaCover,
            3,
            &b"a much longer first body"[..],
        )
        .await
        .unwrap();
        set_custom(&paths, Family::MangaCover, 3, &b"short"[..])
            .await
            .unwrap();
        let file = cover_paths::custom_file(&paths, Family::MangaCover, 3);
        assert_eq!(std::fs::read(file).unwrap(), b"short");
    }

    #[tokio::test]
    async fn set_custom_accepts_any_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        set_custom(&paths, Family::AnimeBackground, 9, &b"not an image"[..])
            .await
            .unwrap();
        let file = cover_paths::custom_file(&paths, Family::AnimeBackground, 9);
        assert_eq!(std::fs::read(file).unwrap(), b"not an image");
    }

    #[test]
    fn delete_custom_is_true_only_when_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert!(!delete_custom_cover(&paths, Family::AnimeCover, 5));
        std::fs::write(
            cover_paths::custom_file(&paths, Family::AnimeCover, 5),
            b"x",
        )
        .unwrap();
        assert!(delete_custom_cover(&paths, Family::AnimeCover, 5));
        assert!(!delete_custom_cover(&paths, Family::AnimeCover, 5));
    }

    #[tokio::test]
    async fn a_picked_file_replaces_the_custom_cover() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let picked = tmp.path().join("picked.jpg");
        std::fs::write(&picked, b"new picture bytes").unwrap();
        let custom = cover_paths::custom_file(&paths, Family::MangaCover, 9);
        std::fs::write(&custom, b"an older, longer custom cover").unwrap();
        set_custom_from_file(&paths, Family::MangaCover, 9, &picked)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&custom).unwrap(), b"new picture bytes");
    }

    #[tokio::test]
    async fn a_missing_picked_file_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let custom = cover_paths::custom_file(&paths, Family::MangaCover, 9);
        std::fs::write(&custom, b"kept").unwrap();
        let missing = tmp.path().join("gone.jpg");
        assert!(
            set_custom_from_file(&paths, Family::MangaCover, 9, &missing)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&custom).unwrap(), b"kept");
    }

    #[test]
    fn background_deletion_counts_each_file_it_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let art = ArtRef::anime_entry(
            true,
            4,
            1,
            true,
            (None, 0),
            (Some("https://example.org/bg.jpg".to_owned()), 0),
        );
        let bg = cover_paths::cover_file_for(&paths, &art).unwrap();
        let custom = cover_paths::custom_file_for(&paths, &art);
        for file in [&bg, &custom] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"bg").unwrap();
        }
        // Without the custom flag only the background goes, and the custom file stays.
        assert_eq!(delete_from_cache(&paths, &art, false), 1);
        assert!(custom.is_file());
        std::fs::write(&bg, b"bg").unwrap();
        assert_eq!(delete_from_cache(&paths, &art, true), 2);
        assert!(!bg.exists() && !custom.exists());
        assert_eq!(delete_from_cache(&paths, &art, true), 0);
    }

    #[test]
    fn delete_custom_reports_whether_a_file_was_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let custom = cover_paths::custom_file(&paths, Family::AnimeBackground, 4);
        std::fs::write(&custom, b"bg").unwrap();
        assert!(delete_custom_cover(&paths, Family::AnimeBackground, 4));
        assert!(!delete_custom_cover(&paths, Family::AnimeBackground, 4));
    }

    #[test]
    fn a_migration_copies_the_custom_file_to_the_new_id() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let from = cover_paths::custom_file(&paths, Family::AnimeBackground, 1);
        std::fs::create_dir_all(from.parent().unwrap()).unwrap();
        std::fs::write(&from, b"custom").unwrap();
        assert!(copy_custom(&paths, Family::AnimeBackground, 1, 2));
        let to = cover_paths::custom_file(&paths, Family::AnimeBackground, 2);
        assert_eq!(std::fs::read(&to).unwrap(), b"custom");
        assert!(from.is_file(), "the source stays");
    }

    #[test]
    fn a_missing_source_or_a_failed_copy_is_reported_and_leaves_no_target() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert!(!copy_custom(&paths, Family::MangaCover, 1, 2));
        let to = cover_paths::custom_file(&paths, Family::MangaCover, 2);
        assert!(!to.exists());
        // A target name that is already a folder makes the copy fail.
        let from = cover_paths::custom_file(&paths, Family::MangaCover, 3);
        std::fs::create_dir_all(from.parent().unwrap()).unwrap();
        std::fs::write(&from, b"x").unwrap();
        std::fs::create_dir_all(cover_paths::custom_file(&paths, Family::MangaCover, 4)).unwrap();
        assert!(!copy_custom(&paths, Family::MangaCover, 3, 4));
    }

    #[test]
    fn the_manga_migration_copies_the_cover_and_the_anime_migration_the_cover_and_the_background() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let put = |family: Family, id: i64, body: &[u8]| {
            let file = cover_paths::custom_file(&paths, family, id);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, body).unwrap();
        };
        let read = |family: Family, id: i64| std::fs::read(cover_paths::custom_file(&paths, family, id)).ok();
        put(Family::MangaCover, 1, b"manga");
        assert_eq!(migrate_custom(&paths, Kind::Manga, 1, 2), 1);
        assert_eq!(read(Family::MangaCover, 2).as_deref(), Some(&b"manga"[..]));

        put(Family::AnimeCover, 1, b"cover");
        put(Family::AnimeBackground, 1, b"background");
        assert_eq!(migrate_custom(&paths, Kind::Anime, 1, 2), 2);
        assert_eq!(read(Family::AnimeCover, 2).as_deref(), Some(&b"cover"[..]));
        assert_eq!(read(Family::AnimeBackground, 2).as_deref(), Some(&b"background"[..]));
    }

    #[test]
    fn a_copy_failure_does_not_stop_the_other_files_of_the_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        for family in [Family::AnimeCover, Family::AnimeBackground] {
            let from = cover_paths::custom_file(&paths, family, 1);
            std::fs::create_dir_all(from.parent().unwrap()).unwrap();
            std::fs::write(&from, b"x").unwrap();
        }
        // the cover target is a folder, so that copy fails; the background still copies
        std::fs::create_dir_all(cover_paths::custom_file(&paths, Family::AnimeCover, 2)).unwrap();
        assert_eq!(migrate_custom(&paths, Kind::Anime, 1, 2), 1);
        assert!(cover_paths::custom_file(&paths, Family::AnimeBackground, 2).is_file());
        // and an entry with no custom files migrates cleanly
        assert_eq!(migrate_custom(&paths, Kind::Manga, 8, 9), 0);
    }
}
