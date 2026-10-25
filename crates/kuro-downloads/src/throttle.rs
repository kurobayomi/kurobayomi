//! The shared download speed limit (T-23a-015).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One byte budget shared by every concurrent page download (T-23a-015). Each chunk books its bytes after the last
/// booked byte and waits until its own share has passed, so the total rate stays at the limit however many pages run.
/// `None` is unlimited.
#[derive(Debug)]
pub struct SpeedLimiter {
    state: Mutex<LimiterState>,
}

#[derive(Debug)]
struct LimiterState {
    bytes_per_sec: Option<u64>,
    /// When the bytes booked so far are paid for.
    free_at: Instant,
}

impl SpeedLimiter {
    pub fn new(bytes_per_sec: Option<u64>) -> Self {
        Self { state: Mutex::new(LimiterState { bytes_per_sec, free_at: Instant::now() }) }
    }

    /// Books `bytes` at `now` and returns the instant at which they are paid for. `None` when unlimited.
    pub fn book(&self, bytes: u64, now: Instant) -> Option<Instant> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let rate = st.bytes_per_sec.filter(|r| *r > 0)?;
        let start = st.free_at.max(now);
        st.free_at = start + Duration::from_secs_f64(bytes as f64 / rate as f64);
        Some(st.free_at)
    }

    /// Waits until `bytes` read from the network are paid for.
    pub async fn take(&self, bytes: u64) {
        if let Some(deadline) = self.book(bytes, Instant::now()) {
            tokio::time::sleep_until(deadline.into()).await;
        }
    }
}
