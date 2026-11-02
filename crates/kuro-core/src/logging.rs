//! Logging: stderr plus a daily rotating file. The filter is reloadable at runtime.

use crate::utc;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, Registry, filter::LevelFilter, fmt, layer::SubscriberExt, reload, util::SubscriberInitExt};

/// Active log file. When the UTC day changes it moves to `kurobayomi.log.yyyy-MM-dd`.
pub const LOG_FILE_NAME: &str = "kurobayomi.log";
/// Files kept in the log directory, the active file included.
pub const KEPT_LOG_FILES: usize = 7;

/// Level selection: everything when verbose, otherwise only WARN and ERROR.
pub fn level_for(verbose: bool) -> LevelFilter {
    if verbose { LevelFilter::TRACE } else { LevelFilter::WARN }
}

/// logcat priorities (`logcat.LogPriority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogPriority {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Assert,
}

/// Verbose is TRACE, Debug is DEBUG, Info is INFO, Warn is WARN, Error and Assert are ERROR.
pub fn priority_to_level(priority: LogPriority) -> tracing::Level {
    match priority {
        LogPriority::Verbose => tracing::Level::TRACE,
        LogPriority::Debug => tracing::Level::DEBUG,
        LogPriority::Info => tracing::Level::INFO,
        LogPriority::Warn => tracing::Level::WARN,
        LogPriority::Error | LogPriority::Assert => tracing::Level::ERROR,
    }
}

/// The error and its `source()` chain on one line, `outer: inner: root`.
pub fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// The `logcat(priority, throwable) { message }` extension. An error is attached as the structured
/// fields `error` (its own message) and `error_chain` (it and every source).
pub fn log(priority: LogPriority, message: &str, error: Option<&(dyn std::error::Error + 'static)>) {
    macro_rules! emit {
        ($level:expr) => {
            match error {
                Some(e) => tracing::event!($level, error = %e, error_chain = %error_chain(e), "{message}"),
                None => tracing::event!($level, "{message}"),
            }
        };
    }
    match priority_to_level(priority) {
        tracing::Level::TRACE => emit!(tracing::Level::TRACE),
        tracing::Level::DEBUG => emit!(tracing::Level::DEBUG),
        tracing::Level::INFO => emit!(tracing::Level::INFO),
        tracing::Level::WARN => emit!(tracing::Level::WARN),
        tracing::Level::ERROR => emit!(tracing::Level::ERROR),
    }
}

/// Dependencies that log every compiler opcode or every byte at TRACE: a plug-in compile at TRACE writes hundreds of
/// megabytes and takes minutes. They stay at WARN (INFO for the network stack) even when verbose logging is on.
const NOISY: &str = "wasmtime=warn,wasmtime_internal_cranelift=warn,wasmtime_internal_cache=warn,cranelift_codegen=warn,cranelift_frontend=warn,cranelift_entity=warn,cranelift_bforest=warn,regalloc2=warn,hyper=info,hyper_util=info,h2=info,rustls=info,reqwest=info,tokio_util=info";

fn env_filter(verbose: bool) -> EnvFilter {
    let base = EnvFilter::builder().with_default_directive(level_for(verbose).into());
    match std::env::var("RUST_LOG") {
        Ok(_) => base.from_env_lossy(),
        // the base level is part of the string: a default directive is dropped as soon as any other directive is given
        Err(_) if verbose => base.parse_lossy(format!("trace,{NOISY}")),
        Err(_) => base.from_env_lossy(),
    }
}

/// The non-blocking file writer's guard. It sits in a static so the panic hook can flush the log
/// before the process exits; see [`flush`].
static GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

/// Waits up to about a second for every record sent so far to reach the log file, then stops the
/// file writer. Records logged after this call are dropped, so call it only on the way out.
pub fn flush() {
    let guard = GUARD.lock().unwrap_or_else(PoisonError::into_inner).take();
    drop(guard);
}

/// The live logging handle. Dropping it flushes the log file.
pub struct Logging {
    filter: reload::Handle<EnvFilter, Registry>,
}

impl Logging {
    /// Switches the filter at runtime, without a restart.
    pub fn set_verbose(&self, verbose: bool) -> Result<(), reload::Error> {
        self.filter.reload(env_filter(verbose))
    }
}

impl Drop for Logging {
    fn drop(&mut self) {
        flush();
    }
}

/// Writes `kurobayomi.log`. A write on a later UTC day first moves the active file to
/// `kurobayomi.log.<day>` and starts a new one, so each file holds one UTC day.
struct DailyFile {
    dir: PathBuf,
    /// UTC day (`yyyy-MM-dd`) of the active file.
    day: String,
    file: File,
}

fn utc_day(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    utc::rfc3339(secs)[..10].to_owned()
}

fn open_active(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl DailyFile {
    fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let active = dir.join(LOG_FILE_NAME);
        // A file left by an earlier day is rotated before anything is appended to it.
        let day = fs::metadata(&active).and_then(|m| m.modified()).map_or_else(|_| utc_day(SystemTime::now()), utc_day);
        let mut this = Self { dir: dir.to_owned(), day, file: open_active(&active)? };
        this.roll(&utc_day(SystemTime::now()))?;
        prune(dir);
        Ok(this)
    }

    /// Starts a new active file when `today` is a later day than the one being written.
    fn roll(&mut self, today: &str) -> io::Result<()> {
        if self.day == today {
            return Ok(());
        }
        let active = self.dir.join(LOG_FILE_NAME);
        fs::rename(&active, self.dir.join(format!("{LOG_FILE_NAME}.{}", self.day)))?;
        self.file = open_active(&active)?;
        self.day = today.to_owned();
        prune(&self.dir);
        Ok(())
    }
}

impl io::Write for DailyFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.roll(&utc_day(SystemTime::now()))?;
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Keeps the active file and the newest `KEPT_LOG_FILES - 1` dated files. Best effort: a file
/// that cannot be listed or removed is left alone.
fn prune(dir: &Path) {
    let prefix = format!("{LOG_FILE_NAME}.");
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut dated: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
        .collect();
    // `yyyy-MM-dd` suffixes sort by date.
    dated.sort();
    let excess = dated.len().saturating_sub(KEPT_LOG_FILES - 1);
    for path in &dated[..excess] {
        let _ = fs::remove_file(path);
    }
}

/// Layers: a reloadable filter, a stderr layer and a plain-text layer on `file`.
fn build_subscriber<E, F>(verbose: bool, stderr: E, file: F) -> (impl Subscriber + Send + Sync, reload::Handle<EnvFilter, Registry>)
where
    E: for<'a> MakeWriter<'a> + Send + Sync + 'static,
    F: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let (filter, handle) = reload::Layer::new(env_filter(verbose));
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(stderr))
        .with(fmt::layer().with_ansi(false).with_writer(file));
    (subscriber, handle)
}

/// Installs the global subscriber. Fails if one is already set.
pub fn init(verbose: bool, log_dir: &Path) -> Result<Logging, Box<dyn std::error::Error>> {
    let (writer, guard) = tracing_appender::non_blocking(DailyFile::open(log_dir)?);
    let (subscriber, filter) = build_subscriber(verbose, io::stderr, writer);
    subscriber.try_init()?;
    *GUARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(guard);
    Ok(Logging { filter })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbose_selects_all_levels() {
        assert_eq!(level_for(true), LevelFilter::TRACE);
    }

    #[test]
    fn quiet_keeps_warn_and_error() {
        assert_eq!(level_for(false), LevelFilter::WARN);
        assert!(tracing::Level::ERROR <= level_for(false));
        assert!(tracing::Level::WARN <= level_for(false));
        assert!(tracing::Level::INFO > level_for(false));
    }

    /// Shared in-memory sink for the file layer.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;

        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    #[test]
    fn quiet_file_layer_keeps_error_and_warn_only() {
        let file = Capture::default();
        let (subscriber, _filter) = build_subscriber(false, io::sink, file.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            tracing::error!("error-msg");
            tracing::warn!("warn-msg");
            tracing::info!("info-msg");
        });
        let text = file.text();
        assert!(text.contains("error-msg") && text.contains("warn-msg"), "{text}");
        assert!(!text.contains("info-msg"), "{text}");
    }

    #[test]
    fn set_verbose_lets_info_through_without_restart() {
        let file = Capture::default();
        let (subscriber, filter) = build_subscriber(false, io::sink, file.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            tracing::info!("before-reload");
            filter.reload(env_filter(true)).unwrap();
            tracing::info!("after-reload");
        });
        let text = file.text();
        assert!(!text.contains("before-reload"), "{text}");
        assert!(text.contains("after-reload"), "{text}");
    }

    #[test]
    fn verbose_keeps_the_app_at_trace_and_the_compiler_at_warn() {
        let file = Capture::default();
        let (subscriber, _filter) = build_subscriber(true, io::sink, file.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            tracing::trace!(target: "kurobayomi::app", "app-line");
            tracing::trace!(target: "cranelift_codegen::egraph", "compiler-line");
        });
        let text = file.text();
        assert!(text.contains("app-line"), "{text}");
        assert!(!text.contains("compiler-line"), "{text}");
    }

    #[test]
    fn line_has_timestamp_level_token_target_and_message() {
        let file = Capture::default();
        let (subscriber, _filter) = build_subscriber(false, io::sink, file.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            tracing::error!(target: "crash", "boom");
        });
        let text = file.text();
        let line = text.lines().find(|l| l.contains("boom")).expect("line");
        let ts = line.split(' ').next().unwrap();
        assert!(ts.len() >= 20 && ts.as_bytes()[4] == b'-' && ts.as_bytes()[10] == b'T' && ts.ends_with('Z'), "{line}");
        assert!(line.contains(" ERROR "), "{line}");
        assert!(line.contains("crash: boom"), "{line}");
    }

    #[test]
    fn priorities_map_to_tracing_levels() {
        use tracing::Level;
        let want = [
            (LogPriority::Verbose, Level::TRACE),
            (LogPriority::Debug, Level::DEBUG),
            (LogPriority::Info, Level::INFO),
            (LogPriority::Warn, Level::WARN),
            (LogPriority::Error, Level::ERROR),
            (LogPriority::Assert, Level::ERROR),
        ];
        for (priority, level) in want {
            assert_eq!(priority_to_level(priority), level, "{priority:?}");
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer failure")]
    struct Outer(#[source] io::Error);

    #[test]
    fn logged_error_prints_both_the_error_and_its_source() {
        let file = Capture::default();
        let (subscriber, _filter) = build_subscriber(false, io::sink, file.clone());
        let err = Outer(io::Error::other("inner cause"));
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            log(LogPriority::Error, "split failed", Some(&err));
        });
        let text = file.text();
        assert!(text.contains("split failed"), "{text}");
        assert!(text.contains("error=outer failure"), "{text}");
        assert!(text.contains("error_chain=outer failure: inner cause"), "{text}");
    }

    #[test]
    fn log_without_an_error_has_no_error_fields_and_honours_the_level() {
        let file = Capture::default();
        let (subscriber, _filter) = build_subscriber(false, io::sink, file.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            log(LogPriority::Warn, "plain-warn", None);
            log(LogPriority::Verbose, "hidden-trace", None);
        });
        let text = file.text();
        assert!(text.contains("plain-warn") && !text.contains("error"), "{text}");
        assert!(!text.contains("hidden-trace"), "{text}");
    }

    #[test]
    fn daily_file_keeps_the_newest_seven_files() {
        let dir = tempfile::tempdir().unwrap();
        // Old dates, so none is today's file: the active file plus the newest 6 dated files remain.
        for day in 1..=9 {
            std::fs::write(dir.path().join(format!("{LOG_FILE_NAME}.2001-01-{day:02}")), "x").unwrap();
        }
        let _writer = DailyFile::open(dir.path()).unwrap();
        let kept: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(kept.len(), KEPT_LOG_FILES, "{kept:?}");
        assert!(kept.contains(&LOG_FILE_NAME.to_owned()), "{kept:?}");
        assert!(!kept.contains(&format!("{LOG_FILE_NAME}.2001-01-03")), "{kept:?}");
    }

    #[test]
    fn day_change_moves_the_active_file_to_its_dated_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = DailyFile::open(dir.path()).unwrap();
        let today = writer.day.clone();
        io::Write::write_all(&mut writer, b"before\n").unwrap();
        // `write` reads the real clock, so the day change is driven through `roll` and the file directly.
        writer.roll("2999-12-31").unwrap();
        io::Write::write_all(&mut writer.file, b"after\n").unwrap();
        let dated = std::fs::read_to_string(dir.path().join(format!("{LOG_FILE_NAME}.{today}"))).unwrap();
        assert_eq!(dated, "before\n");
        let active = std::fs::read_to_string(dir.path().join(LOG_FILE_NAME)).unwrap();
        assert_eq!(active, "after\n");
    }

    #[test]
    fn active_file_is_kurobayomi_log_in_the_log_directory() {
        // plans/ARCHITECTURE.md section 11 and T-01a-005 name this file.
        assert_eq!(LOG_FILE_NAME, "kurobayomi.log");
        let dir = tempfile::tempdir().unwrap();
        let _writer = DailyFile::open(dir.path()).unwrap();
        assert!(dir.path().join("kurobayomi.log").is_file());
    }

    #[test]
    fn open_rotates_an_active_file_left_by_an_earlier_day() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join(LOG_FILE_NAME);
        std::fs::write(&active, "left-over\n").unwrap();
        // 2001-01-01T00:00:00Z
        let old = UNIX_EPOCH + std::time::Duration::from_secs(978_307_200);
        std::fs::OpenOptions::new().write(true).open(&active).unwrap().set_modified(old).unwrap();
        let _writer = DailyFile::open(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join(format!("{LOG_FILE_NAME}.2001-01-01"))).unwrap(), "left-over\n");
        assert_eq!(std::fs::read_to_string(&active).unwrap(), "");
    }
}
