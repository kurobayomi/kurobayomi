//! The unattended library update (T-26a-020, T-26a-029): the `--library-update=auto|manual` option, the hand-over of a
//! manual request to the running app, and the user timer that runs the automatic pass without a window (mode B).
//!
//! Decisions:
//! - `main` reads the option before GTK starts, like `--updates-json`, so `shell::argv` never sees it as an option of its own.
//! - `manual` hands the request to the running app over D-Bus (`app.library-update`) and exits 0. With no app running
//!   there is nobody to hand it to, so the same process runs the pass itself.
//! - The unit folder is an argument: `kuro-platform` has no accessor for the user unit folder and is outside this lane.
//!   Nothing here writes a unit by itself; the settings action calls [`enable_timer`] after the user asks for it.
//! - Mode A (the background portal, autostart) and mode B (this timer) are mutually exclusive: [`Selection`] turns one
//!   off when the other is turned on, so a pass is never started twice for one interval.


use crate::library_update::pass::Trigger;
use crate::misc::runner;

/// The option name, `--library-update=<mode>`.
pub const OPTION: &str = "--library-update";

/// What `--library-update` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The timer's run: one pass per kind, exit 0.
    Auto,
    /// A request for the running app to start a pass now.
    Manual,
}

/// The mode named on the command line: `Ok(None)` when the option is absent, an error text when its value is not
/// `auto` or `manual`. The last occurrence wins.
pub fn parse_mode(args: &[String]) -> Result<Option<Mode>, String> {
    let mut found = None;
    for arg in args {
        let Some(value) = arg.strip_prefix(OPTION).and_then(|rest| rest.strip_prefix('=')) else { continue };
        found = Some(match value {
            "auto" => Mode::Auto,
            "manual" => Mode::Manual,
            other => return Err(format!("{OPTION} takes auto or manual, got {other:?}")),
        });
    }
    Ok(found)
}

/// Runs the option from `main`: the exit status of the process.
pub fn run(mode: Mode) -> u8 {
    match mode {
        Mode::Auto => runner::run_headless(Trigger::Auto),
        Mode::Manual => forward_manual(),
    }
}

/// Activates `app.library-update("manual")` on the running app and exits 0 (T-26a-029). Without a running app, or
/// without a session bus, the pass runs here instead.
fn forward_manual() -> u8 {
    use gtk::gio;
    use gtk::prelude::*;
    let app = gio::Application::new(Some(crate::theme::icons::APP_ID), gio::ApplicationFlags::default());
    if app.register(None::<&gio::Cancellable>).is_ok() && app.is_remote() {
        app.activate_action(runner::UPDATE_ACTION, Some(&"manual".to_variant()));
        if let Some(bus) = app.dbus_connection() {
            if let Err(error) = bus.flush_sync(None::<&gio::Cancellable>) {
                eprintln!("kurobayomi: the request may not have reached the running app: {error}");
            }
        }
        return 0;
    }
    // release the name before the pass, so a window opened meanwhile becomes the primary instance
    drop(app);
    runner::run_headless(Trigger::Manual)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_option_names_auto_or_manual_and_nothing_else() {
        // T-26a-020, T-26a-029
        assert_eq!(parse_mode(&args(&["--library-update=auto"])), Ok(Some(Mode::Auto)));
        assert_eq!(parse_mode(&args(&["x.tachibk", "--library-update=manual"])), Ok(Some(Mode::Manual)));
        assert_eq!(parse_mode(&args(&["--library-update-now"])), Ok(None));
        assert!(parse_mode(&args(&["--library-update=sometimes"])).is_err());
        assert!(parse_mode(&args(&["--library-update="])).is_err());
    }

}
