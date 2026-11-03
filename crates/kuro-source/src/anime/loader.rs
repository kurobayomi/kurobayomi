//! The best-video algorithms of the player loader (`HosterLoader.kt`, `EpisodeLoader.loadHosterVideos`; T-16a-020 to
//! T-16a-023). Playback goes through these functions only.
//!
//! Phase A probes run as one `FuturesUnordered` on the caller's task, because this crate has no runtime. Dropping the
//! set when a probe wins cancels the probes still running, which is what `cancelChildren()` does in Kotlin. Only this
//! task writes the hoster states: each probe returns its own state (E-03).

use futures::StreamExt;
use futures::stream::FuturesUnordered;

use super::{AnimeSource, Hoster, HosterState, Video, VideoEntry, VideoState};
use crate::error::SourceError;

/// `loadHosterVideos` (behaviour 5.2). A lazy hoster is `Idle` unless `force` is set. A hoster with a `video_list` uses
/// it, and one without asks the source. The videos are sorted in every case and start in `Queue`. A failure other than
/// cancellation is `Error`. Cancellation is returned as `Err`.
pub async fn load_hoster(source: &dyn AnimeSource, hoster: &Hoster, force: bool) -> Result<HosterState, SourceError> {
    let hoster_name = hoster.hoster_name.clone();
    if hoster.lazy && !force {
        return Ok(HosterState::Idle { hoster_name });
    }
    let fetched = match &hoster.video_list {
        Some(videos) => Ok(videos.clone()),
        None => source.videos_for_hoster(hoster).await,
    };
    match fetched {
        Ok(videos) => {
            let videos = source.sort_videos(videos).into_iter().map(VideoEntry::queued).collect();
            Ok(HosterState::Ready { hoster_name, videos })
        }
        Err(SourceError::Cancelled) => Err(SourceError::Cancelled),
        Err(err) => {
            tracing::warn!(hoster = %hoster_name, error = %err, "hoster failed to load");
            Ok(HosterState::Error { hoster_name })
        }
    }
}

/// `selectBestVideo` (behaviour 6.1). Only `Ready` hosters count, and only videos in `Queue` or `Ready`. Tier 1 is the
/// first preferred video, tier 2 the first video with a URL. The result is `(hoster index, video index)`.
pub fn select_best(hosters: &[HosterState]) -> Option<(usize, usize)> {
    first_candidate(hosters, |video| video.preferred)
        .or_else(|| first_candidate(hosters, |video| !video.video_url.is_empty()))
}

fn first_candidate(hosters: &[HosterState], wanted: impl Fn(&Video) -> bool) -> Option<(usize, usize)> {
    hosters.iter().enumerate().find_map(|(hi, hoster)| match hoster {
        HosterState::Ready { videos, .. } => videos
            .iter()
            .position(|entry| matches!(entry.state, VideoState::Queue | VideoState::Ready) && wanted(&entry.video))
            .map(|vi| (hi, vi)),
        _ => None,
    })
}

/// `getResolvedVideo` (behaviour 6.3). A video already initialised is not sent to the source. A resolution that gives
/// no URL is `None`. Cancellation is returned as `Err`, and every other failure is `None`. A resolved video is marked
/// initialised.
pub async fn resolve_once(source: &dyn AnimeSource, video: Video) -> Result<Option<Video>, SourceError> {
    if video.initialized {
        return Ok(Some(video).filter(|v| !v.video_url.is_empty()));
    }
    let resolved = match source.resolve_video(video).await {
        Ok(resolved) => resolved,
        Err(SourceError::Cancelled) => return Err(SourceError::Cancelled),
        Err(err) => {
            tracing::warn!(error = %err, "resolve_video failed");
            None
        }
    };
    Ok(resolved
        .map(|mut video| {
            video.initialized = true;
            video
        })
        .filter(|v| !v.video_url.is_empty()))
}

/// `getBestVideo` (behaviour 6.2). Phase A probes, for every hoster at once, its first preferred video that is not
/// initialised yet, and the first probe to resolve wins. Phase B tries `select_best` candidates in order. `Ok(None)`
/// when nothing resolves. Cancellation from any step is returned as `Err`.
pub async fn best_video(source: &dyn AnimeSource, hosters: &[Hoster]) -> Result<Option<Video>, SourceError> {
    let mut probes: FuturesUnordered<_> =
        hosters.iter().enumerate().map(|(index, hoster)| probe(source, index, hoster)).collect();
    let mut states: Vec<HosterState> =
        hosters.iter().map(|hoster| HosterState::Idle { hoster_name: hoster.hoster_name.clone() }).collect();
    while let Some(outcome) = probes.next().await {
        let (index, state, found) = outcome?;
        states[index] = state;
        if let Some(video) = found {
            return Ok(Some(video));
        }
    }

    while let Some((hi, vi)) = select_best(&states) {
        let HosterState::Ready { videos, .. } = &mut states[hi] else { break };
        videos[vi].state = VideoState::LoadVideo;
        let candidate = videos[vi].video.clone();
        match resolve_once(source, candidate).await? {
            Some(video) => return Ok(Some(video)),
            None => videos[vi].state = VideoState::Error,
        }
    }
    Ok(None)
}

/// One phase A task. It loads its hoster, then resolves the first preferred video that is not initialised. It returns
/// its own state, so the collector is the only writer of the hoster list.
async fn probe(
    source: &dyn AnimeSource,
    index: usize,
    hoster: &Hoster,
) -> Result<(usize, HosterState, Option<Video>), SourceError> {
    let mut state = load_hoster(source, hoster, false).await?;
    let mut found = None;
    if let HosterState::Ready { videos, .. } = &mut state
        && let Some(vi) = videos.iter().position(|entry| entry.video.preferred && !entry.video.initialized)
    {
        videos[vi].state = VideoState::LoadVideo;
        found = resolve_once(source, videos[vi].video.clone()).await?;
        if found.is_none() {
            videos[vi].state = VideoState::Error;
        }
    }
    Ok((index, state, found))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use async_trait::async_trait;

    use super::*;
    use crate::anime::AnimesPage;
    use crate::filter_list::FilterList;

    /// A source with named hosters. `resolve_video` takes the title as a script: `slow` waits 300 ms, `broken` fails,
    /// `cancel` reports cancellation, `empty` gives no URL. Anything else resolves unchanged.
    #[derive(Default)]
    struct Site {
        lists: Vec<(&'static str, Vec<Video>)>,
        reverse: bool,
        hoster_calls: AtomicUsize,
        resolve_calls: AtomicUsize,
        completions: AtomicUsize,
    }

    #[async_trait]
    impl AnimeSource for Site {
        fn id(&self) -> i64 {
            1
        }
        fn name(&self) -> &str {
            "Site"
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
        fn sort_videos(&self, mut videos: Vec<Video>) -> Vec<Video> {
            if self.reverse {
                videos.reverse();
            }
            videos
        }
        async fn videos_for_hoster(&self, hoster: &Hoster) -> Result<Vec<Video>, SourceError> {
            self.hoster_calls.fetch_add(1, Ordering::SeqCst);
            if hoster.hoster_name == "cancel" {
                return Err(SourceError::Cancelled);
            }
            self.lists
                .iter()
                .find(|(name, _)| *name == hoster.hoster_name)
                .map(|(_, videos)| videos.clone())
                .ok_or_else(|| SourceError::Parse("no list".into()))
        }
        async fn resolve_video(&self, video: Video) -> Result<Option<Video>, SourceError> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            match video.video_title.as_str() {
                "slow" => {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    self.completions.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(video))
                }
                "broken" => Err(SourceError::Parse("broken".into())),
                "cancel" => Err(SourceError::Cancelled),
                "empty" => Ok(Some(Video { video_url: String::new(), ..video })),
                _ => Ok(Some(video)),
            }
        }
    }

    fn video(url: &str, title: &str) -> Video {
        Video::new(url, title)
    }

    fn preferred(url: &str, title: &str) -> Video {
        Video { preferred: true, ..Video::new(url, title) }
    }

    fn hoster(name: &str, videos: Option<Vec<Video>>) -> Hoster {
        Hoster { hoster_name: name.into(), video_list: videos, ..Default::default() }
    }

    fn entry(url: &str, preferred: bool, state: VideoState) -> VideoEntry {
        VideoEntry { video: Video { preferred, ..Video::new(url, "t") }, state }
    }

    fn ready(name: &str, videos: Vec<VideoEntry>) -> HosterState {
        HosterState::Ready { hoster_name: name.into(), videos }
    }

    #[test]
    fn the_worked_example_of_the_video_pipeline_selects_zero_one() {
        let states = vec![
            ready("A", vec![entry("", false, VideoState::Queue), entry("https://a/1.m3u8", false, VideoState::Queue)]),
            ready(
                "B",
                vec![
                    entry("https://b/1.m3u8", true, VideoState::Error),
                    entry("https://b/2.m3u8", false, VideoState::Ready),
                ],
            ),
            HosterState::Error { hoster_name: "C".into() },
        ];
        assert_eq!(select_best(&states), Some((0, 1)));
    }

    #[test]
    fn a_queued_preferred_video_beats_the_order() {
        let states = vec![
            ready("A", vec![entry("", false, VideoState::Queue), entry("https://a/1.m3u8", false, VideoState::Queue)]),
            ready("B", vec![entry("https://b/1.m3u8", true, VideoState::Queue)]),
        ];
        assert_eq!(select_best(&states), Some((1, 0)));
    }

    #[test]
    fn errors_loading_states_and_no_ready_hosters_are_never_selected() {
        let states = vec![
            ready(
                "A",
                vec![entry("https://a", true, VideoState::Error), entry("https://a2", false, VideoState::LoadVideo)],
            ),
            HosterState::Loading { hoster_name: "L".into() },
            HosterState::Idle { hoster_name: "I".into() },
        ];
        assert_eq!(select_best(&states), None);
        assert_eq!(select_best(&[]), None);
    }

    #[tokio::test]
    async fn a_lazy_hoster_is_idle_until_forced() {
        let site = Site { lists: vec![("L", vec![video("https://l", "l")])], ..Default::default() };
        let lazy = Hoster { lazy: true, ..hoster("L", None) };
        assert!(matches!(load_hoster(&site, &lazy, false).await, Ok(HosterState::Idle { .. })));
        assert_eq!(site.hoster_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(load_hoster(&site, &lazy, true).await, Ok(HosterState::Ready { .. })));
        assert_eq!(site.hoster_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_known_video_list_is_used_without_asking_the_source_and_an_unknown_one_is_asked_once() {
        let site = Site { lists: vec![("H", vec![video("https://h", "h")])], ..Default::default() };
        let known = hoster("H", Some(vec![video("https://k", "k")]));
        let state = load_hoster(&site, &known, false).await.unwrap();
        assert_eq!(site.hoster_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(&state, HosterState::Ready { videos, .. } if videos[0].video.video_url == "https://k"));
        load_hoster(&site, &hoster("H", None), false).await.unwrap();
        assert_eq!(site.hoster_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn videos_start_queued_and_the_source_order_is_applied_before_selection() {
        let site = Site {
            lists: vec![("H", vec![video("https://1", "1"), video("https://2", "2")])],
            reverse: true,
            ..Default::default()
        };
        let state = load_hoster(&site, &hoster("H", None), false).await.unwrap();
        let HosterState::Ready { videos, .. } = &state else { panic!("expected ready") };
        assert!(videos.iter().all(|e| e.state == VideoState::Queue));
        assert_eq!(select_best(&[state]), Some((0, 0)));
    }

    #[tokio::test]
    async fn a_hoster_error_is_an_error_state_and_cancellation_propagates() {
        let site = Site::default();
        assert!(matches!(load_hoster(&site, &hoster("missing", None), false).await, Ok(HosterState::Error { .. })));
        assert!(matches!(load_hoster(&site, &hoster("cancel", None), false).await, Err(SourceError::Cancelled)));
    }

    #[tokio::test]
    async fn the_first_preferred_resolution_wins_and_the_slower_probe_is_dropped() {
        let site = Site::default();
        let hosters = [
            hoster("slow", Some(vec![preferred("slow-1", "slow")])),
            hoster("fast", Some(vec![preferred("https://fast/1", "fast")])),
        ];
        let started = Instant::now();
        let best = best_video(&site, &hosters).await.unwrap().unwrap();
        assert_eq!(best.video_url, "https://fast/1");
        assert!(started.elapsed() < Duration::from_millis(250));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(site.completions.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn with_no_preferred_video_the_fallback_takes_the_first_url() {
        let site = Site::default();
        let hosters = [hoster("one", Some(vec![video("", "empty-url"), video("https://ok", "ok")]))];
        let best = best_video(&site, &hosters).await.unwrap().unwrap();
        assert_eq!(best.video_url, "https://ok");
        assert!(best.initialized);
    }

    #[tokio::test]
    async fn when_nothing_resolves_the_best_video_is_none() {
        let site = Site::default();
        let hosters = [hoster("one", Some(vec![video("https://x", "broken")]))];
        assert_eq!(best_video(&site, &hosters).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_empty_resolved_url_is_a_failure_and_the_next_candidate_is_tried() {
        let site = Site::default();
        let hosters = [hoster("one", Some(vec![video("https://a", "empty"), video("https://b", "ok")]))];
        let best = best_video(&site, &hosters).await.unwrap().unwrap();
        assert_eq!(best.video_url, "https://b");
    }

    #[tokio::test]
    async fn cancellation_during_resolution_aborts_the_whole_call() {
        let site = Site::default();
        let hosters = [hoster("one", Some(vec![preferred("https://c", "cancel")]))];
        assert!(matches!(best_video(&site, &hosters).await, Err(SourceError::Cancelled)));
    }

    #[tokio::test]
    async fn an_initialised_video_is_not_resolved_again() {
        let site = Site::default();
        let done = Video { initialized: true, ..video("https://i", "i") };
        let hosters = [hoster("one", Some(vec![done]))];
        let best = best_video(&site, &hosters).await.unwrap().unwrap();
        assert_eq!(best.video_url, "https://i");
        assert_eq!(site.resolve_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn resolve_once_gives_none_for_an_empty_url_and_marks_success_initialised() {
        let site = Site::default();
        assert_eq!(resolve_once(&site, video("https://e", "empty")).await.unwrap(), None);
        let resolved = resolve_once(&site, video("https://r", "r")).await.unwrap().unwrap();
        assert!(resolved.initialized);
    }

    #[tokio::test]
    async fn a_failing_resolution_is_none_and_cancellation_is_an_error() {
        let site = Site::default();
        assert_eq!(resolve_once(&site, video("https://b", "broken")).await.unwrap(), None);
        assert!(matches!(resolve_once(&site, video("https://c", "cancel")).await, Err(SourceError::Cancelled)));
    }
}
