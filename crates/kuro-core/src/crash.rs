//! Panic hook: writes a crash report file, logs it, starts the detached reporter, chains to the
//! previous hook, exits 101. The reporter window lives in the root crate (`src/reporter.rs`).

use crate::app_info::APP_INFO;
use crate::utc;
use kuro_platform::process::spawn_detached;
use std::any::Any;
use std::backtrace::Backtrace;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAGIC: &str = "kurobayomi-crash-report 1";
/// Newest reports kept in the crash directory.
pub const KEPT_CRASH_REPORTS: usize = 20;

/// Set to `1` (main window, 500 ms after it is shown) or `worker` (inside a tokio task) to crash on
/// purpose. Ignored in release builds unless [`ALLOW_TEST_HOOKS_ENV`] is `1`.
pub const CRASH_TEST_ENV: &str = "KUROBAYOMI_CRASH_TEST";
/// Set to `1` so the reporter prints the report path and exits 0 without a window. Same release gate.
pub const REPORTER_DRY_ENV: &str = "KUROBAYOMI_REPORTER_DRY";
/// Set to `1` to let the test hooks work in a release build.
pub const ALLOW_TEST_HOOKS_ENV: &str = "KUROBAYOMI_ALLOW_TEST_HOOKS";
/// Panic message of the crash test hooks.
pub const CRASH_TEST_MESSAGE: &str = "crash test";
/// Delay between the main window being shown and the `1` crash test panicking.
pub const CRASH_TEST_DELAY_MS: u64 = 500;
/// True in release builds, where the test hooks need [`ALLOW_TEST_HOOKS_ENV`].
pub const RELEASE_BUILD: bool = !cfg!(debug_assertions);

/// Where a crash test panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashTest {
    /// On the main thread, [`CRASH_TEST_DELAY_MS`] after the main window is presented.
    MainWindow,
    /// Inside a tokio task, see [`crash_test_worker`].
    Worker,
}

/// The crash test selected by the `KUROBAYOMI_CRASH_TEST` value. In a release build (`release`) it
/// is ignored unless the `KUROBAYOMI_ALLOW_TEST_HOOKS` value is `1`. Other values select nothing.
pub fn crash_test_hook(crash_test: Option<&str>, allow: Option<&str>, release: bool) -> Option<CrashTest> {
    if release && allow != Some("1") {
        return None;
    }
    match crash_test? {
        "1" => Some(CrashTest::MainWindow),
        "worker" => Some(CrashTest::Worker),
        _ => None,
    }
}

/// Whether the reporter runs in dry-run mode for the `KUROBAYOMI_REPORTER_DRY` value, under the same
/// release gate as [`crash_test_hook`].
pub fn reporter_dry_run(reporter_dry: Option<&str>, allow: Option<&str>, release: bool) -> bool {
    if release && allow != Some("1") {
        return false;
    }
    reporter_dry == Some("1")
}

/// Panics with [`CRASH_TEST_MESSAGE`] on the calling thread. The `1` hook calls it on the main thread.
pub fn crash_test_now() -> ! {
    panic!("{CRASH_TEST_MESSAGE}")
}

/// The `worker` hook: panics with [`CRASH_TEST_MESSAGE`] when polled. Spawn it on a tokio runtime.
pub async fn crash_test_worker() {
    panic!("{CRASH_TEST_MESSAGE}")
}
/// Reports older than this are deleted at startup.
const MAX_REPORT_AGE_MS: u128 = 14 * 24 * 60 * 60 * 1000;
static CRASHING: AtomicBool = AtomicBool::new(false);
/// Set once the app's runtime starts shutting down. From then on a blocking thread still waiting on a request
/// (a slow source, a cover) panics inside tokio ("context is being shutdown"); that is the app quitting, not a crash.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Marks the app as quitting: later panics are logged at WARN, with no report, no crash window and no exit 101.
pub fn mark_shutting_down() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
}

thread_local! {
    /// Nesting depth of [`contain`] on this thread. While it is above zero the hook stays quiet.
    static SUPPRESS: Cell<u32> = const { Cell::new(0) };
}

/// `[facts]` keys in report order, each with the value printed while nothing has set it.
/// Subsystems set their keys with [`set_fact`] once they are up.
const FACT_KEYS: &[(&str, &str)] = &[
    ("app_id", "unknown"),
    ("app_version", "unknown"),
    ("os", "unknown"),
    ("kernel", "unknown"),
    ("desktop", "unknown"),
    ("session", "unknown"),
    ("gtk", "not loaded"),
    ("webview", "not loaded"),
    ("mpv", "not loaded"),
    ("ffmpeg", "not loaded"),
    ("sqlite", "unknown"),
    ("locale", "unknown"),
];

/// Extension sections, always printed in this order, with empty bodies when nothing is set.
const EXTENSION_SECTIONS: &[&str] = &["manga_extensions", "anime_extensions"];

/// Facts and sections the report prints, filled before a crash so the hook does no heavy work.
/// The hook reads them with `try_lock` (see [`snapshot`]).
struct CrashContext {
    facts: Mutex<BTreeMap<&'static str, String>>,
    sections: Mutex<BTreeMap<&'static str, String>>,
}

static CONTEXT: CrashContext = CrashContext {
    facts: Mutex::new(BTreeMap::new()),
    sections: Mutex::new(BTreeMap::new()),
};

/// Sets one `[facts]` value, for example `set_fact("gtk", "4.16.12")`.
pub fn set_fact(key: &'static str, value: String) {
    CONTEXT.facts.lock().unwrap_or_else(PoisonError::into_inner).insert(key, value);
}

/// Sets an extension section body, already formatted. Only `manga_extensions` and
/// `anime_extensions` are printed; other keys are kept but not written.
pub fn set_section(key: &'static str, text: String) {
    CONTEXT.sections.lock().unwrap_or_else(PoisonError::into_inner).insert(key, text);
}

/// Copies a context map for the hook. A lock held elsewhere yields an empty map, so the report
/// prints the defaults instead of waiting inside the panic hook.
fn snapshot(map: &Mutex<BTreeMap<&'static str, String>>) -> BTreeMap<&'static str, String> {
    match map.try_lock() {
        Ok(guard) => guard.clone(),
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().clone(),
        Err(TryLockError::WouldBlock) => BTreeMap::new(),
    }
}

/// The facts known from startup: app identity, then the system facts from `kuro_platform`.
fn fill_install_facts() {
    let i = &*APP_INFO;
    set_fact("app_id", i.app_id.to_owned());
    set_fact("app_version", format!("{} ({}, {}, {})", i.version_name, i.git_sha, i.version_code, i.build_time));
    for (key, value) in kuro_platform::system::facts() {
        set_fact(key, value);
    }
}

/// Everything the report file records about one panic.
pub struct Report {
    pub time: String,
    pub thread: String,
    pub location: String,
    pub message: String,
    pub backtrace: String,
}

/// Renders report format version 1 from the current context. Newlines in the message are escaped as `\n`.
pub fn format_report(r: &Report) -> String {
    render(r, &snapshot(&CONTEXT.facts), &snapshot(&CONTEXT.sections))
}

fn render(r: &Report, facts: &BTreeMap<&'static str, String>, sections: &BTreeMap<&'static str, String>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{MAGIC}");
    let _ = writeln!(out, "time: {}", r.time);
    let _ = writeln!(out, "thread: {}", r.thread);
    let _ = writeln!(out, "location: {}", r.location);
    let _ = writeln!(out, "message: {}", r.message.replace('\n', "\\n"));
    let _ = writeln!(out, "[facts]");
    for (key, default) in FACT_KEYS {
        let _ = writeln!(out, "{key}: {}", facts.get(key).map_or(*default, String::as_str));
    }
    for name in EXTENSION_SECTIONS {
        let _ = writeln!(out, "[{name}]");
        let body = sections.get(name).map_or("", String::as_str);
        out.push_str(body);
        if !body.is_empty() && !body.ends_with('\n') {
            out.push('\n');
        }
    }
    let _ = writeln!(out, "[backtrace]");
    let _ = writeln!(out, "{}", r.backtrace);
    out
}

fn payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".to_owned()
    }
}

/// Current unix time in milliseconds.
pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

fn build_report(info: &PanicHookInfo<'_>) -> Report {
    let message = payload_text(info.payload());
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Report {
        time: utc::rfc3339(secs),
        thread: std::thread::current().name().unwrap_or("unnamed").to_owned(),
        location: info.location().map_or_else(|| "unknown".to_owned(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column())),
        message,
        backtrace: Backtrace::force_capture().to_string(),
    }
}

/// Writes `crash-<unix_ms>.txt` atomically (temp file, fsync, rename) and returns its path.
fn write_report(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let ms = now_ms();
    let path = dir.join(format!("crash-{ms}.txt"));
    let tmp = dir.join(format!("crash-{ms}.txt.tmp"));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Starts `kurobayomi --crash-report <file>` detached. The reporter outlives this process.
fn spawn_reporter(file: &Path) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    spawn_detached(&exe, &[OsStr::new("--crash-report"), file.as_os_str()])
}

/// Replaces the panic hook and fills the install-time facts. Call before other initialisation.
///
/// A panic on another thread while the hook runs exits 101 through `CRASHING`. A panic on the same
/// thread while the hook runs makes std abort the process (SIGABRT, "thread panicked while
/// processing panic"), so the hook body must not panic.
pub fn install(crash_dir: PathBuf) {
    fill_install_facts();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if SUPPRESS.with(Cell::get) > 0 {
            return;
        }
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            tracing::warn!(target: "crash", "panic while quitting, not reported: {}", payload_text(info.payload()));
            return;
        }
        // a second panic while reporting exits without a second file
        if CRASHING.swap(true, Ordering::SeqCst) {
            std::process::exit(101);
        }
        let report = build_report(info);
        let text = format_report(&report);
        tracing::error!(target: "crash", "panic in thread {}: {} at {}", report.thread, report.message, report.location);
        // the record must reach the log file before the exit below
        crate::logging::flush();
        match write_report(&crash_dir, &text) {
            Ok(path) => {
                eprintln!("kurobayomi: crash report written to {}", path.display());
                if let Err(e) = spawn_reporter(&path) {
                    eprintln!("kurobayomi: could not start crash reporter: {e}");
                }
            }
            Err(e) => eprintln!("kurobayomi: could not write crash report: {e}"),
        }
        prev(info);
        std::process::exit(101);
    }));
}

/// Runs `f`, turning a panic into `Err(message)`. A contained panic writes no report and does not
/// exit the process; it is logged at WARN.
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    SUPPRESS.with(|s| s.set(s.get() + 1));
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    SUPPRESS.with(|s| s.set(s.get() - 1));
    result.map_err(|payload| {
        let message = payload_text(&*payload);
        tracing::warn!(target: "crash", "contained panic: {message}");
        message
    })
}

/// Deletes reports older than 14 days, then all but the newest [`KEPT_CRASH_REPORTS`].
/// Reports are named `crash-<unix_ms>.txt`; `now_ms` is the current unix time in milliseconds.
pub fn prune(dir: &Path, now_ms: u128) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut reports: Vec<(u128, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| Some((report_ms(e.file_name().to_str()?)?, e.path())))
        .collect();
    reports.sort_by(|a, b| b.0.cmp(&a.0));
    for (i, (ms, path)) in reports.iter().enumerate() {
        if i >= KEPT_CRASH_REPORTS || now_ms.saturating_sub(*ms) > MAX_REPORT_AGE_MS {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

/// Unix milliseconds from a report file name, or `None` for any other file (including `.tmp`).
fn report_ms(name: &str) -> Option<u128> {
    name.strip_prefix("crash-")?.strip_suffix(".txt")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(message: &str) -> Report {
        Report {
            time: "2026-10-08T14:03:22Z".into(),
            thread: "kuro-rt".into(),
            location: "crates/kuro-core/src/foo.rs:42:9".into(),
            message: message.into(),
            backtrace: "   0: kuro_core::foo::bar".into(),
        }
    }

    #[test]
    fn report_starts_with_magic_and_header_lines() {
        let text = format_report(&sample("boom"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], MAGIC);
        assert_eq!(lines[1], "time: 2026-10-08T14:03:22Z");
        assert_eq!(lines[2], "thread: kuro-rt");
        assert_eq!(lines[3], "location: crates/kuro-core/src/foo.rs:42:9");
        assert_eq!(lines[4], "message: boom");
    }

    #[test]
    fn sections_are_in_order_and_backtrace_is_last() {
        fill_install_facts();
        let text = format_report(&sample("boom"));
        let facts = text.find("[facts]\n").expect("facts");
        let bt = text.find("[backtrace]\n").expect("backtrace");
        assert!(facts < bt);
        assert!(text.contains("app_id: io.github.kurobayomi.Kurobayomi\n"));
        let os = kuro_platform::system::facts().into_iter().find(|(k, _)| *k == "os").expect("os").1;
        assert!(text.contains(&format!("os: {os}\n")), "{text}");
        assert!(text.ends_with("   0: kuro_core::foo::bar\n"));
    }

    #[test]
    fn facts_print_every_key_in_order_with_defaults() {
        let text = render(&sample("boom"), &BTreeMap::new(), &BTreeMap::new());
        let keys: Vec<&str> = text
            .lines()
            .skip_while(|l| *l != "[facts]")
            .skip(1)
            .take_while(|l| !l.starts_with('['))
            .filter_map(|l| l.split_once(": ").map(|(k, _)| k))
            .collect();
        assert_eq!(
            keys,
            ["app_id", "app_version", "os", "kernel", "desktop", "session", "gtk", "webview", "mpv", "ffmpeg", "sqlite", "locale"]
        );
        assert!(text.contains("gtk: not loaded\n"));
        assert!(text.contains("webview: not loaded\n"));
        assert!(text.contains("sqlite: unknown\n"));
    }

    #[test]
    fn set_facts_replace_defaults() {
        let facts = BTreeMap::from([("gtk", "4.16.12".to_owned()), ("mpv", "0.39.0".to_owned())]);
        let text = render(&sample("boom"), &facts, &BTreeMap::new());
        assert!(text.contains("gtk: 4.16.12\n"));
        assert!(text.contains("mpv: 0.39.0\n"));
        assert!(text.contains("ffmpeg: not loaded\n"));
    }

    #[test]
    fn extension_headers_are_always_printed_and_bodies_follow_them() {
        let empty = render(&sample("boom"), &BTreeMap::new(), &BTreeMap::new());
        assert!(empty.contains("[manga_extensions]\n[anime_extensions]\n[backtrace]\n"), "{empty}");

        let body = "Problematic extensions:\n- Some Source\n  Installed: 1.4.20 / Available: 1.4.25\n  Obsolete: false";
        let sections = BTreeMap::from([("manga_extensions", body.to_owned())]);
        let text = render(&sample("boom"), &BTreeMap::new(), &sections);
        assert!(text.contains(&format!("[manga_extensions]\n{body}\n[anime_extensions]\n[backtrace]\n")), "{text}");
    }

    #[test]
    fn test_hooks_are_selected_by_value_and_ignored_in_release_without_the_allow_switch() {
        assert_eq!(crash_test_hook(Some("1"), None, false), Some(CrashTest::MainWindow));
        assert_eq!(crash_test_hook(Some("worker"), None, false), Some(CrashTest::Worker));
        assert_eq!(crash_test_hook(Some("yes"), None, false), None);
        assert_eq!(crash_test_hook(None, None, false), None);
        assert_eq!(crash_test_hook(Some("1"), None, true), None, "release ignores the hook");
        assert_eq!(crash_test_hook(Some("1"), Some("0"), true), None);
        assert_eq!(crash_test_hook(Some("1"), Some("1"), true), Some(CrashTest::MainWindow));
    }

    #[test]
    fn reporter_dry_run_follows_the_same_release_gate() {
        assert!(reporter_dry_run(Some("1"), None, false));
        assert!(!reporter_dry_run(Some("0"), None, false));
        assert!(!reporter_dry_run(Some("1"), None, true));
        assert!(reporter_dry_run(Some("1"), Some("1"), true));
    }

    #[test]
    fn newlines_in_message_are_escaped() {
        let text = format_report(&sample("first\nsecond"));
        assert!(text.contains("message: first\\nsecond\n"));
        assert_eq!(text.lines().filter(|l| l.starts_with("message:")).count(), 1);
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "x").unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn write_report_leaves_only_the_final_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_report(dir.path(), "payload").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "payload");
        let found = names(dir.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("crash-") && found[0].ends_with(".txt"));
    }

    #[test]
    fn prune_drops_old_reports_and_keeps_newest_twenty() {
        let dir = tempfile::tempdir().unwrap();
        let day: u128 = 24 * 60 * 60 * 1000;
        let now: u128 = 1_790_000_000_000;
        for i in 0..3 {
            touch(dir.path(), &format!("crash-{}.txt", now - 20 * day - i));
        }
        for i in 0..22 {
            touch(dir.path(), &format!("crash-{}.txt", now - i * 1000));
        }
        touch(dir.path(), "crash-notes.txt");
        touch(dir.path(), &format!("crash-{now}.txt.tmp"));
        prune(dir.path(), now).unwrap();
        let found = names(dir.path());
        assert_eq!(found.len(), KEPT_CRASH_REPORTS + 2, "{found:?}");
        assert!(found.contains(&format!("crash-{}.txt", now - 19 * 1000)));
        assert!(!found.contains(&format!("crash-{}.txt", now - 20 * 1000)));
        assert!(found.contains(&"crash-notes.txt".to_owned()));
    }

    #[test]
    fn prune_on_missing_directory_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        assert!(prune(&dir.path().join("absent"), now_ms()).is_ok());
    }

    #[test]
    fn contain_turns_panic_into_err_in_process() {
        assert_eq!(contain(|| 7), Ok(7));
        assert_eq!(contain(|| -> u8 { panic!("inside") }), Err("inside".to_owned()));
    }

    const CHILD_DIR: &str = "KURO_CRASH_CHILD_DIR";
    const CHILD_MODE: &str = "KURO_CRASH_CHILD_MODE";
    const CHILD_LOG_DIR: &str = "KURO_CRASH_CHILD_LOG_DIR";

    /// Re-runs this test binary on `child_entry` with the hook pointed at a fresh directory.
    fn run_child(mode: &str) -> (Option<i32>, usize) {
        let (code, reports, _) = run_child_logged(mode);
        (code, reports)
    }

    /// Like `run_child`, and also returns the text of the child's log file (empty when none).
    fn run_child_logged(mode: &str) -> (Option<i32>, usize, String) {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let crashes = dir.path().join("crashes");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash::tests::child_entry", "--nocapture"])
            .env(CHILD_DIR, &crashes)
            .env(CHILD_LOG_DIR, &logs)
            .env(CHILD_MODE, mode)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        let reports = std::fs::read_dir(&crashes).map_or(0, |entries| {
            entries.filter_map(Result::ok).filter(|e| report_ms(e.file_name().to_str().unwrap_or("")).is_some()).count()
        });
        let log = std::fs::read_to_string(logs.join(crate::logging::LOG_FILE_NAME)).unwrap_or_default();
        (status.code(), reports, log)
    }

    /// Only does work when run by `run_child`: installs the hook, then panics as `CHILD_MODE` says.
    #[test]
    fn child_entry() {
        let (Ok(dir), Ok(mode)) = (std::env::var(CHILD_DIR), std::env::var(CHILD_MODE)) else {
            return;
        };
        if mode == "nested" {
            // Runs after the report file is written. A panic on another thread here is the second
            // panic during reporting: the hook must exit 101 without writing a second file.
            std::panic::set_hook(Box::new(|_| {
                let _ = std::thread::spawn(|| panic!("second panic")).join();
            }));
        }
        install(PathBuf::from(dir));
        // kept alive to the end of the child: dropping `Logging` flushes and stops the file writer
        let _log = std::env::var(CHILD_LOG_DIR).ok().map(|logs| crate::logging::init(false, Path::new(&logs)).unwrap());
        match mode.as_str() {
            "thread" => {
                let _ = std::thread::spawn(|| panic!("boom in thread")).join();
            }
            "tokio" => {
                let rt = crate::runtime::build().unwrap();
                rt.block_on(async {
                    let _ = tokio::spawn(async { panic!("boom on tokio worker") }).await;
                });
            }
            "blocking" => {
                let rt = crate::runtime::build().unwrap();
                rt.block_on(async {
                    let _ = tokio::task::spawn_blocking(|| panic!("boom on blocking thread")).await;
                });
            }
            "shutdown" => {
                // a blocking thread waits on a slow request while the app quits (the 2026-10-22 nightly crash)
                let rt = crate::runtime::build().unwrap();
                let handle = rt.handle().clone();
                rt.spawn_blocking(move || handle.block_on(tokio::time::sleep(std::time::Duration::from_secs(1))));
                std::thread::sleep(std::time::Duration::from_millis(100));
                crate::runtime::shutdown(rt);
            }
            "nested" => panic!("first panic"),
            "contain" => assert!(contain(|| panic!("contained")).is_err()),
            other => panic!("unknown mode {other}"),
        }
    }

    #[test]
    fn panic_in_a_thread_writes_one_report_and_exits_101() {
        assert_eq!(run_child("thread"), (Some(101), 1));
    }

    #[test]
    fn panic_on_a_tokio_worker_writes_one_report_and_exits_101() {
        assert_eq!(run_child("tokio"), (Some(101), 1));
    }

    #[test]
    fn panic_on_a_spawn_blocking_thread_writes_one_report_and_exits_101() {
        assert_eq!(run_child("blocking"), (Some(101), 1));
    }

    #[test]
    fn a_panic_while_the_runtime_shuts_down_writes_no_report_and_does_not_exit() {
        assert_eq!(run_child("shutdown"), (Some(0), 0));
    }

    #[test]
    fn second_panic_while_reporting_exits_101_without_a_second_report() {
        assert_eq!(run_child("nested"), (Some(101), 1));
    }

    #[test]
    fn contained_panic_writes_no_report_and_does_not_exit() {
        assert_eq!(run_child("contain"), (Some(0), 0));
    }

    #[test]
    fn panic_record_reaches_the_log_file_before_exit() {
        for mode in ["thread", "tokio", "blocking"] {
            let (code, reports, log) = run_child_logged(mode);
            assert_eq!((code, reports), (Some(101), 1), "{mode}");
            let records = log.lines().filter(|l| l.contains(" ERROR crash: panic in thread ")).count();
            assert_eq!(records, 1, "{mode}: {log}");
        }
    }
}
