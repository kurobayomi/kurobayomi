//! Store limits and the epoch ticker for guest execution (area 18, T-18a-006).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use wasmtime::{Config, Engine, OptLevel, StoreLimits, StoreLimitsBuilder};

use crate::error::WasmError;

/// Largest `source.wasm` the host compiles.
pub const MAX_WASM_BYTES: usize = 32 << 20;
/// Most linear memory one guest instance may hold (W3).
pub const MEMORY_LIMIT_BYTES: usize = 256 << 20;
/// The epoch tick. Deadlines are counted in ticks.
pub const EPOCH_TICK: Duration = Duration::from_millis(100);
/// A guest call traps after this many ticks of guest run time, 30 s at [`EPOCH_TICK`] (W2). Time a guest spends inside
/// host calls (the request spacing and the network) is not guest run time, see [`CallClock`].
pub const CALL_DEADLINE_TICKS: u64 = 300;
/// The wall-clock cap of one guest call, host waits included, so a call cannot run on for ever through host waits.
pub const CALL_WALL_CAP: Duration = Duration::from_secs(300);

/// The clock of one guest call: when it started, how long it has waited inside host calls, and its run-time budget.
pub struct CallClock {
    started: Instant,
    /// Wall time spent inside host calls since the call started.
    pub(crate) host_wait: Duration,
    budget: Duration,
}

impl CallClock {
    /// The clock of a call that starts now, with `deadline_ticks` of guest run time.
    pub fn start(deadline_ticks: u64) -> Self {
        Self {
            started: Instant::now(),
            host_wait: Duration::ZERO,
            budget: EPOCH_TICK * u32::try_from(deadline_ticks).unwrap_or(u32::MAX),
        }
    }

    /// Books the wall time one host call took.
    pub fn book_host_wait(&mut self, waited: Duration) {
        self.host_wait += waited;
    }

    /// Why the call must stop at `now`, or `None` while it may run on. Guest run time is the wall time since the start,
    /// less the host waits. The wall cap is checked on its own, so a call ends even when all its time was host waits.
    pub fn stop_reason(&self, now: Instant) -> Option<&'static str> {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= CALL_WALL_CAP {
            Some("guest call passed its wall-clock cap")
        } else if elapsed.saturating_sub(self.host_wait) >= self.budget {
            Some("guest run time passed its call deadline")
        } else {
            None
        }
    }
}

static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();

/// The process-wide engine. The first call starts the one epoch ticker of the process.
pub fn engine() -> Result<&'static Engine, WasmError> {
    ENGINE
        .get_or_init(build_engine)
        .as_ref()
        .map_err(|e| WasmError::Engine(e.clone()))
}

fn build_engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    // Speed is wasmtime's default, set here so the compile speed is a decision of ours; the compile cache hides the cost
    config.cranelift_opt_level(OptLevel::Speed);
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let ticker = engine.clone();
    std::thread::Builder::new()
        .name("kuro-epoch".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK);
                ticker.increment_epoch();
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(engine)
}

/// The limits of one guest store: at most [`MEMORY_LIMIT_BYTES`] of linear memory.
pub fn store_limits() -> StoreLimits {
    StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT_BYTES).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guest_limits_match_the_plan() {
        assert_eq!(MEMORY_LIMIT_BYTES, 256 * 1024 * 1024);
        assert_eq!(MAX_WASM_BYTES, 32 * 1024 * 1024);
        assert_eq!(EPOCH_TICK.as_millis(), 100);
        assert_eq!(
            CALL_DEADLINE_TICKS, 300,
            "300 ticks of 100 ms is the 30 s call deadline"
        );
    }

    #[test]
    fn host_waits_are_not_guest_run_time_and_the_wall_cap_still_ends_a_call() {
        let start = Instant::now();
        let ms = Duration::from_millis;
        let mut clock = CallClock {
            started: start,
            host_wait: Duration::ZERO,
            budget: ms(500),
        };
        assert_eq!(clock.stop_reason(start + ms(400)), None);
        assert!(clock.stop_reason(start + ms(600)).is_some(), "600 ms of run time is past a 500 ms budget");

        clock.book_host_wait(ms(800));
        assert_eq!(
            clock.stop_reason(start + ms(1200)),
            None,
            "400 ms of run time after 800 ms of host waits is within budget"
        );
        assert!(
            clock.stop_reason(start + ms(1400)).is_some(),
            "600 ms of run time is past the budget, host waits aside"
        );

        let all_waiting = CallClock {
            started: start,
            host_wait: CALL_WALL_CAP,
            budget: ms(500),
        };
        assert!(
            all_waiting.stop_reason(start + CALL_WALL_CAP).is_some(),
            "the wall cap ends a call whose whole time was host waits"
        );
    }

    #[test]
    fn there_is_one_engine_per_process() {
        let first = engine().expect("engine");
        let second = engine().expect("engine");
        assert!(std::ptr::eq(first, second));
    }
}
