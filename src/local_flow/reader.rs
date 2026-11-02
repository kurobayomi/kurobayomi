//! Opens the reader on a local manga chapter (area 14 behaviour 10, area 43 read flow). A local chapter's url is the
//! absolute path of its folder, archive or EPUB, so the loader reads the file it names. The reader's own launch
//! path has no local source, so this module builds the same page with the local loader picked.

use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use kuro_core::sources::SourceGate;
use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;
use kuro_db::Db;
use kuro_platform::kind::Kind;
use kuro_local::LOCAL_SOURCE_ID;
use kuro_local::format::{Format, format_of};
use kuro_platform::toast::{Toaster, toast_channel};
use kuro_prefs::reader::ReaderPrefs;
use tokio::sync::watch;

use crate::reader::launch;
use crate::reader::loader::{LocalFormat, Loader, LoaderInputs, SourceKind, choose_loader};
use crate::reader::model::{LoadError, ReaderChapter};
use crate::reader::screen::{ReaderDeps, open};
use crate::reader::writer::WriterHandle;

/// The local format of a chapter url (inside the Local folder or in a linked one, see `local_flow::path_of`): a folder, an
/// EPUB or a manga archive that exists.
pub fn local_format(url: &str) -> Option<LocalFormat> {
    let path = super::path_of(Kind::Manga, url)?;
    match format_of(&path, path.is_dir())? {
        Format::Directory(dir) => Some(LocalFormat::Directory(dir)),
        Format::Archive(file) => Some(LocalFormat::Archive(file)),
        Format::Epub(file) => Some(LocalFormat::Epub(file)),
    }
}

/// The loader of a local chapter: the format decides it, and the source is the local one.
pub fn build_loader(chapter: &Rc<ReaderChapter>) -> Result<Loader, LoadError> {
    let url = chapter.chapter.url.clone();
    choose_loader(LoaderInputs { download: None, local: local_format(&url), source: SourceKind::Local, chapter_url: url })
}

/// The reader page for a local chapter, closing back to the page below.
pub fn open_local_chapter(manga_id: i64, chapter_id: i64) -> gtk::Box {
    let app = crate::data::app();
    let store = app.base.store().clone();
    let deps = ReaderDeps {
        db: app.db.clone(),
        manga_id,
        chapter_id,
        prefs: ReaderPrefs::new(store.clone()),
        store: Rc::new(WriterHandle::spawn(app.db.clone())),
        build: Rc::new(build_loader),
        is_downloaded: Rc::new(|_| false),
        downloaded_only: app.base.downloaded_only.get(),
        incognito: crate::incognito::live(),
        prefs_store: store,
    };
    open(deps, &open_gate(), toaster(), crate::ui::pop).root
}

/// The reader page for the entry screens' title and chapter name (the route passes names, see `launch::page_for`).
/// A title that names a local entry in the library opens the local reader; any other title keeps the old route. The
/// library's Continue button passes a chapter id as the name, which `find_local_ids` also accepts.
pub fn page_for(title: &str, chapter_name: &str) -> gtk::Box {
    let db = crate::data::app().db.clone();
    match find_local_ids(&db, title, chapter_name, super::open::current()) {
        Some((manga_id, chapter_id)) => open_local_chapter(manga_id, chapter_id),
        None => launch::page_for(title, chapter_name),
    }
}

/// The ids of a local entry and its chapter. The entry is `preferred` (the page that was opened last) when it is a
/// local entry with this title, else the first local library entry with the title, so two folders with the same name
/// keep apart once one of them is open. The chapter is the one with this name, else the one with this id (the library
/// passes ids). `(manga, 0)` when the chapter is unknown, so the reader reports it. `None` when no local entry has that
/// title.
pub fn find_local_ids(db: &Arc<Db>, title: &str, chapter: &str, preferred: Option<i64>) -> Option<(i64, i64)> {
    let manga = EntryRepo::new(Arc::clone(db));
    let manga_id = preferred
        .and_then(|id| manga.by_id(id).ok())
        .filter(|m| m.source == LOCAL_SOURCE_ID && m.title == title)
        .map(|m| m.id)
        .or_else(|| super::open::local_id_with_title(db, Kind::Manga, title).ok().flatten())?;
    let chapters = ItemRepo::new(Arc::clone(db)).by_entry_id(manga_id, false).ok()?;
    let by_id = chapter.parse::<i64>().ok().and_then(|id| chapters.iter().find(|c| c.id == id));
    let chapter_id = chapters.iter().find(|c| c.name == chapter).or(by_id).map_or(0, |c| c.id);
    Some((manga_id, chapter_id))
}

/// A gate that is already open, as `launch` uses (the app has no source registry to wait for).
fn open_gate() -> SourceGate {
    let (_closed_sender, ready) = watch::channel(true);
    SourceGate::new(ready)
}

/// A toaster whose messages go to the window's toast overlay.
fn toaster() -> Toaster {
    let (toaster, feed) = toast_channel();
    glib::spawn_future_local(async move {
        while let Some(message) = feed.next().await {
            crate::toast::notify(message.text);
        }
    });
    toaster
}
