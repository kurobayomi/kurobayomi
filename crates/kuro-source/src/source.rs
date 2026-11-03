//! The source traits (`MangaSource.kt`, `CatalogueSource.kt`, `ConfigurableSource.kt`, `SourceFactory.kt`) and the
//! panic boundary every source call goes through (T-15a-013, T-15a-014).
//!
//! Sources are used as `Arc<dyn CatalogueSource>`. The Rx `fetch*` methods and `chapterPageParse` are not part of
//! the API (T-15a-047).

use std::future::Future;
use std::panic::AssertUnwindSafe;

use async_trait::async_trait;
use futures::FutureExt;

use crate::error::SourceError;
use crate::filter_list::FilterList;
use crate::model::{MangasPage, Page, SChapter, SManga};
use crate::prefs::SourcePref;

/// A manga source: details, chapters and pages for a manga it knows.
#[async_trait]
pub trait MangaSource: Send + Sync {
    /// Stable identity (see [`crate::id::generate_id`]).
    fn id(&self) -> i64;
    fn name(&self) -> &str;
    /// ISO 639-1 lowercase, or `all` or `other`. Empty when the source states none.
    fn lang(&self) -> &str {
        ""
    }
    /// Returns the updated copy. The input is never changed.
    async fn get_manga_details(&self, manga: &SManga) -> Result<SManga, SourceError>;
    /// The chapters in source order.
    async fn get_chapter_list(&self, manga: &SManga) -> Result<Vec<SChapter>, SourceError>;
    /// The pages of a chapter. The `index` values returned are ignored; the app numbers them.
    async fn get_page_list(&self, chapter: &SChapter) -> Result<Vec<Page>, SourceError>;
    /// `true` for a source on the user's own machine or network, which is left out of the queue warnings.
    fn is_unmetered(&self) -> bool {
        false
    }
    /// `true` when the source has a settings screen: the Kotlin `is ConfigurableSource` check, which a trait object
    /// cannot ask. A source that returns `true` must also implement [`ConfigurableSource`].
    fn is_configurable(&self) -> bool {
        false
    }
    /// Called for every chapter on every library sync, before the database lookup. Only HTTP sources override it, so
    /// the default is a no-op. It may rewrite the chapter, so a rewritten URL makes the chapter look new (EC-CH-05).
    fn prepare_new_chapter(&self, _chapter: &mut SChapter, _manga: &SManga) {}
    /// The headers the app sends with each page image of this source. `None` means the source declares none, so the
    /// app's default set applies. Only a WASM source with ABI 1.5 answers it.
    async fn request_headers(&self) -> Result<Option<Vec<(String, String)>>, SourceError> {
        Ok(None)
    }
    /// The page-image fix-up of this source, given the image's response headers: the bytes to show. `None` means show
    /// the bytes as fetched. Only a WASM source with ABI 1.5 answers it.
    async fn unscramble(&self, _body: &[u8], _headers: &[(String, String)]) -> Result<Option<Vec<u8>>, SourceError> {
        Ok(None)
    }
}

/// A source with browsable listings. Pages start at 1.
#[async_trait]
pub trait CatalogueSource: MangaSource {
    /// Whether the source has a "latest" listing. Every source states it; there is no default.
    fn supports_latest(&self) -> bool;
    /// The filters for a fresh browse screen. A new list on every call.
    fn filter_list(&self) -> FilterList;
    async fn popular(&self, page: i32) -> Result<MangasPage, SourceError>;
    async fn search(&self, page: i32, query: &str, filters: &FilterList) -> Result<MangasPage, SourceError>;
    async fn latest(&self, page: i32) -> Result<MangasPage, SourceError>;
}

/// A source with a settings screen. Its rows are read when the screen opens (`preferences`, T-16a-035).
pub trait ConfigurableSource: MangaSource {
    fn preferences(&self) -> Vec<SourcePref>;
    /// Called on the source instance once for each value the user writes, with the key of its row. Nothing is reloaded
    /// (T-20a-005). The settings screen that writes the values calls it; the default does nothing.
    fn on_settings_changed(&self, _key: &str) {}
}

/// What a deep-link URI names on a manga source (`ResolvableSource.kt`; T-16a-045).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MangaUriType {
    Manga,
    Chapter,
    Unknown,
}

/// A source that turns a URI of its own site into a manga or a chapter.
#[async_trait]
pub trait ResolvableSource: MangaSource {
    fn uri_type(&self, uri: &str) -> MangaUriType;
    async fn manga_from_uri(&self, uri: &str) -> Result<Option<SManga>, SourceError>;
    async fn chapter_from_uri(&self, uri: &str) -> Result<Option<SChapter>, SourceError>;
}

/// The answer to a manga deep link.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedManga {
    Manga(Option<SManga>),
    Chapter(Option<SChapter>),
    /// Not a link of this source. No lookup ran.
    Unknown,
}

/// Runs the one lookup that matches the URI type, so a source is asked only for the kind it recognises.
pub async fn resolve_manga_uri(source: &dyn ResolvableSource, uri: &str) -> Result<ResolvedManga, SourceError> {
    match source.uri_type(uri) {
        MangaUriType::Manga => Ok(ResolvedManga::Manga(source.manga_from_uri(uri).await?)),
        MangaUriType::Chapter => Ok(ResolvedManga::Chapter(source.chapter_from_uri(uri).await?)),
        MangaUriType::Unknown => Ok(ResolvedManga::Unknown),
    }
}

/// What a deep link resolved to. The `Loading` state belongs to the caller, which shows it while
/// this runs.
#[derive(Debug, Clone, PartialEq)]
pub enum DeepLinkOutcome {
    NoResults,
    /// `source_id` is the source that recognised the link and resolved it.
    Result { source_id: i64, manga: SManga, chapter: Option<SChapter> },
}

/// The deep-link flow (T-15a-043). The first source in list order that recognises `uri` resolves it (EC-DL-04). A lookup
/// error is logged and gives `NoResults` (EC-DL-03, a deliberate change). A chapter is looked up only for a chapter link
/// whose manga resolved (EC-DL-02).
pub async fn resolve_deep_link(sources: &[&dyn ResolvableSource], uri: &str) -> DeepLinkOutcome {
    let Some(source) = sources.iter().find(|s| s.uri_type(uri) != MangaUriType::Unknown) else {
        return DeepLinkOutcome::NoResults;
    };
    let manga = match source.manga_from_uri(uri).await {
        Ok(Some(manga)) => manga,
        Ok(None) => return DeepLinkOutcome::NoResults,
        Err(err) => {
            tracing::error!(?err, "deep link manga lookup failed");
            return DeepLinkOutcome::NoResults;
        }
    };
    let chapter = if source.uri_type(uri) == MangaUriType::Chapter {
        source.chapter_from_uri(uri).await.unwrap_or_else(|err| {
            tracing::error!(?err, "deep link chapter lookup failed");
            None
        })
    } else {
        None
    };
    DeepLinkOutcome::Result { source_id: source.id(), manga, chapter }
}

/// Runs one source call behind a panic boundary. A panic inside the call becomes `Parse("source panicked")`, so a
/// broken source fails its own request and the process keeps running.
pub async fn isolate<T>(call: impl Future<Output = Result<T, SourceError>>) -> Result<T, SourceError> {
    match AssertUnwindSafe(call).catch_unwind().await {
        Ok(result) => result,
        Err(_) => Err(SourceError::Parse("source panicked".to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;

    /// A catalogue source that records the pages it was asked for. `panic_on_details` makes it panic.
    struct Fake {
        latest: bool,
        asked: Mutex<Vec<i32>>,
        panic_on_details: bool,
        configurable: bool,
        changed: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(latest: bool) -> Self {
            Self {
                latest,
                asked: Mutex::new(Vec::new()),
                panic_on_details: false,
                configurable: false,
                changed: Mutex::new(Vec::new()),
            }
        }

        fn ask(&self, page: i32) -> Result<MangasPage, SourceError> {
            self.asked.lock().unwrap().push(page);
            Ok(MangasPage { mangas: vec![SManga::new("/m", "M")], has_next_page: false })
        }
    }

    #[async_trait]
    impl MangaSource for Fake {
        fn id(&self) -> i64 {
            7
        }

        fn name(&self) -> &str {
            "Fake"
        }

        async fn get_manga_details(&self, manga: &SManga) -> Result<SManga, SourceError> {
            if self.panic_on_details {
                panic!("broken parser");
            }
            Ok(manga.clone())
        }

        async fn get_chapter_list(&self, _manga: &SManga) -> Result<Vec<SChapter>, SourceError> {
            Ok(Vec::new())
        }

        async fn get_page_list(&self, _chapter: &SChapter) -> Result<Vec<Page>, SourceError> {
            Ok(Vec::new())
        }

        fn is_configurable(&self) -> bool {
            self.configurable
        }
    }

    impl ConfigurableSource for Fake {
        fn preferences(&self) -> Vec<SourcePref> {
            Vec::new()
        }

        fn on_settings_changed(&self, key: &str) {
            self.changed.lock().unwrap().push(key.to_owned());
        }
    }

    #[test]
    fn only_a_source_that_says_so_is_configurable() {
        let plain: Arc<dyn MangaSource> = Arc::new(Fake::new(false));
        assert!(!plain.is_configurable());
        let settings: Arc<dyn ConfigurableSource> = Arc::new(Fake { configurable: true, ..Fake::new(false) });
        let as_manga: Arc<dyn MangaSource> = settings.clone();
        assert!(as_manga.is_configurable());
        assert!(settings.preferences().is_empty());
    }

    #[test]
    fn a_settings_change_reaches_the_instance_once_per_write() {
        let fake = Fake { configurable: true, ..Fake::new(false) };
        let settings: &dyn ConfigurableSource = &fake;
        settings.on_settings_changed("quality");
        settings.on_settings_changed("quality");
        assert_eq!(*fake.changed.lock().unwrap(), ["quality", "quality"]);
        assert!(fake.asked.lock().unwrap().is_empty(), "a change does not reload pages");
    }

    #[async_trait]
    impl CatalogueSource for Fake {
        fn supports_latest(&self) -> bool {
            self.latest
        }

        fn filter_list(&self) -> FilterList {
            FilterList::default()
        }

        async fn popular(&self, page: i32) -> Result<MangasPage, SourceError> {
            self.ask(page)
        }

        async fn search(&self, page: i32, _query: &str, _filters: &FilterList) -> Result<MangasPage, SourceError> {
            self.ask(page)
        }

        async fn latest(&self, page: i32) -> Result<MangasPage, SourceError> {
            self.ask(page)
        }
    }

    #[tokio::test]
    async fn listings_are_called_with_page_one_first() {
        let fake = Fake::new(true);
        fake.popular(1).await.unwrap();
        fake.latest(1).await.unwrap();
        fake.search(1, "q", &FilterList::default()).await.unwrap();
        assert_eq!(*fake.asked.lock().unwrap(), vec![1, 1, 1]);
    }

    #[tokio::test]
    async fn the_source_is_used_as_an_arc_dyn_and_upcast_to_a_manga_source() {
        let catalogue: Arc<dyn CatalogueSource> = Arc::new(Fake::new(false));
        let manga: Arc<dyn MangaSource> = catalogue.clone();
        assert_eq!(manga.id(), 7);
        assert_eq!(manga.lang(), "");
        assert!(!manga.is_unmetered());
        assert!(!catalogue.supports_latest());
        let details = manga.get_manga_details(&SManga::new("/m", "M")).await.unwrap();
        assert_eq!(details.title, "M");
    }

    #[tokio::test]
    async fn a_panic_inside_a_source_call_is_a_parse_error_and_the_process_lives_on() {
        let fake = Fake { panic_on_details: true, ..Fake::new(true) };
        let err = isolate(fake.get_manga_details(&SManga::new("/m", "M"))).await.unwrap_err();
        assert_eq!(err, SourceError::Parse("source panicked".to_owned()));
        let ok = isolate(fake.popular(1)).await.unwrap();
        assert_eq!(ok.mangas.len(), 1);
    }

    #[test]
    fn a_non_http_source_leaves_the_chapter_alone_through_the_trait_object() {
        let manga: Arc<dyn MangaSource> = Arc::new(Fake::new(false));
        let mut chapter = SChapter::new("/ch/1", "Ch 1");
        manga.prepare_new_chapter(&mut chapter, &SManga::new("/m", "M"));
        assert_eq!(chapter, SChapter::new("/ch/1", "Ch 1"));
    }

    #[tokio::test]
    async fn isolate_passes_errors_through_unchanged() {
        let err = isolate(async { Err::<(), _>(SourceError::Http(503)) }).await.unwrap_err();
        assert_eq!(err, SourceError::Http(503));
    }

    /// A deep-link source that counts each lookup. `/m/` names a manga and `/c/` a chapter.
    #[derive(Default)]
    struct Links {
        manga_lookups: Mutex<usize>,
        chapter_lookups: Mutex<usize>,
        fail_manga: bool,
    }

    #[async_trait]
    impl MangaSource for Links {
        fn id(&self) -> i64 {
            9
        }
        fn name(&self) -> &str {
            "Links"
        }
        async fn get_manga_details(&self, manga: &SManga) -> Result<SManga, SourceError> {
            Ok(manga.clone())
        }
        async fn get_chapter_list(&self, _manga: &SManga) -> Result<Vec<SChapter>, SourceError> {
            Ok(Vec::new())
        }
        async fn get_page_list(&self, _chapter: &SChapter) -> Result<Vec<Page>, SourceError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl ResolvableSource for Links {
        fn uri_type(&self, uri: &str) -> MangaUriType {
            if uri.contains("/m/") {
                MangaUriType::Manga
            } else if uri.contains("/c/") {
                MangaUriType::Chapter
            } else {
                MangaUriType::Unknown
            }
        }
        async fn manga_from_uri(&self, _uri: &str) -> Result<Option<SManga>, SourceError> {
            *self.manga_lookups.lock().unwrap() += 1;
            if self.fail_manga {
                return Err(SourceError::Parse("boom".into()));
            }
            Ok(Some(SManga::new("/m/1", "One")))
        }
        async fn chapter_from_uri(&self, _uri: &str) -> Result<Option<SChapter>, SourceError> {
            *self.chapter_lookups.lock().unwrap() += 1;
            Ok(None)
        }
    }

    #[tokio::test]
    async fn a_manga_uri_runs_only_the_manga_lookup() {
        let source = Links::default();
        let resolved = resolve_manga_uri(&source, "https://x.test/m/1").await.unwrap();
        assert_eq!(resolved, ResolvedManga::Manga(Some(SManga::new("/m/1", "One"))));
        assert_eq!((*source.manga_lookups.lock().unwrap(), *source.chapter_lookups.lock().unwrap()), (1, 0));
    }

    #[tokio::test]
    async fn a_chapter_uri_runs_only_the_chapter_lookup_and_an_unknown_uri_runs_none() {
        let source = Links::default();
        assert_eq!(resolve_manga_uri(&source, "https://x.test/c/1").await.unwrap(), ResolvedManga::Chapter(None));
        assert_eq!(resolve_manga_uri(&source, "https://other.test/x").await.unwrap(), ResolvedManga::Unknown);
        assert_eq!((*source.manga_lookups.lock().unwrap(), *source.chapter_lookups.lock().unwrap()), (0, 1));
    }

    /// A deep-link source with a fixed title, so two of them can be told apart in list order.
    struct Titled(&'static str);

    #[async_trait]
    impl MangaSource for Titled {
        fn id(&self) -> i64 {
            10
        }
        fn name(&self) -> &str {
            self.0
        }
        async fn get_manga_details(&self, manga: &SManga) -> Result<SManga, SourceError> {
            Ok(manga.clone())
        }
        async fn get_chapter_list(&self, _manga: &SManga) -> Result<Vec<SChapter>, SourceError> {
            Ok(Vec::new())
        }
        async fn get_page_list(&self, _chapter: &SChapter) -> Result<Vec<Page>, SourceError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl ResolvableSource for Titled {
        fn uri_type(&self, _uri: &str) -> MangaUriType {
            MangaUriType::Manga
        }
        async fn manga_from_uri(&self, _uri: &str) -> Result<Option<SManga>, SourceError> {
            Ok(Some(SManga::new("/t", self.0)))
        }
        async fn chapter_from_uri(&self, _uri: &str) -> Result<Option<SChapter>, SourceError> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn ec_dl_01_no_source_recognises_the_uri_so_there_are_no_results() {
        let source = Links::default();
        assert_eq!(resolve_deep_link(&[&source], "https://other.test/x").await, DeepLinkOutcome::NoResults);
        assert_eq!(*source.manga_lookups.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn ec_dl_02_a_chapter_link_with_no_chapter_keeps_the_manga() {
        let source = Links::default();
        let outcome = resolve_deep_link(&[&source], "https://x.test/c/1").await;
        assert_eq!(outcome, DeepLinkOutcome::Result { source_id: 9, manga: SManga::new("/m/1", "One"), chapter: None });
        assert_eq!((*source.manga_lookups.lock().unwrap(), *source.chapter_lookups.lock().unwrap()), (1, 1));
    }

    #[tokio::test]
    async fn ec_dl_02_a_manga_link_never_looks_up_a_chapter() {
        let source = Links::default();
        resolve_deep_link(&[&source], "https://x.test/m/1").await;
        assert_eq!(*source.chapter_lookups.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn ec_dl_03_a_failing_manga_lookup_is_no_results() {
        let source = Links { fail_manga: true, ..Default::default() };
        assert_eq!(resolve_deep_link(&[&source], "https://x.test/m/1").await, DeepLinkOutcome::NoResults);
    }

    #[tokio::test]
    async fn ec_dl_04_the_first_source_in_list_order_wins() {
        let first = Titled("First");
        let second = Titled("Second");
        let outcome = resolve_deep_link(&[&first, &second], "https://x.test/m/1").await;
        assert_eq!(outcome, DeepLinkOutcome::Result { source_id: 10, manga: SManga::new("/t", "First"), chapter: None });
    }
}
