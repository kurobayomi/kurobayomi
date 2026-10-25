//! The Tracking rows this module binds (tasks T-47a-053 to T-47a-059), now on the Sources and tracking page: the
//! progress switch, the auto-track switch (on only while a tracker is logged in) and the MyAnimeList and AniList rows with their login and logout dialogs. Those two are the only
//! trackers with a row; the other trackers keep their code and their token keys but are not shown (settings diet). The three cut switches
//! (open the track menu, next airing time, progress on mark) are not shown either.
//!
//! The rows are driven by `kuro_track::login_rows` over the services of [`tracker_host`]: the check mark follows the
//! stored login, a browser row opens the authorise URL and shows a pending state until the redirect arrives, a
//! credentials row opens the login dialog, and a logged-in row asks to log out. The services are built off the GTK
//! thread, so the page shows logged-out rows first and redraws when they are ready.
//!
//! Decisions: a failed credentials login keeps the old session and the dialog open (the rule of
//! `kuro_track::dialog`, which does not port Aniyomi's logout on failure).
use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use kuro_prefs::prefs::Prefs;
use kuro_track::dialog::{login_title, logout_title, run_login, run_logout, username_label, LoginForm};
use kuro_track::login_rows::{login_row_views, row_subtitle, tap_row, LoginRow, LoginRowView, RowTap, LOGIN_ROWS};
use kuro_track::messages::english;
use kuro_track::oauth::ClientSecrets;
use kuro_track::tracker::Tracker;

use crate::pref_model::model::{Preference, PreferenceItem};
use crate::pref_model::widgets::dialog;
use crate::settings_ui::rows::{group, info, item, switch, switch_with, tap_item, TapRow};
use crate::settings_ui::tracker_host::{self, now_ms, Host};

/// Shown when a row is tapped before the tracker services are built, or when they could not be.
pub fn not_ready() -> String {
    tr(MsgKey::settings_tracking_not_ready)
}
/// The subtitle of a browser login that waits for its redirect.
pub fn waiting_text() -> String {
    tr(MsgKey::settings_tracking_waiting)
}
/// The info text under the tracker rows (`tracking_info`).
pub fn tracking_info() -> String {
    tr(MsgKey::tracking_info)
}

/// Whether the login button is enabled: both fields have text and no login is in flight.
pub fn login_enabled(username: &str, password: &str, in_flight: bool) -> bool {
    LoginForm { username: username.to_owned(), password: password.to_owned(), processing: in_flight, input_error: false }.can_submit()
}

/// The warning rows: one while the secrets live in the fallback file because no keyring is reachable.
pub fn warning_rows(fallback: bool) -> Vec<Preference> {
    tracker_host::keyring_warning(if fallback { kuro_platform::secrets::SecretsMode::EncryptedFile } else { kuro_platform::secrets::SecretsMode::Primary })
        .map(|text| item(info(text)))
        .into_iter()
        .collect()
}

/// The login dialog (Kitsu and MangaUpdates): the username or email field, the password field with its show toggle, and
/// the Login button, which stays disabled until both fields have text and shows `Loading…` while the login runs.
fn open_login_dialog(tracker: Arc<dyn Tracker>, changed: Rc<dyn Fn()>) {
    let name = tracker.name().to_owned();
    let form = LoginForm::for_tracker(tracker.as_ref());
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let user = gtk::Entry::builder().placeholder_text(username_label(tracker.id())).text(&form.username).build();
    let pass = gtk::PasswordEntry::builder().show_peek_icon(true).placeholder_text(&tr(MsgKey::password)).text(&form.password).build();
    column.append(&user);
    column.append(&pass);
    let shown: Rc<std::cell::RefCell<Option<dialog::Dialog>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let (user, pass, shown) = (user.clone(), pass.clone(), Rc::clone(&shown));
        Rc::new(move || {
            if let Some(dialog) = shown.borrow().as_ref() {
                dialog.set_confirm_enabled(login_enabled(&user.text(), &pass.text(), false));
            }
        })
    };
    for changed_field in [user.clone().upcast::<gtk::Editable>(), pass.clone().upcast::<gtk::Editable>()] {
        let refresh = Rc::clone(&refresh);
        changed_field.connect_changed(move |_| refresh());
    }
    let (user_text, pass_text) = (user.clone(), pass.clone());
    let dialog = dialog::show(
        &crate::ui::window(),
        &login_title(&name),
        &column,
        Some(LoginForm::for_tracker(tracker.as_ref()).button_label()),
        move |dialog| {
            let (username, password) = (user_text.text().to_string(), pass_text.text().to_string());
            let busy = LoginForm { username: username.clone(), password: password.clone(), processing: true, input_error: false };
            dialog.set_error(None);
            dialog.set_confirm_enabled(false);
            if let Some(button) = &dialog.confirm {
                button.set_label(busy.button_label());
            }
            let (tracker, changed) = (Arc::clone(&tracker), Rc::clone(&changed));
            crate::source_bridge::call_on_main(
                {
                    let tracker = Arc::clone(&tracker);
                    async move { run_login(tracker.as_ref(), &username, &password).await }
                },
                move |result| match result {
                    Ok(key) => {
                        crate::toast::notify(english(&key));
                        dialog.finish();
                        changed();
                    }
                    Err(key) => {
                        dialog.set_error(Some(&english(&key)));
                        dialog.set_confirm_enabled(true);
                        if let Some(button) = &dialog.confirm {
                            button.set_label(LoginForm { username: String::new(), password: String::new(), processing: false, input_error: true }.button_label());
                        }
                    }
                },
            );
        },
        || {},
    );
    dialog.set_confirm_enabled(login_enabled(&user.text(), &pass.text(), false));
    *shown.borrow_mut() = Some(dialog);
}

/// The logout dialog: the title names the tracker, Log out clears the credentials and says `You are now logged out`.
fn open_logout_dialog(tracker: Arc<dyn Tracker>, changed: Rc<dyn Fn()>) {
    let title = logout_title(tracker.name());
    let body = gtk::Label::builder().label(&title).wrap(true).build();
    dialog::show(
        &crate::ui::window(),
        &title,
        &body,
        Some(&tr(MsgKey::logout)),
        move |dialog| match run_logout(tracker.as_ref()) {
            Ok(key) => {
                crate::toast::notify(english(&key));
                dialog.finish();
                changed();
            }
            Err(key) => dialog.set_error(Some(&english(&key))),
        },
        || {},
    );
}

/// What a tap on a login row does: the decision is `kuro_track::login_rows::tap_row`, this carries it out.
fn tapped(index: usize, changed: &Rc<dyn Fn()>) {
    let Some(host) = tracker_host::host() else {
        crate::toast::notify(not_ready());
        return;
    };
    let Some(view) = login_row_views(&host.manager).get(index).copied() else { return };
    let id = view.row.tracker;
    let tap = tap_row(&view, &mut host.pending.borrow_mut(), &ClientSecrets::default(), &host.manager, now_ms());
    let Some(tracker) = host.manager.get(id) else { return };
    match tap {
        Some(RowTap::ConfirmLogout) => open_logout_dialog(tracker, Rc::clone(changed)),
        Some(RowTap::OpenCredentials) => open_login_dialog(tracker, Rc::clone(changed)),
        Some(RowTap::OpenBrowser(url)) => {
            host.begin_waiting(id, now_ms());
            crate::ui::open_link(&url, None);
            changed();
        }
        Some(RowTap::StartMalLogin) => tracker_host::start_mal_login(&host, Rc::clone(changed)),
        // Simkl is switched off: its row is not shown, so no tap reaches this arm
        Some(RowTap::StartSimklLogin(_)) => {}
        Some(RowTap::Toast(key)) => crate::toast::notify(english(&key)),
        None => {
            if let Some(hint) = tracker.setup_message() {
                crate::toast::notify(english(&hint));
            }
        }
    }
}

/// The check mark of a logged-in row.
fn check_mark() -> gtk::Widget {
    let label = crate::pref_model::strings::logged_in();
    let image = gtk::Image::from_icon_name("object-select-symbolic");
    image.add_css_class("tracker-check");
    image.set_pixel_size(crate::pref_model::widgets::TRACKER_CHECK_SIZE);
    image.set_tooltip_text(Some(&label));
    image.set_accessible_role(gtk::AccessibleRole::Img);
    image.update_property(&[gtk::accessible::Property::Label(&label)]);
    image.upcast()
}

/// The login state of every row, read from the services (logged out while they are not built).
fn views(host: Option<&Host>) -> Vec<LoginRowView> {
    match host {
        Some(host) => login_row_views(&host.manager),
        None => LOGIN_ROWS.iter().map(|&row| LoginRowView { row, logged_in: false, expired: false, configured: true }).collect(),
    }
}

/// The subtitle of a login row: the tracker's own state text, or the pending text while its browser login waits.
fn subtitle(host: Option<&Host>, row: LoginRow) -> Option<String> {
    let host = host?;
    let tracker = host.manager.get(row.tracker)?;
    let waiting = host.is_waiting(row.tracker, now_ms()) && !tracker.is_logged_in();
    row_subtitle(tracker.as_ref(), waiting)
        .map(|key| english(&key))
        .or_else(|| waiting.then(waiting_text))
}

/// The tracker row for one login row: a check when logged in, a subtitle for pending or expired states, and a tap that
/// runs the row's action.
fn tracker_item(index: usize, view: LoginRowView, host: Option<&Host>, changed: &Rc<dyn Fn()>) -> PreferenceItem {
    let trailing: Option<Rc<dyn Fn() -> gtk::Widget>> = view.logged_in.then(|| Rc::new(check_mark) as Rc<dyn Fn() -> gtk::Widget>);
    let changed = Rc::clone(changed);
    let on_tap: Rc<dyn Fn(&TapRow)> = Rc::new(move |_| tapped(index, &changed));
    let logo = match kuro_track::logo::logo_asset(view.row.tracker) {
        Some(kuro_track::tracker::LogoRef::Embedded(bytes)) => Some(bytes),
        _ => None,
    };
    tap_item(view.row.name, subtitle(host, view.row).as_deref(), true, trailing, logo, on_tap)
}

/// Starts a tracker's browser login from the tracking page: the same action as its row here. `changed` redraws the page
/// when the login starts, and again when it finishes.
pub fn connect(tracker: kuro_track::model::TrackerId, changed: Rc<dyn Fn()>) {
    if let Some(index) = LOGIN_ROWS.iter().position(|row| row.tracker == tracker) {
        tapped(index, &changed);
    }
}

/// The trackers the app switches on, in the order the entry's tracking page shows them. The others keep their code and
/// their token keys, but get no row, no progress write and no refresh.
pub const ENABLED: [kuro_track::model::TrackerId; 2] = [kuro_track::model::TrackerId::ANILIST, kuro_track::model::TrackerId::MYANIMELIST];

/// The trackers that have a row: the [`ENABLED`] ones.
pub fn tracker_enabled(tracker: kuro_track::model::TrackerId) -> bool {
    ENABLED.contains(&tracker)
}

/// The typed rows of the Tracking screen. `changed` redraws the page after a login, a logout or a pending state.
pub fn page_rows(prefs: &Prefs, changed: Rc<dyn Fn()>) -> Vec<Preference> {
    let tracking = kuro_prefs::tracking::TrackPrefs::new(prefs.store().clone());
    let host = tracker_host::host();
    let views = views(host.as_deref());
    // the index is the row's place in `LOGIN_ROWS`, which a tap uses to find its service, so it is taken before the filter
    let mut trackers: Vec<PreferenceItem> = views
        .iter()
        .enumerate()
        .filter(|(_, view)| tracker_enabled(view.row.tracker))
        .map(|(index, view)| tracker_item(index, *view, host.as_deref(), &changed))
        .collect();
    trackers.push(info(&tracking_info()));
    // auto-track binds on the switched-on trackers, so its switch works once one of them is logged in
    let any_tracker = views.iter().any(|view| tracker_enabled(view.row.tracker) && view.logged_in);
    let auto_track = kuro_track::prefs::TrackPrefs::new(prefs.store().clone()).auto_track;
    let mut rows = warning_rows(host.as_ref().is_some_and(|host| host.fallback));
    rows.extend([
        item(switch(&tr(MsgKey::pref_auto_update_manga_sync), &tracking.auto_update_track, true)),
        item(switch_with(&tr(MsgKey::track_auto_title), Some(&tr(MsgKey::track_auto_summary)), &auto_track, any_tracker, crate::pref_model::bind::accept())),
        group(&tr(MsgKey::services), trackers),
    ]);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_needs_both_fields_and_no_login_in_flight() {
        assert!(login_enabled("me@example.org", "secret", false));
        assert!(!login_enabled("  ", "secret", false));
        assert!(!login_enabled("me", "", false));
        assert!(!login_enabled("me", "secret", true));
    }

    #[test]
    fn the_seven_tracker_rows_follow_the_login_row_order() {
        let names: Vec<&str> = LOGIN_ROWS.iter().map(|row| row.name).collect();
        assert_eq!(names, ["MyAnimeList", "AniList", "Kitsu", "MangaUpdates", "Shikimori", "Simkl", "Bangumi"]);
    }

    #[test]
    fn before_the_services_exist_every_row_is_logged_out_and_unconfigured_state_is_not_claimed() {
        let rows = views(None);
        assert_eq!(rows.len(), 7);
        assert!(rows.iter().all(|view| !view.logged_in && !view.expired && view.configured));
        assert_eq!(subtitle(None, LOGIN_ROWS[0]), None);
    }

    #[test]
    fn the_warning_row_appears_only_in_fallback_mode() {
        assert!(warning_rows(false).is_empty());
        let rows = warning_rows(true);
        assert!(matches!(rows.as_slice(), [Preference::Item(PreferenceItem::Info { text, .. })] if text == "Tokens stored unencrypted"));
    }

    #[test]
    fn the_page_holds_the_progress_and_auto_track_switches_and_a_tracker_group_with_myanimelist_and_anilist() {
        let prefs = Prefs::in_memory();
        let rows = page_rows(&prefs, Rc::new(|| {}));
        let titles: Vec<&str> = rows.iter().filter_map(|entry| match entry {
            Preference::Item(item) => Some(item.title()),
            Preference::Group(_) => None,
        }).collect();
        assert_eq!(titles, ["Update progress after reading/watching", "Track new titles automatically"]);
        let Some(Preference::Group(trackers)) = rows.iter().find(|entry| matches!(entry, Preference::Group(g) if g.title == "Trackers")) else {
            panic!("the Trackers group")
        };
        let names: Vec<&str> = trackers.items.iter().map(|row| row.title()).collect();
        assert_eq!(names, ["MyAnimeList", "AniList", &tracking_info()]);
    }

    #[test]
    fn only_anilist_and_myanimelist_can_log_in_until_the_others_are_fixed() {
        use kuro_track::model::TrackerId;
        assert!(tracker_enabled(TrackerId::ANILIST));
        assert!(tracker_enabled(TrackerId::MYANIMELIST));
        for other in [TrackerId::KITSU, TrackerId::SIMKL] {
            assert!(!tracker_enabled(other));
        }
    }

    #[test]
    fn a_tracker_row_is_a_tappable_row_titled_with_the_tracker() {
        let changed: Rc<dyn Fn()> = Rc::new(|| {});
        let view = views(None)[2];
        let row = tracker_item(2, view, None, &changed);
        assert!(matches!(row, PreferenceItem::Custom { ref title, .. } if title == "Kitsu"));
    }
}
