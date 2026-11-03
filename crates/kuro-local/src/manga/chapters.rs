//! Manga chapter list (T-14a-014).

use std::path::Path;

use kuro_source::model::{SChapter, SManga};

use crate::dates::parse_local_date;
use crate::epub;
use crate::error::LocalSourceError;
use crate::format::{is_epub, is_manga_archive};
use crate::fs::{FsEntry, find_child, list_dir};
use crate::json_side::{ChapterDetails, read_chapters_json, same_number, side_file};
use crate::names::{is_hidden, stem};
use crate::natsort::natural_cmp_ci;
use crate::recognition::chapter_number;

/// The chapters of the folder named by `manga.url`: non-hidden folders, manga archives and EPUB files. Newest number
/// first, then name descending, natural and case-insensitive. A missing folder gives an empty list.
///
/// Kotlin order: the EPUB metadata fill (T-14a-041) sets the manga author and description and the chapter fields, then
/// the `chapters.json` override (T-14a-039) replaces them. A bad EPUB keeps its file-name title and date and is logged
/// (E-MANGA-3).
pub fn chapters(root: &Path, manga: &mut SManga) -> Result<Vec<SChapter>, LocalSourceError> {
    let Some(dir) = find_child(root, &manga.url).filter(|p| p.is_dir()) else {
        return Err(LocalSourceError::folder_missing());
    };
    let entries = list_dir(&dir)?;
    let overrides = chapter_overrides(&entries);
    let mut chapters: Vec<SChapter> = Vec::new();
    for entry in entries.iter().filter(|e| is_chapter_entry(e)) {
        let mut chapter = chapter_of(manga, entry);
        if is_epub(&entry.name) {
            match epub::read_metadata(&entry.path) {
                Ok(meta) => epub::fill_metadata(manga, &mut chapter, &meta),
                Err(error) => tracing::warn!(
                    "EPUB metadata skipped for {}: {error}",
                    entry.path.display()
                ),
            }
        }
        if let Some(data) = overrides
            .iter()
            .flatten()
            .find(|d| same_number(chapter.chapter_number, d.chapter_number))
        {
            apply_override(&mut chapter, data);
        }
        chapters.push(chapter);
    }
    chapters.sort_by(|a, b| {
        b.chapter_number
            .total_cmp(&a.chapter_number)
            .then_with(|| natural_cmp_ci(&b.name, &a.name))
    });
    Ok(chapters)
}

/// The first `chapters.json` in the folder (name order). A file that does not decode is ignored (E-CHAPTERS-1).
fn chapter_overrides(entries: &[FsEntry]) -> Option<Vec<ChapterDetails>> {
    read_chapters_json(&side_file(entries, "chapters")?.path)
}

/// The override's name and date when present; a date that does not parse keeps the file date (E-DATE-2). The scanlator
/// is replaced even when the override has none (E-CHAPTERS-4).
fn apply_override(chapter: &mut SChapter, data: &ChapterDetails) {
    if let Some(name) = &data.name {
        chapter.name = name.clone();
    }
    if let Some(millis) = data.date_upload.as_deref().and_then(parse_local_date) {
        chapter.date_upload = millis;
    }
    chapter.scanlator = data.scanlator.clone();
}

/// Kotlin's chapter filter: `!startsWith('.') && (isDirectory || manga archive || epub)`.
fn is_chapter_entry(entry: &FsEntry) -> bool {
    !is_hidden(&entry.name)
        && (entry.is_dir || is_manga_archive(&entry.name) || is_epub(&entry.name))
}

fn chapter_of(manga: &SManga, entry: &FsEntry) -> SChapter {
    let name = if entry.is_dir {
        entry.name.clone()
    } else {
        stem(&entry.name).to_owned()
    };
    let mut chapter = SChapter::new(format!("{}/{}", manga.url, entry.name), name);
    chapter.date_upload = entry.modified_ms;
    chapter.chapter_number = chapter_number(&manga.title, &chapter.name);
    chapter
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates;
    use std::fs;
    use std::io;

    /// A manga folder `Show` under a root, with the given entries. A name ending in `/` is a folder.
    fn manga_with(entries: &[&str]) -> io::Result<(tempfile::TempDir, SManga)> {
        let root = tempfile::tempdir()?;
        let show = root.path().join("Show");
        fs::create_dir(&show)?;
        for entry in entries {
            match entry.strip_suffix('/') {
                Some(dir) => fs::create_dir(show.join(dir))?,
                None => fs::write(show.join(entry), b"x")?,
            }
        }
        Ok((root, SManga::new("Show", "Show")))
    }

    fn names(chapters: &[SChapter]) -> Vec<&str> {
        chapters.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn only_folders_manga_archives_and_epubs_are_chapters() -> io::Result<()> {
        let (root, mut manga) = manga_with(&[
            "Ch 01/",
            "Ch 02.cbz",
            "Ch 03.EPUB",
            "readme.txt",
            ".hidden/",
            ".x.cbz",
            "page.jpg",
        ])?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        assert_eq!(got.len(), 3, "{:?}", names(&got));
        let mut sorted = names(&got);
        sorted.sort_unstable();
        assert_eq!(sorted, ["Ch 01", "Ch 02", "Ch 03"]);
        Ok(())
    }

    #[test]
    fn the_url_is_manga_folder_slash_entry_name_with_extension() -> io::Result<()> {
        let (root, mut manga) = manga_with(&["Ch 02.cbz"])?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        assert_eq!(got[0].url, "Show/Ch 02.cbz");
        assert_eq!(got[0].name, "Ch 02");
        assert_eq!(got[0].chapter_number, 2.0);
        assert!(got[0].date_upload > 0);
        Ok(())
    }

    #[test]
    fn newest_number_first_and_unrecognised_last() -> io::Result<()> {
        let (root, mut manga) =
            manga_with(&["Show 2.cbz", "Show 10.cbz", "Show 1.cbz", "Notes.cbz"])?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        assert_eq!(names(&got), ["Show 10", "Show 2", "Show 1", "Notes"]);
        assert_eq!(got.last().map(|c| c.chapter_number), Some(-1.0));
        Ok(())
    }

    #[test]
    fn a_missing_manga_folder_is_an_error_not_an_empty_list() -> io::Result<()> {
        let (root, _) = manga_with(&[])?;
        let mut gone = SManga::new("Gone", "Gone");
        assert!(chapters(root.path(), &mut gone).is_err());
        Ok(())
    }

    #[test]
    fn chapters_json_overrides_name_date_and_scanlator_by_number() -> io::Result<()> {
        let (root, mut manga) = manga_with(&["Ch 01.cbz", "Ch 02.cbz"])?;
        fs::write(
            root.path().join("Show").join("chapters.json"),
            r#"[{"chapter_number":1,"name":"Prologue","date_upload":"2024-03-01T12:00:00","scanlator":"A"},{"chapter_number":2,"date_upload":"2024-02-31T12:00:00"}]"#,
        )?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        let one = got
            .iter()
            .find(|c| c.chapter_number == 1.0)
            .expect("chapter 1");
        assert_eq!(one.name, "Prologue");
        assert_eq!(one.scanlator.as_deref(), Some("A"));
        assert!(one.date_upload > 0);
        let two = got
            .iter()
            .find(|c| c.chapter_number == 2.0)
            .expect("chapter 2");
        assert_eq!(two.name, "Ch 02");
        assert_eq!(two.scanlator, None);
        Ok(())
    }

    #[test]
    fn a_bad_chapters_json_is_ignored_and_the_list_still_comes() -> io::Result<()> {
        let (root, mut manga) = manga_with(&["Ch 01.cbz"])?;
        fs::write(root.path().join("Show").join("chapters.json"), "{")?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        assert_eq!(names(&got), ["Ch 01"]);
        Ok(())
    }

    #[test]
    fn t_14_chapters_epub_title_replaces_name_not_number() -> io::Result<()> {
        let (root, mut manga) = manga_with(&[])?;
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.epub");
        fs::copy(fixture, root.path().join("Show").join("Vol 7.epub"))?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        assert_eq!(got[0].name, "Chapter Title", "the title replaces the file name");
        assert_eq!(got[0].chapter_number, 7.0, "the number still comes from the file name");
        Ok(())
    }

    #[test]
    fn an_epub_chapter_takes_its_title_and_date_from_the_package() -> io::Result<()> {
        let (root, mut manga) = manga_with(&[])?;
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata.epub");
        fs::copy(fixture, root.path().join("Show").join("Vol 1.epub"))?;
        let got = chapters(root.path(), &mut manga).map_err(io::Error::other)?;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "Chapter Title");
        assert_eq!(got[0].scanlator.as_deref(), Some("Team X"));
        assert_eq!(manga.author.as_deref(), Some("Writer"));
        assert_eq!(manga.description.as_deref(), Some("About & more"));
        assert_eq!(
            dates::parse_epub_date("2024-01-02T03:04:05+0100"),
            Some(got[0].date_upload)
        );
        Ok(())
    }
}
