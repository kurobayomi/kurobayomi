//! The one listing loader of both kinds (`NetworkToLocalManga` and `NetworkToLocalAnime`, T-41b-013): a source's popular,
//! latest or search page, each listed entry mapped to its library row through the kind's `FavoriteStore`. The source call
//! runs on the tokio runtime and the row lookups on the blocking pool. [`Catalogue`] holds the few calls that differ
//! between a manga and an anime source; everything else of Browse is shared.

use std::sync::Arc;

use kuro_domain::source::browse::BrowseQuery;
use kuro_source::anime::AnimeSource;
use kuro_source::filter_list::FilterList;
use kuro_source::{CatalogueSource, MangaSource, SourceError};

use super::entries::FavoriteStore;
use super::errors;
use super::listing::{ListItem, ListedPage, LoadFuture, PageLoader};

/// What a listing needs of one remote entry.
pub struct NetworkEntry {
    pub url: String,
    pub title: String,
    pub cover: Option<String>,
}

/// A runnable source of either kind.
#[derive(Clone)]
pub enum Catalogue {
    Manga(Arc<dyn CatalogueSource>),
    Anime(Arc<dyn AnimeSource>),
}

impl Catalogue {
    pub fn id(&self) -> i64 {
        match self {
            Self::Manga(source) => source.id(),
            Self::Anime(source) => source.id(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Manga(source) => source.name(),
            Self::Anime(source) => source.name(),
        }
    }

    pub fn supports_latest(&self) -> bool {
        match self {
            Self::Manga(source) => source.supports_latest(),
            Self::Anime(source) => source.supports_latest(),
        }
    }

    pub fn supports_popular(&self) -> bool {
        match self {
            Self::Manga(source) => source.supports_popular(),
            Self::Anime(source) => source.supports_popular(),
        }
    }

    /// The filters a fresh browse page starts from.
    pub fn filter_list(&self) -> FilterList {
        match self {
            Self::Manga(source) => source.filter_list(),
            Self::Anime(source) => source.filter_list(),
        }
    }

    /// The manga source, for the entry page, the reader and the downloads. `None` for an anime source.
    pub fn manga(&self) -> Option<Arc<dyn MangaSource>> {
        match self {
            Self::Manga(source) => Some(Arc::clone(source) as Arc<dyn MangaSource>),
            Self::Anime(_) => None,
        }
    }

    /// The anime source, for the library update and migration. `None` for a manga source.
    pub fn anime(&self) -> Option<Arc<dyn AnimeSource>> {
        match self {
            Self::Anime(source) => Some(Arc::clone(source)),
            Self::Manga(_) => None,
        }
    }

    /// Page `page` of `query`: each listed entry's url, title and cover, and whether a next page follows.
    pub async fn list(&self, query: BrowseQuery, filters: &FilterList, page: i32) -> Result<(Vec<NetworkEntry>, bool), SourceError> {
        match self {
            Self::Manga(source) => {
                let found = match query {
                    BrowseQuery::Popular => source.popular(page).await,
                    BrowseQuery::Latest => source.latest(page).await,
                    BrowseQuery::Search(text) => source.search(page, &text, filters).await,
                }?;
                let entries = found.mangas.into_iter().map(|manga| NetworkEntry { url: manga.url, title: manga.title, cover: manga.thumbnail_url }).collect();
                Ok((entries, found.has_next_page))
            }
            Self::Anime(source) => {
                let found = match query {
                    BrowseQuery::Popular => source.popular(page).await,
                    BrowseQuery::Latest => source.latest(page).await,
                    BrowseQuery::Search(text) => source.search(page, &text, filters).await,
                }?;
                let entries = found.animes.into_iter().map(|anime| NetworkEntry { url: anime.url, title: anime.title, cover: anime.thumbnail_url }).collect();
                Ok((entries, found.has_next_page))
            }
        }
    }
}

/// The listings of one source, each entry mapped to its library row in `store` (found by `(url, source)`, or inserted
/// as a non-favourite row). An existing row shows its stored title (E-BR-10).
pub struct SourceLoader {
    source: Catalogue,
    store: Arc<dyn FavoriteStore>,
}

impl SourceLoader {
    pub fn new(source: Catalogue, store: Arc<dyn FavoriteStore>) -> Self {
        Self { source, store }
    }
}

impl PageLoader for SourceLoader {
    fn load(&self, query: BrowseQuery, filters: FilterList, page: u32) -> LoadFuture {
        let (source, store) = (self.source.clone(), Arc::clone(&self.store));
        Box::pin(async move {
            let result = list_page(&source, store, query, filters, page).await;
            errors::note(source.id(), result.is_ok());
            result
        })
    }
}

/// One page of a listing. A failure is the sentence for the user; the technical text goes to the log. A panicking or
/// silent source is an error of this listing, not a dead task or an endless spinner.
async fn list_page(source: &Catalogue, store: Arc<dyn FavoriteStore>, query: BrowseQuery, filters: FilterList, page: u32) -> Result<ListedPage, String> {
    let page = i32::try_from(page).map_err(|error| error.to_string())?;
    let name = source.name().to_owned();
    let searching = matches!(query, BrowseQuery::Search(_));
    let (entries, has_next) = match errors::limited(source.list(query, &filters, page)).await {
        // a source without search (a single webcomic, say) has no results for a search, as in Aniyomi; not a failure
        Err(SourceError::Unsupported(_)) if searching => (Vec::new(), false),
        result => result.map_err(|error| errors::plain_message(&name, &error))?,
    };
    let source_id = source.id();
    tokio::task::spawn_blocking(move || -> Result<ListedPage, String> {
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            let local = store.local_entry(source_id, &entry.url, &entry.title, entry.cover.as_deref())?;
            items.push(ListItem { id: local.id, source_id, url: entry.url, title: local.title, favorite: local.favorite, cover: entry.cover });
        }
        Ok(ListedPage { items, has_next })
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|stored| stored)
    .map_err(|text| {
        tracing::warn!(source = name.as_str(), error = text.as_str(), "a listing could not be stored");
        errors::generic().to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::entries::fake::FakeStore;
    use crate::browse::fake_catalogue::{FakeAnime, FakeCatalogue, anime, anime_page};
    use kuro_source::filter::Filter;

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("runtime").block_on(future)
    }

    /// A store that knows `/b` (manga `/m/two`) as a favourite.
    fn store(favourite: &str) -> Arc<FakeStore> {
        let store = FakeStore::default();
        store.known.lock().expect("lock").push((7, favourite.to_owned(), true));
        Arc::new(store)
    }

    /// A source of each kind with id 7 whose popular page lists two entries and has a next page.
    fn sources() -> Vec<(Catalogue, &'static str)> {
        let anime_source = FakeAnime::new(7, vec![Ok(anime_page(vec![anime("/a", "A"), anime("/b", "B")], true))]);
        vec![(Catalogue::Manga(FakeCatalogue::new(7)), "/m/two"), (Catalogue::Anime(anime_source), "/b")]
    }

    #[test]
    fn each_listed_entry_becomes_its_library_row_in_source_order_for_either_kind() {
        for (source, favourite) in sources() {
            let store = store(favourite);
            let loader = SourceLoader::new(source.clone(), store.clone());
            let got = run(loader.load(BrowseQuery::Popular, FilterList::default(), 1)).expect("page");
            assert_eq!(
                got.items.iter().map(|i| (i.id, i.source_id, i.favorite)).collect::<Vec<_>>(),
                [(8, 7, false), (7, 7, true)],
                "an unknown url is a non-favourite row, a known one keeps its favourite flag"
            );
            assert!(got.has_next);
            assert_eq!(store.calls().len(), 2, "one lookup per listed entry");
        }
    }

    #[test]
    fn a_search_passes_its_text_and_its_filters_and_latest_calls_the_latest_listing() {
        let source = FakeAnime::new(3, vec![Ok(anime_page(vec![anime("/s", "S")], false)), Ok(anime_page(vec![], false))]);
        let loader = SourceLoader::new(Catalogue::Anime(source.clone()), Arc::new(FakeStore::default()));
        let filters = FilterList(vec![Filter::check_box("A"), Filter::check_box("B")]);
        let got = run(loader.load(BrowseQuery::Search("one".into()), filters, 2)).expect("page");
        assert!(!got.has_next);
        run(loader.load(BrowseQuery::Latest, FilterList::default(), 1)).expect("page");
        assert_eq!(source.calls(), ["search 2 one 2", "latest 1"]);
        let manga = FakeCatalogue::new(3);
        let loader = SourceLoader::new(Catalogue::Manga(manga.clone()), Arc::new(FakeStore::default()));
        run(loader.load(BrowseQuery::Search("one".into()), FilterList::default(), 1)).expect("page");
        assert_eq!(*manga.calls.lock().expect("calls"), ["search 1 one 0"]);
    }

    #[test]
    fn a_source_without_search_answers_a_search_with_no_results_and_still_fails_a_listing() {
        let search = FakeAnime::new(3, vec![Err(SourceError::Unsupported("search"))]);
        let loader = SourceLoader::new(Catalogue::Anime(search), Arc::new(FakeStore::default()));
        let got = run(loader.load(BrowseQuery::Search("one".into()), FilterList::default(), 1)).expect("no results, not an error");
        assert!(got.items.is_empty() && !got.has_next);
        let latest = FakeAnime::new(3, vec![Err(SourceError::Unsupported("latest"))]);
        let loader = SourceLoader::new(Catalogue::Anime(latest), Arc::new(FakeStore::default()));
        assert!(run(loader.load(BrowseQuery::Latest, FilterList::default(), 1)).is_err(), "only a search reads as empty");
    }

    #[test]
    fn a_source_error_reaches_the_page_as_a_plain_sentence_and_no_row_is_looked_up() {
        let manga = FakeCatalogue::new(3);
        manga.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let anime_source = FakeAnime::new(3, vec![Err(SourceError::Http(503))]);
        for source in [Catalogue::Manga(manga), Catalogue::Anime(anime_source)] {
            let store = Arc::new(FakeStore::default());
            let loader = SourceLoader::new(source, store.clone());
            let error = run(loader.load(BrowseQuery::Popular, FilterList::default(), 1)).expect_err("source error");
            assert!(!error.contains("503") && !error.contains("HTTP"), "the code stays in the log: {error}");
            assert!(error.contains("isn't working right now"), "{error}");
            assert!(store.calls().is_empty());
        }
    }

    #[test]
    fn a_failed_row_lookup_is_the_generic_sentence() {
        let store = FakeStore::default();
        store.fail.lock().expect("lock").push("local_entry");
        let loader = SourceLoader::new(Catalogue::Manga(FakeCatalogue::new(3)), Arc::new(store));
        let error = run(loader.load(BrowseQuery::Popular, FilterList::default(), 1)).expect_err("store error");
        assert_eq!(error, errors::generic());
    }

    #[test]
    fn a_catalogue_hands_out_the_source_of_its_kind_only() {
        let manga = Catalogue::Manga(FakeCatalogue::new(1));
        let anime_source = Catalogue::Anime(FakeAnime::new(2, Vec::new()));
        assert!(manga.manga().is_some() && manga.anime().is_none());
        assert!(anime_source.anime().is_some() && anime_source.manga().is_none());
        assert_eq!((manga.id(), anime_source.id()), (1, 2));
        assert!(manga.supports_latest() && anime_source.supports_latest());
        assert_eq!(manga.filter_list().len(), 1, "the fake manga source has one filter");
    }
}
