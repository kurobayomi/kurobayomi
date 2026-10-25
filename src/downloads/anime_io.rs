//! The two app-side ports of the anime downloads: the hoster resolver, which picks the video of the player's Default
//! quality setting, else the source's best, and the direct-file copy used when ffmpeg is not installed. Both run on the
//! runtime, off the GTK thread, so they only read the live sources and the network client.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use kuro_downloads::anime::downloader::VideoFetch;
use kuro_downloads::anime::model::{AnimeRef, AnimeSource, EpisodeRef, Track, Video, VideoStatus};
use kuro_downloads::anime::progress::ProgressCell;
use kuro_downloads::anime::resolve::HosterResolver;
use kuro_downloads::error::DownloadError;
use kuro_net::NetworkClient;
use kuro_source::anime::{Hoster, SEpisode};
use tokio_util::sync::CancellationToken;

/// The live anime source of an id, the same set Browse and the player use. `None` when it is not installed.
pub(crate) fn live_source(id: i64) -> Option<Arc<dyn kuro_source::anime::AnimeSource>> {
    crate::screens::anime_sources().into_iter().find(|s| s.id() == id)
}

/// The episode as the source sees it. The queue keeps the fields the source reads from the list row.
fn to_s_episode(episode: &EpisodeRef) -> SEpisode {
    SEpisode {
        url: episode.url.clone(),
        name: episode.name.clone(),
        episode_number: episode.episode_number as f32,
        fillermark: episode.fillermark,
        scanlator: episode.scanlator.clone(),
        date_upload: episode.upload_date,
        ..SEpisode::default()
    }
}

fn to_track(track: kuro_source::anime::Track) -> Track {
    Track { url: track.url, lang: track.lang }
}

/// The source's video in the download queue's shape: the stream, its headers and side tracks. A remote HLS stream goes
/// through the player's relay (`player_view::hls_proxy`), as it does when played: ffmpeg speaks HTTP/1.1 only, and some
/// video servers refuse that (AnimePahe's answer 403). The relay then holds the headers and sends them upstream.
fn to_download_video(video: kuro_source::anime::Video) -> Video {
    let headers = video.headers.map(|h| h.0);
    let (url, headers) = if crate::player_view::hls_proxy::wants_relay(&video.video_url) {
        (crate::player_view::hls_proxy::local_url(&video.video_url, headers.as_deref().unwrap_or_default()), None)
    } else {
        (video.video_url, headers)
    };
    Video {
        url,
        title: video.video_title,
        headers,
        subtitle_tracks: video.subtitle_tracks.into_iter().map(to_track).collect(),
        audio_tracks: video.audio_tracks.into_iter().map(to_track).collect(),
        stream_args: video.ffmpeg_stream_args,
        video_args: video.ffmpeg_video_args,
        initialized: true,
        status: VideoStatus::Ready,
    }
}

/// `getHosters` then the video of the Default quality setting (else `getBestVideo`, behaviour 5.2): the source's hosters,
/// sorted, and the first one that resolves. The setting is read at each episode, so a change applies to the queue.
pub struct LiveHosters(pub kuro_prefs::player::PlayerPrefs);

#[async_trait]
impl HosterResolver for LiveHosters {
    async fn best_video(
        &self,
        _anime: &AnimeRef,
        episode: &EpisodeRef,
        source: &AnimeSource,
    ) -> Result<Option<Video>, DownloadError> {
        let live = live_source(source.id)
            .ok_or_else(|| DownloadError::Internal(format!("the source {} is not installed", source.name)))?;
        let s_episode = to_s_episode(episode);
        let hosters: Vec<Hoster> = if live.provides_hosters() {
            live.sort_hosters(live.hosters(&s_episode).await?)
        } else {
            vec![Hoster::synthetic(live.videos_for_episode(&s_episode).await?)]
        };
        let found = crate::player_view::default_or_best(live.as_ref(), &hosters, self.0.default_quality.get()).await?;
        Ok(found.map(to_download_video))
    }
}

/// Copies a direct video file over the app's HTTP client, with the source's request headers.
pub struct HttpVideoFetch {
    client: Arc<NetworkClient>,
}

impl HttpVideoFetch {
    pub fn new(client: NetworkClient) -> Self {
        Self { client: Arc::new(client) }
    }
}

#[async_trait]
impl VideoFetch for HttpVideoFetch {
    // ponytail: the body is read whole before it is written, because kuro_net::Response has no chunk stream yet. A
    // long episode needs that stream (a Response accessor in kuro-net) before this copy is safe for big files.
    async fn fetch_to(
        &self,
        url: &str,
        headers: &[(String, String)],
        out: &Path,
        progress: &ProgressCell,
        cancel: &CancellationToken,
    ) -> Result<(), DownloadError> {
        let mut request = kuro_net::request::get(url).map_err(|e| DownloadError::Internal(e.to_string()))?;
        for (name, value) in headers {
            let (Ok(name), Ok(value)) = (
                http::HeaderName::from_bytes(name.as_bytes()),
                http::HeaderValue::from_str(value),
            ) else {
                continue;
            };
            request.headers.insert(name, value);
        }
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            response = self.client.execute(request) => response.map_err(|e| DownloadError::Internal(e.to_string()))?,
        };
        let response = response.await_success().map_err(|e| DownloadError::Internal(e.to_string()))?;
        let body = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            body = response.bytes() => body.map_err(|e| DownloadError::Internal(e.to_string()))?,
        };
        tokio::fs::write(out, &body).await?;
        progress.set(100);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_hls_stream_downloads_through_the_players_relay_and_a_file_does_not() {
        let headers = Some(kuro_source::anime::Headers(vec![("Referer".into(), "https://site.test/".into())]));
        let video = |url: &str| kuro_source::anime::Video { video_url: url.into(), headers: headers.clone(), ..Default::default() };
        let hls = to_download_video(video("https://cdn.test/stream/uwu.m3u8?t=1"));
        if hls.url != "https://cdn.test/stream/uwu.m3u8?t=1" {
            // the relay started: ffmpeg asks it on this machine, and it holds the headers
            assert!(hls.url.starts_with("http://127.0.0.1:"), "{}", hls.url);
            assert_eq!(crate::player_view::hls_proxy::upstream_of(&hls.url).as_deref(), Some("https://cdn.test/stream/uwu.m3u8?t=1"));
            assert_eq!(hls.headers, None);
        }
        let file = to_download_video(video("https://cdn.test/episode.mp4"));
        assert_eq!((file.url.as_str(), file.headers), ("https://cdn.test/episode.mp4", headers.map(|h| h.0)), "a plain file goes as it is");
    }

    #[test]
    fn the_queued_episode_maps_to_the_source_episode() {
        let episode = EpisodeRef {
            id: 4,
            anime_id: 2,
            url: "/ep/4".into(),
            name: "Episode 4".into(),
            scanlator: Some("Group".into()),
            seen: false,
            bookmark: false,
            fillermark: true,
            source_order: 3,
            upload_date: 1_700_000_000_000,
            episode_number: 4.5,
        };
        let mapped = to_s_episode(&episode);
        assert_eq!(mapped.url, "/ep/4");
        assert_eq!(mapped.name, "Episode 4");
        assert_eq!(mapped.episode_number, 4.5);
        assert!(mapped.fillermark);
        assert_eq!(mapped.scanlator.as_deref(), Some("Group"));
        assert_eq!(mapped.date_upload, 1_700_000_000_000);
    }

    #[test]
    fn the_source_video_keeps_its_headers_tracks_and_ffmpeg_args() {
        let mut video = kuro_source::anime::Video::new("https://cdn.example/v.mp4", "1080p");
        video.headers = Some(kuro_source::anime::Headers(vec![("Referer".into(), "https://site.example".into())]));
        video.subtitle_tracks = vec![kuro_source::anime::Track { url: "https://cdn.example/en.vtt".into(), lang: "en".into() }];
        video.audio_tracks = vec![kuro_source::anime::Track { url: "https://cdn.example/ja.m4a".into(), lang: "ja".into() }];
        video.ffmpeg_stream_args = vec![("-user_agent".into(), "ua".into())];
        video.ffmpeg_video_args = vec![("-c".into(), "copy".into())];
        let mapped = to_download_video(video);
        assert_eq!(mapped.url, "https://cdn.example/v.mp4");
        assert_eq!(mapped.title, "1080p");
        assert_eq!(mapped.headers, Some(vec![("Referer".to_owned(), "https://site.example".to_owned())]));
        assert_eq!((mapped.subtitle_tracks.len(), mapped.subtitle_tracks[0].lang.as_str()), (1, "en"));
        assert_eq!((mapped.audio_tracks.len(), mapped.audio_tracks[0].url.as_str()), (1, "https://cdn.example/ja.m4a"));
        assert_eq!(mapped.stream_args, vec![("-user_agent".to_owned(), "ua".to_owned())]);
        assert_eq!(mapped.video_args, vec![("-c".to_owned(), "copy".to_owned())]);
        assert!(mapped.initialized);
        assert_eq!(mapped.status, VideoStatus::Ready);
    }

    #[test]
    fn a_source_video_without_headers_maps_to_none() {
        assert_eq!(to_download_video(kuro_source::anime::Video::new("u", "t")).headers, None);
    }
}
