//! The bar's Speed button and its menu: six presets, the playing one marked, and the button names the playing speed.
//! The speed is written to mpv, which keeps it while the page plays on. Label text comes from `kuro_player::speed`.

use std::rc::Rc;

use gtk::prelude::*;
use kuro_player::speed::speed_label;

use super::super::Shared;
use super::Choice;

/// The speeds the menu offers, slowest first.
pub const PRESETS: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// Shows the playing speed on the bar's Speed button. Called on every tick, so the text changes only when it differs.
pub fn refresh(button: &gtk::MenuButton, shared: &Shared) {
    let text = speed_label(playing(shared) as f32);
    if button.label().as_deref() != Some(text.as_str()) {
        button.set_label(&text);
    }
}

/// The list's entries: the presets with the playing one marked.
pub fn choices(shared: &Shared) -> Vec<Choice> {
    let playing = playing(shared);
    PRESETS
        .iter()
        .map(|&speed| {
            let shared = Rc::clone(shared);
            Choice {
                title: speed_label(speed),
                subtitle: None,
                current: is_current(speed, playing),
                enabled: true,
                pick: Box::new(move || set_speed(&shared, speed)),
            }
        })
        .collect()
}

/// [ and ]: the next slower or faster preset, shown in a toast too (the bar may be hidden).
pub fn step(shared: &Shared, steps: i32) {
    let target = stepped(playing(shared), steps);
    set_speed(shared, target);
    crate::toast::notify(speed_label(target));
}

/// Backspace: normal speed.
pub fn reset(shared: &Shared) {
    set_speed(shared, 1.0);
    crate::toast::notify(speed_label(1.0));
}

/// The preset `steps` away from `playing` (slower for negative steps), kept within the presets.
fn stepped(playing: f64, steps: i32) -> f32 {
    let at = PRESETS.iter().position(|&p| is_current(p, playing)).map_or_else(
        // between presets: the nearest one in the step's direction
        || if steps > 0 { PRESETS.iter().position(|&p| f64::from(p) > playing).unwrap_or(PRESETS.len() - 1) as i32 - 1 } else { PRESETS.iter().rposition(|&p| f64::from(p) < playing).unwrap_or(0) as i32 + 1 },
        |i| i as i32,
    );
    PRESETS[(at + steps).clamp(0, PRESETS.len() as i32 - 1) as usize]
}

/// The speed mpv plays at; 1.0 before the file has one.
fn playing(shared: &Shared) -> f64 {
    shared.borrow().playback.mpv().get_f64("speed").unwrap_or(1.0)
}

fn set_speed(shared: &Shared, speed: f32) {
    if let Err(e) = shared.borrow().playback.mpv().set_property_f64("speed", f64::from(speed)) {
        tracing::warn!(error = %e, "player: the speed was not set");
    }
}

/// Whether `preset` is the playing speed. mpv stores a float, so the check allows a rounding error.
fn is_current(preset: f32, playing: f64) -> bool {
    (f64::from(preset) - playing).abs() < 0.005
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_are_the_six_speeds_in_order() {
        assert_eq!(PRESETS, [0.5, 0.75, 1.0, 1.25, 1.5, 2.0]);
    }

    #[test]
    fn brackets_step_through_the_presets_and_stop_at_the_ends() {
        assert_eq!(stepped(1.0, 1), 1.25);
        assert_eq!(stepped(1.0, -1), 0.75);
        assert_eq!(stepped(2.0, 1), 2.0, "no wrap");
        assert_eq!(stepped(0.5, -1), 0.5);
        assert_eq!((stepped(1.1, 1), stepped(1.1, -1)), (1.25, 1.0), "between presets: the next one each way");
    }

    #[test]
    fn the_check_follows_the_playing_speed_within_rounding() {
        assert!(is_current(1.25, 1.25));
        assert!(is_current(1.0, 1.0004));
        assert!(!is_current(1.25, 1.0));
    }
}
