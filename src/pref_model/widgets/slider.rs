//! Slider row (Kotlin `BaseSliderItem` on the preference path): the label with the value under it, the scale at the end.
//! The caller owns the value; a write goes through the veto, and the value follows only accepted values.

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, Orientation, Scale};

use super::super::bind::{self, Veto, WriteGate};
use super::super::model::ItemCommon;
use crate::widgets::row::{self, RowSpec};

/// The scale's width at the end of the row.
const SCALE_WIDTH: i32 = 200;

/// The value clamped into `range` (E-38). Applied before display and before the write.
pub fn clamp_to_range(value: i32, range: &RangeInclusive<i32>) -> i32 {
    value.clamp(*range.start(), *range.end())
}

/// Value text: the subtitle when it is not empty, otherwise the number (E-40).
pub fn value_text(subtitle: Option<&str>, value: i32) -> String {
    match subtitle {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => value.to_string(),
    }
}

/// The distance between two stops of a slider with `steps` stops between its ends (Material `steps`): `(last - first)
/// / (steps + 1)`. The default `steps` of the model, `last - first - 1`, gives 1.
pub fn step_size(range: &RangeInclusive<i32>, steps: u32) -> f64 {
    let span = f64::from(*range.end()) - f64::from(*range.start());
    (span / (f64::from(steps) + 1.0)).max(1.0)
}

/// The stop nearest to `value`, counted from the start of the range, kept inside it.
pub fn snap_to_step(value: f64, range: &RangeInclusive<i32>, steps: u32) -> i32 {
    let (first, step) = (f64::from(*range.start()), step_size(range, steps));
    clamp_to_range((first + ((value - first) / step).round() * step).round() as i32, range)
}

/// A drag that lands on the current value is not a change and does not write (E-39).
pub fn is_change(current: i32, next: i32) -> bool {
    current != next
}

/// The value row: title on the left, the value as the subtitle, the scale at the end. The caller owns the value; a write
/// goes through the veto, and the subtitle follows only accepted values.
pub fn row(common: &ItemCommon, value: i32, range: RangeInclusive<i32>, steps: u32, on_change: Veto<i32>) -> adw::ActionRow {
    let start = clamp_to_range(value, &range);
    let current = Rc::new(Cell::new(start));
    let suppress = Rc::new(Cell::new(false));
    let gate = Rc::new(WriteGate::default());

    let step = step_size(&range, steps);
    let scale = Scale::with_range(Orientation::Horizontal, f64::from(*range.start()), f64::from(*range.end()), step);
    scale.set_draw_value(false);
    scale.set_round_digits(0);
    scale.set_size_request(SCALE_WIDTH, -1);
    scale.set_value(f64::from(start));
    scale.set_sensitive(common.enabled);
    let built = row::build(RowSpec {
        title: &common.title,
        subtitle: Some(&value_text(common.subtitle.as_deref(), start)),
        icon: None,
        trailing: Some(scale.upcast_ref()),
        on_click: None,
    });

    let subtitle = common.subtitle.clone();
    scale.connect_value_changed({
        let (current, suppress, gate, built) = (current.clone(), suppress.clone(), gate.clone(), built.clone());
        move |scale| {
            if suppress.get() {
                return;
            }
            let next = snap_to_step(scale.value(), &range, steps);
            if !is_change(current.get(), next) {
                return;
            }
            let ticket = gate.begin();
            let (current, gate, built, subtitle, scale, suppress, on_change) = (
                current.clone(),
                gate.clone(),
                built.clone(),
                subtitle.clone(),
                scale.clone(),
                suppress.clone(),
                on_change.clone(),
            );
            glib::spawn_future_local(async move {
                let decision = bind::decide(on_change(next).await, gate.is_latest(ticket));
                match decision {
                    Ok(true) => {
                        current.set(next);
                        row::set_subtitle(&built, Some(&value_text(subtitle.as_deref(), next)));
                    }
                    Ok(false) | Err(_) => {
                        suppress.set(true);
                        scale.set_value(f64::from(current.get()));
                        suppress.set(false);
                    }
                }
            });
        }
    });
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_outside_the_range_are_clamped() {
        assert_eq!(clamp_to_range(12, &(0..=10)), 10);
        assert_eq!(clamp_to_range(-3, &(0..=10)), 0);
    }

    #[test]
    fn value_falls_back_to_the_number_when_the_subtitle_is_empty() {
        assert_eq!(value_text(Some(""), 7), "7");
        assert_eq!(value_text(None, 7), "7");
        assert_eq!(value_text(Some("ms"), 7), "ms");
    }

    // T-47ba-052: the model's `steps` set the stops of the scale; the default is every whole number.
    #[test]
    fn steps_set_the_stops_and_a_drag_snaps_to_one() {
        assert_eq!(step_size(&(0..=10), 9), 1.0, "the model default");
        assert_eq!(step_size(&(0..=100), 9), 10.0);
        assert_eq!(snap_to_step(34.0, &(0..=100), 9), 30);
        assert_eq!(snap_to_step(36.0, &(0..=100), 9), 40);
        assert_eq!(snap_to_step(7.4, &(0..=10), 9), 7);
        assert_eq!(snap_to_step(500.0, &(0..=100), 9), 100, "inside the range");
        assert_eq!(step_size(&(5..=5), 0), 1.0, "a one-value range does not divide by zero");
    }

    #[test]
    fn equal_values_are_not_a_change() {
        assert!(!is_change(4, 4));
        assert!(is_change(4, 5));
    }
}
