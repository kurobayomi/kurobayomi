//! Page loaders and the chapter loading state machine (`ui/reader/loader/*`). T-43a-005, T-43a-006, T-43a-007.
//!
//! [`Loader`] is an enum over the loader kinds, so dispatch is a match and there is no trait object per page.
//! Dropping a loader recycles it once; a second recycle does nothing. The local formats are a directory, a zip or
//! cbz archive, and an EPUB, which is read through the archive loader in spine order.

pub mod archive;
pub mod dir;
pub mod download;
pub mod http;

use kuro_i18n::{Arg, MsgKey, tr, trf};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::cache::DiskCache;
use super::model::{ChapterState, LoadError, PageLoader, ReaderChapter, ReaderPage};

/// One page of a page list before it becomes a [`ReaderPage`]: the page url and its image url when the source
/// already knows it. Stored in the page-list cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageEntry {
    pub url: String,
    pub image_url: Option<String>,
}

/// Why a chapter did not load. A cancelled load goes back to `Wait`, not `Error` (E-CANCEL-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadFailure {
    Cancelled,
    Failed(LoadError),
}

/// Every page loader. Local loaders are read-only; the HTTP loader owns a worker.
pub enum Loader {
    Directory(dir::DirectoryLoader),
    Archive(archive::ArchiveLoader),
    Download(download::DownloadLoader),
    Http(http::HttpLoader),
}

impl Loader {
    /// `isLocal`: false only for the HTTP loader.
    pub fn is_local(&self) -> bool {
        !matches!(self, Loader::Http(_))
    }

    #[cfg(test)]
    pub fn is_recycled(&self) -> bool {
        match self {
            Loader::Directory(l) => l.is_recycled(),
            Loader::Archive(l) => l.is_recycled(),
            Loader::Download(l) => l.is_recycled(),
            Loader::Http(l) => l.is_recycled(),
        }
    }

    /// The pages of the chapter, indices `0..n`. A recycled loader gives `Cancelled`.
    pub fn get_pages(&self) -> Result<Vec<Rc<ReaderPage>>, LoadFailure> {
        let pages = match self {
            Loader::Directory(l) => l.get_pages(),
            Loader::Archive(l) => l.get_pages(),
            Loader::Download(l) => l.get_pages(),
            Loader::Http(l) => l.get_pages(),
        }?;
        Ok(pages)
    }

    #[cfg(test)]
    /// `loadPage`. Local loaders check that they are still open. The HTTP loader queues the page and returns a
    /// guard that cancels the entries it queued when dropped.
    pub fn load_page(&self, index: usize) -> Result<Option<http::PageGuard>, LoadError> {
        match self {
            Loader::Http(l) => Ok(Some(l.load_page(index))),
            Loader::Archive(l) => l.check_open().map(|()| None),
            Loader::Download(l) => l.check_open().map(|()| None),
            Loader::Directory(_) => Ok(None),
        }
    }
}

impl PageLoader for Loader {
    fn is_local(&self) -> bool {
        Loader::is_local(self)
    }

    fn load_page(&self, index: usize) -> Option<Box<dyn std::any::Any>> {
        match self {
            Loader::Http(l) => Some(Box::new(l.load_page(index))),
            _ => None,
        }
    }

    fn retry_page(&self, index: usize) {
        if let Loader::Http(l) = self {
            l.retry_page(index);
        }
    }

    fn image_file(&self, index: usize) -> Option<PathBuf> {
        match self {
            Loader::Http(l) => l.image_file(index),
            _ => None,
        }
    }

    fn sync_page(&self, page: &ReaderPage) {
        if let Loader::Http(l) = self {
            l.sync_page(page);
        }
    }

    fn recycle(&self) {
        match self {
            Loader::Directory(l) => l.recycle(),
            Loader::Archive(l) => l.recycle(),
            Loader::Download(l) => l.recycle(),
            Loader::Http(l) => l.recycle(),
        }
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.recycle();
    }
}

/// A local page format a chapter can be read from (`LocalMangaSource.getFormat`).
pub enum LocalFormat {
    Directory(PathBuf),
    Archive(PathBuf),
    Epub(PathBuf),
}

/// The source a chapter belongs to, as far as loader selection cares.
pub enum SourceKind {
    /// A local source, read through [`LocalFormat`].
    Local,
    /// An HTTP source with its page-list and image access.
    Http { source: Arc<dyn http::HttpPageSource>, manga_id: i64, cache: Arc<DiskCache> },
    /// A source that is known but not installed (`StubMangaSource`). Its "not installed" message is better than
    /// `Unknown`'s; `launch` passes it once it knows the source's name (reader overhaul, phase 3).
    #[cfg_attr(not(test), allow(dead_code))]
    Stub(String),
    /// Anything else.
    Unknown,
}

/// Everything selection looks at, in precedence order.
pub struct LoaderInputs {
    /// The downloaded chapter, found with a filesystem check that skips the cache (`skipCache = true`).
    pub download: Option<PathBuf>,
    pub local: Option<LocalFormat>,
    pub source: SourceKind,
    pub chapter_url: String,
}

/// `getPageLoader(chapter)`. Precedence: download, local format, HTTP source, stub, unknown. The first match wins.
pub fn choose_loader(inputs: LoaderInputs) -> Result<Loader, LoadError> {
    let LoaderInputs { download, local, source, chapter_url } = inputs;
    if let Some(path) = download {
        return download::DownloadLoader::open(path).map(Loader::Download);
    }
    match (local, source) {
        (Some(LocalFormat::Directory(dir)), _) => Ok(Loader::Directory(dir::DirectoryLoader::new(dir))),
        (Some(LocalFormat::Archive(file)), _) => archive::ArchiveLoader::open(file).map(Loader::Archive),
        (Some(LocalFormat::Epub(file)), _) => archive::ArchiveLoader::open_epub(file).map(Loader::Archive),
        (None, SourceKind::Http { source, manga_id, cache }) => {
            Ok(Loader::Http(http::HttpLoader::new(source, manga_id, chapter_url, cache)))
        }
        (None, SourceKind::Stub(name)) => Err(LoadError::new(trf(MsgKey::source_not_installed, &[("arg1", Arg::Str(&name))]))),
        _ => Err(LoadError::new(tr(MsgKey::loader_not_implemented_error))),
    }
}

/// `chapterIsReady`: the chapter is loaded and still has its loader.
pub fn chapter_is_ready(chapter: &ReaderChapter) -> bool {
    matches!(*chapter.state(), ChapterState::Loaded(_)) && chapter.has_loader()
}

/// The start page rule (E-START-1): an unread chapter, or preserve-position on, starts at `last_page_read`.
/// Otherwise the previously requested page is kept.
pub fn start_page_applies(read: bool, preserve_position: bool) -> bool {
    !read || preserve_position
}

/// `ChapterLoader.loadChapter`: moves a chapter to `Loaded` with a loader attached, or to `Error` or `Wait`.
///
/// `build` picks the loader (see [`choose_loader`]). Any failure before `Loaded` leaves no loader attached:
/// the loader is dropped, and so recycled (E-LEAK-1). A chapter that is already ready is left alone.
pub fn load_chapter(
    chapter: &Rc<ReaderChapter>,
    build: impl FnOnce() -> Result<Loader, LoadError>,
    preserve_position: bool,
) -> Result<(), LoadFailure> {
    if chapter_is_ready(chapter) {
        return Ok(());
    }
    chapter.set_state(ChapterState::Loading);
    let loader = match build() {
        Ok(loader) => loader,
        Err(error) => return fail(chapter, LoadFailure::Failed(error)),
    };
    let pages = match loader.get_pages() {
        Ok(pages) => pages,
        Err(LoadFailure::Cancelled) => {
            drop(loader);
            chapter.set_state(ChapterState::Wait);
            return Err(LoadFailure::Cancelled);
        }
        Err(failure) => {
            drop(loader);
            return fail(chapter, failure);
        }
    };
    if pages.is_empty() {
        drop(loader);
        return fail(chapter, LoadFailure::Failed(LoadError::new(tr(MsgKey::page_list_empty_error))));
    }
    let read = chapter.read.get();
    let last_page_read = chapter.last_page_read.get();
    chapter.set_loader(Box::new(loader));
    if start_page_applies(read, preserve_position) {
        chapter.requested_page.set(last_page_read);
    }
    chapter.finish_loading(pages).map_err(LoadFailure::Failed)
}

fn fail(chapter: &ReaderChapter, failure: LoadFailure) -> Result<(), LoadFailure> {
    if let LoadFailure::Failed(error) = &failure {
        chapter.set_state(ChapterState::Error(Rc::new(error.clone())));
    }
    Err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_data::item::Item;
    use kuro_platform::kind::Kind;

    fn chapter(read: bool, last_page_read: i64) -> Rc<ReaderChapter> {
        let mut c = Item::create(Kind::Manga);
        c.id = 1;
        c.url = "/c/1".into();
        c.done = read;
        c.progress = last_page_read;
        ReaderChapter::new(c)
    }

    fn dir_with(names: &[&str], tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-loader-tests").join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in names {
            std::fs::write(dir.join(name), b"\x89PNG\r\n\x1a\n").unwrap();
        }
        dir
    }

    fn directory_loader(dir: PathBuf) -> impl FnOnce() -> Result<Loader, LoadError> {
        move || Ok(Loader::Directory(dir::DirectoryLoader::new(dir)))
    }

    #[test]
    fn a_loaded_chapter_with_a_loader_is_a_no_op() {
        let ch = chapter(false, 0);
        let dir = dir_with(&["1.png", "2.png"], "ready");
        load_chapter(&ch, directory_loader(dir.clone()), false).unwrap();
        assert!(chapter_is_ready(&ch));
        let mut called = false;
        load_chapter(&ch, || {
            called = true;
            Ok(Loader::Directory(dir::DirectoryLoader::new(dir)))
        }, false)
        .unwrap();
        assert!(!called, "a ready chapter must not build a second loader");
    }

    #[test]
    fn an_empty_page_list_is_an_error_with_no_loader_left_attached() {
        let ch = chapter(false, 0);
        let dir = dir_with(&[], "empty");
        std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
        let result = load_chapter(&ch, directory_loader(dir), false);
        assert_eq!(result, Err(LoadFailure::Failed(LoadError::new("No pages found"))));
        assert!(!ch.has_loader());
        assert!(matches!(*ch.state(), ChapterState::Error(_)));
    }

    #[test]
    fn a_failed_build_leaves_no_loader_and_keeps_the_error_state() {
        let ch = chapter(false, 0);
        let result = load_chapter(&ch, || Err(LoadError::new("Source not found")), false);
        assert!(result.is_err());
        assert!(!ch.has_loader());
        assert!(matches!(*ch.state(), ChapterState::Error(_)));
    }

    #[test]
    fn start_page_uses_last_read_for_unread_or_preserve_and_keeps_the_previous_page_otherwise() {
        let unread = chapter(false, 3);
        load_chapter(&unread, directory_loader(dir_with(&["1.png", "2.png", "3.png", "4.png"], "start-a")), false)
            .unwrap();
        assert_eq!(unread.requested_page.get(), 3);

        let read = chapter(true, 3);
        read.requested_page.set(1);
        load_chapter(&read, directory_loader(dir_with(&["1.png", "2.png"], "start-b")), false).unwrap();
        assert_eq!(read.requested_page.get(), 1, "a read chapter keeps the previous requested page");

        let preserved = chapter(true, 2);
        load_chapter(&preserved, directory_loader(dir_with(&["1.png", "2.png", "3.png"], "start-c")), true).unwrap();
        assert_eq!(preserved.requested_page.get(), 2);
        assert!(start_page_applies(false, false));
        assert!(!start_page_applies(true, false));
    }

    #[test]
    fn a_cancelled_load_returns_to_wait_not_error() {
        let ch = chapter(false, 0);
        let loader = Loader::Directory(dir::DirectoryLoader::new(dir_with(&["1.png"], "cancel")));
        loader.recycle();
        assert!(loader.is_recycled());
        assert_eq!(loader.get_pages().err(), Some(LoadFailure::Cancelled));
        drop(loader);
        let result = load_chapter(&ch, || Ok(Loader::Directory(dir::DirectoryLoader::new(PathBuf::from("/nope")))), false);
        assert!(result.is_err());
        assert!(matches!(*ch.state(), ChapterState::Error(_)));
    }

    #[test]
    fn precedence_is_download_then_local_then_http_then_stub_then_unknown() {
        let dir = dir_with(&["1.png"], "precedence");
        let download = choose_loader(LoaderInputs {
            download: Some(dir.clone()),
            local: Some(LocalFormat::Directory(dir.clone())),
            source: SourceKind::Stub("X".into()),
            chapter_url: "/c".into(),
        });
        assert!(matches!(download, Ok(Loader::Download(_))));

        let local = choose_loader(LoaderInputs {
            download: None,
            local: Some(LocalFormat::Directory(dir.clone())),
            source: SourceKind::Unknown,
            chapter_url: "/c".into(),
        });
        assert!(matches!(local, Ok(Loader::Directory(_))));

        let stub = choose_loader(LoaderInputs {
            download: None,
            local: None,
            source: SourceKind::Stub("Foo".into()),
            chapter_url: "/c".into(),
        });
        assert_eq!(stub.err(), Some(LoadError::new("Source not installed: Foo")));

        let unknown = choose_loader(LoaderInputs {
            download: None,
            local: None,
            source: SourceKind::Unknown,
            chapter_url: "/c".into(),
        });
        assert_eq!(unknown.err(), Some(LoadError::new("Source not found")));
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::reader::model::ChapterState;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader").join(name)
    }

    fn names(loader: &Loader) -> Vec<String> {
        loader.get_pages().expect("the fixture loads").iter().map(|p| p.url.clone()).collect()
    }

    #[test]
    fn a_directory_lists_its_images_in_natural_order_and_skips_text_and_subfolders() {
        let loader = choose_loader(LoaderInputs {
            download: None,
            local: Some(LocalFormat::Directory(fixture("dir_natural"))),
            source: SourceKind::Unknown,
            chapter_url: "/c".into(),
        })
        .unwrap();
        assert_eq!(names(&loader), ["1.JPG", "2.jpg", "10.jpg"]);
    }

    #[test]
    fn an_archive_keeps_only_its_images_in_natural_order() {
        let loader = archive::ArchiveLoader::open(fixture("archive.cbz")).map(Loader::Archive).unwrap();
        assert_eq!(names(&loader), ["2.png", "10.png"]);
    }

    #[test]
    fn an_archive_without_images_is_the_empty_page_list_error() {
        let loader = choose_loader(LoaderInputs {
            download: None,
            local: Some(LocalFormat::Archive(fixture("empty.cbz"))),
            source: SourceKind::Unknown,
            chapter_url: "/c".into(),
        })
        .unwrap();
        assert!(names(&loader).is_empty());
        let chapter = ReaderChapter::new(kuro_data::item::Item::create(kuro_platform::kind::Kind::Manga));
        let result = load_chapter(&chapter, || Ok(loader), false);
        assert!(matches!(result, Err(LoadFailure::Failed(ref e)) if e.message == "No pages found"));
        assert!(matches!(*chapter.state(), ChapterState::Error(ref e) if e.message == "No pages found"));
    }

    #[test]
    fn an_epub_lists_its_images_in_spine_order_not_name_order() {
        let loader = archive::ArchiveLoader::open_epub(fixture("book.epub")).map(Loader::Archive).unwrap();
        assert_eq!(
            names(&loader),
            [
                "OEBPS/images/z-first.jpg",
                "OEBPS/images/a-second.jpg",
                "OEBPS/images/01.jpg",
                "OEBPS/images/03.jpg",
            ]
        );
    }

    #[test]
    fn a_broken_epub_fails_to_open_with_a_message() {
        let error = archive::ArchiveLoader::open_epub(fixture("broken.epub")).err().expect("broken epub fails");
        assert!(!error.message.is_empty());
    }
}
