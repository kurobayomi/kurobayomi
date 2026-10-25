//! Positive custom brightness through the display backlight (T-43a-065, E-BRIGHT-2).
//!
//! A slider value of 1 to 100 maps linearly onto the backlight's `max_brightness`, rounded and never below 1, so a
//! positive value never switches the panel off. Only a backlight whose `brightness` file this process can write is used.
//! With none, the reader shows the custom brightness notice once per session. The level the panel had before the first
//! change is kept and put back when the feature is turned off or the value is no longer positive. Negative values stay
//! with the black layer in [`super::overlay`].

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

/// The sysfs directory that lists the backlights.
pub const SYSFS_ROOT: &str = "/sys/class/backlight";

/// The level for slider value `value` on a backlight with `max` steps: linear, rounded, at least 1.
pub fn level_for(value: i32, max: u32) -> u32 {
    let percent = value.clamp(1, 100) as u64;
    ((percent * u64::from(max) + 50) / 100).max(1) as u32
}

/// One backlight whose `brightness` file can be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    dir: PathBuf,
    max: u32,
}

impl Device {
    pub fn max(&self) -> u32 {
        self.max
    }

    fn current(&self) -> Option<u32> {
        read_u32(&self.dir.join("brightness"))
    }

    fn set(&self, level: u32) -> std::io::Result<()> {
        fs::write(self.dir.join("brightness"), level.to_string())
    }
}

fn read_u32(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The first backlight under `root` with a positive maximum and a writable `brightness` file, or `None`.
pub fn find(root: &Path) -> Option<Device> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root).ok()?.filter_map(|entry| entry.ok().map(|e| e.path())).collect();
    dirs.sort();
    dirs.into_iter().find_map(|dir| {
        let max = read_u32(&dir.join("max_brightness")).filter(|&m| m > 0)?;
        fs::OpenOptions::new().write(true).open(dir.join("brightness")).ok()?;
        Some(Device { dir, max })
    })
}

/// What the reader changed on the backlight in this process: the device and its level before the first change, and
/// whether the notice has been shown.
#[derive(Default)]
struct State {
    changed: Option<(PathBuf, u32)>,
    notified: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// The result of [`apply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The feature is off or the value is not positive: the panel is back at the level it had.
    Restored,
    /// The backlight shows the requested level.
    Applied,
    /// A positive value and no writable backlight. `notify` is true only the first time in the session.
    Unavailable { notify: bool },
}

/// Drives the backlight for the custom brightness preference: `enabled` and the slider `value`, with the devices read
/// from `root`.
pub fn apply(root: &Path, enabled: bool, value: i32) -> Outcome {
    if !enabled || value <= 0 {
        restore();
        return Outcome::Restored;
    }
    let Some(device) = find(root) else {
        return unavailable();
    };
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if state.changed.is_none() {
            state.changed = device.current().map(|level| (device.dir.clone(), level));
        }
    });
    match device.set(level_for(value, device.max())) {
        Ok(()) => Outcome::Applied,
        Err(error) => {
            tracing::warn!(%error, "the backlight did not take the custom brightness");
            unavailable()
        }
    }
}

fn unavailable() -> Outcome {
    let notify = STATE.with(|s| !std::mem::replace(&mut s.borrow_mut().notified, true));
    Outcome::Unavailable { notify }
}

fn restore() {
    let Some((dir, level)) = STATE.with(|s| s.borrow_mut().changed.take()) else {
        return;
    };
    if let Err(error) = fs::write(dir.join("brightness"), level.to_string()) {
        tracing::warn!(%error, "the backlight could not be restored");
    }
}
