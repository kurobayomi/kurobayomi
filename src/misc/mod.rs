//! App-level wiring of the library update jobs (area 26): the automatic update interval, the runner behind the
//! library's refresh button and the schedule loops. Pure decisions are in `interval` and `runner`; GTK only
//! appears where the preference watch is polled on the main thread.

pub mod badge;
pub mod diet_defaults;
pub mod headless;
pub mod ids;
pub mod notify;
pub mod notify_icon;
pub mod reveal;
pub mod router;
pub mod rules;
pub mod runner;
pub mod shortcuts;

/// Starts the update runner and the schedule once, from the GTK main thread inside the app's runtime.
pub fn start() {
    notify::attach();
    runner::start();
}
