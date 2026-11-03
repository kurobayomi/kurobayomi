//! Playback over one libmpv handle: loading an episode, the transport, seeking and volume. No GTK here; the
//! window and the headless test both drive it. Times are read back from mpv and kept in milliseconds.

use kuro_player::load::start_command_ms;
use kuro_player::mpv::{MpvError, MpvHandle};
use kuro_player::seek::{absolute_flag, relative_flag};

/// Seek step of the arrow keys, and of Shift with them (the long step is the shortcut table's 60 s).
pub const SEEK_STEP_MS: i64 = 10_000;
pub const SEEK_LONG_STEP_MS: i64 = 60_000;
/// Volume change of one Up or Down press, on mpv's 0 to 100 scale.
pub const VOLUME_STEP: f64 = 5.0;
pub const VOLUME_MAX: f64 = 100.0;

pub struct Playback {
    mpv: MpvHandle,
}

impl Playback {
    pub fn new(mpv: MpvHandle) -> Self {
        Self { mpv }
    }

    pub fn mpv(&self) -> &MpvHandle {
        &self.mpv
    }

    pub fn mpv_mut(&mut self) -> &mut MpvHandle {
        &mut self.mpv
    }

    /// Loads `target` (a path or a stream URL) at `start_ms` with the `http-header-fields` value `headers`, and plays
    /// it. mpv starts paused, so the load also unpauses.
    pub fn load(&self, target: &str, headers: Option<&str>, start_ms: i64) -> Result<(), String> {
        if headers.is_none() {
            // mpv keeps the option across loadfile: the previous episode's headers must not reach the next host
            self.mpv.set_property_str("http-header-fields", "").map_err(|e| e.to_string())?;
        }
        for command in load_commands(target, headers, start_ms) {
            self.run(&command).map_err(|e| e.to_string())?;
        }
        self.mpv
            .set_property_flag("pause", false)
            .map_err(|e| e.to_string())
    }

    /// A line on the video for a moment (mpv's own on-screen text): what a menu pick chose, shown at once while the
    /// change itself may take a moment (a quality loads, a subtitle shows at its next line).
    pub fn show_text(&self, text: &str) {
        if let Err(e) = self.mpv.command(&["show-text", text, "1500"]) {
            tracing::debug!(error = %e, "player: the on-screen text was not shown");
        }
    }

    /// Adds the side tracks of a stream (`audio-add`, `sub-add`). Runs once mpv has loaded the file; a track that
    /// fails is logged and the others still load.
    /// Adds the side tracks off the GTK thread: each one is fetched over the network, so a source with a dozen
    /// languages froze the window for seconds when they ran here.
    /// `prepare` rewrites each command on that thread first (the subtitle cache, `sub_cache`).
    pub fn add_tracks(&self, commands: &[Vec<String>], prepare: Option<super::sub_cache::Prepare>) {
        match prepare {
            Some(prepare) => self.mpv.run_commands_in_background_with(commands.to_vec(), prepare),
            None => self.mpv.run_commands_in_background(commands.to_vec()),
        }
    }

    fn run(&self, command: &[String]) -> Result<(), MpvError> {
        let args: Vec<&str> = command.iter().map(String::as_str).collect();
        self.mpv.command(&args)
    }

    pub fn is_paused(&self) -> bool {
        self.mpv.get_flag("pause").unwrap_or(true)
    }

    /// Toggles pause and returns the new paused state.
    pub fn toggle_pause(&self) -> Result<bool, MpvError> {
        let paused = !self.is_paused();
        self.mpv.set_property_flag("pause", paused)?;
        Ok(paused)
    }

    /// Ends playback. Used on close, so audio stops before the handle goes away.
    pub fn stop(&self) {
        if let Err(e) = self.mpv.command(&["stop"]) {
            tracing::warn!(error = %e, "mpv stop failed");
        }
    }

    /// The playhead in milliseconds, or `None` while no file has a position.
    pub fn position_ms(&self) -> Option<i64> {
        self.mpv.get_f64("time-pos").map(secs_to_ms)
    }

    pub fn duration_ms(&self) -> Option<i64> {
        self.mpv.get_f64("duration").map(secs_to_ms)
    }

    pub fn eof_reached(&self) -> bool {
        self.mpv.get_flag("eof-reached").unwrap_or(false)
    }

    pub fn seek_to_ms(&self, ms: i64) -> Result<(), MpvError> {
        let secs = format_secs(ms);
        self.mpv.command(&["seek", secs.as_str(), absolute_flag(true)])
    }

    pub fn seek_by_ms(&self, delta_ms: i64) -> Result<(), MpvError> {
        let secs = format_secs(delta_ms);
        self.mpv.command(&["seek", secs.as_str(), relative_flag(true)])
    }

    pub fn volume(&self) -> f64 {
        self.mpv.get_f64("volume").unwrap_or(VOLUME_MAX)
    }

    pub fn set_volume(&self, volume: f64) -> Result<(), MpvError> {
        self.mpv.set_property_f64("volume", clamp_volume(volume))
    }
}

/// The commands of one load: the start position, the request headers when there are any, then `loadfile`. A window
/// plays one stream, so the headers are set on the core rather than as `loadfile` options.
pub fn load_commands(target: &str, headers: Option<&str>, start_ms: i64) -> Vec<Vec<String>> {
    let mut commands = vec![start_command_ms(start_ms)];
    commands.extend(headers.map(|h| vec!["set".to_owned(), "http-header-fields".to_owned(), h.to_owned()]));
    // No index argument: mpv before 0.38 reads the third argument as the options.
    commands.push(vec!["loadfile".to_owned(), target.to_owned(), "replace".to_owned()]);
    commands
}

fn secs_to_ms(secs: f64) -> i64 {
    (secs * 1000.0).round() as i64
}

/// Seconds with millisecond precision, as mpv's command arguments take them.
fn format_secs(ms: i64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

pub fn clamp_volume(volume: f64) -> f64 {
    volume.clamp(0.0, VOLUME_MAX)
}

/// The volume after `steps` presses of Up (positive) or Down (negative), clamped to the scale.
pub fn volume_after_steps(current: f64, steps: i32) -> f64 {
    clamp_volume(current + f64::from(steps) * VOLUME_STEP)
}

/// The seek offset of an arrow key: forward or back, by the short or the long step.
pub fn seek_delta_ms(forward: bool, long: bool) -> i64 {
    let step = if long { SEEK_LONG_STEP_MS } else { SEEK_STEP_MS };
    if forward { step } else { -step }
}

/// The position a seek-bar value asks for. `fraction` is the bar's 0 to 1 value, clamped to the duration.
pub fn seek_target_ms(fraction: f64, duration_ms: i64) -> i64 {
    (fraction.clamp(0.0, 1.0) * duration_ms as f64).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_clamped_to_the_mpv_scale() {
        assert_eq!(clamp_volume(-3.0), 0.0);
        assert_eq!(clamp_volume(140.0), VOLUME_MAX);
        assert_eq!(clamp_volume(55.5), 55.5);
    }

    #[test]
    fn volume_steps_move_by_five_and_stop_at_the_ends() {
        assert_eq!(volume_after_steps(50.0, 1), 55.0);
        assert_eq!(volume_after_steps(50.0, -2), 40.0);
        assert_eq!(volume_after_steps(98.0, 1), VOLUME_MAX);
        assert_eq!(volume_after_steps(2.0, -1), 0.0);
    }

    #[test]
    fn arrow_keys_seek_ten_seconds_and_shift_seeks_sixty() {
        assert_eq!(seek_delta_ms(true, false), 10_000);
        assert_eq!(seek_delta_ms(false, false), -10_000);
        assert_eq!(seek_delta_ms(true, true), 60_000);
        assert_eq!(seek_delta_ms(false, true), -60_000);
    }

    #[test]
    fn seek_bar_fraction_maps_to_the_duration_and_is_clamped() {
        assert_eq!(seek_target_ms(0.5, 4_000), 2_000);
        assert_eq!(seek_target_ms(0.0, 4_000), 0);
        assert_eq!(seek_target_ms(1.7, 4_000), 4_000);
        assert_eq!(seek_target_ms(-0.2, 4_000), 0);
    }

    #[test]
    fn a_local_file_sets_no_headers_and_loadfile_has_no_index_argument() {
        assert_eq!(
            load_commands("/v/ep.mkv", None, 90_000),
            vec![start_command_ms(90_000), vec!["loadfile".to_owned(), "/v/ep.mkv".to_owned(), "replace".to_owned()]]
        );
    }

    #[test]
    fn seconds_round_to_the_nearest_millisecond() {
        assert_eq!(secs_to_ms(2.9999), 3000);
        assert_eq!(secs_to_ms(1.2344), 1234);
        assert_eq!(format_secs(2500), "2.500");
        assert_eq!(format_secs(-10_000), "-10.000");
    }
}
