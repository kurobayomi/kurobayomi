//! ffmpeg remux and ffprobe duration (T-24a-018, T-24a-019, T-24a-020, T-24a-021; behaviour 5.7, design 7).
//!
//! The argv is built as `Vec<OsString>` and passed to the child without a shell, so no quoting is needed. Every stream
//! is copied, never re-encoded. Progress comes from `-progress pipe:1`: `out_time_us` lines are turned into a percent
//! and published at most every 100 ms.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, error};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::anime::model::{Track, Video};
use crate::anime::progress::{ProgressCell, ffmpeg_percent};
use crate::error::DownloadError;
use crate::redact::{redact_line};

/// Binary names on `PATH`. The binary is the contract (design 2).
pub const FFMPEG: &str = "ffmpeg";
pub const FFPROBE: &str = "ffprobe";
/// `-stats_period` passed to ffmpeg (design 7).
pub const STATS_PERIOD: &str = "0.5";
/// Shortest gap between two progress publishes (T-24a-019).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Header names must be HTTP token characters (RFC 9110). Anything else is dropped.
fn is_token(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Keeps the header pairs ffmpeg can take (T-24a-070). A name that is not a token is dropped with a debug log. From a
/// value, CR, LF and NUL are removed (E-F12, E-F13), so an injected header cannot end the line early.
pub fn sanitize_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            if !is_token(name) {
                debug!(target: "kuro_downloads", header = %name, "header dropped: name is not a token");
                return None;
            }
            let value = value.chars().filter(|c| !matches!(c, '\r' | '\n' | '\0')).collect();
            Some((name.clone(), value))
        })
        .collect()
}

/// One ffmpeg stderr line. Lines at warning level and above are logged at error level, with URLs redacted (behaviour 5.7,
/// T-24a-060). Other lines are not logged.
pub fn log_ffmpeg_line(line: &str) {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("warning") || lower.contains("fatal") {
        error!(target: "kuro_downloads", ffmpeg = %redact_line(line), "ffmpeg reported a problem");
    }
}

/// Header pairs as one `-headers` value: `Name: value\r\n` per pair, in order, after [`sanitize_headers`].
pub fn header_option(headers: &[(String, String)]) -> String {
    sanitize_headers(headers).iter().map(|(name, value)| format!("{name}: {value}\r\n")).collect()
}

fn is_http(url: &str) -> bool {
    url.starts_with("http")
}

/// The headers an input gets: the video's own when present, else the source's (behaviour 5.7 step 1).
fn effective_headers<'a>(video: &'a Video, source_headers: &'a [(String, String)]) -> &'a [(String, String)] {
    video.headers.as_deref().unwrap_or(source_headers)
}

fn push(args: &mut Vec<OsString>, value: impl Into<OsString>) {
    args.push(value.into());
}

/// `-headers <value>` before an `http` input. Nothing is added for a local file, or when there are no headers.
fn push_input_headers(args: &mut Vec<OsString>, url: &str, header_arg: &str) {
    if is_http(url) && !header_arg.is_empty() {
        push(args, "-headers");
        push(args, header_arg);
    }
}

/// `-i <url>` for one subtitle or audio track, with its headers (T-24a-021).
fn push_track_input(args: &mut Vec<OsString>, track: &Track, header_arg: &str) {
    push_input_headers(args, &track.url, header_arg);
    push(args, "-i");
    push(args, &track.url);
}

/// Builds the remux argv in the order of design 7 and behaviour 5.7.1. Input 0 is the video, then the subtitle inputs,
/// then the audio inputs. Audio maps use `i + 1 + subtitles`, subtitle maps use `i + 1`.
pub fn build_remux_args(video: &Video, source_headers: &[(String, String)], out: &Path) -> Vec<OsString> {
    let header_arg = header_option(effective_headers(video, source_headers));
    let mut args: Vec<OsString> = Vec::new();
    push(&mut args, "-progress");
    push(&mut args, "pipe:1");
    push(&mut args, "-nostats");
    push(&mut args, "-stats_period");
    push(&mut args, STATS_PERIOD);

    push_input_headers(&mut args, &video.url, &header_arg);
    for (key, value) in &video.stream_args {
        push(&mut args, format!("-{key}"));
        push(&mut args, value);
    }
    push(&mut args, "-i");
    push(&mut args, &video.url);
    for track in &video.subtitle_tracks {
        push_track_input(&mut args, track, &header_arg);
    }
    for track in &video.audio_tracks {
        push_track_input(&mut args, track, &header_arg);
    }

    push(&mut args, "-map");
    push(&mut args, "0:v");
    let subtitles = video.subtitle_tracks.len();
    for i in 0..video.audio_tracks.len() {
        push(&mut args, "-map");
        push(&mut args, format!("{}:a", i + 1 + subtitles));
    }
    push(&mut args, "-map");
    push(&mut args, "0:a?");
    for i in 0..subtitles {
        push(&mut args, "-map");
        push(&mut args, format!("{}:s", i + 1));
    }
    push(&mut args, "-map");
    push(&mut args, "0:s?");
    push(&mut args, "-map");
    push(&mut args, "0:t?");

    for value in ["-f", "matroska", "-c:a", "copy", "-c:v", "copy", "-c:s", "copy"] {
        push(&mut args, value);
    }
    for (i, track) in video.subtitle_tracks.iter().enumerate() {
        push(&mut args, format!("-metadata:s:s:{i}"));
        push(&mut args, format!("title={}", track.lang));
    }
    for (i, track) in video.audio_tracks.iter().enumerate() {
        push(&mut args, format!("-metadata:s:a:{i}"));
        push(&mut args, format!("title={}", track.lang));
    }
    for (key, value) in &video.video_args {
        push(&mut args, format!("-{key}"));
        push(&mut args, value);
    }
    push(&mut args, out);
    push(&mut args, "-y");
    args
}

/// ffprobe argv for the duration of a stream: `[-headers ..] -v quiet -show_entries format=duration ... <url>`.
pub fn build_ffprobe_args(video_url: &str, headers: &[(String, String)]) -> Vec<OsString> {
    let header_arg = header_option(headers);
    let mut args: Vec<OsString> = Vec::new();
    push_input_headers(&mut args, video_url, &header_arg);
    // errors only, so a failure names its reason ("403 Forbidden", "not in allowed_segment_extensions") instead of nothing
    for value in ["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"] {
        push(&mut args, value);
    }
    push(&mut args, video_url);
    args
}

/// Input options an HLS playlist needs: sites hide their video segments behind image names (`000.jpg`), which ffmpeg
/// 7.1 and later refuse unless `extension_picky` is off. Older ffmpeg has no such option (and no such check), so it is
/// passed only when the installed ffprobe knows it.
pub fn hls_input_options(ffprobe: &OsStr, url: &str) -> Vec<(String, String)> {
    let is_playlist = url.split(['?', '#']).next().is_some_and(|path| path.ends_with(".m3u8"));
    if is_playlist && knows_extension_picky(ffprobe) { vec![("extension_picky".to_owned(), "0".to_owned())] } else { Vec::new() }
}

/// `args` with `options` inserted before every `-i` whose input is an HLS playlist: the video and each separate
/// subtitle or audio track (their playlists hide segments the same way).
pub fn with_playlist_options(args: Vec<OsString>, options: &[(String, String)]) -> Vec<OsString> {
    if options.is_empty() {
        return args;
    }
    let is_playlist = |arg: &OsString| arg.to_str().and_then(|a| a.split(['?', '#']).next()).is_some_and(|p| p.ends_with(".m3u8"));
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-i" && args.get(i + 1).is_some_and(is_playlist) {
            for (key, value) in options {
                out.push(format!("-{key}").into());
                out.push(value.into());
            }
        }
        out.push(args[i].clone());
        i += 1;
    }
    out
}

/// Whether this ffprobe's HLS demuxer has `extension_picky`. Asked once per program and run.
fn knows_extension_picky(ffprobe: &OsStr) -> bool {
    static KNOWS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<OsString, bool>>> = std::sync::OnceLock::new();
    let mut known = KNOWS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    *known.entry(ffprobe.to_owned()).or_insert_with(|| {
        kuro_platform::process::no_console(&mut std::process::Command::new(ffprobe))
            .args(["-hide_banner", "-h", "demuxer=hls"])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("extension_picky"))
    })
}

/// The duration in whole seconds (behaviour 5.7: `toFloatOrNull()` then `toLong()`). Anything unparsable, such as
/// `N/A`, is 0, which keeps progress indeterminate.
pub fn parse_duration(output: &str) -> i64 {
    match output.trim().parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds > 0.0 => seconds.trunc() as i64,
        _ => 0,
    }
}

/// One line of `-progress pipe:1` output that matters to the download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressTick {
    /// `out_time_us` in microseconds. The Kotlin `time` is milliseconds, so the Rust port divides by 1_000_000.
    OutTimeMicros(i64),
    /// `progress=end`: the run is finished.
    Ended,
}

/// Parses one `key=value` line. Anything else, including `N/A` values, is `None`.
pub fn parse_progress_line(line: &str) -> Option<ProgressTick> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        "out_time_us" => value.trim().parse::<i64>().ok().map(ProgressTick::OutTimeMicros),
        "progress" if value.trim() == "end" => Some(ProgressTick::Ended),
        _ => None,
    }
}

fn spawn_error(err: io::Error) -> DownloadError {
    if err.kind() == io::ErrorKind::NotFound { DownloadError::FfmpegMissing } else { DownloadError::Io(err) }
}

/// Runs ffprobe and returns the duration in whole seconds. A non-zero exit is `Ffprobe` with the first line of its
/// stderr (retryable, behaviour 5.7 step 5).
pub async fn probe_duration(program: &OsStr, video_url: &str, headers: &[(String, String)]) -> Result<i64, DownloadError> {
    let mut args = build_ffprobe_args(video_url, headers);
    // the input options go before the input, which is the last argument
    let at = args.len() - 1;
    for (key, value) in hls_input_options(program, video_url).into_iter().rev() {
        args.insert(at, value.into());
        args.insert(at, format!("-{key}").into());
    }
    let mut command = Command::new(program);
    kuro_platform::process::no_console(command.as_std_mut());
    let output = command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(spawn_error)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("ffprobe failed").to_owned();
        return Err(DownloadError::Ffprobe(first));
    }
    Ok(parse_duration(&String::from_utf8_lossy(&output.stdout)))
}

/// Runs an ffmpeg remux and publishes its progress. Returns `Cancelled` after killing the child when `cancel` fires.
/// On cancel or failure the partial `output` is removed. `duration_secs` of 0 disables the percent (indeterminate).
pub async fn run_remux(
    program: &OsStr,
    args: &[OsString],
    duration_secs: i64,
    output: &Path,
    progress: &ProgressCell,
    cancel: &CancellationToken,
) -> Result<(), DownloadError> {
    // A run cancelled before the spawn must not start ffmpeg, and must not touch `output`: a newer run may own it now.
    if cancel.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    let mut command = Command::new(program);
    kuro_platform::process::no_console(command.as_std_mut());
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(spawn_error)?;
    let Some(stdout) = child.stdout.take() else {
        return Err(DownloadError::Internal("ffmpeg stdout was not piped".into()));
    };
    // stderr is drained while ffmpeg runs, so a full pipe never blocks it. It ends when ffmpeg exits.
    let stderr_log = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log_ffmpeg_line(&line);
            }
        })
    });
    let (tx, mut ticks) = mpsc::unbounded_channel::<ProgressTick>();
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(tick) = parse_progress_line(&line) {
                if tx.send(tick).is_err() {
                    break;
                }
            }
        }
    });

    let mut last_publish: Option<Instant> = None;
    let cancelled = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break true,
            tick = ticks.recv() => match tick {
                Some(ProgressTick::OutTimeMicros(micros)) => {
                    let now = Instant::now();
                    if last_publish.is_none_or(|t| now.duration_since(t) >= PROGRESS_INTERVAL) {
                        last_publish = Some(now);
                        progress.set(i64::from(ffmpeg_percent(micros / 1_000_000, duration_secs)));
                    }
                }
                Some(ProgressTick::Ended) => {}
                None => break false,
            }
        }
    };

    if cancelled {
        let _ = child.start_kill();
        let _ = child.wait().await;
        reader.abort();
        if let Some(log) = &stderr_log {
            log.abort();
        }
        let _ = std::fs::remove_file(output);
        return Err(DownloadError::Cancelled);
    }
    let status = child.wait().await?;
    if let Some(log) = stderr_log {
        let _ = log.await;
    }
    // An exit of 0 with an empty file is a failure too (T-24a-067, E-F07).
    if status.success() && std::fs::metadata(output).is_ok_and(|m| m.len() > 0) {
        return Ok(());
    }
    let _ = std::fs::remove_file(output);
    Err(DownloadError::FfmpegFailed)
}

/// Startup check (T-24a-067): `ffmpeg` and `ffprobe` must both be on `path_var`, else `FfmpegMissing`.
pub fn check_binaries(path_var: &OsStr) -> Result<(), DownloadError> {
    let found = |program| kuro_platform::process::find_program(program, path_var).is_some();
    if found(FFMPEG) && found(FFPROBE) {
        Ok(())
    } else {
        Err(DownloadError::FfmpegMissing)
    }
}

/// [`check_binaries`] against the process `PATH`.
pub fn check_on_path() -> Result<(), DownloadError> {
    check_binaries(&std::env::var_os("PATH").unwrap_or_default())
}

/// Where a user can get ffmpeg. Shown as the link of the Settings row when the startup check fails (E-F08).
pub const FFMPEG_HELP_URL: &str = "https://ffmpeg.org/download.html";

/// The Settings row text for a failed startup check: what is missing and where to get it (T-24a-067).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FfmpegNotice {
    pub message: &'static str,
    pub help_url: &'static str,
}

/// The startup check as a Settings notice: `None` when `ffmpeg` and `ffprobe` are both on `path_var`.
pub fn startup_notice(path_var: &OsStr) -> Option<FfmpegNotice> {
    check_binaries(path_var)
        .err()
        .map(|_| FfmpegNotice { message: "ffmpeg is not installed", help_url: FFMPEG_HELP_URL })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file name of a program on this system (`ffmpeg.exe` on Windows).
    fn program_file(name: &str) -> String {
        format!("{name}{}", std::env::consts::EXE_SUFFIX)
    }
    use crate::anime::model::VideoStatus;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn track(url: &str, lang: &str) -> Track {
        Track { url: url.into(), lang: lang.into() }
    }

    fn source_headers() -> Vec<(String, String)> {
        vec![("Referer".into(), "https://example.org/".into()), ("User-Agent".into(), "UA/1.0".into())]
    }

    fn topic_video() -> Video {
        Video {
            url: "https://cdn.example/v.m3u8".into(),
            title: "t".into(),
            headers: None,
            subtitle_tracks: vec![track("https://cdn.example/en.vtt", "English")],
            audio_tracks: vec![track("https://cdn.example/ja.m3u8", "Japanese")],
            stream_args: vec![("allowed_extensions".into(), "ALL".into())],
            video_args: vec![],
            initialized: true,
            status: VideoStatus::Ready,
        }
    }

    const HDR: &str = "Referer: https://example.org/\r\nUser-Agent: UA/1.0\r\n";

    #[test]
    fn golden_argv_with_subtitle_and_audio_matches_the_topic_example() {
        let out = Path::new("/data/dl/Example (EN)/Show/Scan_Episode 1_tmp/Episode 1.tmp");
        let args = build_remux_args(&topic_video(), &source_headers(), out);
        let expected = os(&[
            "-progress", "pipe:1", "-nostats", "-stats_period", "0.5",
            "-headers", HDR, "-allowed_extensions", "ALL", "-i", "https://cdn.example/v.m3u8",
            "-headers", HDR, "-i", "https://cdn.example/en.vtt",
            "-headers", HDR, "-i", "https://cdn.example/ja.m3u8",
            "-map", "0:v", "-map", "2:a", "-map", "0:a?", "-map", "1:s", "-map", "0:s?", "-map", "0:t?",
            "-f", "matroska", "-c:a", "copy", "-c:v", "copy", "-c:s", "copy",
            "-metadata:s:s:0", "title=English", "-metadata:s:a:0", "title=Japanese",
            "/data/dl/Example (EN)/Show/Scan_Episode 1_tmp/Episode 1.tmp", "-y",
        ]);
        assert_eq!(args, expected);
    }

    #[test]
    fn video_only_has_no_extra_inputs_or_maps() {
        let video = Video { url: "https://cdn.example/v.m3u8".into(), ..Default::default() };
        let args = build_remux_args(&video, &source_headers(), Path::new("/o.tmp"));
        let expected = os(&[
            "-progress", "pipe:1", "-nostats", "-stats_period", "0.5",
            "-headers", HDR, "-i", "https://cdn.example/v.m3u8",
            "-map", "0:v", "-map", "0:a?", "-map", "0:s?", "-map", "0:t?",
            "-f", "matroska", "-c:a", "copy", "-c:v", "copy", "-c:s", "copy",
            "/o.tmp", "-y",
        ]);
        assert_eq!(args, expected);
    }

    #[test]
    fn audio_only_maps_to_input_one() {
        let video = Video {
            url: "https://cdn.example/v.m3u8".into(),
            audio_tracks: vec![track("https://cdn.example/ja.m3u8", "Japanese")],
            ..Default::default()
        };
        let args = build_remux_args(&video, &[], Path::new("/o.tmp"));
        let text: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let at = text.iter().position(|a| a == "-map").unwrap();
        assert_eq!(text[at + 1], "0:v");
        assert_eq!(text[at + 3], "1:a", "first audio input is input 1 when there are no subtitles");
        assert!(text.contains(&"-metadata:s:a:0".to_string()));
        assert!(!text.iter().any(|a| a.ends_with(":s") && a.starts_with("1")));
    }

    #[test]
    fn local_subtitle_gets_no_header_option() {
        let video = Video {
            url: "https://cdn.example/v.m3u8".into(),
            subtitle_tracks: vec![track("/home/me/en.srt", "English")],
            ..Default::default()
        };
        let args = build_remux_args(&video, &source_headers(), Path::new("/o.tmp"));
        let text: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let at = text.iter().position(|a| a == "/home/me/en.srt").unwrap();
        assert_eq!(text[at - 1], "-i");
        assert_eq!(text[at - 2], "https://cdn.example/v.m3u8", "the local track has no -headers before it");
        assert_eq!(text.iter().filter(|a| *a == "-headers").count(), 1, "only the http video input has headers");
    }

    #[test]
    fn video_headers_win_over_source_headers_and_some_empty_stays_empty() {
        let mut video = Video { url: "https://cdn.example/v.m3u8".into(), ..Default::default() };
        video.headers = Some(vec![("Cookie".into(), "a=b".into())]);
        let args = build_remux_args(&video, &source_headers(), Path::new("/o"));
        assert!(args.contains(&OsString::from("Cookie: a=b\r\n")));
        assert!(!args.iter().any(|a| a.to_string_lossy().contains("example.org")));

        video.headers = Some(Vec::new());
        let args = build_remux_args(&video, &source_headers(), Path::new("/o"));
        assert!(!args.contains(&OsString::from("-headers")), "an empty header list adds no option");
    }

    #[test]
    fn crlf_inside_header_values_is_stripped() {
        let headers = vec![("X-Evil".into(), "a\r\nInjected: 1".into())];
        assert_eq!(header_option(&headers), "X-Evil: aInjected: 1\r\n");
    }

    #[test]
    fn video_and_stream_args_are_dashed_pairs() {
        let video = Video {
            url: "https://cdn.example/v.m3u8".into(),
            video_args: vec![("preset".into(), "fast".into())],
            ..Default::default()
        };
        let args = build_remux_args(&video, &[], Path::new("/o"));
        let tail = &args[args.len() - 4..];
        assert_eq!(tail, os(&["-preset", "fast", "/o", "-y"]).as_slice());
    }

    #[test]
    fn ffprobe_argv_has_header_prefix_for_http_only() {
        let http = build_ffprobe_args("https://cdn.example/v.m3u8", &source_headers());
        assert_eq!(http[0], "-headers");
        assert_eq!(http[1], HDR);
        assert_eq!(http[2], "-v");
        let local = build_ffprobe_args("/home/me/v.mkv", &source_headers());
        assert_eq!(local[0], "-v");
        assert_eq!(local.last().unwrap(), "/home/me/v.mkv");
    }

    #[test]
    fn duration_parses_floats_and_treats_junk_as_zero() {
        assert_eq!(parse_duration("1421.504000\n"), 1421);
        assert_eq!(parse_duration("N/A"), 0);
        assert_eq!(parse_duration(""), 0);
        assert_eq!(parse_duration("-5"), 0);
    }

    #[test]
    fn progress_lines_parse_out_time_and_end() {
        assert_eq!(parse_progress_line("out_time_us=4800000"), Some(ProgressTick::OutTimeMicros(4_800_000)));
        assert_eq!(parse_progress_line("progress=end"), Some(ProgressTick::Ended));
        assert_eq!(parse_progress_line("progress=continue"), None);
        assert_eq!(parse_progress_line("out_time_us=N/A"), None);
        assert_eq!(parse_progress_line("frame=120"), None);
        assert_eq!(parse_progress_line("garbage line"), None);
    }

    /// Script tests fork children. A script written by one test while another forks can fail to run with "Text file
    /// busy", so these tests run one at a time.
    static SPAWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn spawn_guard() -> tokio::sync::MutexGuard<'static, ()> {
        SPAWN_LOCK.lock().await
    }

    /// Writes an executable shell script into `dir` and returns its path.
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_ffmpeg_reports_progress_and_gets_the_argv_in_order() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv.log");
        let out = dir.path().join("Episode 1.tmp");
        let body = format!(
            "printf '%s\\n' \"$@\" > '{}'\nprintf 'video' > '{}'\nprintf 'out_time_us=5000000\\n'\nsleep 0.3\nprintf 'out_time_us=10000000\\nprogress=end\\n'",
            log.display(),
            out.display()
        );
        let bin = script(dir.path(), "ffmpeg", &body);
        let args = build_remux_args(&Video { url: "/local/v.mkv".into(), ..Default::default() }, &[], &out);
        let progress = ProgressCell::new();
        let cancel = CancellationToken::new();
        run_remux(bin.as_os_str(), &args, 10, &out, &progress, &cancel).await.unwrap();
        assert_eq!(progress.get(), 100, "the second tick is published after the throttle window");
        let written = std::fs::read_to_string(&log).unwrap();
        assert!(written.starts_with("-progress\npipe:1\n-nostats"));
        assert!(written.ends_with("-y\n"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_ffmpeg_is_an_error_and_removes_the_partial_output() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), "ffmpeg", "for last; do :; done\nprintf partial > \"$last\"\nexit 1");
        let out = dir.path().join("Episode 1.tmp");
        let args = vec![out.clone().into_os_string()];
        let err = run_remux(bin.as_os_str(), &args, 0, &out, &ProgressCell::new(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::FfmpegFailed));
        assert_eq!(err.to_string(), "Error in ffmpeg!");
        assert!(!out.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_the_child_and_reports_cancelled() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), "ffmpeg", "sleep 5");
        let out = dir.path().join("x.tmp");
        std::fs::write(&out, b"half").unwrap();
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            trigger.cancel();
        });
        let started = Instant::now();
        let err = run_remux(bin.as_os_str(), &[], 0, &out, &ProgressCell::new(), &cancel).await.unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(4), "the child is killed, not waited for");
        assert!(!out.exists());
    }

    #[tokio::test]
    async fn a_missing_binary_is_ffmpeg_missing() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let err = run_remux(OsStr::new("/nonexistent/ffmpeg-x"), &[], 0, &dir.path().join("o"), &ProgressCell::new(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::FfmpegMissing));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ffprobe_failure_carries_the_first_stderr_line() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), "ffprobe", "echo 'Invalid data found' >&2\necho 'second' >&2\nexit 1");
        let err = probe_duration(bin.as_os_str(), "https://cdn.example/v.m3u8", &[]).await.unwrap_err();
        assert_eq!(err.to_string(), "Invalid data found");
    }

    #[test]
    fn every_playlist_input_gets_the_options_and_a_file_input_does_not() {
        let args: Vec<OsString> = ["-i", "https://a/v.m3u8", "-i", "https://a/sub.vtt", "-i", "https://a/audio.m3u8?t=1", "out.mkv"].map(OsString::from).to_vec();
        let opts = vec![("extension_picky".to_owned(), "0".to_owned())];
        let out: Vec<String> = with_playlist_options(args, &opts).into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(out, ["-extension_picky", "0", "-i", "https://a/v.m3u8", "-i", "https://a/sub.vtt", "-extension_picky", "0", "-i", "https://a/audio.m3u8?t=1", "out.mkv"]);
    }

    #[cfg(unix)]
    #[test]
    fn only_a_playlist_gets_the_lenient_segment_option_and_only_from_an_ffprobe_that_knows_it() {
        let dir = tempfile::tempdir().unwrap();
        let new = script(dir.path(), "ffprobe-new", "echo '  -extension_picky   <boolean>'");
        let old = script(dir.path(), "ffprobe-old", "echo '  -allowed_extensions <string>'");
        let lenient = vec![("extension_picky".to_owned(), "0".to_owned())];
        assert_eq!(hls_input_options(new.as_os_str(), "https://cdn/x/master.m3u8?token=1"), lenient);
        assert!(hls_input_options(new.as_os_str(), "https://cdn/x/video.mp4").is_empty());
        assert!(hls_input_options(old.as_os_str(), "https://cdn/x/master.m3u8").is_empty(), "older ffmpeg has no such option");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ffprobe_na_is_duration_zero() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), "ffprobe", "echo N/A");
        assert_eq!(probe_duration(bin.as_os_str(), "/v.mkv", &[]).await.unwrap(), 0);
        let bin = script(dir.path(), "ffprobe2", "echo 1421.504000");
        assert_eq!(probe_duration(bin.as_os_str(), "/v.mkv", &[]).await.unwrap(), 1421);
    }

    #[test]
    fn invalid_header_names_are_dropped_and_values_lose_control_bytes() {
        let headers = vec![
            ("Referer".to_owned(), "https://a.test/\r\nX-Evil: 1".to_owned()),
            ("Bad Name".to_owned(), "x".to_owned()),
            ("Cookie:".to_owned(), "x".to_owned()),
            ("".to_owned(), "x".to_owned()),
            ("X-Nul".to_owned(), "a\0b".to_owned()),
        ];
        let kept = sanitize_headers(&headers);
        assert_eq!(
            kept,
            vec![("Referer".to_owned(), "https://a.test/X-Evil: 1".to_owned()), ("X-Nul".to_owned(), "ab".to_owned())]
        );
        assert_eq!(header_option(&headers), "Referer: https://a.test/X-Evil: 1\r\nX-Nul: ab\r\n");
    }

    #[test]
    fn binaries_are_found_on_the_given_path_only() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([dir.path(), other.path()]).unwrap();
        assert!(matches!(check_binaries(&path), Err(DownloadError::FfmpegMissing)));
        std::fs::write(other.path().join(program_file(FFMPEG)), b"x").unwrap();
        assert!(matches!(check_binaries(&path), Err(DownloadError::FfmpegMissing)), "ffprobe is still missing");
        std::fs::write(dir.path().join(program_file(FFPROBE)), b"x").unwrap();
        check_binaries(&path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_empty_output_after_exit_zero_is_an_ffmpeg_error() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), "ffmpeg", "exit 0");
        let out = dir.path().join("Episode 1.tmp");
        std::fs::write(&out, b"").unwrap();
        let progress = ProgressCell::new();
        let cancel = CancellationToken::new();
        let err = run_remux(bin.as_os_str(), &[], 0, &out, &progress, &cancel).await.unwrap_err();
        assert_eq!(err.to_string(), "Error in ffmpeg!");
        assert!(!out.exists(), "the empty partial file is removed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_cancelled_before_the_spawn_starts_no_ffmpeg_and_keeps_the_partial() {
        let _guard = spawn_guard().await;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("ffmpeg.log");
        let bin = script(dir.path(), "ffmpeg", &format!("echo run >> '{}'", log.display()));
        let out = dir.path().join("x.tmp");
        std::fs::write(&out, b"newer run").unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = run_remux(bin.as_os_str(), &[], 0, &out, &ProgressCell::new(), &cancel).await.unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
        assert!(!log.exists(), "ffmpeg was not started");
        assert_eq!(std::fs::read(&out).unwrap(), b"newer run", "the partial of a newer run is untouched");
    }

    #[test]
    fn the_startup_notice_links_to_the_download_page_only_when_a_binary_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        let notice = startup_notice(&path).expect("both binaries are missing");
        assert_eq!(notice.message, "ffmpeg is not installed");
        assert_eq!(notice.help_url, FFMPEG_HELP_URL);
        std::fs::write(dir.path().join(program_file(FFMPEG)), b"x").unwrap();
        std::fs::write(dir.path().join(program_file(FFPROBE)), b"x").unwrap();
        assert_eq!(startup_notice(&path), None);
    }
}
