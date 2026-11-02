//! Layout chrome: the sidebar gives way to the bottom bar below one width. libadwaita decides it with a breakpoint on the
//! window; `main.rs` only adds the immersive rule (the reader and the player show neither bar).

use adw::prelude::*;

/// Width from which the window is wide (table TC-53: 719 is compact, 720 is wide).
pub const WIDE_MIN_WIDTH_PX: i32 = 720;

/// The smallest window the app can be shrunk to (table TC-53, the phone size of the layout rules).
pub const MIN_WINDOW_SIZE: (i32, i32) = (360, 480);

/// Sets the smallest size the window can be shrunk to (T-52a-044: 360 x 480).
pub fn apply_minimum_size(win: &impl IsA<gtk::Widget>) {
    win.set_size_request(MIN_WINDOW_SIZE.0, MIN_WINDOW_SIZE.1);
}

/// The condition of a narrow window: 719 px is compact, 720 px is wide.
fn narrow_condition() -> adw::BreakpointCondition {
    adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, f64::from(WIDE_MIN_WIDTH_PX - 1), adw::LengthUnit::Px)
}

/// The window's breakpoint for a narrow window (under [`WIDE_MIN_WIDTH_PX`]). While it holds, `wide_only` is hidden,
/// `narrow_only` is shown and the window has the class `kb-narrow` (`style.css` packs the header buttons tighter).
/// `wide_only` shows and `narrow_only` hides in the widgets' own state when the window is built.
pub fn add_narrow_breakpoint(win: &adw::ApplicationWindow, wide_only: &[&gtk::Widget], narrow_only: &[&gtk::Widget]) {
    let breakpoint = adw::Breakpoint::new(narrow_condition());
    for (widgets, shown) in [(wide_only, false), (narrow_only, true)] {
        for widget in widgets {
            breakpoint.add_setter(*widget, "visible", Some(&shown.to_value()));
        }
    }
    let on = win.clone();
    breakpoint.connect_apply(move |_| on.add_css_class("kb-narrow"));
    let off = win.clone();
    breakpoint.connect_unapply(move |_| off.remove_css_class("kb-narrow"));
    win.add_breakpoint(breakpoint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_720_px_breakpoint_decides_the_chrome() {
        // TC-53: 719 px is compact, 720 px is wide
        crate::testing::with_display(|| {
            assert_eq!(narrow_condition().to_str(), "max-width: 719px");
        });
    }

    #[test]
    fn the_minimum_window_is_360_by_480() {
        assert_eq!(MIN_WINDOW_SIZE, (360, 480));
    }
}
