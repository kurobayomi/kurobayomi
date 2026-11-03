//! Serialized `SerializableVideo` and `SerializableHoster` JSON (`model/Video.kt`, `model/Hoster.kt`; T-16a-018).
//!
//! Writes every key, nulls included (Kotlin `explicitNulls = true`, `encodeDefaults = true`). Reads ignore unknown keys
//! (superset tolerance). A hoster without `internalData` fails to decode, as in Kotlin.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{ChapterType, Headers, Hoster, TimeStamp, Track, Video};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTrack {
    url: String,
    lang: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
enum WireChapterType {
    Opening,
    Ending,
    Recap,
    MixedOp,
    Other,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTimeStamp {
    start: f64,
    end: f64,
    name: String,
    #[serde(rename = "type")]
    kind: WireChapterType,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireVideo {
    #[serde(default)]
    video_url: String,
    #[serde(default)]
    video_title: String,
    resolution: Option<i32>,
    bitrate: Option<i32>,
    headers: Option<Vec<(String, String)>>,
    #[serde(default)]
    preferred: bool,
    #[serde(default)]
    subtitle_tracks: Vec<WireTrack>,
    #[serde(default)]
    audio_tracks: Vec<WireTrack>,
    #[serde(default)]
    timestamps: Vec<WireTimeStamp>,
    #[serde(default)]
    mpv_args: Vec<(String, String)>,
    #[serde(default)]
    ffmpeg_stream_args: Vec<(String, String)>,
    #[serde(default)]
    ffmpeg_video_args: Vec<(String, String)>,
    #[serde(default)]
    internal_data: String,
    #[serde(default)]
    initialized: bool,
    #[serde(default)]
    memo: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireHoster {
    #[serde(default)]
    hoster_url: String,
    #[serde(default)]
    hoster_name: String,
    /// Nested JSON text of a video list, not an array.
    video_list: Option<String>,
    /// Required on read: a missing key is a decode error.
    internal_data: String,
    #[serde(default)]
    lazy: bool,
    #[serde(default)]
    memo: Map<String, Value>,
}

impl From<&Track> for WireTrack {
    fn from(t: &Track) -> Self {
        Self { url: t.url.clone(), lang: t.lang.clone() }
    }
}

impl From<WireTrack> for Track {
    fn from(t: WireTrack) -> Self {
        Self { url: t.url, lang: t.lang }
    }
}

impl From<ChapterType> for WireChapterType {
    fn from(kind: ChapterType) -> Self {
        match kind {
            ChapterType::Opening => Self::Opening,
            ChapterType::Ending => Self::Ending,
            ChapterType::Recap => Self::Recap,
            ChapterType::MixedOp => Self::MixedOp,
            ChapterType::Other => Self::Other,
        }
    }
}

impl From<WireChapterType> for ChapterType {
    fn from(kind: WireChapterType) -> Self {
        match kind {
            WireChapterType::Opening => Self::Opening,
            WireChapterType::Ending => Self::Ending,
            WireChapterType::Recap => Self::Recap,
            WireChapterType::MixedOp => Self::MixedOp,
            WireChapterType::Other => Self::Other,
        }
    }
}

impl From<&Video> for WireVideo {
    fn from(v: &Video) -> Self {
        Self {
            video_url: v.video_url.clone(),
            video_title: v.video_title.clone(),
            resolution: v.resolution,
            bitrate: v.bitrate,
            headers: v.headers.as_ref().map(|h| h.0.clone()),
            preferred: v.preferred,
            subtitle_tracks: v.subtitle_tracks.iter().map(WireTrack::from).collect(),
            audio_tracks: v.audio_tracks.iter().map(WireTrack::from).collect(),
            timestamps: v
                .timestamps
                .iter()
                .map(|t| WireTimeStamp { start: t.start, end: t.end, name: t.name.clone(), kind: t.kind.into() })
                .collect(),
            mpv_args: v.mpv_args.clone(),
            ffmpeg_stream_args: v.ffmpeg_stream_args.clone(),
            ffmpeg_video_args: v.ffmpeg_video_args.clone(),
            internal_data: v.internal_data.clone(),
            initialized: v.initialized,
            memo: v.memo.clone(),
        }
    }
}

impl From<WireVideo> for Video {
    fn from(w: WireVideo) -> Self {
        Self {
            video_url: w.video_url,
            video_title: w.video_title,
            resolution: w.resolution,
            bitrate: w.bitrate,
            headers: w.headers.map(Headers),
            preferred: w.preferred,
            subtitle_tracks: w.subtitle_tracks.into_iter().map(Track::from).collect(),
            audio_tracks: w.audio_tracks.into_iter().map(Track::from).collect(),
            timestamps: w
                .timestamps
                .into_iter()
                .map(|t| TimeStamp { start: t.start, end: t.end, name: t.name, kind: t.kind.into() })
                .collect(),
            mpv_args: w.mpv_args,
            ffmpeg_stream_args: w.ffmpeg_stream_args,
            ffmpeg_video_args: w.ffmpeg_video_args,
            internal_data: w.internal_data,
            initialized: w.initialized,
            memo: w.memo,
        }
    }
}

/// Encodes a video list as the JSON text of `SerializableVideo` objects.
pub fn encode_videos(videos: &[Video]) -> serde_json::Result<String> {
    let wire: Vec<WireVideo> = videos.iter().map(WireVideo::from).collect();
    serde_json::to_string(&wire)
}

/// Decodes the JSON text of a `SerializableVideo` list. Unknown keys are ignored.
pub fn decode_videos(text: &str) -> serde_json::Result<Vec<Video>> {
    let wire: Vec<WireVideo> = serde_json::from_str(text)?;
    Ok(wire.into_iter().map(Video::from).collect())
}

/// Encodes a hoster list. A hoster with no video list writes `videoList` as null.
pub fn encode_hosters(hosters: &[Hoster]) -> serde_json::Result<String> {
    let wire = hosters
        .iter()
        .map(|h| {
            Ok(WireHoster {
                hoster_url: h.hoster_url.clone(),
                hoster_name: h.hoster_name.clone(),
                video_list: h.video_list.as_deref().map(encode_videos).transpose()?,
                internal_data: h.internal_data.clone(),
                lazy: h.lazy,
                memo: h.memo.clone(),
            })
        })
        .collect::<serde_json::Result<Vec<_>>>()?;
    serde_json::to_string(&wire)
}

/// Decodes a hoster list. Fails when `internalData` is missing; `videoList` is parsed as a nested video list.
pub fn decode_hosters(text: &str) -> serde_json::Result<Vec<Hoster>> {
    let wire: Vec<WireHoster> = serde_json::from_str(text)?;
    wire.into_iter()
        .map(|h| {
            let video_list = h.video_list.as_deref().map(decode_videos).transpose()?;
            Ok(Hoster {
                hoster_url: h.hoster_url,
                hoster_name: h.hoster_name,
                video_list,
                internal_data: h.internal_data,
                lazy: h.lazy,
                memo: h.memo,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample_video() -> Video {
        Video {
            video_url: "https://v.example/a.m3u8".into(),
            video_title: "1080p".into(),
            resolution: Some(1080),
            bitrate: None,
            headers: Some(Headers(vec![("Referer".into(), "https://a.example/".into()), ("Cookie".into(), "x=1".into())])),
            preferred: true,
            subtitle_tracks: vec![Track { url: "https://s.example/en.vtt".into(), lang: "en".into() }],
            audio_tracks: vec![],
            timestamps: vec![TimeStamp { start: 0.0, end: 90.5, name: "Opening".into(), kind: ChapterType::Opening }],
            mpv_args: vec![("ytdl".into(), "no".into())],
            ffmpeg_stream_args: vec![],
            ffmpeg_video_args: vec![("crf".into(), "20".into())],
            internal_data: "{}".into(),
            initialized: true,
            memo: Map::new(),
        }
    }

    fn sample_hoster() -> Hoster {
        Hoster {
            hoster_url: "https://h.example/".into(),
            hoster_name: "Server A".into(),
            video_list: Some(vec![sample_video(), Video::new("https://v.example/b.mp4", "480p")]),
            internal_data: "".into(),
            lazy: false,
            memo: Map::new(),
        }
    }

    #[test]
    fn a_video_list_round_trips_and_re_encodes_byte_equal() {
        let text = encode_videos(&[sample_video()]).unwrap();
        let back = decode_videos(&text).unwrap();
        assert_eq!(back, vec![sample_video()]);
        assert_eq!(encode_videos(&back).unwrap(), text);
    }

    #[test]
    fn headers_are_an_array_of_two_element_arrays_in_order() {
        let text = encode_videos(&[sample_video()]).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value[0]["headers"], json!([["Referer", "https://a.example/"], ["Cookie", "x=1"]]));
        assert_eq!(decode_videos(&text).unwrap()[0].headers.as_ref().unwrap().0[1], ("Cookie".into(), "x=1".into()));
    }

    #[test]
    fn every_key_is_written_with_nulls_and_timestamps_use_the_type_name() {
        let value: Value = serde_json::from_str(&encode_videos(&[sample_video()]).unwrap()).unwrap();
        assert!(value[0]["bitrate"].is_null());
        assert_eq!(value[0]["timestamps"][0]["type"], "Opening");
        assert_eq!(value[0]["memo"], json!({}));
        assert_eq!(value[0].as_object().unwrap().len(), 15);
    }

    #[test]
    fn an_unknown_key_is_ignored_on_read() {
        let text = r#"[{"videoUrl":"u","videoTitle":"t","extra":{"x":1},"memo":{}}]"#;
        let videos = decode_videos(text).unwrap();
        assert_eq!((videos[0].video_url.as_str(), videos[0].video_title.as_str()), ("u", "t"));
    }

    #[test]
    fn a_hoster_round_trips_and_its_video_list_is_nested_text() {
        let hosters = vec![sample_hoster()];
        let text = encode_hosters(&hosters).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(value[0]["videoList"].is_string());
        assert!(value[0]["videoList"].as_str().unwrap().starts_with('['));
        let back = decode_hosters(&text).unwrap();
        assert_eq!(back, hosters);
        assert_eq!(encode_hosters(&back).unwrap(), text);
    }

    #[test]
    fn a_hoster_without_internal_data_fails_to_decode() {
        let text = r#"[{"hosterUrl":"h","hosterName":"n","videoList":null,"lazy":false,"memo":{}}]"#;
        assert!(decode_hosters(text).is_err());
    }

    #[test]
    fn a_null_video_list_stays_none() {
        let mut hoster = sample_hoster();
        hoster.video_list = None;
        let back = decode_hosters(&encode_hosters(&[hoster.clone()]).unwrap()).unwrap();
        assert_eq!(back, vec![hoster]);
    }
}
