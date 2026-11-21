//! The `AnimeSource` trait (`AnimeSource.kt`, `AnimeCatalogueSource.kt`; T-16a-003, T-16a-004, T-16a-005).
//!
//! Sources are used as `Arc<dyn AnimeSource>`, so the trait is `async_trait`. A method the Kotlin base class left
//! abstract is required here, and a method it left unimplemented returns `Unsupported`; no default calls itself
//! (E-02).

use std::future::Future;

use async_trait::async_trait;
use bytes::Bytes;

use super::{
    AnimeRelation, AnimesPage, Hoster, SAnime, SAnimeEpisodeUpdate, SAnimeSeasonUpdate, SEpisode, ThumbnailInfo, Video,
};
use crate::error::SourceError;
use crate::filter_list::FilterList;
use crate::identity::display_name;
use crate::prefs::SourcePref;

#[async_trait]
pub trait AnimeSource: Send + Sync {
    /// Stable identity (see [`crate::id::generate_id`]).
    fn id(&self) -> i64;
    fn name(&self) -> &str;
    /// ISO 639-1 lowercase, or `all` or `other`. Empty when the source states none.
    fn lang(&self) -> &str {
        ""
    }
    /// Whether a "latest" listing exists. Every source states it; there is no default (`AnimeSource.kt`).
    fn supports_latest(&self) -> bool;
    /// Whether a "popular" listing exists. Most sources have one; a site that only lists its newest titles says
    /// `false`, and Browse then opens on Latest and offers no Popular.
    fn supports_popular(&self) -> bool {
        true
    }
    /// `false` means the details call is never made, so a source without details cannot fail a combined update (E-27).
    fn supports_details(&self) -> bool {
        true
    }
    /// `false` means the episode call is never made (E-28).
    fn supports_episode_list(&self) -> bool {
        true
    }
    /// `false` means the season call is never made.
    fn supports_season_list(&self) -> bool {
        true
    }
    /// The filters for a fresh browse screen.
    fn filter_list(&self) -> FilterList {
        FilterList::default()
    }
    /// `"{name} ({LANG})"`, the `AnimeHttpSource.toString()` text.
    fn display_name(&self) -> String {
        display_name(self.name(), self.lang())
    }

    async fn popular(&self, page: i32) -> Result<AnimesPage, SourceError>;
    async fn latest(&self, page: i32) -> Result<AnimesPage, SourceError>;
    async fn search(&self, page: i32, query: &str, filters: &FilterList) -> Result<AnimesPage, SourceError>;

    /// The details of one anime. Returns the updated copy.
    async fn anime_details(&self, _anime: &SAnime) -> Result<SAnime, SourceError> {
        Err(SourceError::Unsupported("anime_details"))
    }
    /// The poster of one anime, for a listing that gave only a `kb-cover:<anime url>` picture (see `kuro_images::COVER_MARKER`). `None` when the source has none.
    async fn cover_url(&self, _anime: &SAnime) -> Result<Option<String>, SourceError> {
        Ok(None)
    }
    /// The episodes of one anime, in source order.
    async fn episode_list(&self, _anime: &SAnime) -> Result<Vec<SEpisode>, SourceError> {
        Err(SourceError::Unsupported("episode_list"))
    }
    /// The seasons of one anime, in source order.
    async fn season_list(&self, _anime: &SAnime) -> Result<Vec<SAnime>, SourceError> {
        Err(SourceError::Unsupported("season_list"))
    }

    /// The combined episode update (`AnimeCatalogueSource.getAnimeEpisodeUpdate`, T-16a-004). With both flags off it
    /// makes no source call. The details and the episode calls run concurrently, and a failure in either fails the
    /// whole call (E-26), so the caller never applies half an update.
    async fn anime_episode_update(
        &self,
        anime: SAnime,
        episodes: Vec<SEpisode>,
        fetch_details: bool,
        fetch_episodes: bool,
    ) -> Result<SAnimeEpisodeUpdate, SourceError> {
        let details_on = fetch_details && self.supports_details();
        let episodes_on = fetch_episodes && self.supports_episode_list();
        let anime_ref = &anime;
        let details =
            async move { if details_on { self.anime_details(anime_ref).await } else { Ok(anime_ref.clone()) } };
        let list = async move { if episodes_on { self.episode_list(anime_ref).await } else { Ok(episodes) } };
        let (anime, episodes) = futures::future::try_join(details, list).await?;
        Ok(SAnimeEpisodeUpdate { anime, episodes })
    }

    /// The combined season update, the season twin of [`AnimeSource::anime_episode_update`].
    async fn anime_season_update(
        &self,
        anime: SAnime,
        seasons: Vec<SAnime>,
        fetch_details: bool,
        fetch_seasons: bool,
    ) -> Result<SAnimeSeasonUpdate, SourceError> {
        let details_on = fetch_details && self.supports_details();
        let seasons_on = fetch_seasons && self.supports_season_list();
        let anime_ref = &anime;
        let details =
            async move { if details_on { self.anime_details(anime_ref).await } else { Ok(anime_ref.clone()) } };
        let list = async move { if seasons_on { self.season_list(anime_ref).await } else { Ok(seasons) } };
        let (anime, seasons) = futures::future::try_join(details, list).await?;
        Ok(SAnimeSeasonUpdate { anime, seasons })
    }

    /// `true` when the source lists hosters for an episode (`hosters`). Explicit, so no reflection is needed (E-01).
    fn provides_hosters(&self) -> bool;
    /// The hosters of an episode. Only called when [`AnimeSource::provides_hosters`] is `true`.
    async fn hosters(&self, _episode: &SEpisode) -> Result<Vec<Hoster>, SourceError> {
        Err(SourceError::Unsupported("hosters"))
    }
    /// The videos of one hoster that has no `video_list` yet. Its default is `Unsupported`, never a call back into
    /// itself (E-02).
    async fn videos_for_hoster(&self, _hoster: &Hoster) -> Result<Vec<Video>, SourceError> {
        Err(SourceError::Unsupported("videos_for_hoster"))
    }
    /// The legacy path: the videos of an episode without a hoster level.
    async fn videos_for_episode(&self, _episode: &SEpisode) -> Result<Vec<Video>, SourceError> {
        Err(SourceError::Unsupported("videos_for_episode"))
    }

    /// The source's order of hosters. The first hoster wins ties (behaviour 7). The default keeps the order.
    fn sort_hosters(&self, hosters: Vec<Hoster>) -> Vec<Hoster> {
        hosters
    }
    /// The source's order of videos, applied before selection in every path. No global quality sort exists (behaviour
    /// 7). The default keeps the order.
    fn sort_videos(&self, videos: Vec<Video>) -> Vec<Video> {
        videos
    }

    /// Turns a page URL into a playable stream (lazy resolution). The default returns the video unchanged.
    async fn resolve_video(&self, video: Video) -> Result<Option<Video>, SourceError> {
        Ok(Some(video))
    }

    /// `true` for a source that lists related anime (`supportsRelatedAnime`). Default false.
    fn supports_related_anime(&self) -> bool {
        false
    }
    /// The related anime groups of one anime, in source order. Callers reach it through [`related_animes`], which
    /// checks [`AnimeSource::supports_related_anime`] first.
    async fn related_anime(&self, _anime: &SAnime) -> Result<Vec<AnimeRelation>, SourceError> {
        Err(SourceError::Unsupported("related_anime"))
    }
    /// The `UnmeteredSource` marker (T-16a-037). `true` for a source on the user's own machine or network: it is left
    /// out of the queue warnings and fetches its seasons in parallel. Trait objects cannot be asked whether they
    /// implement a marker trait, so the marker is this method, as [`crate::MangaSource::is_unmetered`] is.
    fn is_unmetered(&self) -> bool {
        false
    }
    /// `true` when the source has a settings screen: the Kotlin `is ConfigurableAnimeSource` check, which a trait object
    /// cannot ask. A source that returns `true` must also implement [`ConfigurableSource`].
    fn is_configurable(&self) -> bool {
        false
    }

    /// The seek-preview sheet of a video. `None` when the source has none.
    async fn video_thumbnails(&self, _video: &Video) -> Result<Option<ThumbnailInfo>, SourceError> {
        Ok(None)
    }
    /// The bytes of one image tile. `None` when the tile cannot be had (E-09).
    async fn image_tile(&self, _url: &str) -> Result<Option<Bytes>, SourceError> {
        Ok(None)
    }
}

/// A source with a settings screen (`ConfigurableAnimeSource`, T-16a-035). Its rows are read when the screen opens. A
/// duplicate key in the list is rejected by [`crate::prefs::check_descriptors`].
pub trait ConfigurableSource: AnimeSource {
    /// The settings rows, as plain data. The UI renders them.
    fn preferences(&self) -> Vec<SourcePref>;
}

/// Fetches the seasons of one anime, the per-season loop of `AnimeScreenModel` (T-16a-037). An unmetered source runs
/// every fetch at once. Any other source runs them in order and asks `cancelled` before each one, so a cancelled call
/// stops at the next season. The first failure ends the call. The branch sits here rather than in
/// [`AnimeSource::anime_season_update`], which makes one season-list call and has no per-season step.
pub async fn fetch_each_season<F, Fut>(
    source: &dyn AnimeSource,
    seasons: Vec<SAnime>,
    cancelled: impl Fn() -> bool,
    fetch: F,
) -> Result<(), SourceError>
where
    F: Fn(SAnime) -> Fut,
    Fut: Future<Output = Result<(), SourceError>>,
{
    if source.is_unmetered() {
        futures::future::try_join_all(seasons.into_iter().map(&fetch)).await?;
        return Ok(());
    }
    for season in seasons {
        if cancelled() {
            return Err(SourceError::Cancelled);
        }
        fetch(season).await?;
    }
    Ok(())
}

/// The related anime of one anime (T-16a-046). A source without related anime is never asked: the call is
/// `Unsupported` and no source method runs.
pub async fn related_animes(source: &dyn AnimeSource, anime: &SAnime) -> Result<Vec<AnimeRelation>, SourceError> {
    if !source.supports_related_anime() {
        return Err(SourceError::Unsupported("related_anime"));
    }
    source.related_anime(anime).await
}

/// What a deep-link URI names on an anime source (`ResolvableAnimeSource.kt`; T-16a-036).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UriType {
    Anime,
    Episode,
    Unknown,
}

/// A source that turns a URI of its own site into an anime or an episode.
#[async_trait]
pub trait ResolvableAnimeSource: AnimeSource {
    fn uri_type(&self, uri: &str) -> UriType;
    async fn anime_for_uri(&self, uri: &str) -> Result<Option<SAnime>, SourceError>;
    async fn episode_for_uri(&self, uri: &str) -> Result<Option<SEpisode>, SourceError>;
}

/// The answer to an anime deep link.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedAnime {
    Anime(Option<SAnime>),
    Episode(Option<SEpisode>),
    /// Not a link of this source. No lookup ran.
    Unknown,
}

/// Runs the one lookup that matches the URI type, so a source is asked only for the kind it recognises.
pub async fn resolve_anime_uri(source: &dyn ResolvableAnimeSource, uri: &str) -> Result<ResolvedAnime, SourceError> {
    match source.uri_type(uri) {
        UriType::Anime => Ok(ResolvedAnime::Anime(source.anime_for_uri(uri).await?)),
        UriType::Episode => Ok(ResolvedAnime::Episode(source.episode_for_uri(uri).await?)),
        UriType::Unknown => Ok(ResolvedAnime::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::anime::hoster::{NO_HOSTER_LIST, http_hosters};
    use crate::anime::model::FetchType;
    use crate::prefs::{PrefDescriptorError, check_descriptors};

    /// A source that counts its calls. `fail_details` makes the details call fail; `barrier` makes the details and
    /// episode calls wait for each other, so they only finish when they run at the same time. `unmetered` and
    /// `configurable` set the two markers.
    #[derive(Default)]
    struct Mock {
        details_calls: AtomicUsize,
        list_calls: AtomicUsize,
        season_calls: AtomicUsize,
        fail_details: bool,
        details_supported: bool,
        barrier: Option<tokio::sync::Barrier>,
        provides: bool,
        unmetered: bool,
        configurable: bool,
    }

    impl Mock {
        fn new() -> Self {
            Self { details_supported: true, ..Default::default() }
        }
    }

    #[async_trait]
    impl AnimeSource for Mock {
        fn id(&self) -> i64 {
            1
        }
        fn name(&self) -> &str {
            "Mock"
        }
        fn lang(&self) -> &str {
            "en"
        }
        fn supports_latest(&self) -> bool {
            false
        }
        fn supports_details(&self) -> bool {
            self.details_supported
        }
        fn is_unmetered(&self) -> bool {
            self.unmetered
        }
        fn is_configurable(&self) -> bool {
            self.configurable
        }
        async fn popular(&self, _page: i32) -> Result<AnimesPage, SourceError> {
            Ok(AnimesPage { animes: vec![SAnime { title: "P".into(), ..Default::default() }], has_next_page: true })
        }
        async fn latest(&self, _page: i32) -> Result<AnimesPage, SourceError> {
            Ok(AnimesPage { animes: Vec::new(), has_next_page: false })
        }
        async fn search(&self, _page: i32, _query: &str, _filters: &FilterList) -> Result<AnimesPage, SourceError> {
            Ok(AnimesPage { animes: Vec::new(), has_next_page: false })
        }
        async fn anime_details(&self, anime: &SAnime) -> Result<SAnime, SourceError> {
            self.details_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(barrier) = &self.barrier {
                barrier.wait().await;
            }
            if self.fail_details {
                return Err(SourceError::Parse("details".into()));
            }
            Ok(SAnime { title: format!("{} (detailed)", anime.title), initialized: true, ..anime.clone() })
        }
        async fn season_list(&self, _anime: &SAnime) -> Result<Vec<SAnime>, SourceError> {
            self.season_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![SAnime { title: "S1".into(), ..Default::default() }])
        }
        async fn episode_list(&self, _anime: &SAnime) -> Result<Vec<SEpisode>, SourceError> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(barrier) = &self.barrier {
                barrier.wait().await;
            }
            Ok(vec![SEpisode { name: "E1".into(), ..Default::default() }])
        }
        fn provides_hosters(&self) -> bool {
            self.provides
        }
        async fn videos_for_episode(&self, _episode: &SEpisode) -> Result<Vec<Video>, SourceError> {
            Ok(vec![Video::new("https://v.test/1.m3u8", "A"), Video::new("https://v.test/2.m3u8", "B")])
        }
    }

    fn anime() -> SAnime {
        SAnime { title: "Show".into(), ..Default::default() }
    }

    #[tokio::test]
    async fn a_mock_with_only_the_required_methods_compiles_and_popular_returns_its_page() {
        let source: Arc<dyn AnimeSource> = Arc::new(Mock::new());
        let page = source.popular(1).await.unwrap();
        assert_eq!(page.animes[0].title, "P");
        assert!(page.has_next_page);
        assert_eq!(source.display_name(), "Mock (EN)");
        assert!(source.filter_list().is_empty());
    }

    #[tokio::test]
    async fn a_season_entry_never_calls_the_episode_path_and_an_episode_entry_never_the_season_path() {
        for (fetch_type, episode_calls, season_calls) in [(FetchType::Seasons, 0, 1), (FetchType::Episodes, 1, 0)] {
            let mock = Mock::new();
            mock.anime_episode_update(anime(), Vec::new(), false, fetch_type.fetches_episodes()).await.unwrap();
            mock.anime_season_update(anime(), Vec::new(), false, fetch_type.fetches_seasons()).await.unwrap();
            assert_eq!(mock.list_calls.load(Ordering::SeqCst), episode_calls, "{fetch_type:?} episodes");
            assert_eq!(mock.season_calls.load(Ordering::SeqCst), season_calls, "{fetch_type:?} seasons");
        }
    }

    #[tokio::test]
    async fn the_default_resolution_and_sort_keep_the_input_unchanged() {
        let mock = Mock::new();
        let video = Video::new("https://v.test/x.m3u8", "A");
        assert_eq!(mock.resolve_video(video.clone()).await.unwrap(), Some(video));
        let hosters: Vec<Hoster> =
            ["a", "b", "c"].iter().map(|n| Hoster { hoster_name: (*n).into(), ..Default::default() }).collect();
        let names: Vec<String> = mock.sort_hosters(hosters.clone()).into_iter().map(|h| h.hoster_name).collect();
        assert_eq!(names, ["a", "b", "c"]);
        let videos = vec![Video::new("1", "one"), Video::new("2", "two"), Video::new("3", "three")];
        assert_eq!(mock.sort_videos(videos.clone()), videos);
    }

    #[tokio::test]
    async fn an_episode_update_with_both_flags_off_makes_no_source_call() {
        let mock = Mock::new();
        let episodes = vec![SEpisode { name: "kept".into(), ..Default::default() }];
        let update = mock.anime_episode_update(anime(), episodes.clone(), false, false).await.unwrap();
        assert_eq!(update.anime, anime());
        assert_eq!(update.episodes, episodes);
        assert_eq!(mock.details_calls.load(Ordering::SeqCst) + mock.list_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_details_and_episode_calls_run_at_the_same_time() {
        let mock = Mock { barrier: Some(tokio::sync::Barrier::new(2)), ..Mock::new() };
        let run = mock.anime_episode_update(anime(), Vec::new(), true, true);
        let update = tokio::time::timeout(Duration::from_millis(900), run)
            .await
            .expect("both calls must be in flight together")
            .unwrap();
        assert_eq!(update.anime.title, "Show (detailed)");
        assert_eq!(update.episodes[0].name, "E1");
    }

    #[tokio::test]
    async fn a_failure_in_either_child_fails_the_whole_update() {
        let mock = Mock { fail_details: true, ..Mock::new() };
        let err = mock.anime_episode_update(anime(), Vec::new(), true, true).await.unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)));
    }

    #[tokio::test]
    async fn an_unsupported_details_capability_is_skipped_even_when_asked() {
        let mock = Mock { details_supported: false, ..Mock::new() };
        let update = mock.anime_episode_update(anime(), Vec::new(), true, true).await.unwrap();
        assert_eq!(update.anime, anime());
        assert_eq!(update.episodes[0].name, "E1");
        assert_eq!(mock.details_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_source_without_details_falls_back_to_unsupported_and_is_not_called_when_disabled() {
        struct Bare;
        #[async_trait]
        impl AnimeSource for Bare {
            fn id(&self) -> i64 {
                2
            }
            fn name(&self) -> &str {
                "Bare"
            }
            fn supports_latest(&self) -> bool {
                false
            }
            async fn popular(&self, _: i32) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("popular"))
            }
            async fn latest(&self, _: i32) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("latest"))
            }
            async fn search(&self, _: i32, _: &str, _: &FilterList) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("search"))
            }
            fn provides_hosters(&self) -> bool {
                false
            }
        }
        let bare = Bare;
        assert!(matches!(bare.anime_details(&anime()).await, Err(SourceError::Unsupported("anime_details"))));
        let update = bare.anime_episode_update(anime(), Vec::new(), false, false).await.unwrap();
        assert_eq!(update.anime, anime());
    }

    #[tokio::test]
    async fn a_source_without_hosters_yields_one_no_hoster_list_with_its_videos_in_order() {
        let mock = Mock::new();
        let hosters = http_hosters(&mock, &SEpisode::default()).await.unwrap();
        assert_eq!(hosters.len(), 1);
        assert_eq!(hosters[0].hoster_name, NO_HOSTER_LIST);
        let urls: Vec<&str> = hosters[0].video_list.as_ref().unwrap().iter().map(|v| v.video_title.as_str()).collect();
        assert_eq!(urls, ["A", "B"]);
    }

    #[tokio::test]
    async fn a_source_that_declares_hosters_is_routed_to_them() {
        struct WithHosters;
        #[async_trait]
        impl AnimeSource for WithHosters {
            fn id(&self) -> i64 {
                3
            }
            fn name(&self) -> &str {
                "Hosted"
            }
            fn supports_latest(&self) -> bool {
                false
            }
            async fn popular(&self, _: i32) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("popular"))
            }
            async fn latest(&self, _: i32) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("latest"))
            }
            async fn search(&self, _: i32, _: &str, _: &FilterList) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("search"))
            }
            fn provides_hosters(&self) -> bool {
                true
            }
            async fn hosters(&self, _: &SEpisode) -> Result<Vec<Hoster>, SourceError> {
                Ok(vec![Hoster {
                    hoster_url: "h".into(),
                    hoster_name: "Server".into(),
                    lazy: true,
                    ..Default::default()
                }])
            }
        }
        let hosters = http_hosters(&WithHosters, &SEpisode::default()).await.unwrap();
        assert_eq!(hosters[0].hoster_name, "Server");
        assert!(hosters[0].lazy);
    }

    impl ConfigurableSource for Mock {
        fn preferences(&self) -> Vec<SourcePref> {
            vec![
                SourcePref::Switch { key: "show_hd".into(), title: "Show HD".into(), summary: None, default: true },
                SourcePref::List {
                    key: "quality".into(),
                    title: "Quality".into(),
                    summary: Some("Preferred".into()),
                    entries: vec!["480p".into(), "720p".into()],
                    entry_values: vec!["480".into(), "720".into()],
                    default: "720".into(),
                },
                SourcePref::MultiSelect {
                    key: "hosts".into(),
                    title: "Hosts".into(),
                    summary: None,
                    entries: vec!["A".into(), "B".into()],
                    entry_values: vec!["a".into(), "b".into()],
                    default: vec!["a".into()],
                },
                SourcePref::EditText {
                    key: "domain".into(),
                    title: "Domain".into(),
                    summary: None,
                    default: String::new(),
                },
                SourcePref::Category {
                    title: "Advanced".into(),
                    children: vec![SourcePref::Switch {
                        key: "debug".into(),
                        title: "Debug".into(),
                        summary: None,
                        default: false,
                    }],
                },
            ]
        }
    }

    #[test]
    fn a_configurable_anime_source_declares_every_row_kind_and_the_flag_reaches_a_trait_object() {
        let source = Mock { configurable: true, ..Mock::new() };
        let dyn_source: &dyn AnimeSource = &source;
        assert!(dyn_source.is_configurable());
        assert!(!Mock::new().is_configurable());
        let rows = ConfigurableSource::preferences(&source);
        assert_eq!(rows.len(), 5);
        assert!(matches!(rows[0], SourcePref::Switch { .. }) && matches!(rows[1], SourcePref::List { .. }));
        assert!(matches!(rows[2], SourcePref::MultiSelect { .. }) && matches!(rows[3], SourcePref::EditText { .. }));
        assert!(matches!(&rows[4], SourcePref::Category { children, .. } if children.len() == 1));
        assert!(check_descriptors(rows).is_ok());
    }

    #[test]
    fn a_descriptor_list_with_a_duplicate_key_is_rejected_at_any_depth() {
        let mut rows = ConfigurableSource::preferences(&Mock::new());
        rows.push(SourcePref::Switch { key: "debug".into(), title: "Again".into(), summary: None, default: true });
        assert_eq!(check_descriptors(rows), Err(PrefDescriptorError::DuplicateKey("debug".into())));
    }

    #[tokio::test]
    async fn an_unmetered_source_fetches_its_seasons_at_once_and_a_metered_one_in_turn() {
        let seasons = || (0..3).map(|i| SAnime { title: format!("S{i}"), ..Default::default() }).collect::<Vec<_>>();
        let fetch = |_season: SAnime| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<(), SourceError>(())
        };
        let start = std::time::Instant::now();
        fetch_each_season(&Mock { unmetered: true, ..Mock::new() }, seasons(), || false, fetch).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(500), "parallel took {:?}", start.elapsed());
        let start = std::time::Instant::now();
        fetch_each_season(&Mock::new(), seasons(), || false, fetch).await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(600), "sequential took {:?}", start.elapsed());
    }

    #[tokio::test]
    async fn a_metered_season_fetch_stops_at_the_next_season_once_cancelled() {
        let calls = AtomicUsize::new(0);
        let seasons: Vec<SAnime> = (0..3).map(|i| SAnime { title: format!("S{i}"), ..Default::default() }).collect();
        let fetch = |_season: SAnime| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<(), SourceError>(()) }
        };
        let cancelled = || calls.load(Ordering::SeqCst) >= 1;
        let result = fetch_each_season(&Mock::new(), seasons, cancelled, fetch).await;
        assert_eq!(result, Err(SourceError::Cancelled));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[derive(Default)]
    struct Links {
        anime_lookups: AtomicUsize,
        episode_lookups: AtomicUsize,
        related_calls: AtomicUsize,
        related: bool,
    }

    #[async_trait]
    impl AnimeSource for Links {
        fn id(&self) -> i64 {
            11
        }
        fn name(&self) -> &str {
            "Links"
        }
        fn supports_latest(&self) -> bool {
            false
        }
        fn supports_related_anime(&self) -> bool {
            self.related
        }
        async fn related_anime(&self, _anime: &SAnime) -> Result<Vec<AnimeRelation>, SourceError> {
            self.related_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![
                AnimeRelation {
                    name: "Sequels".into(),
                    animes: vec![SAnime { title: "2".into(), ..Default::default() }],
                },
                AnimeRelation { name: "Specials".into(), animes: Vec::new() },
            ])
        }
        async fn popular(&self, _: i32) -> Result<AnimesPage, SourceError> {
            Err(SourceError::Unsupported("popular"))
        }
        async fn latest(&self, _: i32) -> Result<AnimesPage, SourceError> {
            Err(SourceError::Unsupported("latest"))
        }
        async fn search(&self, _: i32, _: &str, _: &FilterList) -> Result<AnimesPage, SourceError> {
            Err(SourceError::Unsupported("search"))
        }
        fn provides_hosters(&self) -> bool {
            false
        }
    }

    #[async_trait]
    impl ResolvableAnimeSource for Links {
        fn uri_type(&self, uri: &str) -> UriType {
            if uri.contains("/anime/") {
                UriType::Anime
            } else if uri.contains("/ep/") {
                UriType::Episode
            } else {
                UriType::Unknown
            }
        }
        async fn anime_for_uri(&self, _uri: &str) -> Result<Option<SAnime>, SourceError> {
            self.anime_lookups.fetch_add(1, Ordering::SeqCst);
            Ok(Some(SAnime { title: "Found".into(), ..Default::default() }))
        }
        async fn episode_for_uri(&self, _uri: &str) -> Result<Option<SEpisode>, SourceError> {
            self.episode_lookups.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
    }

    #[tokio::test]
    async fn an_anime_uri_runs_only_the_anime_lookup_and_an_episode_uri_only_the_episode_lookup() {
        let source = Links::default();
        assert_eq!(
            resolve_anime_uri(&source, "https://x.test/anime/1").await.unwrap(),
            ResolvedAnime::Anime(Some(SAnime { title: "Found".into(), ..Default::default() }))
        );
        assert_eq!(resolve_anime_uri(&source, "https://x.test/ep/1").await.unwrap(), ResolvedAnime::Episode(None));
        assert_eq!(
            (source.anime_lookups.load(Ordering::SeqCst), source.episode_lookups.load(Ordering::SeqCst)),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn a_uri_the_source_does_not_recognise_is_unknown_with_no_network_call() {
        let source = Links::default();
        assert_eq!(resolve_anime_uri(&source, "https://other.test/x").await.unwrap(), ResolvedAnime::Unknown);
        assert_eq!(
            (source.anime_lookups.load(Ordering::SeqCst), source.episode_lookups.load(Ordering::SeqCst)),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn related_anime_is_never_called_when_the_source_does_not_support_it() {
        let off = Links::default();
        assert_eq!(related_animes(&off, &anime()).await.unwrap_err(), SourceError::Unsupported("related_anime"));
        assert_eq!(off.related_calls.load(Ordering::SeqCst), 0);
        assert!(!off.supports_related_anime());
    }

    #[tokio::test]
    async fn related_groups_keep_the_source_order_of_groups_and_of_animes() {
        let on = Links { related: true, ..Default::default() };
        let groups = related_animes(&on, &anime()).await.unwrap();
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Sequels", "Specials"]);
        assert_eq!(groups[0].animes[0].title, "2");
        assert_eq!(on.related_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_default_related_anime_is_not_supported_and_the_unmetered_marker_is_off() {
        let bare = Mock::new();
        assert!(matches!(bare.related_anime(&anime()).await, Err(SourceError::Unsupported("related_anime"))));
        assert!(!bare.is_unmetered());
    }

    #[tokio::test]
    async fn a_source_that_implements_neither_hoster_method_returns_unsupported_on_every_call_without_recursion() {
        struct Neither;
        #[async_trait]
        impl AnimeSource for Neither {
            fn id(&self) -> i64 {
                12
            }
            fn name(&self) -> &str {
                "Neither"
            }
            fn supports_latest(&self) -> bool {
                false
            }
            async fn popular(&self, _: i32) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("popular"))
            }
            async fn latest(&self, _: i32) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("latest"))
            }
            async fn search(&self, _: i32, _: &str, _: &FilterList) -> Result<AnimesPage, SourceError> {
                Err(SourceError::Unsupported("search"))
            }
            fn provides_hosters(&self) -> bool {
                true
            }
        }
        let source = Neither;
        for _ in 0..1000 {
            assert!(matches!(source.hosters(&SEpisode::default()).await, Err(SourceError::Unsupported("hosters"))));
            let hoster = Hoster::default();
            assert!(matches!(
                source.videos_for_hoster(&hoster).await,
                Err(SourceError::Unsupported("videos_for_hoster"))
            ));
        }
    }
}
