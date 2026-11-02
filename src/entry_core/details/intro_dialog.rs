//! The skip-intro length dialog (T-39a-075, `SkipIntroLengthDialog`): a number of seconds from 0 to 255, Cancel and OK.
//! OK hands the value back; 0 turns the skip off. The start value is `viewer::dialog_start`. Thin widget.

use gtk::SpinButton;
use kuro_i18n::{MsgKey, tr};

use crate::ui::{self, DialogButton, DialogOpen as _, Role};

pub fn open(start: i32, on_ok: impl Fn(i32) + 'static) -> adw::Dialog {
    let wheel = SpinButton::with_range(0.0, 255.0, 1.0);
    wheel.set_value(f64::from(start));
    let buttons = vec![
        DialogButton::cancel(tr(MsgKey::action_cancel)),
        DialogButton::new(tr(MsgKey::action_ok), Role::Suggested, {
            let wheel = wheel.clone();
            move || on_ok(wheel.value_as_int())
        }),
    ];
    let window = ui::dialog(&tr(MsgKey::pref_intro_length), &wheel, 320, buttons);
    window.open();
    window
}
