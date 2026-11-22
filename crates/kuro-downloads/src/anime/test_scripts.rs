//! The fake ffmpeg and ffprobe scripts of the anime tests, and the one lock every test that writes or runs one takes.
//!
//! A script being written while another thread forks is held open by that child until the child runs its own program,
//! and running the script in that moment fails with "Text file busy" (ETXTBSY). The tests of every module run in this
//! one test binary, so they share one lock: with a lock per module, and a test that took none, it happened (Linux CI,
//! 2026-11-22, `ffprobe_na_is_duration_zero`).

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::{Path, PathBuf};

static SPAWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Taken by an async test for as long as it writes or runs a script.
pub async fn spawn_guard() -> tokio::sync::MutexGuard<'static, ()> {
    SPAWN_LOCK.lock().await
}

/// [`spawn_guard`] for a test that is not async (outside any runtime).
#[cfg(unix)]
pub fn spawn_guard_blocking() -> tokio::sync::MutexGuard<'static, ()> {
    SPAWN_LOCK.blocking_lock()
}

/// Writes an executable shell script into `dir` and returns its path. Call it only while holding the guard.
#[cfg(unix)]
pub fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
