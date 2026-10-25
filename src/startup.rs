//! Startup work on the GTK side: the texture-limit probe (T-01a-022), the splash window shown when migrations take
//! longer than 300 ms (T-01a-043) and the launcher badge for unread updates (T-01a-025).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chrono::Local;
use gtk::glib;
use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_domain::updates::repo::UpdatesRepo;
use kuro_domain::updates::window::three_months_ago;
use kuro_platform::launcher;
use kuro_prefs::store::PrefStore;

use crate::data;
use crate::watch::bind_watch;

/// How long migrations may run before the splash window shows.
pub const SPLASH_DELAY: Duration = Duration::from_millis(300);

/// `GL_MAX_TEXTURE_SIZE`.
const GL_MAX_TEXTURE_SIZE: u32 = 0x0D33;

/// Reads `GL_MAX_TEXTURE_SIZE` in a short-lived GL context and records it for `device_texture_limit()`. Without a
/// display or a GL context nothing is recorded and the limit falls back to 16384.
pub fn probe_texture_limit() {
    if let Some(size) = gl_max_texture_size() {
        kuro_platform::device::record_texture_probe(size);
    }
}

fn gl_max_texture_size() -> Option<i32> {
    let display = gtk::gdk::Display::default()?;
    let context = display.create_gl_context().ok()?;
    context.realize().ok()?;
    context.make_current();
    let size = kuro_platform::gl::get_integer(GL_MAX_TEXTURE_SIZE);
    gtk::gdk::GLContext::clear_current();
    size.filter(|s| *s > 0)
}

/// Runs `work` on a worker thread while the main loop keeps turning. When it is still running after `delay`,
/// `show_splash` runs once. `finish` then gets the result and the splash, so the caller presents its window first and
/// removes the splash after. No splash is created when `work` ends within `delay`.
pub fn run_with_splash<T, S>(
    work: impl FnOnce() -> T + Send + 'static,
    delay: Duration,
    show_splash: impl FnOnce() -> S + 'static,
    finish: impl FnOnce(T, Option<S>) + 'static,
) where
    T: Send + 'static,
    S: 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    let splash: Rc<RefCell<Option<S>>> = Rc::default();
    let done = Rc::new(Cell::new(false));
    {
        let (splash, done) = (Rc::clone(&splash), Rc::clone(&done));
        glib::spawn_future_local(async move {
            glib::timeout_future(delay).await;
            if !done.get() {
                *splash.borrow_mut() = Some(show_splash());
            }
        });
    }
    glib::spawn_future_local(async move {
        let result = rx.recv().await.expect("the startup worker panicked");
        done.set(true);
        let splash = splash.borrow_mut().take();
        finish(result, splash);
    });
}

/// Starts loading the sources from the app paths (call once on the GTK thread, inside the app's runtime). When the paths
/// cannot be found no source loads, and the gate opens empty so no screen waits on it forever.
pub fn start_sources(store: &PrefStore) {
    match kuro_platform::system_paths::SystemPaths::system() {
        Ok(paths) => {
            let shared: std::sync::Arc<dyn kuro_platform::paths::Paths + Send + Sync> = std::sync::Arc::new(paths);
            load_sources(store, &shared);
        }
        Err(error) => {
            tracing::warn!("the sources do not load: {error}");
            crate::source_gate::mark_loaded();
        }
    }
}

/// Loads the sources the screens read, then opens the source gate (`source_gate::subscribe`). The recipe sources (and
/// the shared client and image service) come first; the gate opens only after them, so a screen that waits on it never
/// reads a half-loaded list. Sources of added repositories must be loaded before this call too. When the recipe sources
/// cannot start (the client does not build) the gate still opens, with no sources, so screens show their empty state
/// instead of a spinner that never ends; `start_sources` does the same when the app paths cannot be found.
pub fn load_sources(store: &PrefStore, paths: &std::sync::Arc<dyn kuro_platform::paths::Paths + Send + Sync>) {
    let started = std::time::Instant::now();
    crate::recipe_sources::start(store, paths);
    crate::source_gate::mark_loaded();
    tracing::info!(elapsed = ?started.elapsed(), "sources loaded");
}

/// Empties the chapter cache directory once, on a worker thread, when `auto_clear_chapter_cache` is on (T-08a-037).
/// The caller runs it after the main window is presented, so the window never waits for the deletion. Returns the
/// worker, or `None` when the setting is off.
pub fn clear_chapter_cache_on_launch(enabled: bool, cache_dir: PathBuf) -> Option<std::thread::JoinHandle<()>> {
    enabled.then(|| {
        std::thread::spawn(move || {
            kuro_domain::storage::cache::clear_on_launch(true, &cache_dir);
        })
    })
}

/// The splash: a small undecorated window with the app name and a spinner. It takes no focus, and the caller
/// destroys it after presenting the main window.
pub fn splash_window(app: &gtk::Application) -> gtk::Window {
    let spinner = crate::ui::loading(crate::ui::LoadSize::Page);
    let name = gtk::Label::builder().label(crate::channel::APP_NAME).css_classes(["title-2"]).build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for widget in [name.upcast_ref::<gtk::Widget>(), spinner.upcast_ref()] {
        column.append(widget);
    }
    column.set_margin_top(32);
    column.set_margin_bottom(32);
    column.set_margin_start(48);
    column.set_margin_end(48);
    let window = gtk::Window::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .can_focus(false)
        .focusable(false)
        .child(&column)
        .build();
    window.present();
    window
}

/// The launcher entry name of the application: GApplication ids are the desktop file's name.
pub fn desktop_file_name(app_id: &str) -> String {
    format!("{app_id}.desktop")
}

/// Keeps the launcher badge at the unread updates of the last three months, cleared at 0. Needs the data to be installed.
pub fn publish_launcher_badge(app_id: &str) {
    let desktop = desktop_file_name(app_id);
    let counts = Rc::new(Cell::new([0_i64; 2]));
    let push: Rc<dyn Fn()> = Rc::new({
        let counts = Rc::clone(&counts);
        move || {
            let badge = launcher::badge(counts.get().iter().sum());
            let desktop = desktop.clone();
            tokio::spawn(async move { launcher::publish(&desktop, badge).await });
        }
    });
    let since = three_months_ago(&Local::now());
    for (slot, kind) in [Kind::Manga, Kind::Anime].into_iter().enumerate() {
        let (counts, push) = (Rc::clone(&counts), Rc::clone(&push));
        let feed = UpdatesRepo::new(data::app().db.clone()).watch_with_read(kind, false, since);
        bind_watch(feed, move |rows| {
            let Ok(rows) = rows else { return };
            let mut all = counts.get();
            all[slot] = rows.len() as i64;
            counts.set(all);
            push();
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Instant;

    use super::*;

    #[test]
    fn the_texture_probe_reads_the_limit_whenever_a_gl_context_realizes() {
        crate::testing::with_display(|| {
            let Some(display) = gtk::gdk::Display::default() else { return };
            let Ok(context) = display.create_gl_context() else { return };
            if context.realize().is_err() {
                return; // no GL on this display
            }
            assert!(gl_max_texture_size().is_some_and(|size| size >= 1024), "glGetIntegerv was not resolved");
        });
    }

    /// Runs `body` on a private main context that the test thread owns, and pumps it until `done` holds.
    fn on_context<R>(body: impl FnOnce() -> R, done: impl Fn(&R) -> bool, extra: Duration) -> R {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let result = body();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !done(&result) && Instant::now() < deadline {
                ctx.iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            // let a late timer fire, to prove it does nothing
            let end = Instant::now() + extra;
            while Instant::now() < end {
                ctx.iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            result
        })
        .expect("the test thread owns the context")
    }

    fn run(work_for: Duration, extra: Duration) -> (Rc<Cell<u32>>, Rc<Cell<Option<bool>>>) {
        let shown = Rc::new(Cell::new(0));
        let finished = Rc::new(Cell::new(None));
        let (s, f, watched) = (Rc::clone(&shown), Rc::clone(&finished), Rc::clone(&finished));
        on_context(
            move || {
                run_with_splash(
                    move || std::thread::sleep(work_for),
                    Duration::from_millis(60),
                    move || s.set(s.get() + 1),
                    move |(), splash| f.set(Some(splash.is_some())),
                );
                watched
            },
            |f| f.get().is_some(),
            extra,
        );
        (shown, finished)
    }

    #[test]
    fn no_splash_when_the_work_ends_within_the_delay() {
        // the pump keeps turning past the 60 ms delay: the timer must not create a splash after the work is done
        let (shown, finished) = run(Duration::ZERO, Duration::from_millis(120));
        assert_eq!(finished.get(), Some(false));
        assert_eq!(shown.get(), 0);
    }

    #[test]
    fn a_splash_shows_once_when_the_work_outlasts_the_delay() {
        let (shown, finished) = run(Duration::from_millis(200), Duration::ZERO);
        assert_eq!(shown.get(), 1);
        assert_eq!(finished.get(), Some(true), "finish receives the splash so the caller can remove it");
    }

    #[test]
    fn the_desktop_file_is_the_application_id() {
        assert_eq!(desktop_file_name("io.github.anas1412.Kurobayomi"), "io.github.anas1412.Kurobayomi.desktop");
    }
}
