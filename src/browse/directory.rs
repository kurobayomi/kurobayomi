//! The installed sources of each kind, as the Browse pages read them (T-40b-060, T-40b-070, T-41a-027, T-41b-014). The
//! pages ask a [`SourceDirectory`] for the sources and their loaders. [`current`] answers for a kind with the sources the
//! repository load installed ([`install`]) and, for anime, the packages the extension host published, which are listed
//! but cannot run: their page is the missing-source page. With nothing installed every page still builds: global search
//! has no sources to ask, and the migration picker lists each library source as a stub.

use std::sync::{Arc, Mutex};

use kuro_domain::source::model::Source;
use kuro_source::SourcePref;
use kuro_source::filter_list::FilterList;

use super::entries::FavoriteStore;
use super::listing::{LinkFuture, PageLoader};
use super::loader::{Catalogue, SourceLoader};
use kuro_platform::kind::Kind;

/// One installed catalogue source and the package it came from (`""` when it has none, as the local source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectorySource {
    pub source: Source,
    pub package: String,
    /// The source's site, for Open in WebView.
    pub home_url: Option<String>,
}

/// What the browse pages need of the installed sources of one kind.
pub trait SourceDirectory: Send + Sync + 'static {
    /// The installed catalogue sources.
    fn sources(&self) -> Vec<DirectorySource>;
    /// The loader of a source's listings, with each entry mapped to its library row. `None` when it is not installed.
    fn loader(&self, source_id: i64) -> Option<Arc<dyn PageLoader>>;
    /// The filter list a source's search sheet starts from. `None` when it is not installed.
    fn filters(&self, source_id: i64) -> Option<FilterList>;
    /// The settings rows a configurable source declares (`ConfigurableSource::preferences`). `None` when the source is not
    /// installed or has no settings screen: the page then shows no settings item for it.
    fn preferences(&self, _source_id: i64) -> Option<Vec<SourcePref>> {
        None
    }
    /// Resolves a pasted link to the library row of the entry it names, asking each source's `uri_type` first (see
    /// `deep_link::resolve`). `None` when this directory has no source that can resolve links: global search then
    /// searches the catalogues with the text (T-20a-031).
    fn resolve_link(&self, _uri: &str) -> Option<LinkFuture> {
        None
    }
    /// The runnable source of an id, for the entry pages, the reader, the downloads, the library update and migration.
    /// `None` when it is not installed or this directory does not hand out its sources.
    fn catalogue(&self, _source_id: i64) -> Option<Catalogue> {
        None
    }
}

/// The library store a listing maps its entries into. `None` before the app data is open.
pub type StoreFor = Arc<dyn Fn() -> Option<Arc<dyn FavoriteStore>> + Send + Sync>;

/// The sources of one kind: each runnable source with its row, and the rows of sources that are listed but cannot run.
#[derive(Clone)]
pub struct Catalogues {
    runnable: Vec<(Catalogue, DirectorySource)>,
    listed: Vec<DirectorySource>,
    store: StoreFor,
}

impl Catalogues {
    /// The runnable sources of `kind`; a listing maps its entries into the app's library of that kind.
    pub fn new(kind: Kind, runnable: Vec<(Catalogue, DirectorySource)>) -> Self {
        Self::with_store(runnable, Arc::new(move || crate::data::is_installed().then(|| crate::updates::history_add::store(kind))))
    }

    /// The runnable sources over `store`. Each row states whether its source has a Latest listing.
    pub fn with_store(runnable: Vec<(Catalogue, DirectorySource)>, store: StoreFor) -> Self {
        let runnable = runnable
            .into_iter()
            .map(|(catalogue, mut row)| {
                row.source.supports_latest = catalogue.supports_latest();
                row.source.supports_popular = catalogue.supports_popular();
                (catalogue, row)
            })
            .collect();
        Self { runnable, listed: Vec::new(), store }
    }

    #[cfg(test)]
    /// Adds the rows of sources that are listed but cannot run here. They come first, as the extension host lists them.
    pub fn with_listed(mut self, listed: Vec<DirectorySource>) -> Self {
        self.listed = listed;
        self
    }

    fn find(&self, source_id: i64) -> Option<&Catalogue> {
        self.runnable.iter().find(|(_, row)| row.source.id == source_id).map(|(catalogue, _)| catalogue)
    }
}

impl SourceDirectory for Catalogues {
    fn sources(&self) -> Vec<DirectorySource> {
        self.listed.iter().cloned().chain(self.runnable.iter().map(|(_, row)| row.clone())).collect()
    }

    fn loader(&self, source_id: i64) -> Option<Arc<dyn PageLoader>> {
        let source = self.find(source_id)?.clone();
        Some(Arc::new(SourceLoader::new(source, (self.store)()?)))
    }

    fn filters(&self, source_id: i64) -> Option<FilterList> {
        self.find(source_id).map(Catalogue::filter_list)
    }

    fn catalogue(&self, source_id: i64) -> Option<Catalogue> {
        self.find(source_id).cloned()
    }
}

static INSTALLED: Mutex<Vec<(Kind, Catalogues)>> = Mutex::new(Vec::new());

/// Installs the sources of `kind` the pages read. A second call for the same kind replaces the first.
pub fn install(kind: Kind, catalogues: Catalogues) {
    remember_stubs(kind, &catalogues);
    let mut installed = INSTALLED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    installed.retain(|(of, _)| *of != kind);
    installed.push((kind, catalogues));
}

/// The address of the site of a loaded source of `kind`, when the source has one (the repository sources do).
pub fn home_url(kind: Kind, source_id: i64) -> Option<String> {
    current(kind).sources().into_iter().find(|entry| entry.source.id == source_id).and_then(|entry| entry.home_url)
}

/// The sources whose name or language changed since the last load, as (kind, old key, new key), for the download folders.
static RENAMED: Mutex<Vec<(Kind, kuro_downloads::unit::SourceKey, kuro_downloads::unit::SourceKey)>> = Mutex::new(Vec::new());

/// Takes the renames the last loads noticed, so the download folders follow them.
pub fn take_renames() -> Vec<(Kind, kuro_downloads::unit::SourceKey, kuro_downloads::unit::SourceKey)> {
    std::mem::take(&mut *RENAMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// Records every loaded source (id, language, name) in the stub table, as Aniyomi's sources table does, so a backup made before
/// the sources load and the restore screen can still name them.
fn remember_stubs(kind: Kind, catalogues: &Catalogues) {
    if !crate::data::is_installed() {
        return;
    }
    let stubs = kuro_data::source::StubSourceRepo::new(crate::data::app().db.clone(), kind);
    for entry in catalogues.sources() {
        // a source that changed its name or language has its download folder renamed after this load (`take_renames`)
        if let Some(old) = stubs.get(entry.source.id).ok().flatten().filter(|old| !old.is_invalid() && (old.name != entry.source.name || old.lang != entry.source.lang)) {
            let key = |name: &str, lang: &str, availability| kuro_downloads::unit::SourceKey { id: entry.source.id, name: name.to_owned(), lang: lang.to_owned(), unmetered: false, availability };
            let renamed = (kind, key(&old.name, &old.lang, kuro_downloads::unit::Availability::Stub), key(&entry.source.name, &entry.source.lang, kuro_downloads::unit::Availability::Live));
            RENAMED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(renamed);
        }
        if let Err(error) = stubs.upsert(entry.source.id, &entry.source.lang, &entry.source.name) {
            tracing::warn!(source = entry.source.id, %error, "a source could not be recorded for the backup");
        }
    }
}

/// The directory the pages of `kind` read: the installed sources (none before the first load), the local source, and for
/// anime the packages of the extension host. It is a snapshot taken when a page is built.
pub fn current(kind: Kind) -> Arc<dyn SourceDirectory> {
    let installed = INSTALLED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().find(|(of, _)| *of == kind).map(|(_, catalogues)| catalogues.clone());
    let mut catalogues = installed.unwrap_or_else(|| Catalogues::new(kind, Vec::new()));
    catalogues.runnable.push(local(kind));
    Arc::new(catalogues)
}

/// The local source of `kind` over its folder (`Local/Manga` or `Local/Anime` under the storage folder), with its row. It
/// needs no repository, so it is always listed; it is made at each lookup, from any thread, so a new storage folder applies
/// at once. `None` before the Local folders are known (`local_flow::load_roots`).
fn local(kind: Kind) -> (Catalogue, DirectorySource) {
    use kuro_source::local::{LOCAL_ANIME_NAME, LOCAL_MANGA_NAME};
    // listed even without a Local folder, so backups know source 0; it then reads nothing and empties no title
    let root = crate::local_flow::local_root(kind);
    let (catalogue, name) = match kind {
        Kind::Manga => (Catalogue::Manga(Arc::new(kuro_local::LocalMangaSource::new(root))), LOCAL_MANGA_NAME),
        Kind::Anime => (Catalogue::Anime(Arc::new(kuro_local::LocalAnimeSource::new(root))), LOCAL_ANIME_NAME),
    };
    let mut source = Source::new(kuro_local::LOCAL_SOURCE_ID, name, "other");
    source.supports_latest = catalogue.supports_latest();
    source.supports_popular = catalogue.supports_popular();
    (catalogue, DirectorySource { source, package: "local".to_owned(), home_url: None })
}

/// A source as the migration picker shows it: the installed one, or a stub with no name or language when the directory
/// does not know the id (`getOrStub`, T-40b-070). `directory` is `None` when the caller has none.
pub fn source_or_stub(directory: Option<&dyn SourceDirectory>, id: i64) -> Source {
    directory
        .and_then(|directory| directory.sources().into_iter().find(|entry| entry.source.id == id))
        .map(|entry| entry.source)
        .unwrap_or_else(|| Source { is_stub: true, ..Source::new(id, "", "") })
}

#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use crate::browse::entries::fake::FakeStore;
    use crate::browse::fake_catalogue::{FakeAnime, FakeCatalogue, anime, anime_page};
    use crate::browse::listing::{ListedPage, LoadFuture};
    use kuro_domain::source::browse::BrowseQuery;

    /// A directory over a list of sources whose search answers with the scripted result of the source id.
    pub struct FakeDirectory {
        pub entries: Vec<DirectorySource>,
        pub answers: Arc<dyn Fn(i64, &BrowseQuery) -> Result<Vec<super::super::listing::ListItem>, String> + Send + Sync>,
    }

    struct FakeLoader {
        id: i64,
        answers: Arc<dyn Fn(i64, &BrowseQuery) -> Result<Vec<super::super::listing::ListItem>, String> + Send + Sync>,
    }

    impl PageLoader for FakeLoader {
        fn load(&self, query: BrowseQuery, _filters: FilterList, _page: u32) -> LoadFuture {
            let answer = (self.answers)(self.id, &query);
            Box::pin(async move { answer.map(|items| ListedPage { items, has_next: false }) })
        }
    }

    impl SourceDirectory for FakeDirectory {
        fn sources(&self) -> Vec<DirectorySource> {
            self.entries.clone()
        }
        fn loader(&self, source_id: i64) -> Option<Arc<dyn PageLoader>> {
            self.entries
                .iter()
                .any(|entry| entry.source.id == source_id)
                .then(|| Arc::new(FakeLoader { id: source_id, answers: Arc::clone(&self.answers) }) as Arc<dyn PageLoader>)
        }
        fn filters(&self, source_id: i64) -> Option<FilterList> {
            self.entries.iter().any(|entry| entry.source.id == source_id).then(FilterList::default)
        }
    }

    pub fn entry(id: i64, name: &str, lang: &str, package: &str) -> DirectorySource {
        DirectorySource { source: Source::new(id, name, lang), package: package.to_owned(), home_url: None }
    }

    /// The source 42 of `kind`, runnable over a fake store (the anime one answers four listings), and the source 43,
    /// listed but not runnable.
    pub fn catalogues(kind: Kind) -> Catalogues {
        let source = match kind {
            Kind::Manga => Catalogue::Manga(FakeCatalogue::new(42)),
            Kind::Anime => Catalogue::Anime(FakeAnime::new(42, (0..4).map(|_| Ok(anime_page(vec![anime("/a", "A")], false))).collect())),
        };
        let mut row = entry(42, "Src42", "en", "repo.alpha");
        row.home_url = Some("https://src.example".into());
        let store: StoreFor = Arc::new(|| Some(Arc::new(FakeStore::default()) as Arc<dyn FavoriteStore>));
        Catalogues::with_store(vec![(source, row)], store).with_listed(vec![entry(43, "Src43", "en", "eu.example.alpha")])
    }

    #[test]
    fn an_unknown_source_is_a_stub_and_a_known_one_is_itself() {
        let directory = FakeDirectory { entries: vec![entry(1, "Alpha", "en", "ext.a")], answers: Arc::new(|_, _| Ok(Vec::new())) };
        let known = source_or_stub(Some(&directory), 1);
        assert!(!known.is_stub, "an installed source is not a stub");
        assert_eq!(known.name, "Alpha");
        let unknown = source_or_stub(Some(&directory), 9);
        assert!(unknown.is_stub, "an id the directory does not know is a stub (T-40b-070)");
        assert_eq!(unknown.id, 9);
        assert!(source_or_stub(None, 1).is_stub, "with no directory every source is a stub");
    }

    #[test]
    fn the_runnable_source_of_either_kind_has_a_loader_filters_and_its_latest_flag_and_a_listed_one_has_none() {
        for kind in [Kind::Manga, Kind::Anime] {
            let directory = catalogues(kind);
            assert_eq!(directory.sources().iter().map(|row| row.source.id).collect::<Vec<_>>(), [43, 42], "{kind:?}: the listed rows come first");
            assert!(directory.sources()[1].source.supports_latest, "{kind:?}: the row states the source's Latest listing");
            assert!(directory.loader(42).is_some() && directory.filters(42).is_some() && directory.catalogue(42).is_some(), "{kind:?}");
            assert!(directory.loader(43).is_none() && directory.filters(43).is_none() && directory.catalogue(43).is_none(), "{kind:?}: listed only");
            assert!(directory.preferences(42).is_none(), "{kind:?}: no source declares settings rows here");
        }
    }

    #[test]
    fn without_a_library_store_a_source_has_no_loader() {
        let directory = Catalogues::with_store(vec![(Catalogue::Manga(FakeCatalogue::new(1)), entry(1, "A", "en", ""))], Arc::new(|| None));
        assert!(directory.loader(1).is_none() && directory.filters(1).is_some());
    }

    #[test]
    fn the_installed_sources_of_a_kind_are_read_back_and_replaced() {
        let one = Catalogues::with_store(vec![(Catalogue::Manga(FakeCatalogue::new(1)), entry(1, "A", "en", ""))], Arc::new(|| None));
        install(Kind::Manga, one);
        // the local source is always listed after the installed ones
        assert_eq!(current(Kind::Manga).sources().len(), 2);
        let two = Catalogues::with_store(
            vec![(Catalogue::Manga(FakeCatalogue::new(1)), entry(1, "A", "en", "")), (Catalogue::Manga(FakeCatalogue::new(2)), entry(2, "B", "en", ""))],
            Arc::new(|| None),
        );
        install(Kind::Manga, two);
        let manga = current(Kind::Manga);
        assert_eq!(manga.sources().len(), 3);
        assert!(manga.catalogue(kuro_local::LOCAL_SOURCE_ID).is_some());
        assert!(manga.catalogue(2).and_then(|source| source.manga()).is_some() && manga.catalogue(3).is_none());
        INSTALLED.lock().expect("lock").retain(|(kind, _)| *kind != Kind::Manga);
        let left: Vec<i64> = current(Kind::Manga).sources().iter().map(|s| s.source.id).collect();
        assert_eq!(left, [kuro_local::LOCAL_SOURCE_ID], "nothing installed lists only the local source");
    }
}
