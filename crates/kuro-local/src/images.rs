//! Image type detection, `find_image` and `write_image` (area:14 sections 4.4 and 9.2).
//!
//! The magic-byte table and the case-sensitive extension test are `kuro_core::image` (the port of `ImageUtil`), so
//! they are not repeated here.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub use kuro_core::image::{ImageKind as ImageType, is_image_name, sniff};

use crate::fs::{FsEntry, create_nomedia, file_url, list_dir};
use crate::names::stem;

/// Bytes read for sniffing: `ImageUtil.getImageType` reads 32.
const HEAD_LEN: usize = 32;

/// The first bytes of a file, for `is_image_name`. `None` when the file cannot be read.
pub(crate) fn head_of(path: &Path) -> Option<Vec<u8>> {
    let mut head = vec![0; HEAD_LEN];
    let read = File::open(path).and_then(|mut f| f.read(&mut head)).ok()?;
    head.truncate(read);
    Some(head)
}

/// An image entry of a folder: a file whose name or first bytes say it is an image.
fn is_image_entry(entry: &FsEntry) -> bool {
    entry.is_file && is_image_name(&entry.name, || head_of(&entry.path))
}

/// Kotlin `LocalMangaCoverManager.find` (and the anime cover, background and thumbnail finds): the first file of
/// `dir` whose stem equals `stem` ignoring ASCII case and which is an image. Name order decides ties.
pub fn find_image(dir: &Path, stem_name: &str) -> Option<PathBuf> {
    list_dir(dir)
        .ok()?
        .into_iter()
        .filter(|e| e.is_file && stem(&e.name).eq_ignore_ascii_case(stem_name))
        .find(is_image_entry)
        .map(|e| e.path)
}

/// Kotlin `update`: write `bytes` over the existing image named `stem_name` (keeping its name and extension), or
/// create `default_name`. Then create `.nomedia` in `dir`. Bytes are not checked (E-IMG-1).
pub fn write_image(dir: &Path, stem_name: &str, default_name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "image folder is missing"));
    }
    let target = find_image(dir, stem_name).unwrap_or_else(|| dir.join(default_name));
    atomic_write(&target, bytes)?;
    create_nomedia(dir)?;
    Ok(target)
}

/// Writes `bytes` to a sibling `<name>.tmp`, then renames it over `path`, so a reader never sees a half-written file
/// (RC-1). The temporary file is removed when either step fails.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    match fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            Err(error)
        }
    }
}

/// The `file://` URL of an image the managers wrote (`write_image` then the URL the row stores).
pub fn set_image(dir: &Path, stem_name: &str, default_name: &str, bytes: &[u8]) -> io::Result<String> {
    let path = write_image(dir, stem_name, default_name, bytes)?;
    file_url(&path).ok_or_else(|| io::Error::other("image path is not absolute"))
}

/// `<sanitised episode name>-thumbnail.jpg`: the episode thumbnail's file name (behavior 13.2, E-IMG-7, E-IMG-8).
pub fn thumbnail_file_name(episode_name: &str) -> String {
    format!("{}-thumbnail.jpg", kuro_core::text::build_valid_filename(episode_name, true))
}

/// The thumbnail of an episode when the file exists and is not empty. The auto-generation policy (E-ANIME-6) uses it.
pub fn existing_thumbnail(dir: &Path, episode_name: &str) -> Option<PathBuf> {
    let path = dir.join(thumbnail_file_name(episode_name));
    fs::metadata(&path)
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        .then_some(path)
}

/// Writes the episode thumbnail, overwriting any earlier one, and creates `.nomedia`. Returns the file URL.
pub fn write_thumbnail(dir: &Path, episode_name: &str, bytes: &[u8]) -> io::Result<String> {
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "anime folder is missing"));
    }
    let path = dir.join(thumbnail_file_name(episode_name));
    atomic_write(&path, bytes)?;
    create_nomedia(dir)?;
    file_url(&path).ok_or_else(|| io::Error::other("thumbnail path is not absolute"))
}

/// Where a set-as-art action on a row goes (behavior 16): a local row writes its folder, a favorite from a remote
/// source goes to the custom cover cache, anything else must be added to the library first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtRoute {
    LocalFolder,
    CustomCache,
    AddToLibraryFirst,
}

pub fn art_route(is_local: bool, favorite: bool) -> ArtRoute {
    match (is_local, favorite) {
        (true, _) => ArtRoute::LocalFolder,
        (false, true) => ArtRoute::CustomCache,
        (false, false) => ArtRoute::AddToLibraryFirst,
    }
}

/// The outcome of a set-as-art write for a local row (behavior 16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtResult {
    Success(String),
    Error(String),
}

/// Writes a local row's art through the image manager. Any failure is `Error`, and no half-written file is left.
pub fn write_local_art(dir: &Path, stem_name: &str, default_name: &str, bytes: &[u8]) -> ArtResult {
    match set_image(dir, stem_name, default_name, bytes) {
        Ok(url) => ArtResult::Success(url),
        Err(error) => ArtResult::Error(error.to_string()),
    }
}

/// The result of a cover write: the URL the row stores, and the cache stamp the row's `cover_last_modified` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverWrite {
    pub thumbnail_url: String,
    pub cover_last_modified: i64,
}

/// Set-as-cover for a local manga: writes `cover` in `dir` and returns the new thumbnail URL.
pub fn set_cover(dir: &Path, bytes: &[u8]) -> io::Result<CoverWrite> {
    let path = write_image(dir, "cover", "cover.jpg", bytes)?;
    let thumbnail_url = file_url(&path).ok_or_else(|| io::Error::other("cover path is not absolute"))?;
    Ok(CoverWrite {
        thumbnail_url,
        cover_last_modified: crate::browse::now_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_HEAD: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D];

    #[test]
    fn the_sniff_table_knows_the_local_formats() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        let gif87 = b"GIF87a\x01\x00";
        let gif89 = b"GIF89a\x01\x00";
        let webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ";
        let avif = b"\x00\x00\x00\x1cftypavif\x00\x00\x00\x00";
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00";
        let jxl_codestream = [0xFF, 0x0A, 0x00, 0x00];
        let jxl_container = b"\x00\x00\x00\x0cJXL \r\n\x87\n";
        for (head, expected) in [
            (PNG_HEAD, ImageType::Png),
            (&jpeg[..], ImageType::Jpeg),
            (&gif87[..], ImageType::Gif),
            (&gif89[..], ImageType::Gif),
            (&webp[..], ImageType::Webp),
            (&avif[..], ImageType::Avif),
            (&heic[..], ImageType::Heif),
            (&jxl_codestream[..], ImageType::Jxl),
            (&jxl_container[..], ImageType::Jxl),
        ] {
            assert_eq!(sniff(head).map(|s| s.kind), Some(expected), "{head:?}");
        }
    }

    #[test]
    fn random_bytes_are_not_images() {
        assert!(sniff(b"plain text, nothing to see").is_none());
        assert!(sniff(&[]).is_none());
    }

    #[test]
    fn an_image_extension_is_enough_without_reading_the_bytes() {
        assert!(is_image_name("cover.jpg", || panic!("bytes must not be read")));
        assert!(
            !is_image_name("cover.JPG", || None),
            "the extension test is case-sensitive"
        );
        assert!(is_image_name("page", || Some(PNG_HEAD.to_vec())));
    }

    #[test]
    fn find_takes_the_first_image_in_name_order_and_ignores_case() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("cover.txt"), b"not an image")?;
        fs::write(dir.path().join("cover.jpg"), b"\xFF\xD8\xFF")?;
        fs::write(dir.path().join("Cover.PNG"), PNG_HEAD)?;
        // Byte order puts `Cover.PNG` first. `cover.txt` matches the stem but its bytes are not an image.
        assert_eq!(find_image(dir.path(), "cover"), Some(dir.path().join("Cover.PNG")));
        assert_eq!(find_image(dir.path(), "background"), None);
        Ok(())
    }

    #[test]
    fn t_14_images_background_case_insensitive() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("BACKGROUND.Png"), PNG_HEAD)?;
        assert_eq!(find_image(dir.path(), "background"), Some(dir.path().join("BACKGROUND.Png")));
        let written = write_image(dir.path(), "background", "background.jpg", b"bg")?;
        assert_eq!(written, dir.path().join("BACKGROUND.Png"));
        Ok(())
    }

    #[test]
    fn overwrite_keeps_the_existing_name_and_creates_nomedia() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("cover.png"), PNG_HEAD)?;
        let written = write_image(dir.path(), "cover", "cover.jpg", b"new bytes")?;
        assert_eq!(written, dir.path().join("cover.png"));
        assert_eq!(fs::read(&written)?, b"new bytes");
        assert!(dir.path().join(".nomedia").is_file());
        Ok(())
    }

    #[test]
    fn a_new_cover_takes_the_default_name() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let written = write_image(dir.path(), "cover", "cover.jpg", b"bytes")?;
        assert_eq!(written, dir.path().join("cover.jpg"));
        Ok(())
    }

    #[test]
    fn writing_into_a_missing_folder_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = write_image(&dir.path().join("gone"), "cover", "cover.jpg", b"x").expect_err("missing");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn set_cover_returns_the_file_url_and_a_stamp() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let set = set_cover(dir.path(), b"bytes")?;
        assert!(set.thumbnail_url.starts_with("file://"));
        assert!(set.thumbnail_url.ends_with("/cover.jpg"));
        assert!(set.cover_last_modified > 0);
        Ok(())
    }

    #[test]
    fn t_14_images_write_atomic_rename() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("cover.jpg"), b"old")?;
        write_image(dir.path(), "cover", "cover.jpg", b"new")?;
        assert_eq!(fs::read(dir.path().join("cover.jpg"))?, b"new");
        assert!(!dir.path().join("cover.jpg.tmp").exists());
        Ok(())
    }

    #[test]
    fn set_image_returns_the_url_and_writes_nomedia() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let url = set_image(dir.path(), "background", "background.jpg", b"bg")?;
        assert!(url.starts_with("file://") && url.ends_with("/background.jpg"), "{url}");
        assert!(dir.path().join(".nomedia").is_file());
        Ok(())
    }

    #[test]
    fn t_14_thumbnail_write_overwrites_same_name() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let first = write_thumbnail(dir.path(), "EP 01", b"one")?;
        let second = write_thumbnail(dir.path(), "EP 01", b"two")?;
        assert_eq!(first, second);
        assert_eq!(fs::read(dir.path().join("EP 01-thumbnail.jpg"))?, b"two");
        assert!(existing_thumbnail(dir.path(), "EP 01").is_some());
        Ok(())
    }

    #[test]
    fn an_empty_thumbnail_does_not_count_as_present() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("EP 01-thumbnail.jpg"), b"")?;
        assert!(existing_thumbnail(dir.path(), "EP 01").is_none());
        Ok(())
    }

    #[test]
    fn t_14_thumbnail_name_sanitized_for_windows() {
        assert_eq!(thumbnail_file_name("EP: 1?"), "EP_ 1_-thumbnail.jpg");
        assert_eq!(thumbnail_file_name("Show - 01"), "Show - 01-thumbnail.jpg");
    }

    #[test]
    fn set_art_routes_by_local_and_favorite() {
        assert_eq!(art_route(true, false), ArtRoute::LocalFolder);
        assert_eq!(art_route(true, true), ArtRoute::LocalFolder);
        assert_eq!(art_route(false, true), ArtRoute::CustomCache);
        assert_eq!(art_route(false, false), ArtRoute::AddToLibraryFirst);
    }

    #[test]
    fn a_failed_local_art_write_is_an_error_with_no_partial_file() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let gone = dir.path().join("gone");
        assert!(matches!(
            write_local_art(&gone, "cover", "cover.jpg", b"x"),
            ArtResult::Error(_)
        ));
        let ok = write_local_art(dir.path(), "cover", "cover.jpg", b"x");
        assert!(matches!(ok, ArtResult::Success(_)));
        assert!(!dir.path().join("cover.jpg.tmp").exists());
        Ok(())
    }
}
