//! Device conditions that gate library updates: Wi-Fi, an unmetered network and the power source (T-08a-028).
//! Linux reads them from NetworkManager and UPower; other platforms report every condition as not met.
//! Also here: low-RAM detection, the texture limit choices and the hardware bitmap threshold (T-01a-022, T-01a-030).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};

/// The three conditions at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceState {
    /// The primary connection is Wi-Fi.
    pub on_wifi: bool,
    /// The primary connection is not metered.
    pub network_not_metered: bool,
    /// The device runs on external power.
    pub charging: bool,
}

/// Reads the current conditions. Never fails: a condition that cannot be read is not met.
pub async fn read() -> DeviceState {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    {
        crate::linux::device::read().await
    }
    #[cfg(not(all(target_os = "linux", feature = "platform-linux")))]
    {
        DeviceState::default()
    }
}

/// Whether the machine shows any sign of NetworkManager (its runtime directory). [`read`] cannot tell "wired" from
/// "NetworkManager unreadable", so callers use this to know whether a `false` Wi-Fi reading means anything.
pub fn network_manager_present() -> bool {
    cfg!(target_os = "linux") && std::path::Path::new("/run/NetworkManager").exists()
}

/// `MemTotal` below this many bytes counts as low RAM: 3 GiB (`ActivityManager.isLowRamDevice` in Android).
pub const LOW_RAM_BYTES: u64 = 3_221_225_472;

/// The device texture limit is never below this (`GLUtil.SAFE_TEXTURE_LIMIT`).
pub const SAFE_TEXTURE_LIMIT: i32 = 2048;
/// The limit used when no GL context can be created for the probe.
pub const FALLBACK_TEXTURE_LIMIT: i32 = 16384;
/// Step of the custom texture limit options (`GLUtil.MULTIPLIER`).
const TEXTURE_STEP: i32 = 1024;

/// `MemTotal` from `/proc/meminfo` text, in bytes. `None` when the line is missing or does not parse.
pub fn mem_total_bytes(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Low RAM from the text of `/proc/meminfo`. A missing file or an unparsable total is not low RAM.
pub fn is_low_ram_from(meminfo: Option<&str>) -> bool {
    meminfo.and_then(mem_total_bytes).is_some_and(|bytes| bytes < LOW_RAM_BYTES)
}

/// True when `MemTotal` is below 3 GiB. Read once per process; a missing or unparsable `/proc/meminfo` is false. On
/// Windows the total comes from `windows::device`.
pub fn is_low_ram() -> bool {
    static LOW_RAM: OnceLock<bool> = OnceLock::new();
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return *LOW_RAM.get_or_init(|| crate::windows::device::total_memory().is_some_and(|bytes| bytes < LOW_RAM_BYTES));
    #[allow(unreachable_code)]
    *LOW_RAM.get_or_init(|| is_low_ram_from(std::fs::read_to_string("/proc/meminfo").ok().as_deref()))
}

/// The `GL_MAX_TEXTURE_SIZE` the app probed at startup, or 0 when no probe succeeded. The probe itself needs a
/// short-lived `gdk::GLContext`, so the GTK layer calls [`record_texture_probe`] with its result.
static PROBED_TEXTURE_LIMIT: AtomicI32 = AtomicI32::new(0);

/// Records the probe result. Values of zero or below are ignored.
pub fn record_texture_probe(max_texture_size: i32) {
    if max_texture_size > 0 {
        PROBED_TEXTURE_LIMIT.store(max_texture_size, Ordering::Relaxed);
    }
}

/// The limit with its inputs passed in: `env` is the `KUROBAYOMI_TEXTURE_LIMIT` value and `probe` the probe result
/// (0 when none). A positive env value wins. Otherwise the limit is `max(probe, 2048)`, or 16384 without a probe.
pub fn texture_limit_from(env: Option<&str>, probe: i32) -> i32 {
    if let Some(value) = env.and_then(|s| s.trim().parse::<i32>().ok()).filter(|v| *v > 0) {
        return value;
    }
    let probe = if probe > 0 { probe } else { FALLBACK_TEXTURE_LIMIT };
    probe.max(SAFE_TEXTURE_LIMIT)
}

/// The device texture limit, resolved on the first call and then fixed for the process.
pub fn device_texture_limit() -> i32 {
    static LIMIT: OnceLock<i32> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let env = std::env::var("KUROBAYOMI_TEXTURE_LIMIT").ok();
        texture_limit_from(env.as_deref(), PROBED_TEXTURE_LIMIT.load(Ordering::Relaxed))
    })
}

/// The custom texture limit choices for a device limit, largest first: the device limit, then every multiple of
/// 1024 below it down to 2048 (`GLUtil.CUSTOM_TEXTURE_LIMIT_OPTIONS`).
///
/// Kept for T-12a-039 (it requires the generator and its vectors), which conflicts with T-13a-030 (do not port it
/// unless a maximum-decode-size setting is added). T-12a-039 wins for the generator only: there is no setting, no
/// texture-limit row and no stored `pref_hardware_bitmap_threshold`, so T-13a-030's other rules hold.
pub fn custom_texture_limit_options(device: i32) -> Vec<i32> {
    let mut out = vec![device];
    let steps = device / TEXTURE_STEP;
    for step in (2..=steps).rev() {
        let value = step * TEXTURE_STEP;
        if value < device {
            out.push(value);
        }
    }
    out
}

/// Picture-in-picture is not offered on the desktop (T-01a-046): GTK has no PiP window, and the always-on-top mini
/// player replaces it. Constant, so no code path depends on PiP.
pub const fn device_has_pip() -> bool {
    false
}

#[cfg(test)]
mod limits_tests {
    use super::*;

    #[test]
    fn options_match_the_kotlin_algorithm() {
        let sixteen = custom_texture_limit_options(16384);
        assert_eq!(sixteen.len(), 15);
        assert_eq!(sixteen.first(), Some(&16384));
        assert_eq!(sixteen.last(), Some(&2048));
        assert_eq!(sixteen[1], 15360, "steps of 1024 below the device limit");
        assert_eq!(custom_texture_limit_options(8192).len(), 7);
        assert_eq!(custom_texture_limit_options(4096), vec![4096, 3072, 2048]);
        assert_eq!(custom_texture_limit_options(3000), vec![3000, 2048]);
        assert_eq!(custom_texture_limit_options(2048), vec![2048]);
    }

    #[test]
    fn env_override_wins_and_bad_values_fall_back_to_the_probe() {
        assert_eq!(texture_limit_from(Some("4096"), 16384), 4096);
        assert_eq!(texture_limit_from(Some(" 3000 "), 0), 3000);
        assert_eq!(texture_limit_from(Some("0"), 8192), 8192);
        assert_eq!(texture_limit_from(Some("many"), 8192), 8192);
    }

    #[test]
    fn probe_is_floored_at_the_safe_limit_and_missing_probe_uses_the_fallback() {
        assert_eq!(texture_limit_from(None, 1024), SAFE_TEXTURE_LIMIT);
        assert_eq!(texture_limit_from(None, 8192), 8192);
        assert_eq!(texture_limit_from(None, 0), FALLBACK_TEXTURE_LIMIT);
    }

    #[test]
    fn low_ram_threshold_is_strictly_below_three_gib() {
        let kib = |k: u64| format!("MemFree: 1 kB\nMemTotal:       {k} kB\nMemAvailable: 1 kB\n");
        assert!(is_low_ram_from(Some(&kib(3_145_727)))); // 3 GiB minus 1 KiB
        assert!(!is_low_ram_from(Some(&kib(3_145_728))), "exactly 3 GiB is not low RAM");
        assert!(!is_low_ram_from(Some(&kib(8_000_000))));
        assert!(!is_low_ram_from(None), "a missing file is not low RAM");
        assert!(!is_low_ram_from(Some("MemTotal: many kB\n")), "an unparsable total is not low RAM");
        assert!(!is_low_ram_from(Some("MemFree: 1 kB\n")));
        assert_eq!(mem_total_bytes("MemTotal: 2 kB\n"), Some(2048));
    }
}
