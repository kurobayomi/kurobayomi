//! Loading an episode into mpv (area 45 `PlayerActivity.setVideo`, `setHttpOptions`,
//! `setMpvMediaTitle`): the `loadfile` command, the start position, the HTTP header option and
//! the title properties. All pure; the caller sends the returned arguments.

use crate::options::kotlin_f32;
use crate::source_url::parse_video_url;

/// The fields of a `Video` that the loader reads (data doc 2.1).
#[derive(Clone, Debug, PartialEq)]
pub struct VideoSpec {
    pub url: String,
    /// `Video.mpvArgs`: per-video mpv options, applied by `loadfile`.
    pub mpv_args: Vec<(String, String)>,
}

/// `loadfile <url> replace 0 <options>` (T-45a-011). `None` for a missing video: nothing is sent.
/// Options are `key="value"` pairs joined by `,`, with no escaping, as Kotlin does.
pub fn loadfile_command(video: Option<&VideoSpec>) -> Option<Vec<String>> {
    let video = video?;
    let url = parse_video_url(Some(&video.url))?;
    let options = video
        .mpv_args
        .iter()
        .map(|(key, value)| format!("{key}=\"{value}\""))
        .collect::<Vec<_>>()
        .join(",");
    Some(vec![
        "loadfile".to_owned(),
        url,
        "replace".to_owned(),
        "0".to_owned(),
        options,
    ])
}

/// Start position in milliseconds, or `None` when no `set start` is sent (T-45a-012).
/// A new episode (`loading_episode`) uses an explicit position if one is given. Otherwise a seen
/// episode restarts at 0 unless the watching position is preserved. A reload of the same episode
/// uses the current `time-pos`, and sends nothing when there is none.
pub fn start_position_ms(
    loading_episode: bool,
    explicit_ms: Option<i64>,
    seen: bool,
    preserve_position: bool,
    last_second_seen_ms: i64,
    time_pos_ms: Option<i64>,
) -> Option<i64> {
    if !loading_episode {
        return time_pos_ms;
    }
    let saved = if seen && !preserve_position {
        0
    } else {
        last_second_seen_ms
    };
    Some(explicit_ms.unwrap_or(saved))
}

/// `set start <seconds as f32>`. The Kotlin divides milliseconds by `1000F` (a Float).
pub fn start_command_ms(ms: i64) -> Vec<String> {
    vec![
        "set".to_owned(),
        "start".to_owned(),
        kotlin_f32(ms as f32 / 1000.0),
    ]
}

/// `http-header-fields` value for online episodes (T-45a-014). `None` for offline and local
/// episodes, which never set the option. Video headers win over source headers. Within one
/// list the first value of a key is kept, and `,` inside a value is escaped as `\,`.
pub fn http_header_fields(
    online: bool,
    video_headers: Option<&[(String, String)]>,
    source_headers: &[(String, String)],
) -> Option<String> {
    if !online {
        return None;
    }
    let headers = video_headers.unwrap_or(source_headers);
    let mut seen: Vec<String> = Vec::new();
    let mut parts = Vec::new();
    for (key, value) in headers {
        let lower = key.to_lowercase();
        if seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        parts.push(format!("{key}: {}", value.replace(',', "\\,")));
    }
    Some(parts.join(","))
}

/// Title properties written when an episode is loaded (T-45a-018).
#[derive(Clone, Debug, PartialEq)]
pub struct MediaTitle {
    /// `user-data/current-anime/episode-title`.
    pub episode_title: String,
    /// `force-media-title`, `<anime> - E<NN> - <episode>`.
    pub force_media_title: String,
    /// `user-data/current-anime/episode-number`, sent as a double.
    pub episode_number: f64,
}

/// Episode number as printed in the title: integral values lose `.0`, and the text is padded to
/// two characters with leading zeros (`1` gives `01`, `1.5` stays `1.5`, `12.0` gives `12`).
pub fn episode_number_text(number: f32) -> String {
    let text = if number.fract() == 0.0 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    };
    format!("{text:0>2}")
}

pub fn media_title(anime_title: &str, episode_number: f32, episode_name: &str) -> MediaTitle {
    MediaTitle {
        episode_title: episode_name.to_owned(),
        force_media_title: format!(
            "{anime_title} - E{} - {episode_name}",
            episode_number_text(episode_number)
        ),
        episode_number: f64::from(episode_number),
    }
}

/// The file path of an on-device episode (T-45a-068). A local-source episode or a downloaded one
/// plays from its path as it is. The Android content-URI step is dropped on desktop.
pub fn on_device_path(local_source: bool, downloaded: bool, video_url: &str) -> Option<&str> {
    (local_source || downloaded).then_some(video_url)
}

/// The steps that run on `FILE_LOADED`, in order (behaviour 4.2).
pub trait FileLoadedSteps {
    /// `setMpvOptions`: metadata to mpv options.
    fn mpv_options(&mut self);
    /// `setMpvMediaTitle`.
    fn media_title(&mut self);
    /// `setupChapters`.
    fn chapters(&mut self);
    /// `setupTracks`.
    fn tracks(&mut self);
    /// The skip-intro lookup (part 2).
    fn skip_intro(&mut self);
}

/// Runs the `FILE_LOADED` steps. `is_exiting` is read before each step, so a step is skipped once
/// the window starts exiting (T-45a-084).
pub fn run_file_loaded(steps: &mut impl FileLoadedSteps, is_exiting: impl Fn() -> bool) {
    if is_exiting() {
        return;
    }
    steps.mpv_options();
    if is_exiting() {
        return;
    }
    steps.media_title();
    if is_exiting() {
        return;
    }
    steps.chapters();
    if is_exiting() {
        return;
    }
    steps.tracks();
    if is_exiting() {
        return;
    }
    steps.skip_intro();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(url: &str, args: &[(&str, &str)]) -> VideoSpec {
        VideoSpec {
            url: url.to_owned(),
            mpv_args: args
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn loadfile_replaces_with_quoted_options() {
        let v = video(
            "file:///a/b.mp4",
            &[("http-header-fields", "A: b"), ("title", "x")],
        );
        assert_eq!(
            loadfile_command(Some(&v)).unwrap(),
            vec![
                "loadfile",
                "/a/b.mp4",
                "replace",
                "0",
                "http-header-fields=\"A: b\",title=\"x\""
            ]
        );
    }

    #[test]
    fn loadfile_with_no_options_sends_empty_option_string() {
        let v = video("http://h/v.m3u8", &[]);
        assert_eq!(loadfile_command(Some(&v)).unwrap()[4], "");
    }

    #[test]
    fn null_video_sends_nothing() {
        assert_eq!(loadfile_command(None), None);
    }

    #[test]
    fn resume_rule_covers_seen_and_unseen() {
        // New episode, seen and not preserved: starts at 0.
        assert_eq!(
            start_position_ms(true, None, true, false, 60_000, None),
            Some(0)
        );
        // New episode, not seen: the saved second.
        assert_eq!(
            start_position_ms(true, None, false, false, 60_000, None),
            Some(60_000)
        );
    }

    #[test]
    fn explicit_position_wins_for_new_episode_only() {
        assert_eq!(
            start_position_ms(true, Some(5_000), true, false, 60_000, None),
            Some(5_000)
        );
        assert_eq!(
            start_position_ms(false, Some(5_000), true, false, 60_000, Some(7_000)),
            Some(7_000)
        );
    }

    #[test]
    fn same_episode_reload_uses_time_pos() {
        assert_eq!(
            start_position_ms(false, None, true, false, 60_000, Some(42_000)),
            Some(42_000)
        );
        assert_eq!(
            start_position_ms(false, None, true, false, 60_000, None),
            None
        );
    }

    #[test]
    fn start_command_uses_float_seconds() {
        assert_eq!(start_command_ms(0), vec!["set", "start", "0.0"]);
        assert_eq!(start_command_ms(12_500), vec!["set", "start", "12.5"]);
        assert_eq!(start_command_ms(90_000), vec!["set", "start", "90.0"]);
    }

    #[test]
    fn online_headers_are_passed_and_escaped() {
        let source = vec![("Referer".to_owned(), "https://s".to_owned())];
        let video = vec![
            ("User-Agent".to_owned(), "a,b".to_owned()),
            ("user-agent".to_owned(), "dup".to_owned()),
        ];
        assert_eq!(
            http_header_fields(true, Some(&video), &source).as_deref(),
            Some("User-Agent: a\\,b")
        );
        assert_eq!(
            http_header_fields(true, None, &source).as_deref(),
            Some("Referer: https://s")
        );
    }

    #[test]
    fn offline_and_local_never_set_headers() {
        let source = vec![("Referer".to_owned(), "x".to_owned())];
        assert_eq!(http_header_fields(false, None, &source), None);
    }

    #[test]
    fn episode_number_text_drops_point_zero_and_pads() {
        assert_eq!(episode_number_text(1.0), "01");
        assert_eq!(episode_number_text(1.5), "1.5");
        assert_eq!(episode_number_text(12.0), "12");
        assert_eq!(episode_number_text(0.0), "00");
    }

    #[test]
    fn media_title_format_and_number_property() {
        let t = media_title("Show", 1.5, "Pilot");
        assert_eq!(t.force_media_title, "Show - E1.5 - Pilot");
        assert_eq!(t.episode_title, "Pilot");
        assert_eq!(t.episode_number, 1.5);
        assert_eq!(
            media_title("Show", 3.0, "x").force_media_title,
            "Show - E03 - x"
        );
    }

    #[test]
    fn on_device_episodes_use_their_path_as_is() {
        assert_eq!(on_device_path(true, false, "/a.mkv"), Some("/a.mkv"));
        assert_eq!(on_device_path(false, true, "/a.mkv"), Some("/a.mkv"));
        assert_eq!(on_device_path(false, false, "https://x/a"), None);
    }

    struct Recorder {
        calls: Vec<&'static str>,
        /// Set to `true` after the step with this 1-based number has run.
        exit_after: Option<usize>,
        exiting: std::rc::Rc<std::cell::Cell<bool>>,
    }

    impl Recorder {
        fn new(exit_after: Option<usize>) -> Self {
            Self {
                calls: Vec::new(),
                exit_after,
                exiting: std::rc::Rc::new(std::cell::Cell::new(false)),
            }
        }

        fn step(&mut self, name: &'static str) {
            self.calls.push(name);
            if self.exit_after == Some(self.calls.len()) {
                self.exiting.set(true);
            }
        }
    }

    impl FileLoadedSteps for Recorder {
        fn mpv_options(&mut self) {
            self.step("mpv_options");
        }
        fn media_title(&mut self) {
            self.step("media_title");
        }
        fn chapters(&mut self) {
            self.step("chapters");
        }
        fn tracks(&mut self) {
            self.step("tracks");
        }
        fn skip_intro(&mut self) {
            self.step("skip_intro");
        }
    }

    #[test]
    fn file_loaded_runs_steps_in_documented_order() {
        let mut rec = Recorder::new(None);
        run_file_loaded(&mut rec, || false);
        assert_eq!(
            rec.calls,
            vec![
                "mpv_options",
                "media_title",
                "chapters",
                "tracks",
                "skip_intro"
            ]
        );
    }

    #[test]
    fn file_loaded_does_nothing_while_exiting() {
        let mut rec = Recorder::new(None);
        run_file_loaded(&mut rec, || true);
        assert!(rec.calls.is_empty());
    }

    #[test]
    fn file_loaded_stops_when_exit_starts_mid_way() {
        let mut rec = Recorder::new(Some(2));
        let flag = rec.exiting.clone();
        run_file_loaded(&mut rec, || flag.get());
        assert_eq!(rec.calls, vec!["mpv_options", "media_title"]);
    }
}
