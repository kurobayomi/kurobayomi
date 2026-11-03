//! `Hoster`, `HosterState` and the hoster discovery per episode kind (`Hoster.kt`, `EpisodeLoader.kt`; T-16a-017,
//! T-16a-019).

use serde_json::{Map, Value};

use super::{AnimeSource, SEpisode, Video, VideoEntry};
use crate::error::SourceError;

/// The name of the single hoster that stands for a source without hosters (`Hoster.kt:57`). The UI shows no server name
/// for it (E-24).
pub const NO_HOSTER_LIST: &str = "no_hoster_list";

/// A server that holds the videos of an episode. `video_list` is `None` when the videos are fetched later, and `lazy`
/// means they are not fetched until the hoster is chosen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hoster {
    pub hoster_url: String,
    pub hoster_name: String,
    pub video_list: Option<Vec<Video>>,
    pub internal_data: String,
    pub lazy: bool,
    pub memo: Map<String, Value>,
}

impl Hoster {
    /// `List<Video>.toHosterList()`: one unnamed hoster holding the videos in the order given.
    pub fn synthetic(videos: Vec<Video>) -> Self {
        Self { hoster_name: NO_HOSTER_LIST.to_owned(), video_list: Some(videos), ..Default::default() }
    }
}

/// The runtime state of one hoster while an episode is played (`Hoster.State`, not persisted). `Ready` carries the
/// videos with their own state.
#[derive(Debug, Clone, PartialEq)]
pub enum HosterState {
    Idle { hoster_name: String },
    Loading { hoster_name: String },
    Ready { hoster_name: String, videos: Vec<VideoEntry> },
    Error { hoster_name: String },
}

/// The HTTP branch of `EpisodeLoader.getHostersOnHttp` (behaviour 5.1). A source that provides hosters lists them and
/// sorts them. Otherwise the legacy videos are sorted and wrapped in one synthetic hoster. Errors propagate to the
/// caller, so the UI can show them for the episode.
pub async fn http_hosters(source: &dyn AnimeSource, episode: &SEpisode) -> Result<Vec<Hoster>, SourceError> {
    if source.provides_hosters() {
        let hosters = source.hosters(episode).await?;
        Ok(source.sort_hosters(hosters))
    } else {
        let videos = source.videos_for_episode(episode).await?;
        Ok(vec![Hoster::synthetic(source.sort_videos(videos))])
    }
}

/// The downloaded branch (`getHostersOnDownloaded`): one hoster built from the download lookup, with no source call.
/// Any error becomes an empty list, as in Kotlin. The lookup itself belongs to the downloads layer (area 24 and 25).
pub fn downloaded_hosters<E: std::fmt::Debug>(lookup: Result<Video, E>) -> Vec<Hoster> {
    match lookup {
        Ok(video) => vec![Hoster::synthetic(vec![video])],
        Err(err) => {
            tracing::debug!(?err, "downloaded episode lookup failed, no hoster");
            Vec::new()
        }
    }
}

/// The local branch (`getHostersOnLocal`): `file_uri` is the file the local source found for `episode_url`. A missing
/// file gives an empty list. The video title is `Local source: <episode url>`.
pub fn local_hosters(episode_url: &str, file_uri: Option<String>) -> Vec<Hoster> {
    match file_uri {
        Some(uri) => vec![Hoster::synthetic(vec![Video::new(uri, format!("Local source: {episode_url}"))])],
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::anime::{AnimesPage, SAnime};
    use crate::filter_list::FilterList;

    /// Records which discovery calls were made. `provides` decides the routing.
    struct Site {
        provides: bool,
        calls: Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl AnimeSource for Site {
        fn id(&self) -> i64 {
            7
        }
        fn name(&self) -> &str {
            "Site"
        }
        fn supports_latest(&self) -> bool {
            false
        }
        async fn popular(&self, _: i32) -> Result<AnimesPage, SourceError> {
            Ok(AnimesPage { animes: Vec::<SAnime>::new(), has_next_page: false })
        }
        async fn latest(&self, _: i32) -> Result<AnimesPage, SourceError> {
            Ok(AnimesPage { animes: Vec::new(), has_next_page: false })
        }
        async fn search(&self, _: i32, _: &str, _: &FilterList) -> Result<AnimesPage, SourceError> {
            Ok(AnimesPage { animes: Vec::new(), has_next_page: false })
        }
        fn provides_hosters(&self) -> bool {
            self.provides
        }
        async fn hosters(&self, _: &SEpisode) -> Result<Vec<Hoster>, SourceError> {
            self.calls.lock().unwrap().push("hosters");
            Ok(vec![
                Hoster { hoster_name: "B".into(), ..Default::default() },
                Hoster { hoster_name: "A".into(), ..Default::default() },
            ])
        }
        async fn videos_for_episode(&self, _: &SEpisode) -> Result<Vec<Video>, SourceError> {
            self.calls.lock().unwrap().push("videos_for_episode");
            Ok(vec![Video::new("u1", "one"), Video::new("u2", "two")])
        }
        fn sort_hosters(&self, mut hosters: Vec<Hoster>) -> Vec<Hoster> {
            hosters.sort_by(|a, b| a.hoster_name.cmp(&b.hoster_name));
            hosters
        }
    }

    #[tokio::test]
    async fn a_source_with_hosters_is_asked_for_them_and_its_sort_applies() {
        let site = Site { provides: true, calls: Mutex::new(Vec::new()) };
        let hosters = http_hosters(&site, &SEpisode::default()).await.unwrap();
        let names: Vec<&str> = hosters.iter().map(|h| h.hoster_name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(*site.calls.lock().unwrap(), ["hosters"]);
    }

    #[tokio::test]
    async fn a_legacy_source_gets_one_synthetic_hoster_and_never_the_hoster_call() {
        let site = Site { provides: false, calls: Mutex::new(Vec::new()) };
        let hosters = http_hosters(&site, &SEpisode::default()).await.unwrap();
        assert_eq!(hosters.len(), 1);
        assert_eq!(hosters[0].hoster_name, NO_HOSTER_LIST);
        let titles: Vec<&str> =
            hosters[0].video_list.as_ref().unwrap().iter().map(|v| v.video_title.as_str()).collect();
        assert_eq!(titles, ["one", "two"]);
        assert_eq!(*site.calls.lock().unwrap(), ["videos_for_episode"]);
    }

    #[test]
    fn a_downloaded_episode_is_one_hoster_and_an_error_is_an_empty_list() {
        let hosters = downloaded_hosters::<()>(Ok(Video::new("file:///d.mkv", "Downloaded")));
        assert_eq!(hosters.len(), 1);
        assert!(downloaded_hosters(Err::<Video, _>("io")).is_empty());
    }

    #[test]
    fn a_local_episode_names_its_file_and_a_missing_file_is_empty() {
        let hosters = local_hosters("dir/file.mkv", Some("file:///base/dir/file.mkv".into()));
        assert_eq!(hosters.len(), 1);
        let video = &hosters[0].video_list.as_ref().unwrap()[0];
        assert_eq!(video.video_title, "Local source: dir/file.mkv");
        assert_eq!(video.video_url, "file:///base/dir/file.mkv");
        assert!(local_hosters("dir/file.mkv", None).is_empty());
    }

    #[test]
    fn a_synthetic_hoster_has_the_kotlin_fields() {
        let hoster = Hoster::synthetic(vec![Video::new("u", "t")]);
        assert_eq!(hoster.hoster_url, "");
        assert_eq!(hoster.hoster_name, NO_HOSTER_LIST);
        assert!(!hoster.lazy);
        assert_eq!(hoster.video_list.unwrap().len(), 1);
        assert!(!Hoster::default().lazy);
    }
}
