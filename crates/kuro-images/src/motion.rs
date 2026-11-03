//! Transition timing for images and screens (area:36 navigation, T-34a-054 and T-34a-060).

use std::time::Duration;

/// Length of the image crossfade when animations are on.
pub const CROSSFADE: Duration = Duration::from_millis(300);
/// Length of the screen crossfade. Forward and back use the same fade.
pub const SCREEN_FADE: Duration = Duration::from_millis(300);

/// The crossfade for a new image. `animations_enabled` is `gtk-enable-animations` read from `GtkSettings` at the time
/// of the load. With animations off the image appears at once.
pub fn crossfade(animations_enabled: bool) -> Duration {
    if animations_enabled {
        CROSSFADE
    } else {
        Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_is_300_ms_only_when_animations_are_on() {
        assert_eq!(crossfade(true), Duration::from_millis(300));
        assert_eq!(crossfade(false), Duration::ZERO);
    }
}
