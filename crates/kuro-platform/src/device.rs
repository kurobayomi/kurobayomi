//! Device conditions that gate library updates: Wi-Fi, an unmetered network and the power source (T-08a-028).
//! Linux reads them from NetworkManager and UPower; other platforms report every condition as not met.
//! Also here: low-RAM detection and the texture probe result (T-01a-022, T-01a-030).

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
#[allow(unreachable_code)]
pub fn is_low_ram() -> bool {
    static LOW_RAM: OnceLock<bool> = OnceLock::new();
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return *LOW_RAM.get_or_init(|| crate::windows::device::total_memory().is_some_and(|bytes| bytes < LOW_RAM_BYTES));
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

#[cfg(test)]
mod limits_tests {
    use super::*;

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
