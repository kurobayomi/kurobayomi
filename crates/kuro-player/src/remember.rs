//! Remembered volume (T-45b-053). `player_volume_value` is the volume a video starts at (the Settings row): a level, or
//! `-1.0` for the last one used, which the player keeps under `player_volume_last` (`-1.0` until a level was set). The
//! two are apart, so a chosen start level is never overwritten by the slider (user, 2026-11-02). Pure: the app reads
//! and writes the preferences.

/// The stored value that means no value was stored.
pub const UNSET: f32 = -1.0;

/// The level a session starts at: the chosen start level, else the last one used, or `None` to leave mpv's own.
pub fn value_to_restore(start: f32, last: f32) -> Option<f32> {
    [start, last].into_iter().find(|level| *level >= 0.0)
}

/// The value to store after a change, or `None` when the level is not a real one.
pub fn value_to_store(level: f32) -> Option<f32> {
    (level >= 0.0).then_some(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_45b_053_a_chosen_start_level_wins_then_the_last_one_used() {
        assert_eq!(value_to_restore(40.0, 70.0), Some(40.0), "a chosen start level");
        assert_eq!(value_to_restore(0.0, 70.0), Some(0.0));
        assert_eq!(value_to_restore(UNSET, 70.0), Some(70.0), "Last used");
        assert_eq!(value_to_restore(UNSET, UNSET), None, "nothing set yet");
    }

    #[test]
    fn t_45b_053_a_real_level_is_stored() {
        assert_eq!(value_to_store(55.0), Some(55.0));
        assert_eq!(value_to_store(UNSET), None);
    }
}
