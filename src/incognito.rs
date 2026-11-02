//! Incognito indicator (T-01a-020): the desktop notification, the `app.incognito-off` action and the `app.incognito`
//! check item of the app menu. The preference is the one source of truth; every change goes through [`set`], which
//! updates the notification and the check item (the panel's title icon follows the item) so nothing needs a restart.
//! The window banner is `shell::banners`.

use std::cell::RefCell;

use gtk::prelude::*;
use kuro_platform::incognito as platform;
use kuro_prefs::pref::Pref;

use crate::gio_notifier::GioNotifier;

#[derive(Clone)]
struct State {
    incognito: Pref<bool>,
    notifier: GioNotifier,
    /// The app menu's check item, `app.incognito`.
    toggle: gtk::gio::SimpleAction,
}

/// The name of the app menu's check item, an application action with a boolean state.
pub const TOGGLE_NAME: &str = "incognito";
/// The check item as the app menu names it.
pub const TOGGLE_ACTION: &str = "app.incognito";
/// The incognito icon of the sidebar row: an eye with a line through it.
pub const ICON: &str = "view-conceal-symbolic";

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Registers `app.incognito-off` and `app.incognito` and applies the stored value (notification and check item).
/// Call once from the GTK main thread.
pub fn attach(app: &gtk::Application, incognito: Pref<bool>) {
    let off = gtk::gio::SimpleAction::new("incognito-off", None);
    off.connect_activate(|_, _| set(false));
    app.add_action(&off);
    let toggle = gtk::gio::SimpleAction::new_stateful(TOGGLE_NAME, None, &incognito.get().to_variant());
    toggle.connect_change_state(|_, value| {
        if let Some(on) = value.and_then(|v| v.get::<bool>()) {
            set(on);
        }
    });
    app.add_action(&toggle);

    let notifier = GioNotifier::new(app);
    STATE.with(|s| {
        *s.borrow_mut() = Some(State { incognito, notifier, toggle });
    });
    refresh();
}

/// Stores `on` and updates the notification and the check item. No-op when unchanged.
pub fn set(on: bool) {
    let Some(st) = STATE.with(|s| s.borrow().clone()) else { return };
    if st.incognito.get() == on {
        return;
    }
    if let Err(e) = st.incognito.set(on) {
        tracing::warn!(error = %e, "could not save incognito mode");
    }
    refresh();
}

fn refresh() {
    let Some(st) = STATE.with(|s| s.borrow().clone()) else { return };
    let on = st.incognito.get();
    platform::sync_notification(on, &st.notifier);
    st.toggle.set_state(&on.to_variant());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_menu_item_names_the_application_action() {
        assert_eq!(TOGGLE_ACTION, format!("app.{TOGGLE_NAME}"));
    }

    #[test]
    fn the_notification_default_action_is_the_incognito_off_action() {
        assert_eq!(platform::DISABLE_ACTION, "app.incognito-off");
        assert_eq!(platform::notification_key(), "incognito_mode_channel:-701");
    }
}
