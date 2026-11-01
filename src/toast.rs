//! The app's one notification bar (T-12a-014 presenter). `kuro_platform::toast` queues the messages; this shows them one
//! at a time at the bottom centre of the main window, each for its length, as `adw::Toast`s of one `adw::ToastOverlay`.
//! The entry pages' Undo and Retry prompts use it too (`entry_core::message::Notice`). GTK objects stay on the main thread.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use kuro_platform::toast::{ToastFeed, Toaster, toast_channel};
use crate::theme::tokens::{SPACE_M};

// A new message replaces the one on screen (the old toast is dismissed first). The toast's close button or Escape
// dismisses it.
//
// Manual verification (no display in CI): the toast never takes keyboard focus. The Escape handler is a capture-phase
// controller that grabs nothing, so with a text entry focused, showing a toast leaves the caret in the entry.

static GLOBAL: std::sync::OnceLock<Toaster> = std::sync::OnceLock::new();

/// A toast that shows a spinner and `text` over the main window until the returned guard is dropped: the "Loading..."
/// state of an action that waits for the network. Without an overlay (tests without a window) it does nothing.
pub fn busy(text: &str) -> Busy {
    let toast = LAYER.with(|l| l.borrow().clone()).map(|layer| {
        let spinner = gtk::Spinner::builder().spinning(true).width_request(24).height_request(24).build();
        let label = gtk::Label::new(Some(text));
        let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(SPACE_M).build();
        row.append(&spinner);
        row.append(&label);
        let toast = adw::Toast::builder().custom_title(&row).timeout(0).build();
        layer.overlay.add_toast(toast.clone());
        toast
    });
    Busy(toast)
}

/// Dismisses the toast of [`busy`] when dropped.
pub struct Busy(Option<adw::Toast>);

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(toast) = self.0.take() {
            toast.dismiss();
        }
    }
}

/// Short toast through the overlay attached by [`attach`]; dropped when no overlay exists yet.
pub fn notify(text: impl Into<String>) {
    if let Some(t) = GLOBAL.get() {
        t.show(text, kuro_platform::toast::ToastLength::Short);
    }
}

/// The toast layer: the overlay and the toast on screen. One per window ([`attach`]).
struct Layer {
    overlay: adw::ToastOverlay,
    /// The plain toast or prompt shown last; a new one dismisses it first.
    current: RefCell<Option<adw::Toast>>,
}

thread_local! {
    static LAYER: RefCell<Option<Rc<Layer>>> = const { RefCell::new(None) };
}

impl Layer {
    fn dismiss(&self) {
        // the borrow ends before `dismiss`, whose `dismissed` signal reads `current`
        let old = self.current.borrow_mut().take();
        if let Some(toast) = old {
            toast.dismiss();
        }
    }

    /// Shows `text`, replacing the toast on screen. `duration` None keeps it until dismissed or replaced.
    fn show(self: &Rc<Self>, text: &str, duration: Option<Duration>, action: Option<(&str, Rc<dyn Fn()>)>) {
        self.dismiss();
        let secs = duration.map_or(0, |d| d.as_millis().div_ceil(1000).max(1) as u32);
        let toast = adw::Toast::builder().title(text).use_markup(false).timeout(secs).build();
        if let Some((label, run)) = action {
            toast.set_button_label(Some(label));
            let this = Rc::downgrade(self);
            toast.connect_button_clicked(move |t| {
                if let Some(this) = this.upgrade() {
                    let mine = this.current.borrow().as_ref() == Some(t);
                    if mine {
                        this.current.borrow_mut().take();
                    }
                }
                run();
            });
        }
        let this = Rc::downgrade(self);
        toast.connect_dismissed(move |t| {
            if let Some(this) = this.upgrade() {
                let mine = this.current.borrow().as_ref() == Some(t);
                if mine {
                    this.current.borrow_mut().take();
                }
            }
        });
        *self.current.borrow_mut() = Some(toast.clone());
        self.overlay.add_toast(toast);
    }
}

/// How long a toast with an action stays: long enough to reach for Undo or Retry.
pub const ACTION_DURATION: Duration = Duration::from_secs(5);

/// A toast for `duration` instead of the fixed 2 s. Must run on the GTK main thread; dropped when no layer is attached.
pub fn notify_for(text: &str, duration: Duration) {
    if let Some(layer) = LAYER.with(|l| l.borrow().clone()) {
        layer.show(text, Some(duration), None);
    }
}

/// A toast with one action button (Undo, Retry): `run` runs when it is pressed, and the toast goes away. `duration`
/// None keeps the toast until the user dismisses it (its close button or Escape) or another toast replaces it.
/// Must run on the GTK main thread; dropped when no layer is attached.
pub fn notify_action(text: &str, action_label: &str, duration: Option<Duration>, run: impl Fn() + 'static) {
    if let Some(layer) = LAYER.with(|l| l.borrow().clone()) {
        let run: Rc<dyn Fn()> = Rc::new(run);
        layer.show(text, duration, Some((action_label, run)));
    }
}

/// Starts the toast presenter on `overlay` (the main window's `adw::ToastOverlay`) and on the main context. Returns the
/// sender that producers use. Call once, from the GTK main thread.
pub fn attach(overlay: &adw::ToastOverlay) -> Toaster {
    let (toaster, feed) = toast_channel();
    let layer = Rc::new(Layer { overlay: overlay.clone(), current: RefCell::new(None) });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let layer = layer.clone();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape && layer.current.borrow().is_some() {
                layer.dismiss();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    overlay.add_controller(keys);

    LAYER.with(|l| *l.borrow_mut() = Some(layer.clone()));
    glib::spawn_future_local(present(feed, layer, toaster.clone()));
    let _ = GLOBAL.set(toaster.clone());
    toaster
}

/// `keep_open` holds a sender for as long as the presenter runs, so the feed stays open even when the
/// returned handle is dropped.
async fn present(feed: ToastFeed, layer: Rc<Layer>, keep_open: Toaster) {
    while let Some(message) = feed.next().await {
        layer.show(&message.text, Some(message.length.duration()), None);
    }
    drop(keep_open);
}

/// The toast on screen, for tests.
#[cfg(test)]
pub(crate) fn current() -> Option<adw::Toast> {
    LAYER.with(|l| l.borrow().clone()).and_then(|layer| layer.current.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_platform::toast::ToastLength;
    use std::cell::Cell;

    fn pump(ms: u64) {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn title() -> Option<String> {
        current().and_then(|t| t.title()).map(|t| t.to_string())
    }

    // T-47b-052, E73: one toast at a time in an adw::ToastOverlay; a new one replaces the visible one and the last one
    // times out after its 2 s.
    #[test]
    fn a_new_toast_replaces_the_visible_one_and_the_last_times_out_after_two_seconds() {
        crate::testing::with_display(|| {
            assert_eq!(ToastLength::Short.duration(), std::time::Duration::from_secs(2));
            let overlay = adw::ToastOverlay::new();
            let toaster = attach(&overlay);
            assert!(current().is_none(), "nothing shows before the first toast");

            toaster.show("first", ToastLength::Short);
            pump(300);
            assert_eq!(title().as_deref(), Some("first"));
            assert_eq!(current().expect("toast").timeout(), 2);
            toaster.show("second", ToastLength::Short);
            pump(100);
            assert_eq!(title().as_deref(), Some("second"), "a new toast replaces the visible one");

            // the first toast was dismissed; it must not clear the second one
            pump(1000);
            assert_eq!(title().as_deref(), Some("second"));
            pump(2500);
            assert!(current().is_none(), "gone 2 s after the last toast");
        });
    }

    // The entry pages' Undo and Retry prompts go through the same layer. A prompt has its button; pressing it runs it
    // once and clears the toast; a new toast replaces the prompt.
    #[test]
    fn a_prompt_runs_its_action_once_and_a_new_toast_replaces_it() {
        crate::testing::with_display(|| {
            let overlay = adw::ToastOverlay::new();
            attach(&overlay);
            notify_for("plain", Duration::from_secs(2));
            assert_eq!(current().expect("toast").button_label(), None, "a plain toast has no button");

            let runs = Rc::new(Cell::new(0));
            let count = Rc::clone(&runs);
            notify_action("Delete downloaded chapters?", "Delete", None, move || count.set(count.get() + 1));
            let toast = current().expect("the prompt");
            assert_eq!(toast.button_label().as_deref(), Some("Delete"));
            assert_eq!(toast.timeout(), 0, "a prompt without a duration stays until dismissed");
            toast.emit_by_name::<()>("button-clicked", &[]);
            assert_eq!(runs.get(), 1);
            assert!(current().is_none(), "the action clears the toast");

            notify_action("Retry?", "Retry", None, || {});
            notify_for("plain", Duration::from_secs(2));
            assert_eq!(current().expect("toast").button_label(), None, "a new toast replaces the prompt and its button");
        });
    }

    // busy() adds a toast with a spinner and dismisses it when the guard drops.
    #[test]
    fn busy_holds_a_spinner_toast_until_dropped() {
        crate::testing::with_display(|| {
            let overlay = adw::ToastOverlay::new();
            attach(&overlay);
            let guard = busy("Loading");
            let toast = guard.0.clone().expect("the busy toast");
            assert!(toast.custom_title().is_some() && toast.timeout() == 0);
            let dismissed = Rc::new(Cell::new(false));
            let flag = Rc::clone(&dismissed);
            toast.connect_dismissed(move |_| flag.set(true));
            drop(guard);
            pump(500);
            assert!(dismissed.get(), "dropping the guard dismisses the toast");
        });
    }
}
