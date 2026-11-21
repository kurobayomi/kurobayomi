//! The JSON shapes of the ABI 1.4 ops, and the 1.5 additions (`requestGapMs`). Field names are camelCase. They mirror the
//! `kuro-source` models, which carry no serde derive of their own, so each model has a wire twin and a conversion.

use kuro_source::model::{MangasPage, Page, SChapter, SManga, UpdateStrategy};
use serde::{Deserialize, Serialize};

/// One source a package declares: the `create-sources` list item, or the `info` object of a single-source package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDescriptor {
    pub id: i64,
    pub name: String,
    pub lang: String,
    #[serde(default)]
    pub supports_latest: bool,
    /// Whether the source has a popular listing. Absent means yes (every guest before this field had one). A guest
    /// that says `false` still answers `popular` for apps that do not read the field; this app then shows Latest only.
    #[serde(default = "yes")]
    pub supports_popular: bool,
    /// The least gap, in milliseconds, between two requests the host sends to one host for this source (ABI 1.5).
    /// Absent means no spacing.
    #[serde(default)]
    pub request_gap_ms: Option<u64>,
}

fn yes() -> bool {
    true
}

impl SourceDescriptor {
    pub fn request_gap(&self) -> Option<std::time::Duration> {
        self.request_gap_ms.map(std::time::Duration::from_millis)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireManga {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub status: i32,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub update_strategy: UpdateStrategy,
    #[serde(default)]
    pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireChapter {
    pub url: String,
    pub name: String,
    #[serde(default)]
    pub date_upload: i64,
    #[serde(default = "unknown_chapter_number")]
    pub chapter_number: f32,
    #[serde(default)]
    pub scanlator: Option<String>,
}

fn unknown_chapter_number() -> f32 {
    -1.0
}

/// A page as the guest lists it. The index is not sent: the app numbers pages by their position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WirePage {
    pub url: String,
    #[serde(default)]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireMangasPage {
    #[serde(default)]
    pub mangas: Vec<WireManga>,
    #[serde(default)]
    pub has_next_page: bool,
}

impl From<&SManga> for WireManga {
    fn from(manga: &SManga) -> Self {
        Self {
            url: manga.url.clone(),
            title: manga.title.clone(),
            artist: manga.artist.clone(),
            author: manga.author.clone(),
            description: manga.description.clone(),
            genre: manga.genre.clone(),
            status: manga.status,
            thumbnail_url: manga.thumbnail_url.clone(),
            update_strategy: manga.update_strategy,
            initialized: manga.initialized,
        }
    }
}

impl From<WireManga> for SManga {
    fn from(wire: WireManga) -> Self {
        Self {
            url: wire.url,
            title: wire.title,
            artist: wire.artist,
            author: wire.author,
            description: wire.description,
            genre: wire.genre,
            status: wire.status,
            thumbnail_url: wire.thumbnail_url,
            update_strategy: wire.update_strategy,
            initialized: wire.initialized,
        }
    }
}

impl From<&SChapter> for WireChapter {
    fn from(chapter: &SChapter) -> Self {
        Self {
            url: chapter.url.clone(),
            name: chapter.name.clone(),
            date_upload: chapter.date_upload,
            chapter_number: chapter.chapter_number,
            scanlator: chapter.scanlator.clone(),
        }
    }
}

impl From<WireChapter> for SChapter {
    fn from(wire: WireChapter) -> Self {
        Self {
            url: wire.url,
            name: wire.name,
            date_upload: wire.date_upload,
            chapter_number: wire.chapter_number,
            scanlator: wire.scanlator,
        }
    }
}

impl WirePage {
    /// The page at `index` in the guest's list. The index is the position, as the app assigns it.
    pub fn into_page(self, index: i32) -> Page {
        let mut page = Page::new(index, self.url);
        page.image_url = self.image_url;
        page
    }
}

impl From<WireMangasPage> for MangasPage {
    fn from(wire: WireMangasPage) -> Self {
        Self {
            mangas: wire.mangas.into_iter().map(SManga::from).collect(),
            has_next_page: wire.has_next_page,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_reads_camel_case_and_ignores_unknown_keys() {
        let json = r#"{"mangas":[{"url":"/m/1","title":"One","thumbnailUrl":"t","status":1,"extra":true}],"hasNextPage":true}"#;
        let page: WireMangasPage = serde_json::from_str(json).expect("valid");
        let page = MangasPage::from(page);
        assert!(page.has_next_page);
        assert_eq!(page.mangas[0].title, "One");
        assert_eq!(page.mangas[0].thumbnail_url.as_deref(), Some("t"));
        assert_eq!(page.mangas[0].status, 1);
    }

    #[test]
    fn missing_optional_fields_take_the_source_defaults() {
        let page: WireMangasPage = serde_json::from_str("{}").expect("valid");
        assert_eq!(
            MangasPage::from(page),
            MangasPage {
                mangas: Vec::new(),
                has_next_page: false
            }
        );
        let chapter: WireChapter = serde_json::from_str(r#"{"url":"/c/1","name":"Ch 1"}"#).expect("valid");
        let chapter = SChapter::from(chapter);
        assert_eq!(chapter.chapter_number, -1.0);
        assert_eq!(chapter.date_upload, 0);
        assert_eq!(chapter.scanlator, None);
    }

    #[test]
    fn update_strategy_is_the_stored_ordinal() {
        let manga: WireManga = serde_json::from_str(r#"{"url":"/m","title":"T","updateStrategy":1}"#).expect("valid");
        assert_eq!(SManga::from(manga).update_strategy, UpdateStrategy::OnlyFetchOnce);
    }

    #[test]
    fn a_manga_round_trips_through_its_wire_form() {
        let mut manga = SManga::new("/m/1", "One");
        manga.genre = Some("a, b".to_owned());
        manga.initialized = true;
        let wire = WireManga::from(&manga);
        let json = serde_json::to_string(&wire).expect("serialises");
        let back: WireManga = serde_json::from_str(&json).expect("parses");
        assert_eq!(SManga::from(back), manga);
    }

    #[test]
    fn pages_take_their_index_from_the_position() {
        let page = WirePage {
            url: "/p".into(),
            image_url: Some("i".into()),
        }
        .into_page(3);
        assert_eq!(page.index, 3);
        assert_eq!(page.image_url.as_deref(), Some("i"));
    }

    #[test]
    fn a_descriptor_needs_its_id_name_and_language() {
        let ok: SourceDescriptor =
            serde_json::from_str(r#"{"id":9007199254740993,"name":"A","lang":"en"}"#).expect("valid");
        assert_eq!(ok.id, 9_007_199_254_740_993, "ids are exact integers, not floats");
        assert!(!ok.supports_latest);
        assert!(serde_json::from_str::<SourceDescriptor>(r#"{"name":"A","lang":"en"}"#).is_err());
    }

    #[test]
    fn a_descriptor_has_a_popular_listing_unless_it_says_it_has_none() {
        let before: SourceDescriptor = serde_json::from_str(r#"{"id":1,"name":"A","lang":"en"}"#).expect("valid");
        assert!(before.supports_popular, "a guest written before the field has a popular listing");
        let none: SourceDescriptor =
            serde_json::from_str(r#"{"id":1,"name":"A","lang":"en","supportsLatest":true,"supportsPopular":false}"#).expect("valid");
        assert!(!none.supports_popular && none.supports_latest);
    }

    #[test]
    fn the_info_gap_is_read_in_milliseconds_and_absent_means_none() {
        let spaced: SourceDescriptor =
            serde_json::from_str(r#"{"id":1,"name":"A","lang":"en","requestGapMs":2000}"#).expect("valid");
        assert_eq!(spaced.request_gap(), Some(std::time::Duration::from_secs(2)));
        let plain: SourceDescriptor = serde_json::from_str(r#"{"id":1,"name":"A","lang":"en"}"#).expect("valid");
        assert_eq!(plain.request_gap(), None, "a 1.4 info has no gap");
    }
}
