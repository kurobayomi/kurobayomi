//! `EpubReader`: EPUB page images through the container, package document, spine and manifest.
//!
//! Port of `core/archive/.../EpubReader.kt` (T-13a-033).
//! The XML documents are read with `scraper` (html5ever), a lenient HTML parser, as the
//! Kotlin reads them with jsoup. Selectors are descendant selectors (`manifest item`,
//! `spine itemref`) because html5ever ignores the `/` of a self-closing `<item/>` and nests the
//! following items inside it.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use scraper::{ElementRef, Html, Selector};

use crate::reader::{ArchiveError, ArchiveReader};

/// Separator of entry names in the archive (behaviour 4.5 item 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSep {
    Slash,
    Backslash,
}

impl PathSep {
    fn ch(self) -> char {
        match self {
            Self::Slash => '/',
            Self::Backslash => '\\',
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Slash => "/",
            Self::Backslash => "\\",
        }
    }
}

/// Separator used for every path inside the archive. libarchive reports zip entry names with `/`
/// even when the zip stores `\` (observed), so the reader works in `/` and normalises the
/// backslash values read from the XML (see `slashes`).
const SEP: PathSep = PathSep::Slash;

/// EPUB pages and images of one archive. Holds no open handle: every read opens its own.
pub struct EpubReader {
    archive: ArchiveReader,
}

impl EpubReader {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        Ok(Self {
            archive: ArchiveReader::open(path)?,
        })
    }

    /// Path of the package document: the first `rootfile[full-path]` of the container. A missing
    /// container, a missing rootfile or an empty `full-path` falls back to `OEBPS/content.opf`
    /// (deviation D-8).
    pub fn package_href(&self) -> Result<String, ArchiveError> {
        let container = resolve_zip_path("META-INF", "container.xml", SEP);
        if let Some(bytes) = self.read_entry(&container)? {
            let doc = parse(&bytes);
            let full_path = doc
                .select(&selector("rootfile"))
                .next()
                .and_then(|el| el.attr("full-path"))
                .filter(|p| !p.is_empty());
            if let Some(path) = full_path {
                return Ok(slashes(path));
            }
        }
        Ok(resolve_zip_path("OEBPS", "content.opf", SEP))
    }

    /// Image entry paths of every spine page, in spine order, each listed once in first-seen
    /// order (deviation D-10). A spine page missing from the archive is skipped.
    pub fn image_paths(&self) -> Result<Vec<String>, ArchiveError> {
        let package = self.package_href()?;
        let opf = self
            .read_entry(&package)?
            .ok_or_else(|| ArchiveError::PackageMissing(package.clone()))?;
        let base = parent_directory(&package, SEP);

        let mut seen = HashSet::new();
        let mut images = Vec::new();
        for page in spine_pages(&opf) {
            let page_path = resolve_zip_path(&base, &page, SEP);
            let Some(bytes) = self.read_entry(&page_path)? else {
                continue;
            };
            let image_base = parent_directory(&page_path, SEP);
            for src in image_refs(&bytes) {
                let path = resolve_zip_path(&image_base, &src, SEP);
                if seen.insert(path.clone()) {
                    images.push(path);
                }
            }
        }
        Ok(images)
    }

    /// Whole bytes of the entry with exactly this name, or `None` when it is absent.
    pub fn read_entry(&self, name: &str) -> Result<Option<Vec<u8>>, ArchiveError> {
        let Some(mut entry) = self.archive.open_entry(name)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| ArchiveError::Backend(e.to_string()))?;
        Ok(Some(bytes))
    }
}

/// Joins `rel` onto `base` and normalises the result: `.` and empty segments are dropped, `..`
/// removes the previous segment and clamps at the root, and separators collapse. A `rel` that
/// starts with the separator is absolute and returned unchanged. Pure string code: archives have
/// no symlinks, so this equals the Kotlin `canonicalPath` on the same input.
pub fn resolve_zip_path(base: &str, rel: &str, sep: PathSep) -> String {
    if rel.starts_with(sep.ch()) {
        return rel.to_owned();
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in base
        .split(|c| is_sep(c, sep))
        .chain(rel.split(|c| is_sep(c, sep)))
    {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    parts.join(sep.text())
}

/// Directory part of `path`: everything before the last separator, or `""` when there is none.
pub fn parent_directory(path: &str, sep: PathSep) -> String {
    path.rfind(sep.ch())
        .map_or_else(String::new, |i| path[..i].to_owned())
}

/// Backslashes become `/`: in an EPUB written with backslashes, `\` separates path segments.
fn slashes(value: &str) -> String {
    value.replace('\\', "/")
}

/// `/` always separates; `\` separates only in a backslash archive. The Kotlin replaces the
/// archive separator with `/` before it normalises, so `/` is a separator in both modes.
fn is_sep(c: char, sep: PathSep) -> bool {
    c == '/' || c == sep.ch()
}

/// `href` of every XHTML manifest item, in spine order. Items whose `media-type` is not exactly
/// `application/xhtml+xml` are not pages; a spine `idref` with no such item is skipped. A repeated
/// manifest id keeps the last item, as `associateBy` does in the Kotlin.
fn spine_pages(opf: &[u8]) -> Vec<String> {
    let doc = parse(opf);
    let mut manifest = HashMap::new();
    for item in doc.select(&selector("manifest item")) {
        if item.attr("media-type") == Some("application/xhtml+xml") {
            let id = item.attr("id").unwrap_or_default();
            let href = item.attr("href").unwrap_or_default();
            manifest.insert(id.to_owned(), slashes(href));
        }
    }
    doc.select(&selector("spine itemref"))
        .filter_map(|r| r.attr("idref"))
        .filter_map(|id| manifest.get(id).cloned())
        .collect()
}

/// Non-empty `img[src]` and `image[xlink:href]` values in document order (behaviour 4.5 item 6).
/// An empty value is dropped: the Kotlin would resolve it to the directory itself (deviation D-9).
/// `image` is matched by the `xlink` prefix, so a plain SVG2 `href` is not an image reference.
fn image_refs(xhtml: &[u8]) -> Vec<String> {
    let doc = parse(xhtml);
    doc.select(&selector("img, image"))
        .filter_map(|el| {
            let value = match el.value().name() {
                "img" => el.attr("src"),
                "image" => xlink_href(el),
                _ => None,
            };
            value.filter(|v| !v.is_empty()).map(slashes)
        })
        .collect()
}

/// Value of the attribute written `xlink:href`. html5ever keeps the prefix on the qualified name,
/// which scraper's `attr()` does not expose, so the attribute list is scanned directly.
fn xlink_href(el: ElementRef<'_>) -> Option<&str> {
    el.value()
        .attrs
        .iter()
        .find(|(name, _)| name.prefix.as_deref() == Some("xlink") && &*name.local == "href")
        .map(|(_, value)| &**value)
}

fn parse(bytes: &[u8]) -> Html {
    Html::parse_document(&String::from_utf8_lossy(bytes))
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("selectors in this module are constants and parse")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    /// Writes a stored zip with exactly these entry names, backslashes kept.
    fn make_epub(dir: &Path, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("book.epub");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, body) in entries {
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file(*name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    /// Self-closing items and itemrefs, a text/html spine item and an id missing from the manifest.
    const OPF: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
<manifest>
<item id="p2" href="text/p2.xhtml" media-type="application/xhtml+xml"/>
<item id="p1" href="text/p1.xhtml" media-type="application/xhtml+xml"/>
<item id="css" href="style.css" media-type="text/css"/>
<item id="notes" href="text/notes.html" media-type="text/html"/>
</manifest>
<spine>
<itemref idref="p1"/>
<itemref idref="notes"/>
<itemref idref="missing"/>
<itemref idref="p2"/>
</spine>
</package>"#;

    const P1: &str = r#"<html><body>
<img src="../images/a.jpg"/>
<div><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
<image xlink:href="../images/b.png"/>
</svg><img src=""/><img/></div>
</body></html>"#;

    const P2: &str = r#"<html><body>
<img src="../images/a.jpg"/><img src="../images/c.png"/>
<svg><image href="../images/plain.png"/><image xlink:href="../images/d.png" xmlns:xlink="http://www.w3.org/1999/xlink"/></svg>
</body></html>"#;

    fn standard_entries() -> Vec<(&'static str, &'static str)> {
        vec![
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/text/p1.xhtml", P1),
            ("OEBPS/text/p2.xhtml", P2),
        ]
    }

    #[test]
    fn resolve_matches_the_kotlin_vectors() {
        let s = PathSep::Slash;
        let cases = [
            ("OEBPS", "text/a.xhtml", "OEBPS/text/a.xhtml"),
            ("OEBPS/text", "../img/a.png", "OEBPS/img/a.png"),
            ("OEBPS", "../../../x", "x"),
            ("", "x", "x"),
            ("OEBPS", "/abs/x", "/abs/x"),
            ("OEBPS", "./a.png", "OEBPS/a.png"),
            ("OEBPS//x", "a", "OEBPS/x/a"),
        ];
        for (base, rel, want) in cases {
            assert_eq!(resolve_zip_path(base, rel, s), want, "{base} + {rel}");
        }
    }

    #[test]
    fn resolve_handles_backslash_archives() {
        assert_eq!(resolve_zip_path("OEBPS", "..\\x", PathSep::Backslash), "x");
        assert_eq!(
            resolve_zip_path("OEBPS\\text", "..\\img\\a.png", PathSep::Backslash),
            "OEBPS\\img\\a.png"
        );
        // In a backslash archive a `/` in a relative path is still a separator.
        assert_eq!(
            resolve_zip_path("OEBPS", "text/a.xhtml", PathSep::Backslash),
            "OEBPS\\text\\a.xhtml"
        );
    }

    #[test]
    fn parent_directory_matches_the_kotlin_vectors() {
        assert_eq!(
            parent_directory("OEBPS/content.opf", PathSep::Slash),
            "OEBPS"
        );
        assert_eq!(parent_directory("content.opf", PathSep::Slash), "");
        assert_eq!(
            parent_directory("OEBPS\\content.opf", PathSep::Backslash),
            "OEBPS"
        );
    }

    #[test]
    fn package_href_reads_the_first_rootfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_epub(dir.path(), &standard_entries());
        let epub = EpubReader::open(&path).unwrap();
        assert_eq!(epub.package_href().unwrap(), "OEBPS/content.opf");
    }

    #[test]
    fn package_href_falls_back_when_full_path_is_missing_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        for container in [
            "<container><rootfiles><rootfile/></rootfiles></container>",
            r#"<container><rootfiles><rootfile full-path=""/></rootfiles></container>"#,
        ] {
            let path = make_epub(dir.path(), &[("META-INF/container.xml", container)]);
            let epub = EpubReader::open(&path).unwrap();
            assert_eq!(epub.package_href().unwrap(), "OEBPS/content.opf");
        }
    }

    #[test]
    fn package_href_falls_back_when_container_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_epub(dir.path(), &[("OEBPS/content.opf", OPF)]);
        let epub = EpubReader::open(&path).unwrap();
        assert_eq!(epub.package_href().unwrap(), "OEBPS/content.opf");
    }

    #[test]
    fn image_paths_follow_spine_order_and_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_epub(dir.path(), &standard_entries());
        let images = EpubReader::open(&path).unwrap().image_paths().unwrap();
        assert_eq!(
            images,
            [
                "OEBPS/images/a.jpg",
                "OEBPS/images/b.png",
                "OEBPS/images/c.png",
                "OEBPS/images/d.png",
            ]
        );
    }

    #[test]
    fn fixture_epub_lists_spine_images_once_in_spine_order() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.epub");
        let epub = EpubReader::open(&path).unwrap();
        assert_eq!(epub.package_href().unwrap(), "OEBPS/content.opf");
        assert_eq!(
            epub.image_paths().unwrap(),
            [
                "OEBPS/images/a.jpg",
                "OEBPS/images/b.png",
                "OEBPS/images/c.png",
            ]
        );
    }

    #[test]
    fn image_paths_use_the_fallback_package_when_container_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut entries = standard_entries();
        entries.remove(0);
        let path = make_epub(dir.path(), &entries);
        let images = EpubReader::open(&path).unwrap().image_paths().unwrap();
        assert_eq!(images.len(), 4);
    }

    #[test]
    fn backslash_epub_resolves_with_backslash_separators() {
        let dir = tempfile::tempdir().unwrap();
        let container = r#"<container><rootfiles><rootfile full-path="OEBPS\content.opf"/></rootfiles></container>"#;
        let opf = r#"<package><manifest>
<item id="p1" href="text\p1.xhtml" media-type="application/xhtml+xml"/>
</manifest><spine><itemref idref="p1"/></spine></package>"#;
        let page = r#"<html><body><img src="..\images\a.jpg"/>
<svg><image xlink:href="..\images\a.jpg" xmlns:xlink="http://www.w3.org/1999/xlink"/></svg></body></html>"#;
        let path = make_epub(
            dir.path(),
            &[
                ("META-INF\\container.xml", container),
                ("OEBPS\\content.opf", opf),
                ("OEBPS\\text\\p1.xhtml", page),
            ],
        );
        let epub = EpubReader::open(&path).unwrap();
        assert_eq!(epub.image_paths().unwrap(), ["OEBPS/images/a.jpg"]);
    }

    #[test]
    fn missing_package_is_a_package_error() {
        let dir = tempfile::tempdir().unwrap();
        let container = r#"<container><rootfiles><rootfile full-path="OEBPS/gone.opf"/></rootfiles></container>"#;
        let path = make_epub(dir.path(), &[("META-INF/container.xml", container)]);
        let err = EpubReader::open(&path).unwrap().image_paths().unwrap_err();
        assert!(
            matches!(err, ArchiveError::PackageMissing(ref p) if p == "OEBPS/gone.opf"),
            "{err}"
        );
    }
}
