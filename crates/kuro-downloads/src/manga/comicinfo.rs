//! `ComicInfo.xml` writer (T-23a-021; the status table of Kotlin
//! `ComicInfoPublishingStatus`). The file goes into the chapter folder before it is packed as a CBZ.
//!
//! Element order is the one of the Kotlin data class. Fields that are null in Kotlin (`Inker`, `Colorist`, `Letterer`,
//! `CoverArtist`, `Tags`) are never written. Decision: the `ty` and `ay` prefixes are declared on the root, because XML
//! needs the declaration; Kotlin's xmlutil declares them too.

use kuro_core::xml::write;

/// The file name inside the chapter folder (`COMIC_INFO_FILE`).
pub const FILE_NAME: &str = "ComicInfo.xml";
const XML_SCHEMA: &str = "http://www.w3.org/2001/XMLSchema";
const XML_SCHEMA_INSTANCE: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The fields `getComicInfo` reads. Optional fields are `None` when Kotlin's value is null.
#[derive(Debug, Clone, Default)]
pub struct ComicInfo<'a> {
    pub chapter_name: &'a str,
    pub series: &'a str,
    /// `chapter.chapterNumber`. Negative numbers are not written.
    pub number: Option<f64>,
    pub summary: Option<&'a str>,
    pub writer: Option<&'a str>,
    pub penciller: Option<&'a str>,
    pub translator: Option<&'a str>,
    pub genre: Option<&'a [String]>,
    /// The `Web` value, from [`web_value`].
    pub web: &'a str,
    /// `manga.status` (SManga values; anything but 1 to 6 is "Unknown").
    pub status: i64,
    /// Category names; an empty list omits `Categories` (EC-64).
    pub categories: &'a [String],
    pub source_name: &'a str,
}

/// The text of a chapter number: an integer when whole, `Double.toString()` otherwise (EC-66). `None` for a negative
/// number, which is not written (EC-65).
pub fn number_text(number: f64) -> Option<String> {
    if number < 0.0 {
        return None;
    }
    if number.fract() == 0.0 {
        Some(format!("{}", number as i64))
    } else {
        Some(format!("{number}"))
    }
}

/// The publishing status text (EC-67).
pub fn status_text(status: i64) -> &'static str {
    match status {
        1 => "Ongoing",
        2 => "Completed",
        3 => "Licensed",
        4 => "Publishing finished",
        5 => "Cancelled",
        6 => "On hiatus",
        _ => "Unknown",
    }
}

/// `Web`: the tracker remote URLs and the chapter URL, trimmed, blanks dropped, duplicates removed, joined by a space.
pub fn web_value(tracker_urls: &[String], chapter_url: &str) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for url in tracker_urls.iter().map(String::as_str).chain(std::iter::once(chapter_url)) {
        let url = url.trim();
        if !url.is_empty() && !seen.contains(&url) {
            seen.push(url);
        }
    }
    seen.join(" ")
}

/// The full document, UTF-8, with the element order of the data class.
pub fn to_xml(info: &ComicInfo<'_>) -> String {
    let mut children: Vec<(&str, String)> = vec![("Title", info.chapter_name.to_owned()), ("Series", info.series.to_owned())];
    if let Some(number) = info.number.and_then(number_text) {
        children.push(("Number", number));
    }
    if let Some(summary) = info.summary {
        children.push(("Summary", summary.to_owned()));
    }
    if let Some(writer) = info.writer {
        children.push(("Writer", writer.to_owned()));
    }
    if let Some(penciller) = info.penciller {
        children.push(("Penciller", penciller.to_owned()));
    }
    if let Some(translator) = info.translator {
        children.push(("Translator", translator.to_owned()));
    }
    if let Some(genre) = info.genre {
        children.push(("Genre", genre.join(", ")));
    }
    children.push(("Web", info.web.to_owned()));
    children.push(("ty:PublishingStatusTachiyomi", status_text(info.status).to_owned()));
    if !info.categories.is_empty() {
        children.push(("ty:Categories", info.categories.join(", ")));
    }
    children.push(("ay:SourceAniyomi", info.source_name.to_owned()));
    let pairs: Vec<(&str, &str)> = children.iter().map(|(k, v)| (*k, v.as_str())).collect();
    write(
        "ComicInfo",
        &[
            ("xmlns:xsd", XML_SCHEMA),
            ("xmlns:xsi", XML_SCHEMA_INSTANCE),
            ("xmlns:ty", XML_SCHEMA),
            ("xmlns:ay", XML_SCHEMA),
        ],
        &pairs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(genre: &'a [String], categories: &'a [String]) -> ComicInfo<'a> {
        ComicInfo {
            chapter_name: "Chapter 12.5",
            series: "Test: Manga/1",
            number: Some(12.5),
            summary: Some("A & B <tag>"),
            writer: Some("Writer"),
            penciller: None,
            translator: Some("Group"),
            genre: Some(genre),
            web: "https://tracker.example/1 https://example.com/ch",
            status: 2,
            categories,
            source_name: "Source (EN)",
        }
    }

    #[test]
    fn element_order_namespaces_and_escaping_match_the_golden_text() {
        let genre = vec!["Action".to_owned(), "Drama".to_owned()];
        let xml = to_xml(&sample(&genre, &["Reading".to_owned()]));
        let golden = include_str!("../../testdata/comicinfo/golden.xml");
        assert_eq!(xml, golden, "TST-31");
    }

    // TST-32
    #[test]
    fn number_formatting_follows_the_kotlin_text() {
        assert_eq!(number_text(12.0).as_deref(), Some("12"), "whole numbers are integers");
        assert_eq!(number_text(12.5).as_deref(), Some("12.5"), "EC-66");
        assert_eq!(number_text(-1.0), None, "EC-65: a negative number is omitted");
    }

    #[test]
    fn status_mapping_and_the_unknown_default() {
        assert_eq!(status_text(1), "Ongoing");
        assert_eq!(status_text(6), "On hiatus");
        assert_eq!(status_text(0), "Unknown");
        assert_eq!(status_text(99), "Unknown", "EC-67");
    }

    #[test]
    fn empty_categories_and_missing_optional_fields_are_left_out() {
        let xml = to_xml(&ComicInfo { number: Some(-1.0), genre: None, ..sample(&[], &[]) });
        assert!(!xml.contains("Categories"), "EC-64");
        assert!(!xml.contains("<Number>"), "EC-65");
        assert!(!xml.contains("Genre"));
        assert!(!xml.contains("Penciller"));
    }

    #[test]
    fn web_is_trimmed_distinct_and_space_joined() {
        let trackers = vec![" https://a.example ".to_owned(), "".to_owned(), "https://a.example".to_owned()];
        assert_eq!(web_value(&trackers, " https://a.example/ch "), "https://a.example https://a.example/ch");
        assert_eq!(web_value(&[], ""), "", "an empty Web is still written");
    }
}
