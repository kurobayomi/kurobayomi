//! Frame extraction with `ffprobe` and `ffmpeg` (area:14 section 14, T-14a-026 and T-14a-049).
//!
//! The runner executes both tools as argument vectors, never through a shell. One frame is written to
//! `tmp_<random><suffix>` in the app temp directory, held by a guard that removes it when the extraction ends, fails
//! or is dropped (E-ANIME-7). A child process is killed and reaped when its guard drops, so abandoning an in-flight
//! extraction leaves neither a process nor a temp file (RC-7). Extractions never run in parallel (E-MEDIA-8).
//!
//! The runner is synchronous: the episode listing that calls it is already on a blocking thread, and a timeout is a
//! poll of `try_wait` followed by a kill.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long `ffprobe` may take (E-MEDIA-5).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long one `ffmpeg` frame extraction may take (E-MEDIA-5).
pub const EXTRACT_TIMEOUT: Duration = Duration::from_secs(120);

/// One extraction at a time per process.
static SERIAL: Mutex<()> = Mutex::new(());
/// The missing-tool warning is logged once per process (E-MEDIA-1).
static WARNED_MISSING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// `ffprobe` printed something that is not a number (E-MEDIA-2).
    #[error("ffprobe gave no duration: {0:?}")]
    BadDuration(String),
    /// The tool ran past its timeout and was killed (E-MEDIA-5).
    #[error("{0} timed out")]
    Timeout(&'static str),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// An external tool: the program and the arguments that always come first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    program: OsString,
    args: Vec<OsString>,
}

impl Tool {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self { program: program.into(), args: Vec::new() }
    }

    /// Arguments placed before the ones the runner adds.
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// The program `name` on `PATH` (`name.exe` on Windows).
    pub fn on_path(name: &str) -> Option<Self> {
        Self::in_path_var(name, std::env::var_os("PATH")?.as_os_str())
    }

    fn in_path_var(name: &str, path: &std::ffi::OsStr) -> Option<Self> {
        kuro_platform::process::find_program(name, path).map(Self::new)
    }
}

/// A child process that is killed and reaped when the guard drops.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs `tool` with `args` and returns its standard output. Standard error is discarded. A child still running
/// after `timeout` is killed.
fn run(tool: &Tool, args: &[OsString], timeout: Duration, label: &'static str) -> Result<Vec<u8>, MediaError> {
    let mut child = ChildGuard(
        kuro_platform::process::no_console(&mut Command::new(&tool.program))
            .args(&tool.args)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let started = Instant::now();
    loop {
        if child.0.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() >= timeout {
            return Err(MediaError::Timeout(label));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.0.stdout.take() {
        io::Read::read_to_end(&mut stdout, &mut out)?;
    }
    Ok(out)
}

/// The second of the frame: half the duration, truncated to whole seconds (`duration.toInt() / 2`, E-MEDIA-7).
pub fn frame_position(duration_secs: f64) -> i64 {
    (duration_secs as i64) / 2
}

/// Makes one JPEG from the middle of a video file.
#[derive(Clone, Debug)]
pub struct FrameExtractor {
    ffprobe: Option<Tool>,
    ffmpeg: Option<Tool>,
    temp_dir: PathBuf,
    probe_timeout: Duration,
    extract_timeout: Duration,
}

impl FrameExtractor {
    /// The tools found on `PATH`; when one is missing every `frame` call gives no image (E-MEDIA-1).
    pub fn from_path(temp_dir: impl Into<PathBuf>) -> Self {
        Self::new(Tool::on_path("ffprobe"), Tool::on_path("ffmpeg"), temp_dir)
    }

    pub fn new(ffprobe: Option<Tool>, ffmpeg: Option<Tool>, temp_dir: impl Into<PathBuf>) -> Self {
        Self { ffprobe, ffmpeg, temp_dir: temp_dir.into(), probe_timeout: PROBE_TIMEOUT, extract_timeout: EXTRACT_TIMEOUT }
    }

    pub fn with_timeouts(mut self, probe: Duration, extract: Duration) -> Self {
        self.probe_timeout = probe;
        self.extract_timeout = extract;
        self
    }

    /// The duration of `input` in seconds, from `ffprobe`.
    pub fn probe_duration(&self, ffprobe: &Tool, input: &Path) -> Result<f64, MediaError> {
        let args: Vec<OsString> = ["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"]
            .into_iter()
            .map(OsString::from)
            .chain([input.as_os_str().to_owned()])
            .collect();
        let out = run(ffprobe, &args, self.probe_timeout, "ffprobe")?;
        let text = String::from_utf8_lossy(&out).trim().to_owned();
        match text.parse::<f64>() {
            Ok(seconds) if seconds.is_finite() => Ok(seconds),
            _ => Err(MediaError::BadDuration(text)),
        }
    }

    /// The JPEG bytes of the middle frame of `input`, or `None` when no tool is installed or ffmpeg wrote nothing
    /// (an empty output is not an error, E-MEDIA-4). The temp output is gone when this returns, whatever the result.
    pub fn frame(&self, input: &Path) -> Result<Option<Vec<u8>>, MediaError> {
        let (Some(ffprobe), Some(ffmpeg)) = (&self.ffprobe, &self.ffmpeg) else {
            if !WARNED_MISSING.swap(true, Ordering::Relaxed) {
                tracing::warn!("ffmpeg or ffprobe is not installed: video thumbnails and covers are skipped");
            }
            return Ok(None);
        };
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let second = frame_position(self.probe_duration(ffprobe, input)?);
        let output = temp_output(&self.temp_dir, "-frame.jpg")?;
        let args: Vec<OsString> = vec![
            "-ss".into(),
            second.to_string().into(),
            "-i".into(),
            input.as_os_str().to_owned(),
            "-frames:v".into(),
            "1".into(),
            "-update".into(),
            "true".into(),
            output.path().as_os_str().to_owned(),
            "-y".into(),
        ];
        run(ffmpeg, &args, self.extract_timeout, "ffmpeg")?;
        let bytes = std::fs::read(output.path())?;
        Ok((!bytes.is_empty()).then_some(bytes))
    }

    /// `frame` in the shape the episode listing takes.
    pub fn frame_io(&self, input: &Path) -> io::Result<Option<Vec<u8>>> {
        self.frame(input).map_err(io::Error::other)
    }
}

/// A temporary file named `tmp_*<suffix>` in `dir`, removed when dropped.
pub fn temp_output(dir: &Path, suffix: &str) -> io::Result<tempfile::NamedTempFile> {
    tempfile::Builder::new()
        .prefix("tmp_")
        .suffix(suffix)
        .tempfile_in(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_has_the_prefix_and_suffix_and_goes_away_on_drop() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = {
            let guard = temp_output(dir.path(), "-thumb.jpg")?;
            let name = guard
                .path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            assert!(
                name.starts_with("tmp_") && name.ends_with("-thumb.jpg"),
                "{name}"
            );
            guard.path().to_path_buf()
        };
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn a_missing_temp_folder_is_an_error_and_leaves_nothing() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let gone = dir.path().join("missing");
        assert!(temp_output(&gone, ".jpg").is_err());
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    // ---- the runner, with fake tools: `sh <script> <args...>` so no executable file is written ----

    use std::fs;

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> Tool {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        Tool::new("sh").with_args([path])
    }

    /// A fake ffprobe that prints `duration`.
    #[cfg(unix)]
    fn probe_printing(dir: &Path, duration: &str) -> Tool {
        script(dir, "ffprobe.sh", &format!("echo '{duration}'\n"))
    }

    /// A fake ffmpeg: records its arguments, one per line, in `args.txt`, then runs `then`. The output file is `$9`.
    #[cfg(unix)]
    fn ffmpeg_doing(dir: &Path, then: &str) -> Tool {
        let args = dir.join("args.txt");
        script(
            dir,
            "ffmpeg.sh",
            &format!("for a in \"$@\"; do printf '%s\\n' \"$a\" >> '{}'; done\n{then}\n", args.display()),
        )
    }

    #[cfg(unix)]
    fn extractor(probe: Tool, ffmpeg: Tool, temp: &Path) -> FrameExtractor {
        FrameExtractor::new(Some(probe), Some(ffmpeg), temp)
    }

    fn alive(pid: &str) -> bool {
        Path::new(&format!("/proc/{}", pid.trim())).exists()
    }

    #[test]
    fn t_14_media_frame_position_truncates() {
        assert_eq!(frame_position(5.9), 2);
        assert_eq!(frame_position(0.0), 0);
        assert_eq!(frame_position(7.0), 3);
    }

    #[cfg(unix)]
    #[test]
    fn the_frame_is_the_middle_second_and_the_temp_file_is_removed() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let ex = extractor(probe_printing(tools.path(), "9.7"), ffmpeg_doing(tools.path(), "printf 'jpeg' > \"$9\""), temp.path());
        let got = ex.frame(Path::new("/videos/ep.mkv")).map_err(io::Error::other)?;
        assert_eq!(got.as_deref(), Some(&b"jpeg"[..]));
        let args = fs::read_to_string(tools.path().join("args.txt"))?;
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(&args[..4], ["-ss", "4", "-i", "/videos/ep.mkv"]);
        assert_eq!(&args[4..8], ["-frames:v", "1", "-update", "true"]);
        assert!(Path::new(args[8]).file_name().unwrap_or_default().to_string_lossy().starts_with("tmp_"));
        assert_eq!(Path::new(args[8]).parent(), Some(temp.path()));
        assert_eq!(args[9], "-y");
        assert_eq!(fs::read_dir(temp.path())?.count(), 0, "t_14_thumbnail_temp_removed: nothing is left in the temp folder");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn t_14_media_zero_duration_frame_zero() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let ex = extractor(probe_printing(tools.path(), "0"), ffmpeg_doing(tools.path(), "printf 'j' > \"$9\""), temp.path());
        ex.frame(Path::new("v.mkv")).map_err(io::Error::other)?;
        assert_eq!(fs::read_to_string(tools.path().join("args.txt"))?.lines().nth(1), Some("0"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn t_14_media_path_with_quotes_and_spaces() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let ex = extractor(probe_printing(tools.path(), "4"), ffmpeg_doing(tools.path(), "printf 'j' > \"$9\""), temp.path());
        let odd = "/v/My \"Show\" ep 1 ; rm -rf x.mkv";
        ex.frame(Path::new(odd)).map_err(io::Error::other)?;
        assert_eq!(fs::read_to_string(tools.path().join("args.txt"))?.lines().nth(3), Some(odd));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn t_14_media_bad_duration() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let ex = extractor(probe_printing(tools.path(), "N/A"), ffmpeg_doing(tools.path(), "printf 'j' > \"$9\""), temp.path());
        let err = ex.frame(Path::new("v.mkv")).unwrap_err();
        assert!(matches!(err, MediaError::BadDuration(ref t) if t == "N/A"), "{err:?}");
        assert!(!tools.path().join("args.txt").exists(), "ffmpeg never ran");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn t_14_media_empty_output_no_image() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let ex = extractor(probe_printing(tools.path(), "4"), ffmpeg_doing(tools.path(), "exit 1"), temp.path());
        assert_eq!(ex.frame(Path::new("v.mkv")).map_err(io::Error::other)?, None);
        assert_eq!(fs::read_dir(temp.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn t_14_media_missing_tool_skips() -> io::Result<()> {
        let temp = tempfile::tempdir()?;
        let ex = FrameExtractor::new(None, None, temp.path());
        assert_eq!(ex.frame(Path::new("v.mkv")).map_err(io::Error::other)?, None);
        assert_eq!(ex.frame_io(Path::new("v.mkv"))?, None);
        let empty = tempfile::tempdir()?;
        assert!(Tool::in_path_var("ffmpeg", empty.path().as_os_str()).is_none(), "an empty PATH finds nothing");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn t_14_media_timeout_kills_child() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let pid_file = tools.path().join("pid");
        let hang = ffmpeg_doing(tools.path(), &format!("echo $$ > '{}'; exec sleep 30", pid_file.display()));
        let ex = extractor(probe_printing(tools.path(), "4"), hang, temp.path())
            .with_timeouts(Duration::from_secs(5), Duration::from_millis(300));
        let err = ex.frame(Path::new("v.mkv")).unwrap_err();
        assert!(matches!(err, MediaError::Timeout("ffmpeg")), "{err:?}");
        let pid = fs::read_to_string(&pid_file)?;
        assert!(!alive(&pid), "the child is dead");
        assert_eq!(fs::read_dir(temp.path())?.count(), 0, "the temp file is gone");
        Ok(())
    }

    // checks the child process through /proc
    #[cfg(unix)]
    #[test]
    fn t_14_media_drop_cleans_temp() -> io::Result<()> {
        // dropping the child guard of an in-flight extraction kills and reaps the process
        let child = Command::new("sleep").arg("30").spawn()?;
        let pid = child.id().to_string();
        assert!(alive(&pid));
        drop(ChildGuard(child));
        assert!(!alive(&pid));
        // and the temp guard removes its file when dropped mid-way
        let temp = tempfile::tempdir()?;
        let guard = temp_output(temp.path(), "-frame.jpg")?;
        let path = guard.path().to_path_buf();
        assert!(path.exists());
        drop(guard);
        assert!(!path.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn t_14_media_serialized() -> io::Result<()> {
        let tools = tempfile::tempdir()?;
        let temp = tempfile::tempdir()?;
        let log = tools.path().join("log");
        let ffmpeg = ffmpeg_doing(tools.path(), &format!("echo start >> '{0}'; sleep 0.2; printf 'j' > \"$9\"; echo end >> '{0}'", log.display()));
        let ex = extractor(probe_printing(tools.path(), "4"), ffmpeg, temp.path());
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let ex = ex.clone();
                std::thread::spawn(move || ex.frame(Path::new("v.mkv")).map(|f| f.is_some()))
            })
            .collect();
        for t in threads {
            assert!(t.join().unwrap().unwrap());
        }
        let lines: Vec<String> = fs::read_to_string(&log)?.lines().map(String::from).collect();
        assert_eq!(lines, ["start", "end", "start", "end", "start", "end"], "no two extractions overlap");
        Ok(())
    }

    #[test]
    fn a_real_ffmpeg_makes_a_jpeg_from_a_generated_video() -> io::Result<()> {
        let (Some(ffprobe), Some(ffmpeg)) = (Tool::on_path("ffprobe"), Tool::on_path("ffmpeg")) else { return Ok(()) };
        let dir = tempfile::tempdir()?;
        let video = dir.path().join("tiny video.mp4");
        let made = Command::new(&ffmpeg.program)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=duration=4:size=64x64:rate=5", "-y"])
            .arg(&video)
            .status()?;
        if !made.success() {
            return Ok(());
        }
        let temp = tempfile::tempdir()?;
        let ex = FrameExtractor::new(Some(ffprobe), Some(ffmpeg), temp.path());
        let jpeg = ex.frame(&video).map_err(io::Error::other)?.expect("a frame");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(fs::read_dir(temp.path())?.count(), 0);
        Ok(())
    }
}
