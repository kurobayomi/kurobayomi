//! The info banner (T-41a-045): a strip with a label and an optional button, shown until it is hidden or for a timeout of
//! 4 s, 2 s, or never. `hide_after` is the pure rule and is tested. `InfoBanner` is the thin GTK widget that runs it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use super::primitives::ToastDuration;

/// The text, the button and the duration of one banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerSpec {
    pub message: String,
    /// The button's label, when the banner has a button.
    pub action: Option<String>,
    pub duration: ToastDuration,
}

/// How long a banner stays before it hides itself. `None` for an indefinite banner.
pub fn hide_after(duration: ToastDuration) -> Option<Duration> {
    match duration {
        ToastDuration::Seconds(seconds) => Some(Duration::from_secs(u64::from(seconds))),
        ToastDuration::Indefinite => None,
    }
}

type Action = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// The banner widget: an `adw::Banner` with an optional button. Each `show` takes a new generation, so a timer started by
/// an earlier message cannot hide a newer one.
#[derive(Clone)]
pub struct InfoBanner {
    banner: adw::Banner,
    action: Action,
    generation: Rc<Cell<u64>>,
}

impl InfoBanner {
    pub fn new() -> Self {
        let banner = crate::banner::new("");
        let action: Action = Rc::new(RefCell::new(None));
        let pressed = Rc::clone(&action);
        banner.connect_button_clicked(move |_| {
            if let Some(run) = pressed.borrow().as_ref() {
                run();
            }
        });
        Self { banner, action, generation: Rc::new(Cell::new(0)) }
    }

    /// The widget to place in the page.
    pub fn widget(&self) -> &adw::Banner {
        &self.banner
    }

    /// Shows `spec` and runs `on_action` when its button is pressed. A timed banner hides itself when its time is up.
    pub fn show(&self, spec: &BannerSpec, on_action: Option<Box<dyn Fn()>>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        match (&spec.action, on_action) {
            (Some(text), Some(run)) => {
                crate::banner::set(&self.banner, &spec.message, Some(text));
                *self.action.borrow_mut() = Some(run);
            }
            _ => {
                crate::banner::set(&self.banner, &spec.message, None);
                *self.action.borrow_mut() = None;
            }
        }
        self.banner.set_revealed(true);
        if let Some(after) = hide_after(spec.duration) {
            let banner = self.clone();
            gtk::glib::timeout_add_local_once(after, move || {
                if banner.generation.get() == generation {
                    banner.hide();
                }
            });
        }
    }

    /// Hides the banner now and cancels any pending timeout.
    pub fn hide(&self) {
        self.generation.set(self.generation.get() + 1);
        self.banner.set_revealed(false);
    }
}

impl Default for InfoBanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_four_second_and_a_two_second_banner_hide_after_their_time() {
        assert_eq!(hide_after(ToastDuration::Seconds(4)), Some(Duration::from_secs(4)));
        assert_eq!(hide_after(ToastDuration::Seconds(2)), Some(Duration::from_secs(2)));
    }

    #[test]
    fn an_indefinite_banner_never_hides_itself() {
        assert_eq!(hide_after(ToastDuration::Indefinite), None);
    }

    #[test]
    fn a_default_toast_is_three_seconds_long() {
        assert_eq!(hide_after(super::super::primitives::DEFAULT_TOAST), Some(Duration::from_secs(3)));
    }
}
