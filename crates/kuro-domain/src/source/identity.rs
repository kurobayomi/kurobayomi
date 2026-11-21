//! `SourceIdentity`: the facts the registry needs about one live source, adapted from the source API (areas 15 and 16;
//! T-17a-012). The registry reads an installed source through [`InstalledSource`], so this adapter builds that view from
//! the live trait objects. `supports_latest` comes from the live source (`MangaSourceRepositoryImpl.kt:25-29`), and
//! `is_stub` is never set here: a live source is not a stub.

use kuro_source::anime::AnimeSource;
use kuro_source::source::{CatalogueSource, MangaSource};

use crate::source::model::{Id, Source};
use crate::source::registry::InstalledSource;

/// The fixed view of one installed source. Build it when the extension loader publishes the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    id: Id,
    source: Source,
    http: bool,
    catalogue: bool,
    configurable: bool,
}

impl SourceIdentity {
    /// A catalogue manga source. `supports_latest` is the live source's own answer.
    pub fn of_catalogue(source: &dyn CatalogueSource, http: bool) -> Self {
        Self::new(source, source.supports_latest(), http, true)
    }

    /// A manga source that is not a catalogue: it never offers a latest list.
    pub fn of_manga(source: &dyn MangaSource, http: bool) -> Self {
        Self::new(source, false, http, false)
    }

    /// A live anime source. Every live anime source counts as a catalogue (behaviour 1.4).
    pub fn of_anime(source: &dyn AnimeSource, http: bool) -> Self {
        Self {
            id: source.id(),
            source: Source {
                supports_latest: source.supports_latest(),
                supports_popular: source.supports_popular(),
                ..Source::new(source.id(), source.name(), source.lang())
            },
            http,
            catalogue: true,
            configurable: source.is_configurable(),
        }
    }

    fn new(source: &dyn MangaSource, supports_latest: bool, http: bool, catalogue: bool) -> Self {
        Self {
            id: source.id(),
            source: Source { supports_latest, ..Source::new(source.id(), source.name(), source.lang()) },
            http,
            catalogue,
            configurable: source.is_configurable(),
        }
    }
}

impl InstalledSource for SourceIdentity {
    fn id(&self) -> Id {
        self.id
    }
    fn is_http(&self) -> bool {
        self.http
    }
    fn is_configurable(&self) -> bool {
        self.configurable
    }
    fn is_catalogue(&self) -> bool {
        self.catalogue
    }
    fn source(&self) -> Source {
        self.source.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::fakes::LiveCatalogue;

    #[test]
    fn a_live_catalogue_source_with_supports_latest_yields_supports_latest() {
        let live = LiveCatalogue::new(11, "Live", "en", true);
        let identity = SourceIdentity::of_catalogue(&live, true);
        let source = identity.source();
        assert!(source.supports_latest, "the live supportsLatest is copied");
        assert!(!source.is_stub, "a live source is never a stub");
        assert_eq!((identity.id(), identity.is_http(), identity.is_catalogue()), (11, true, true));
    }

    #[test]
    fn a_live_source_without_latest_yields_false() {
        let live = LiveCatalogue::new(12, "Plain", "ja", false);
        assert!(!SourceIdentity::of_catalogue(&live, false).source().supports_latest);
    }

    #[test]
    fn a_plain_manga_source_never_offers_latest_and_is_not_a_catalogue() {
        let live = LiveCatalogue::new(13, "Plain", "fr", true);
        let identity = SourceIdentity::of_manga(&live, true);
        assert!(!identity.source().supports_latest);
        assert!(!identity.is_catalogue());
        assert_eq!(identity.source().name, "Plain");
    }
}
