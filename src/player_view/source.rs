//! Episodes of an online anime source (the recipe sources and the installed packages). The source's best video is resolved off
//! the GTK thread through the `kuro-source` loader (the source's own hoster and video order, then the first preferred
//! or playable video), and the window plays its URL with the request headers, subtitles and audio tracks it returned.
//! Progress is saved by episode row, as for a local episode. A failure is a toast.

use std::sync::Arc;

use gtk::glib;
use kuro_data::entry::EntryRepo;
use kuro_data::item::{Item, ItemRepo};
use kuro_domain::source::remote_update::to_s_episode;
use kuro_i18n::{MsgKey, tr};
use kuro_player::load::http_header_fields;
use kuro_player::track_select::{TrackCandidate, preferred_track};
use kuro_player::tracks::external_track_commands;
use kuro_source::anime::{AnimeSource, Hoster, SEpisode, Track, Video};

use super::Media;

thread_local! {
    /// An episode of a source is being resolved: further clicks wait for it instead of opening more players.
    static RESOLVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Plays episode row `episode_id` when its entry belongs to one of `sources`. `false` when it does not (or the row is
/// missing), so the caller keeps its own player. The video is resolved in the background; a "Loading…" toast shows
/// meanwhile and the window opens when it is ready.
pub fn open_source_episode(sources: &[Arc<dyn AnimeSource>], episode_id: i64) -> bool {
    if !crate::data::is_installed() {
        return false;
    }
    let Some(found) = source_episode(sources, episode_id) else {
        return false;
    };
    // a downloaded episode plays from its file, without asking the source
    if !super::is_open() {
        if let Some(media) = super::downloaded_media(episode_id) {
            super::open_media(media, episode_id, 0);
            return true;
        }
    }
    // a second click while the first still loads, or while a player is open, does nothing: the first answer opens it
    if super::is_open() || RESOLVING.with(|r| r.replace(true)) {
        return true;
    }
    let started = resolve_episode(found, move |answer| {
        RESOLVING.with(|r| r.set(false));
        match answer {
            Ok(media) => super::open_media(media, episode_id, 0),
            Err(text) => crate::toast::notify(text),
        }
    });
    if !started {
        RESOLVING.with(|r| r.set(false));
    }
    started
}

/// A source episode ready to resolve: its source, its row and the title it plays under.
pub struct SourceEpisode {
    source: Arc<dyn AnimeSource>,
    episode: Item,
    title: crate::widgets::page_header::MediaTitle,
}

/// The row `episode_id` when its entry belongs to one of `sources`. `None` for a missing row, a local entry, a source
/// that is not installed, or a failed read (logged).
pub fn source_episode(sources: &[Arc<dyn AnimeSource>], episode_id: i64) -> Option<SourceEpisode> {
    let db = &crate::data::app().db;
    let found = ItemRepo::new(Arc::clone(db))
        .by_id(episode_id)
        .and_then(|episode| episode.map(|e| EntryRepo::new(Arc::clone(db)).by_id(e.entry_id).map(|a| (a, e))).transpose());
    let (anime, episode) = match found {
        Ok(Some(found)) => found,
        Ok(None) => return None,
        Err(e) => {
            tracing::error!(error = %e, "player: the episode lookup failed");
            return None;
        }
    };
    let source = sources.iter().find(|s| s.id() == anime.source).cloned()?;
    let title = super::episode_title(&anime.title, &episode.name);
    Some(SourceEpisode { source, episode, title })
}

/// Resolves the best stream of `found` in the background. `done` runs on the GTK thread with the media or the toast
/// text. `false` when no async runtime runs on this thread, and then nothing starts.
pub fn resolve_episode(found: SourceEpisode, done: impl FnOnce(Result<Media, String>) + 'static) -> bool {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::error!("player: no async runtime on this thread; the source episode is not opened");
        return false;
    };
    let SourceEpisode { source, episode, title } = found;
    let s_episode = to_s_episode(&episode);
    let preferred = super::stored_text("pref_subtitle_lang");
    let preferred_audio = super::stored_text("pref_audio_lang");
    // read here: the app data lives on the GTK thread, the stream is chosen on the source's runtime
    let quality = super::default_quality();
    let (tx, rx) = async_channel::bounded(1);
    handle.spawn(async move {
        // the "Loading…" card never stays: after 30 seconds the toast says the source is not answering
        let answer = crate::browse::errors::settle(crate::browse::errors::LOAD_TIMEOUT, source.name(), best_stream(source.as_ref(), &s_episode, quality)).await;
        let answer = match answer {
            Ok(video) => allowed(source.id(), video).await,
            Err(text) => Err(text),
        };
        let _ = tx.send(answer).await;
    });
    let busy = crate::toast::busy(&tr(MsgKey::loading));
    glib::spawn_future_local(async move {
        // a closed channel means the source task panicked
        let answer = rx.recv().await.unwrap_or_else(|_| Err(tr(MsgKey::video_list_empty_error)));
        drop(busy);
        done(answer.map(|video| media_of(video, title, &preferred, &preferred_audio)));
    });
    true
}

/// The video to play: the source's hosters (or its plain video list, as one hoster) through `choose`, which keeps the
/// session's last quality choice, else plays the `quality` height, else the best video. The error is the text of the toast.
async fn best_stream(source: &dyn AnimeSource, episode: &SEpisode, quality: i32) -> Result<Video, String> {
    use crate::browse::errors::plain;
    let name = source.name();
    // every call of the source ends within 30 seconds with a sentence for the toast; the technical text is in the log
    let hosters = if source.provides_hosters() {
        source.sort_hosters(plain(name, source.hosters(episode)).await?)
    } else {
        vec![Hoster::synthetic(plain(name, source.videos_for_episode(episode)).await?)]
    };
    if hosters.is_empty() {
        return Err(tr(MsgKey::no_hosters));
    }
    plain(name, super::sheets::quality::choose(source, &hosters, quality)).await?.ok_or_else(|| tr(MsgKey::video_list_empty_error))
}

/// Side tracks are added with the `auto` flag, which selects a track only when it matches `alang` or `slang`; the source's
/// tracks carry a display name and no language code, so nothing matched and the video played without sound or subtitles.
/// The first audio track (the source puts its default first) is selected, and so is the subtitle in the preferred
/// language (`pref_subtitle_lang`) when one matches; else the English subtitle, else the first one.
fn select_defaults(mut commands: Vec<Vec<String>>, preferred: &str, preferred_audio: &str) -> Vec<Vec<String>> {
    let audios: Vec<usize> = commands.iter().enumerate().filter(|(_, c)| c[0] == "audio-add").map(|(i, _)| i).collect();
    let audio_candidates: Vec<TrackCandidate> = audios
        .iter()
        .map(|&i| TrackCandidate { id: i as i64, name: commands[i].get(3).cloned().unwrap_or_default(), lang: None })
        .collect();
    let audio_by_pref = (!preferred_audio.trim().is_empty())
        .then(|| preferred_track(&audio_candidates, preferred_audio, "", "", ""))
        .flatten()
        .map(|id| id as usize);
    let first_audio = audio_by_pref.or_else(|| audios.first().copied());
    let subs: Vec<usize> = commands.iter().enumerate().filter(|(_, c)| c[0] == "sub-add").map(|(i, _)| i).collect();
    let candidates: Vec<TrackCandidate> = subs
        .iter()
        .map(|&i| TrackCandidate { id: i as i64, name: commands[i].get(3).cloned().unwrap_or_default(), lang: None })
        .collect();
    let by_pref = (!preferred.trim().is_empty()).then(|| preferred_track(&candidates, preferred, "", "", "")).flatten().map(|id| id as usize);
    let english = subs.iter().copied().find(|&i| commands[i].get(3).is_some_and(|name| name.to_lowercase().starts_with("english") || name.to_lowercase().starts_with("en")));
    for i in first_audio.into_iter().chain(by_pref.or(english).or_else(|| subs.first().copied())) {
        commands[i][2] = "select".to_owned();
    }
    commands
}

/// `video` when the player may be given its links: the video, audio and subtitle links of an online source must be http
/// or https on the internet (`kuro_net::public_only::link_allowed`), since anyone may write a source and the player
/// opens what it is handed. The local source plays the files of your own folders and is not checked.
pub(super) async fn allowed(source_id: i64, video: Video) -> Result<Video, String> {
    if source_id == kuro_local::LOCAL_SOURCE_ID {
        return Ok(video);
    }
    let tracks = video.audio_tracks.iter().chain(&video.subtitle_tracks).map(|track| track.url.as_str());
    let links: Vec<&str> = std::iter::once(video.video_url.as_str()).chain(tracks).filter(|link| !link.is_empty()).collect();
    for link in links {
        let ok = match url::Url::parse(link) {
            Ok(url) => kuro_net::public_only::link_allowed(&url).await,
            Err(_) => false,
        };
        if !ok {
            tracing::warn!(link, "a source's video link was refused: it is not an internet address");
            return Err(tr(MsgKey::player_link_refused));
        }
    }
    Ok(video)
}

pub(super) fn media_of(video: Video, title: crate::widgets::page_header::MediaTitle, preferred: &str, preferred_audio: &str) -> Media {
    let tracks = |list: &[Track]| list.iter().map(|t| kuro_player::types::Track { url: t.url.clone(), lang: t.lang.clone() }).collect::<Vec<_>>();
    let headers = http_header_fields(true, video.headers.as_ref().map(|h| h.0.as_slice()), &[]).filter(|h| !h.is_empty());
    // a remote HLS stream plays through the local relay, which fetches it the way a browser does (`hls_proxy`)
    let target = if super::hls_proxy::wants_relay(&video.video_url) {
        super::hls_proxy::local_url(&video.video_url, video.headers.as_ref().map_or(&[][..], |h| h.0.as_slice()))
    } else {
        video.video_url
    };
    Media {
        track_headers: video.headers.as_ref().map(|h| h.0.clone()).unwrap_or_default(),
        tracks: select_defaults(external_track_commands(&tracks(&video.audio_tracks), &tracks(&video.subtitle_tracks)).commands, preferred, preferred_audio),
        target,
        title,
        headers,
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use kuro_source::SourceError;
    use kuro_source::anime::{AnimesPage, Headers};
    use kuro_source::filter_list::FilterList;

    use super::super::playback::load_commands;
    use super::*;

    /// A source over fixed hosters. `fail` makes `hosters` fail; `sort_videos` puts the highest resolution first, as
    /// a quality order does.
    struct Fake {
        hosters: Vec<Hoster>,
        fail: bool,
    }

    #[async_trait]
    impl AnimeSource for Fake {
        fn id(&self) -> i64 {
            1
        }
        fn name(&self) -> &str {
            "Fake"
        }
        fn supports_latest(&self) -> bool {
            false
        }
        async fn popular(&self, _: i32) -> Result<AnimesPage, SourceError> {
            unreachable!()
        }
        async fn latest(&self, _: i32) -> Result<AnimesPage, SourceError> {
            unreachable!()
        }
        async fn search(&self, _: i32, _: &str, _: &FilterList) -> Result<AnimesPage, SourceError> {
            unreachable!()
        }
        fn provides_hosters(&self) -> bool {
            true
        }
        async fn hosters(&self, _: &SEpisode) -> Result<Vec<Hoster>, SourceError> {
            if self.fail { Err(SourceError::Parse("HTTP 503".into())) } else { Ok(self.hosters.clone()) }
        }
        fn sort_videos(&self, mut videos: Vec<Video>) -> Vec<Video> {
            videos.sort_by_key(|v| std::cmp::Reverse(v.resolution));
            videos
        }
    }

    fn video(url: &str, resolution: i32) -> Video {
        Video {
            resolution: Some(resolution),
            headers: Some(Headers(vec![
                ("Origin".into(), "https://krussdomi.com".into()),
                ("User-Agent".into(), "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko)".into()),
            ])),
            subtitle_tracks: vec![Track { url: "https://cdn.test/en.vtt".into(), lang: "English".into() }],
            audio_tracks: vec![Track { url: "https://cdn.test/ja.m3u8".into(), lang: "Japanese".into() }],
            ..Video::new(url, format!("{resolution}p"))
        }
    }

    /// Anyone may write a source, and the player opens what it is handed: an online source's links must be web links
    /// on the internet, its subtitles and audio included; the local source plays your own files.
    #[test]
    fn the_player_gets_only_internet_links_from_an_online_source() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let check = |source_id: i64, video: Video| rt.block_on(allowed(source_id, video));
        let plain = |url: &str| Video { subtitle_tracks: Vec::new(), audio_tracks: Vec::new(), ..video(url, 720) };
        assert!(check(42, plain("https://1.1.1.1/ep.mp4")).is_ok());
        for refused in ["file:///home/me/.ssh/id_ed25519", "http://192.168.1.1/ep.mp4", "av://lavfi/testsrc"] {
            assert_eq!(check(42, plain(refused)).err(), Some(tr(MsgKey::player_link_refused)), "{refused}");
        }
        let local_subtitle = Video { subtitle_tracks: vec![Track { url: "http://127.0.0.1:9/x.vtt".into(), lang: "English".into() }], ..plain("https://1.1.1.1/ep.mp4") };
        assert!(check(42, local_subtitle).is_err(), "a subtitle link is checked too");
        assert!(check(kuro_local::LOCAL_SOURCE_ID, plain("file:///home/me/Kurobayomi/Local/Anime/Show/01.mkv")).is_ok(), "the local source plays files");
    }

    fn resolve(fake: &Fake) -> Result<Video, String> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        rt.block_on(best_stream(fake, &SEpisode::default(), 1080))
    }

    #[test]
    fn the_first_audio_and_the_english_subtitle_are_selected_the_rest_stay_available() {
        let strings = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let cmds = vec![
            strings(&["audio-add", "a1", "auto", "Japanese"]),
            strings(&["audio-add", "a2", "auto", "English"]),
            strings(&["sub-add", "s1", "auto", "French (fr)"]),
            strings(&["sub-add", "s2", "auto", "English (en)"]),
        ];
        let out = select_defaults(cmds, "", "");
        let flags: Vec<&str> = out.iter().map(|c| c[2].as_str()).collect();
        assert_eq!(flags, ["select", "auto", "auto", "select"]);
        // no English: the first subtitle is selected; no side tracks: nothing changes
        let only_fr = select_defaults(vec![strings(&["sub-add", "s1", "auto", "French (fr)"])], "", "");
        assert_eq!(only_fr[0][2], "select");
        assert!(select_defaults(Vec::new(), "", "").is_empty());
    }

    #[test]
    fn the_default_quality_plays_when_a_server_has_it_else_the_best() {
        // a server name of its own: the session's last choice (a static) names another server, so it does not apply
        let hoster = |videos| Hoster { hoster_name: "Quality".into(), video_list: Some(videos), ..Hoster::default() };
        let fake = Fake { hosters: vec![hoster(vec![video("https://cdn.test/q1080.m3u8", 1080), video("https://cdn.test/q480.m3u8", 480)])], fail: false };
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        assert_eq!(rt.block_on(best_stream(&fake, &SEpisode::default(), 480)).expect("a video").video_url, "https://cdn.test/q480.m3u8");
        let fake = Fake { hosters: vec![Hoster { hoster_name: "Other".into(), ..hoster(vec![video("https://cdn.test/o1080.m3u8", 1080)]) }], fail: false };
        assert_eq!(rt.block_on(best_stream(&fake, &SEpisode::default(), 720)).expect("a video").video_url, "https://cdn.test/o1080.m3u8", "no 720p: the best");
    }

    #[test]
    fn a_download_takes_the_default_quality_else_the_best() {
        let hoster = |name: &str, videos| Hoster { hoster_name: name.into(), video_list: Some(videos), ..Hoster::default() };
        let fake = Fake { hosters: vec![hoster("A", vec![video("https://cdn.test/a1080.m3u8", 1080), video("https://cdn.test/a480.m3u8", 480)])], fail: false };
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
        let pick = |quality| rt.block_on(super::super::default_or_best(&fake, &fake.hosters, quality)).expect("no error").expect("a video").video_url;
        assert_eq!(pick(480), "https://cdn.test/a480.m3u8");
        assert_eq!(pick(720), "https://cdn.test/a1080.m3u8", "no 720p: the best");
    }

    #[test]
    fn the_best_quality_plays_with_the_source_headers_and_side_tracks() {
        let hoster = |name: &str, videos| Hoster { hoster_name: name.into(), video_list: Some(videos), ..Hoster::default() };
        let fake = Fake {
            hosters: vec![hoster("VidStreaming", vec![video("https://cdn.test/480.m3u8", 480), video("https://cdn.test/1080.m3u8", 1080)])],
            fail: false,
        };
        let media = media_of(resolve(&fake).expect("a video"), crate::widgets::page_header::MediaTitle::new("Show", "1"), "", "");
        assert_eq!(
            super::super::hls_proxy::upstream_of(&media.target).as_deref(),
            Some("https://cdn.test/1080.m3u8"),
            "the source's best quality is the default, played through the HLS relay"
        );

        let headers = "Origin: https://krussdomi.com,User-Agent: Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML\\, like Gecko)";
        assert_eq!(media.headers.as_deref(), Some(headers), "commas inside a value are escaped for the mpv list");
        assert_eq!(
            load_commands(&media.target, media.headers.as_deref(), 0),
            vec![
                kuro_player::load::start_command_ms(0),
                vec!["set".to_owned(), "http-header-fields".to_owned(), headers.to_owned()],
                vec!["loadfile".to_owned(), media.target.clone(), "replace".to_owned()],
            ],
            "the headers are set before loadfile, which has no index argument (mpv before 0.38)"
        );
        let strings = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            media.tracks,
            vec![
                strings(&["audio-add", "https://cdn.test/ja.m3u8", "select", "Japanese"]),
                strings(&["sub-add", "https://cdn.test/en.vtt", "select", "English"]),
            ]
        );
    }

    #[test]
    fn a_video_without_headers_sets_none() {
        let mut plain = video("https://cdn.test/a.m3u8", 720);
        plain.headers = None;
        assert_eq!(media_of(plain, Default::default(), "", "").headers, None);
    }

    #[test]
    fn the_preferred_subtitle_language_is_selected_before_english() {
        let strings = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let cmds = vec![strings(&["sub-add", "s1", "auto", "English (en)"]), strings(&["sub-add", "s2", "auto", "French (fr)"])];
        let out = select_defaults(cmds, "fr", "");
        assert_eq!(out[0][2], "auto", "English waits when French is preferred");
        assert_eq!(out[1][2], "select");
    }

    #[test]
    fn the_preferred_audio_language_is_selected_before_the_first_track() {
        let strings = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let cmds = vec![strings(&["audio-add", "a1", "auto", "Japanese (ja)"]), strings(&["audio-add", "a2", "auto", "English (en)"])];
        let out = select_defaults(cmds, "", "en");
        assert_eq!(out[0][2], "auto");
        assert_eq!(out[1][2], "select");
    }

    #[test]
    fn a_failed_or_empty_video_list_is_the_toast_text() {
        let failed = resolve(&Fake { hosters: Vec::new(), fail: true }).unwrap_err();
        assert!(failed.starts_with("Fake ") && failed.contains("isn't working right now") && !failed.contains("503"), "{failed}");
        assert_eq!(resolve(&Fake { hosters: Vec::new(), fail: false }).unwrap_err(), tr(MsgKey::no_hosters));
        let empty = Fake { hosters: vec![Hoster { hoster_name: "A".into(), video_list: Some(Vec::new()), ..Hoster::default() }], fail: false };
        assert_eq!(resolve(&empty).unwrap_err(), tr(MsgKey::video_list_empty_error));
    }
}
