//! Cover, custom cover and background file names over `Paths`.
//!
//! The folders sit under `Paths::data_dir()` so that covers survive a cache clear.
//! `Paths` has no getter for the anime folders yet, so the three family folder names are the only path
//! literals in this crate.

use std::io;
use std::path::{Path, PathBuf};

use kuro_platform::paths::Paths;

use crate::art::ArtRef;
use crate::keys::hash_key;

/// The folder family an image is stored in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    MangaCover,
    AnimeCover,
    AnimeBackground,
}

const CUSTOM_FOLDER: &str = "custom";

impl Family {
    fn folder(self) -> &'static str {
        match self {
            Family::MangaCover => "covers",
            Family::AnimeCover => "animecovers",
            Family::AnimeBackground => "animebackgrounds",
        }
    }
}

/// `<data>/<family>`, created on first access. A failed creation is logged and the
/// path is still returned, so the caller's own I/O reports the error, as Kotlin's `File` does.
pub fn dir(paths: &dyn Paths, family: Family) -> PathBuf {
    let dir = paths.cover_family_dir(family.folder());
    ensure_dir(&dir);
    dir
}

/// `<data>/<family>/custom`, created on first access.
pub fn custom_dir(paths: &dyn Paths, family: Family) -> PathBuf {
    let dir = dir(paths, family).join(CUSTOM_FOLDER);
    ensure_dir(&dir);
    dir
}

fn ensure_dir(dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!(path = %dir.display(), error = %e, "cannot create image folder");
    }
}

/// `<family>/<md5(url)>`. `None` for a missing URL. An empty URL gives the `md5("")` name.
pub fn cover_file(paths: &dyn Paths, family: Family, url: Option<&str>) -> Option<PathBuf> {
    url.map(|url| dir(paths, family).join(hash_key(url)))
}

/// `<family>/custom/<md5(decimal id)>`. Always a path, even when the file is absent.
pub fn custom_file(paths: &dyn Paths, family: Family, entry_id: i64) -> PathBuf {
    custom_dir(paths, family).join(hash_key(&entry_id.to_string()))
}

/// `<data>/animebackgrounds/<md5(url)>`, the background file for `url` (T-34a-007). Always a
/// path, even when the file is absent.
pub fn background_file(paths: &dyn Paths, url: &str) -> PathBuf {
    dir(paths, Family::AnimeBackground).join(hash_key(url))
}

/// `<data>/animebackgrounds/custom/<md5(anime id)>`, the custom background of an anime (T-34a-007).
pub fn custom_background_file(paths: &dyn Paths, anime_id: i64) -> PathBuf {
    custom_file(paths, Family::AnimeBackground, anime_id)
}

/// The thumbnail or background file for the image's current URL.
pub fn cover_file_for(paths: &dyn Paths, art: &ArtRef) -> Option<PathBuf> {
    cover_file(paths, art.family(), art.url.as_deref())
}

/// The custom cover or custom background file of the image's entry.
pub fn custom_file_for(paths: &dyn Paths, art: &ArtRef) -> PathBuf {
    custom_file(paths, art.family(), art.entry_id)
}

/// Whether the custom file exists. This is a blocking `stat`; async callers use `tokio::fs::try_exists`.
pub fn has_custom(paths: &dyn Paths, art: &ArtRef) -> bool {
    custom_file_for(paths, art).exists()
}

/// The marker that keeps Android's media scanner out of a folder (`DiskUtil.NOMEDIA_FILE`).
pub const NOMEDIA_FILE: &str = ".nomedia";

/// Creates an empty `.nomedia` in `dir` when the folder exists and the marker is absent
/// (`DiskUtil.createNoMediaFile`). A folder that does not exist is left alone. `true` when the file was
/// created, so a second call changes nothing.
pub fn create_nomedia(dir: &Path) -> io::Result<bool> {
    if !dir.is_dir() {
        return Ok(false);
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(NOMEDIA_FILE))
    {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;
    use super::*;
    use crate::art::ArtKind;
    use crate::testing::TestPaths;

    #[test]
    fn roots_are_under_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert_eq!(
            dir(&paths, Family::MangaCover),
            tmp.path().join("data/covers")
        );
        assert_eq!(
            custom_dir(&paths, Family::MangaCover),
            tmp.path().join("data/covers/custom")
        );
        assert_eq!(
            dir(&paths, Family::AnimeCover),
            tmp.path().join("data/animecovers")
        );
        assert_eq!(
            custom_dir(&paths, Family::AnimeCover),
            tmp.path().join("data/animecovers/custom")
        );
        assert_eq!(
            dir(&paths, Family::AnimeBackground),
            tmp.path().join("data/animebackgrounds")
        );
        assert_eq!(
            custom_dir(&paths, Family::AnimeBackground),
            tmp.path().join("data/animebackgrounds/custom")
        );
    }

    #[test]
    fn folders_are_created_on_first_access() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert!(!tmp.path().join("data/covers").exists());
        let _ = custom_file(&paths, Family::MangaCover, 1);
        assert!(tmp.path().join("data/covers/custom").is_dir());
    }

    #[test]
    fn cover_file_is_md5_of_url() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let file = cover_file(&paths, Family::MangaCover, Some("abc")).unwrap();
        assert_eq!(
            file,
            dir(&paths, Family::MangaCover).join("900150983cd24fb0d6963f7d28e17f72")
        );
    }

    #[test]
    fn missing_url_has_no_cover_file_and_empty_url_has_the_md5_of_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert_eq!(cover_file(&paths, Family::AnimeCover, None), None);
        let empty = cover_file(&paths, Family::AnimeCover, Some("")).unwrap();
        assert_eq!(
            empty.file_name().unwrap().to_str().unwrap(),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
    }

    #[test]
    fn custom_file_is_md5_of_decimal_id() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let file = custom_file(&paths, Family::MangaCover, 1);
        assert_eq!(
            file,
            custom_dir(&paths, Family::MangaCover).join("c4ca4238a0b923820dcc509a6f75849b")
        );
    }

    #[test]
    fn has_custom_follows_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        let art = ArtRef {
            owner: Kind::Manga,
            entry_id: 7,
            source_id: 1,
            kind: ArtKind::Cover,
            url: Some("u".into()),
            favorite: true,
            last_modified: 0,
        };
        assert!(!has_custom(&paths, &art));
        std::fs::write(custom_file_for(&paths, &art), b"x").unwrap();
        assert!(has_custom(&paths, &art));
    }

    #[test]
    fn nomedia_is_created_once_in_an_existing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("library");
        std::fs::create_dir(&folder).unwrap();
        assert!(create_nomedia(&folder).unwrap());
        let marker = folder.join(".nomedia");
        assert_eq!(std::fs::metadata(&marker).unwrap().len(), 0);
        std::fs::write(&marker, b"kept").unwrap();
        assert!(!create_nomedia(&folder).unwrap());
        assert_eq!(std::fs::read(&marker).unwrap(), b"kept");
    }

    #[test]
    fn background_and_custom_background_paths_are_md5_names_in_the_background_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TestPaths::new(tmp.path());
        assert_eq!(
            background_file(&paths, "https://example.org/bg.jpg"),
            tmp.path().join("data/animebackgrounds").join(hash_key("https://example.org/bg.jpg"))
        );
        assert_eq!(
            custom_background_file(&paths, 7),
            tmp.path().join("data/animebackgrounds/custom").join(hash_key("7"))
        );
    }

    #[test]
    fn nomedia_is_not_created_in_a_missing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("missing");
        assert!(!create_nomedia(&folder).unwrap());
        assert!(!folder.exists());
    }
}
