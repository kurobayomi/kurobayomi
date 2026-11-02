//! Saving a page to the pictures folder (T-43a-054) and the notice that follows (T-43a-055). The file is written off
//! the GTK thread. Names come from the image crate's reader name rule, and a name already taken gets " (2)", " (3)"
//! and so on (E-SAVE-2). Failures show as the error notice; nothing here panics.

use kuro_i18n::{MsgKey, tr};
use std::path::{Path, PathBuf};

use kuro_images::cover_file::write_atomic;
use kuro_images::names::{build_valid_filename, reader_file_name};
use kuro_images::save::PICTURES_FOLDER;
use kuro_images::sniff::sniff;

use super::strings;
use super::viewers::page_bytes::{ByteSource, read};

/// `base` when `base.ext` is free, else `base (2).ext`, `base (3).ext`, and so on. `taken` says whether a file name
/// is already in the target folder.
pub fn free_name(base: &str, ext: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(&format!("{base}.{ext}")) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}{}", strings::name_collision_suffix(n)))
        .find(|candidate| !taken(&format!("{candidate}.{ext}")))
        .unwrap_or_else(|| base.to_owned())
}

/// The folder a saved page goes in: the Kurobayomi folder of the pictures folder, and the manga's own folder under it
/// when folder-per-manga is on.
pub fn target_folder(pictures: &Path, title: &str, folder_per_manga: bool) -> PathBuf {
    let mut dir = pictures.join(PICTURES_FOLDER);
    if folder_per_manga {
        dir.push(build_valid_filename(title));
    }
    dir
}

/// The user's pictures folder, or `Pictures` under the home folder when the desktop does not name one.
pub fn pictures_dir() -> PathBuf {
    gtk::glib::user_special_dir(gtk::glib::UserDirectory::Pictures).unwrap_or_else(|| gtk::glib::home_dir().join("Pictures"))
}

/// Where a page's bytes are read from right now, or `None` when a fetched page is not ready yet.
pub fn page_source(page: &std::rc::Rc<crate::reader::model::ReaderPage>) -> Option<ByteSource> {
    super::viewers::page_bytes::locate(page)?.resolve()
}

/// Saves the page's bytes into the pictures folder on a worker thread and calls `done` on the GTK thread with the
/// written path, or the error, and the thumbnail of the picture for the notice (T-43a-054, T-43a-055). `file_base` is
/// the reader name without an extension, from [`reader_file_name`].
pub fn save_in_background(
    source: ByteSource,
    title: &str,
    file_base: &str,
    folder_per_manga: bool,
    done: impl FnOnce(Result<PathBuf, String>, Option<Vec<u8>>) + 'static,
) {
    let (title, file_base) = (title.to_owned(), file_base.to_owned());
    crate::background::run(
        move || {
            let saved = save_now(&source, &title, &file_base, folder_per_manga, &pictures_dir());
            let thumbnail = saved.as_ref().ok().and_then(|path| super::saved_notice::thumbnail_png(path));
            (saved, thumbnail)
        },
        move |(saved, thumbnail)| done(saved, thumbnail),
    );
}

/// The blocking part of a save: read, sniff, pick a free name, write. Returns the written path.
pub fn save_now(source: &ByteSource, title: &str, file_base: &str, folder_per_manga: bool, pictures: &Path) -> Result<PathBuf, String> {
    let bytes = read(source).map_err(|e| e.to_string())?;
    let format = sniff(&bytes).ok_or_else(|| tr(MsgKey::ui_not_an_image).to_owned())?;
    let ext = format.extension();
    let dir = target_folder(pictures, title, folder_per_manga);
    let name = free_name(file_base, ext, |candidate| dir.join(build_valid_filename(candidate)).exists());
    let target = dir.join(build_valid_filename(&format!("{name}.{ext}")));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(write_atomic(&target, &bytes)).map_err(|e| e.to_string())?;
    Ok(target)
}

/// The bytes of a page, read where they are. Blocking: call it through `background::run`.
pub fn read_bytes(source: &ByteSource) -> Result<Vec<u8>, String> {
    read(source).map_err(|e| e.to_string())
}

/// Writes a page's bytes as the manga's cover file (T-43a-058). The file is replaced atomically, so a failure leaves
/// the old cover in place. Blocking: call it through `background::run`.
pub fn write_cover_file(source: &ByteSource, cover_file: &Path) -> Result<(), String> {
    let bytes = read(source).map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(write_atomic(cover_file, &bytes)).map_err(|e| e.to_string())
}

/// Writes a local manga's cover into its folder, over the folder's `cover` image (Kotlin `LocalMangaCoverManager`). The
/// result carries the thumbnail url and the cover time that the manga row takes. Blocking: call it through `background::run`.
pub fn write_local_cover(source: &ByteSource, folder: &Path) -> Result<kuro_local::images::CoverWrite, String> {
    let bytes = read(source).map_err(|e| e.to_string())?;
    kuro_local::images::set_cover(folder, &bytes).map_err(|e| e.to_string())
}

/// Copies a page into the share staging folder, emptied first so only the latest share is kept (T-43a-056), and
/// returns the staged file.
pub fn stage_for_share(source: &ByteSource, file_base: &str, staging: &Path) -> Result<PathBuf, String> {
    let bytes = read(source).map_err(|e| e.to_string())?;
    let ext = sniff(&bytes).ok_or_else(|| tr(MsgKey::ui_not_an_image).to_owned())?.extension();
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    let target = staging.join(build_valid_filename(&format!("{file_base}.{ext}")));
    std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
    Ok(target)
}

/// The reader name of a saved page: `<title> - <chapter> - <page>` (T-43a-054).
pub fn page_file_base(title: &str, chapter: &str, page_number: usize) -> String {
    reader_file_name(title, chapter, page_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_name_is_kept_and_a_taken_one_gets_a_counter() {
        assert_eq!(free_name("page", "jpg", |_| false), "page");
        let taken = ["page.jpg", "page (2).jpg"];
        assert_eq!(free_name("page", "jpg", |c| taken.contains(&c)), "page (3)");
    }

    #[test]
    fn the_folder_is_per_manga_only_when_the_option_is_on() {
        let pictures = Path::new("/p");
        assert_eq!(target_folder(pictures, "Title", false), Path::new("/p/Kurobayomi"));
        assert_eq!(target_folder(pictures, "Title", true), Path::new("/p/Kurobayomi/Title"));
    }

    #[test]
    fn a_saved_page_gets_the_sniffed_extension_and_a_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.bin");
        std::fs::write(&src, b"\x89PNG\r\n\x1a\n0123").unwrap();
        let first = save_now(&ByteSource::File(src.clone()), "Title", "Title - Ch - 1", false, dir.path()).unwrap();
        assert_eq!(first, dir.path().join("Kurobayomi/Title - Ch - 1.png"));
        let second = save_now(&ByteSource::File(src), "Title", "Title - Ch - 1", false, dir.path()).unwrap();
        assert_eq!(second, dir.path().join("Kurobayomi/Title - Ch - 1 (2).png"));
    }

    #[test]
    fn bytes_that_are_not_an_image_fail_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.txt");
        std::fs::write(&src, b"hello").unwrap();
        assert_eq!(save_now(&ByteSource::File(src), "T", "T - C - 1", false, dir.path()), Err("Not an image".to_owned()));
        assert!(!dir.path().join("Kurobayomi").exists());
    }
}
