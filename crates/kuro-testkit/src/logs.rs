//! Log capture for tests that check what was logged.
//!
//! A subscriber set per thread (`tracing::subscriber::with_default`) races with the tests running beside it: a log
//! statement's "is anyone listening" answer is cached for the whole process, so another thread can cache "no" just
//! before or after this one installs its subscriber, and the line is lost at random. Here one subscriber is installed
//! for the whole process, once, and it writes each line into a buffer of the thread that logged it, so every log
//! statement is always listened to and each test reads only its own lines.

use std::cell::RefCell;
use std::io;
use std::sync::Once;

thread_local! {
    static CAPTURED: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// The writer of the process-wide subscriber: the thread's buffer while it captures, nowhere otherwise.
struct ThreadBuffer;

impl io::Write for ThreadBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        CAPTURED.with(|c| {
            if let Some(lines) = c.borrow_mut().as_mut() {
                lines.extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(|| ThreadBuffer)
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .finish();
        // another global subscriber (a test binary that set its own) keeps working; capture then sees nothing
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

/// Runs `f` and returns its value with every line logged on this thread while it ran (level, target and message, no
/// time). Logs from other threads, such as a multi-thread runtime's workers, are not caught: run async code on a
/// current-thread runtime inside `f`.
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
    install();
    CAPTURED.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let value = f();
    let lines = CAPTURED.with(|c| c.borrow_mut().take()).unwrap_or_default();
    (value, String::from_utf8_lossy(&lines).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_logged_inside_is_captured_and_one_outside_is_not() {
        tracing::warn!("before");
        let ((), text) = capture(|| tracing::warn!(id = 3, "inside"));
        tracing::warn!("after");
        assert!(text.contains("WARN") && text.contains("inside") && text.contains("id=3"), "{text}");
        assert!(!text.contains("before") && !text.contains("after"));
    }

    #[test]
    fn parallel_captures_keep_their_own_lines() {
        let threads: Vec<_> = (0..8)
            .map(|i| std::thread::spawn(move || capture(|| tracing::error!("line {i}")).1))
            .collect();
        for (i, t) in threads.into_iter().enumerate() {
            let text = t.join().unwrap();
            assert!(text.contains(&format!("line {i}")) && text.matches("line").count() == 1, "{text}");
        }
    }
}
