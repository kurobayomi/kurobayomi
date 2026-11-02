//! Archive page loader (`ArchivePageLoader.kt`) for zip and cbz, and for EPUB images (T-43a-009, T-43a-010).
//!
//! The entry index is built once at open: only file entries that pass the image rule, naturally sorted.
//! Opening a page after recycle fails with an error (upstream `check(!isRecycled)`).
//!
//! ponytail: `ArchiveReader::open_entry` scans from the start for each page, so a chapter costs O(n^2) entry
//! headers. The mapping is shared and read-only, so this stays cheap for chapter-sized archives. Upgrade path:
//! a random-access zip index in kuro-archive.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use kuro_archive::EpubReader;
use kuro_archive::reader::ArchiveReader;
#[cfg(test)]
use kuro_archive::reader::EntryReader;
#[cfg(test)]
use kuro_i18n::{Arg, MsgKey, trf};

use super::LoadFailure;
use crate::reader::model::{LoadError, PageSource, ReaderPage};

/// macOS archive metadata is not a page: the `__MACOSX/` folder and the `._` resource-fork files beside each image.
pub fn is_archive_junk(name: &str) -> bool {
    name.split('/').any(|part| part == "__MACOSX") || name.rsplit('/').next().is_some_and(|file| file.starts_with("._"))
}

pub struct ArchiveLoader {
    path: PathBuf,
    reader: RefCell<Option<ArchiveReader>>,
    entries: Vec<String>,
    recycled: Cell<bool>,
}

impl ArchiveLoader {
    /// Opens the archive and indexes its page entries once.
    pub fn open(path: PathBuf) -> Result<Self, LoadError> {
        let reader = ArchiveReader::open(&path).map_err(|e| LoadError::new(e.to_string()))?;
        let entries = reader
            .page_entries()
            .map_err(|e| LoadError::new(e.to_string()))?
            .into_iter()
            .map(|e| e.name)
            .filter(|name| !is_archive_junk(name))
            .collect();
        Ok(Self { path, reader: RefCell::new(Some(reader)), entries, recycled: Cell::new(false) })
    }

    /// Opens an EPUB and indexes its page images in spine order (T-43a-010). The archive handle is the same as for
    /// a zip, since the EPUB pages are zip entries.
    pub fn open_epub(path: PathBuf) -> Result<Self, LoadError> {
        let entries = EpubReader::open(&path)
            .and_then(|epub| epub.image_paths())
            .map_err(|e| LoadError::new(e.to_string()))?;
        let reader = ArchiveReader::open(&path).map_err(|e| LoadError::new(e.to_string()))?;
        Ok(Self { path, reader: RefCell::new(Some(reader)), entries, recycled: Cell::new(false) })
    }

    pub fn is_recycled(&self) -> bool {
        self.recycled.get()
    }

    /// Closes the archive. Idempotent.
    pub fn recycle(&self) {
        if !self.recycled.replace(true) {
            self.reader.borrow_mut().take();
        }
    }

    #[cfg(test)]
    /// The typed "closed" error for any page access after recycle.
    pub fn check_open(&self) -> Result<(), LoadError> {
        if self.is_recycled() { Err(LoadError::new("Archive loader was recycled")) } else { Ok(()) }
    }

    pub fn get_pages(&self) -> Result<Vec<Rc<ReaderPage>>, LoadFailure> {
        if self.is_recycled() {
            return Err(LoadFailure::Cancelled);
        }
        Ok(self
            .entries
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let source = PageSource::ArchiveEntry { archive: self.path.clone(), entry: name.clone() };
                ReaderPage::new(i as u32, name.clone(), None, source)
            })
            .collect())
    }

    #[cfg(test)]
    /// Streams one indexed page entry.
    pub fn open_page(&self, entry: &str) -> Result<EntryReader, LoadError> {
        self.check_open()?;
        let guard = self.reader.borrow();
        let reader = guard.as_ref().ok_or_else(|| LoadError::new("Archive loader was recycled"))?;
        match reader.open_entry(entry).map_err(|e| LoadError::new(e.to_string()))? {
            Some(entry_reader) => Ok(entry_reader),
            None => Err(LoadError::new(trf(MsgKey::reader_page_not_in_archive, &[("entry", Arg::Str(entry))]))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Builds a stored cbz with the given entries through the kuro-archive writer.
    fn zip_with(tag: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/reader-archive-tests").join(tag);
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let mut sources = Vec::new();
        for (name, data) in entries {
            let path = src.join(name.replace('/', "_"));
            std::fs::write(&path, data).unwrap();
            sources.push(path);
        }
        let mut writer = kuro_archive::CbzWriter::create(&base.join("book.cbz")).unwrap();
        for source in &sources {
            writer.add_file(source).unwrap();
        }
        writer.finish().unwrap()
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0123";

    #[test]
    fn indexes_only_image_files_in_natural_order() {
        let path = zip_with("index", &[("10.png", PNG), ("notes.txt", b"hi"), ("2.png", PNG), ("dir/", b"")]);
        let loader = ArchiveLoader::open(path).unwrap();
        let pages = loader.get_pages().ok().unwrap();
        let names: Vec<String> = pages.iter().map(|p| p.url.clone()).collect();
        assert_eq!(names, ["2.png", "10.png"]);
        assert_eq!(pages[1].index, 1);
    }

    #[test]
    fn opens_a_page_by_its_indexed_name_and_streams_its_bytes() {
        let path = zip_with("stream", &[("1.png", PNG), ("2.png", b"second")]);
        let loader = ArchiveLoader::open(path).unwrap();
        let mut reader = loader.open_page("2.png").unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"second");
    }

    #[test]
    fn opening_a_page_after_recycle_fails_with_a_typed_error_and_recycle_is_idempotent() {
        let path = zip_with("recycled", &[("1.png", PNG)]);
        let loader = ArchiveLoader::open(path).unwrap();
        loader.recycle();
        loader.recycle();
        assert!(loader.open_page("1.png").is_err());
        assert_eq!(loader.check_open(), Err(LoadError::new("Archive loader was recycled")));
        assert_eq!(loader.get_pages().err(), Some(LoadFailure::Cancelled));
    }

    #[test]
    fn macos_metadata_is_not_a_page() {
        assert!(is_archive_junk("__MACOSX/._10.png"));
        assert!(is_archive_junk("chapter/._01.jpg"));
        assert!(!is_archive_junk("chapter/01.jpg"));
        assert!(!is_archive_junk("MACOSX.txt"));
    }
}
