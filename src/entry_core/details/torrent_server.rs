//! The torrent server start before a torrent episode plays (T-39b-052; behaviour 22, `T-TRK-9`, fix 23.4). The server is a
//! child process, not an Android service: the start spawns the server binary, then the page polls it up to eleven times,
//! one second apart. Pure: the spawn, the probe and the pause are passed in, so the wait is tested without a process. The
//! tracker list is applied after this returns, whatever the result.

/// The number of polls after the start.
pub const POLLS: u32 = 11;

/// The pause between two polls, in milliseconds.
pub const POLL_INTERVAL_MS: u64 = 1000;

/// Why the server could not be started. Each one ends in a toast, never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    /// The server binary is not found.
    MissingBinary,
    /// Any other spawn failure, with its text.
    Failed(String),
}

/// What the start did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The setting is off or the source is not a torrent source: nothing starts and nothing waits.
    Skipped,
    /// The server answered within the polls.
    Up,
    /// The server did not answer in time. Playback goes on without it.
    TimedOut,
    /// The start failed. The text is the toast; playback goes on.
    Toast(String),
}

/// The start happens only when the torrent setting is on and the source is a torrent source (17.5).
pub fn should_start(enabled: bool, torrent_source: bool) -> bool {
    enabled && torrent_source
}

/// The toast text of a failed start.
pub fn toast_text(error: &StartError) -> String {
    match error {
        StartError::MissingBinary => "The torrent server was not found".to_owned(),
        StartError::Failed(text) => format!("The torrent server did not start: {text}"),
    }
}

/// Runs the start: `spawn` once, then up to [`POLLS`] probes of `is_up`, with `pause` between them. A probe that succeeds
/// at once (the server already runs) skips the wait.
pub fn run(
    enabled: bool,
    torrent_source: bool,
    spawn: impl FnOnce() -> Result<(), StartError>,
    mut is_up: impl FnMut() -> bool,
    mut pause: impl FnMut(u64),
) -> Outcome {
    if !should_start(enabled, torrent_source) {
        return Outcome::Skipped;
    }
    if let Err(error) = spawn() {
        return Outcome::Toast(toast_text(&error));
    }
    for poll in 0..POLLS {
        if is_up() {
            return Outcome::Up;
        }
        if poll + 1 < POLLS {
            pause(POLL_INTERVAL_MS);
        }
    }
    Outcome::TimedOut
}

/// The port the torrent server listens on (the server's own default).
pub const DEFAULT_PORT: u16 = 8090;

/// The name of the server binary looked up on the search path.
pub const BINARY: &str = "torrserver";

/// The real start: spawns the server binary found on the search path, detached, on [`DEFAULT_PORT`]. A missing binary is
/// `MissingBinary`; a failed spawn carries its text.
pub fn spawn_server() -> Result<(), StartError> {
    let search = std::env::var_os("PATH").unwrap_or_default();
    let binary = kuro_platform::process::find_program(BINARY, &search).ok_or(StartError::MissingBinary)?;
    kuro_platform::process::no_console(&mut std::process::Command::new(binary))
        .args(["--port", &DEFAULT_PORT.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| StartError::Failed(e.to_string()))
}

/// One probe: the server answers `GET /echo` with a success status within 300 ms.
pub fn probe(port: u16) -> bool {
    use std::io::{Read, Write};
    let timeout = std::time::Duration::from_millis(300);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, timeout) else { return false };
    let _ = stream.set_read_timeout(Some(timeout));
    if stream.write_all(b"GET /echo HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    stream.read(&mut head).is_ok_and(|n| std::str::from_utf8(&head[..n]).is_ok_and(|text| text.contains(" 2")))
}

/// The whole start with the real spawn, probe and pause. It blocks for up to eleven seconds: call it off the GTK thread.
pub fn start_blocking(enabled: bool, torrent_source: bool) -> Outcome {
    run(enabled, torrent_source, spawn_server, || probe(DEFAULT_PORT), |ms| std::thread::sleep(std::time::Duration::from_millis(ms)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_non_torrent_entry_or_a_switched_off_setting_never_starts() {
        let started = Cell::new(false);
        let out = run(false, true, || { started.set(true); Ok(()) }, || true, |_| {});
        assert_eq!(out, Outcome::Skipped);
        assert_eq!(run(true, false, || Ok(()), || true, |_| {}), Outcome::Skipped);
        assert!(!started.get());
    }

    #[test]
    fn it_waits_at_most_eleven_one_second_polls() {
        let probes = Cell::new(0u32);
        let pauses = Cell::new(0u32);
        let out = run(
            true,
            true,
            || Ok(()),
            || { probes.set(probes.get() + 1); false },
            |ms| { assert_eq!(ms, 1000); pauses.set(pauses.get() + 1) },
        );
        assert_eq!(out, Outcome::TimedOut);
        assert_eq!(probes.get(), 11);
        assert_eq!(pauses.get(), 10, "no pause after the last poll");
    }

    #[test]
    fn a_server_that_answers_in_time_is_up_and_a_running_one_skips_the_wait() {
        let mut calls = 0;
        let out = run(true, true, || Ok(()), || { calls += 1; calls == 3 }, |_| {});
        assert_eq!(out, Outcome::Up);
        assert_eq!(calls, 3);
        assert_eq!(run(true, true, || Ok(()), || true, |_| panic!("no wait")), Outcome::Up);
    }

    #[test]
    fn a_missing_binary_or_a_failed_start_is_a_toast_and_never_a_panic() {
        assert_eq!(
            run(true, true, || Err(StartError::MissingBinary), || true, |_| {}),
            Outcome::Toast("The torrent server was not found".to_owned())
        );
        assert!(toast_text(&StartError::Failed("boom".to_owned())).ends_with("boom"));
    }

    #[test]
    fn a_probe_of_a_port_with_no_server_is_down_and_one_with_an_echo_is_up() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let _ = conn.read(&mut buf);
            conn.write_all(b"HTTP/1.0 200 OK\r\n\r\nMatriX").unwrap();
        });
        assert!(probe(port));
        server.join().unwrap();
        assert!(!probe(port), "the listener is gone");
    }

    #[test]
    fn a_missing_binary_is_a_toast_and_not_a_wait() {
        let outcome = run(true, true, || Err(StartError::MissingBinary), || false, |_| panic!("no wait after a failed start"));
        assert_eq!(outcome, Outcome::Toast("The torrent server was not found".to_owned()));
    }
}
