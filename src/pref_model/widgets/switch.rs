//! Switch row (Kotlin `SwitchPreferenceWidget`), an `adw::SwitchRow`: a click, Enter or Space anywhere on the row
//! toggles. The row shows a new state only after the veto accepts it and the value is stored.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use kuro_prefs::pref::Pref;

use super::super::bind::{self, Veto, WriteGate};
use super::super::model::ItemCommon;
use crate::widgets::row;

/// Makes the row show `active`. `shown` is the state the row is meant to show: a notification that agrees with it is
/// our own doing, one that does not is the user's flip.
fn show(row: &adw::SwitchRow, shown: &Cell<bool>, active: bool) {
    shown.set(active);
    if row.is_active() != active {
        row.set_active(active);
    }
}

pub fn build(common: &ItemCommon, pref: Pref<bool>, veto: Veto<bool>) -> adw::SwitchRow {
    let built = row::switch(&common.title, common.subtitle.as_deref(), common.icon, pref.get());
    let shown = Rc::new(Cell::new(pref.get()));
    let gate = Rc::new(WriteGate::default());
    built.connect_active_notify({
        let (pref, shown) = (pref.clone(), shown.clone());
        move |switch| {
            let next = switch.is_active();
            if next == shown.get() {
                return;
            }
            // the row flips at once; put it back and let the write decide
            switch.set_active(shown.get());
            let (done_switch, done_pref, done_shown) = (switch.clone(), pref.clone(), shown.clone());
            bind::write(gate.clone(), veto.clone(), pref.clone(), next, move |outcome| match outcome {
                bind::Outcome::Written => show(&done_switch, &done_shown, next),
                bind::Outcome::Kept => {}
                bind::Outcome::Failed(message) => {
                    tracing::warn!(key = done_pref.key(), %message, "setting not saved");
                }
            });
        }
    });

    let weak = built.downgrade();
    bind::follow(&pref, { let weak = weak.clone(); move || weak.upgrade().is_some() }, {
        let shown = shown.clone();
        move |value| {
            if let Some(switch) = weak.upgrade() {
                show(&switch, &shown, value);
            }
        }
    });
    built.set_sensitive(common.enabled);
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::prefs::Prefs;

    fn pump() {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(150);
        while std::time::Instant::now() < until {
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    // T-47ba-048, T-E20: a flip of the row (what a click, Enter or Space does through libadwaita's switch) is written,
    // and a veto of false leaves both the row and the stored value alone. A headless GTK does not toggle a switch by its
    // `activate` signal, so the test sets the state the way the switch does after a click.
    #[test]
    fn a_flip_of_the_row_is_written_and_a_vetoed_write_changes_nothing() {
        crate::testing::with_display(|| {
            let prefs = Prefs::in_memory();
            let pref = prefs.store().pref("switch_row_test", false);
            let accepted = build(&ItemCommon::new("Accepts"), pref.clone(), bind::accept());
            accepted.set_active(true);
            pump();
            assert!(accepted.is_active() && pref.get(), "a flip is written and shown");
            accepted.set_active(false);
            pump();
            assert!(!accepted.is_active() && !pref.get(), "and flips back");

            let vetoed_pref = prefs.store().pref("switch_row_vetoed", false);
            let vetoed = build(&ItemCommon::new("Vetoed"), vetoed_pref.clone(), bind::veto(|_: bool| async { Ok(false) }));
            vetoed.set_active(true);
            pump();
            assert!(!vetoed.is_active() && !vetoed_pref.get(), "a vetoed write leaves the row and the value");
        });
    }
}
