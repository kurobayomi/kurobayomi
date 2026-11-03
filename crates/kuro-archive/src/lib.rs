//! Kurobayomi archive reading and writing: CBZ, EPUB and libarchive formats.
//!
//! `reader` lists and streams archive entries through the system libarchive (`libarchive.rs`).
//! `epub` reads EPUB page images (T-13a-033); `zip_writer` writes stored CBZ files (T-13a-034).
//!
//! Layout: the T-13a-001 `kuro-core/src/archive/` module lives in this crate instead
//! (kuro-archive owns archives; kuro-core has no `archive` module or feature).

pub mod epub;
mod libarchive;
pub mod reader;
pub mod zip_writer;

pub use epub::EpubReader;
pub use reader::{ArchiveError, ArchiveReader, EntryInfo, EntryReader};
pub use zip_writer::CbzWriter;
