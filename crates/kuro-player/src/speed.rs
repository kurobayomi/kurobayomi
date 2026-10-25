//! Playback speed label, the speed cycle and the keyboard step (area 45 `PlayerViewModel.playbackSpeed`,
//! behaviour 8.6; area 46 `BottomLeftPlayerControls.kt`,
//! T-45b-038 and T-46a-008). Pure: the caller writes the chosen speed to mpv and reads the new value
//! back through the observer.

/// The speed after one tap on the speed button: `+0.25`, and back to `0.25` once the speed reaches
/// `2.0` (`BottomLeftPlayerControls.kt:69`).
pub fn next_speed(current: f32) -> f32 {
    if current >= 2.0 { 0.25 } else { current + 0.25 }
}

/// Speed button text, `%.2fx`.
pub fn speed_label(speed: f32) -> String {
    format!("{speed:.2}x")
}

/// The slider range (`PlaybackSpeedSheet`, E67): 0.01 to 6.00, continuous.
pub const SPEED_MIN: f32 = 0.01;
pub const SPEED_MAX: f32 = 6.0;

/// Keyboard step on the desktop: 0.05, or 0.01 with Shift (p2-ui-parity section 3.2).
pub fn keyboard_step(shift: bool) -> f32 {
    if shift { 0.01 } else { 0.05 }
}

/// A speed moved by `steps` keyboard steps, kept in the slider range.
pub fn step_speed(current: f32, steps: i32, shift: bool) -> f32 {
    (current + keyboard_step(shift) * steps as f32).clamp(SPEED_MIN, SPEED_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_cycle_wraps_at_two() {
        assert_eq!(next_speed(1.1), 1.35);
        assert_eq!(next_speed(0.25), 0.5);
        assert_eq!(next_speed(1.0), 1.25);
        assert_eq!(next_speed(1.75), 2.0);
        assert_eq!(next_speed(2.0), 0.25);
        assert_eq!(next_speed(3.0), 0.25);
    }

    #[test]
    fn label_has_two_decimals() {
        assert_eq!(speed_label(1.5), "1.50x");
        assert_eq!(speed_label(1.0), "1.00x");
    }

    #[test]
    fn keyboard_steps_are_005_and_001_with_shift_and_stay_in_range() {
        assert_eq!(keyboard_step(false), 0.05);
        assert_eq!(keyboard_step(true), 0.01);
        assert_eq!(step_speed(1.0, 1, false), 1.05);
        assert_eq!(step_speed(1.0, -1, true), 0.99);
        assert_eq!(step_speed(0.02, -50, false), SPEED_MIN);
        assert_eq!(step_speed(5.99, 10, false), SPEED_MAX);
    }
}
