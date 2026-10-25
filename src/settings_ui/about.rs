//! The About page rows this module binds (tasks T-47a-093 and T-47a-094): the Version row, whose subtitle carries the
//! build and which copies the debug information when tapped, and the Check for updates row, which runs the manual
//! release check of `app_update`. The row is hidden when the updater is off in this build. The other rows of the spec
//! page stay as they are.
//!
//! Decisions: the build date is the raw build time string (the Android date-format fallback), and the debug block uses
//! the labels of the debug-info page's own groups. The Check for updates row runs its own copy of the manual check so it
//! knows when the check ends: a spinner shows while it runs and a second tap is ignored (T-47a-094). Aniyomi's "no
//! longer supported" toast belongs to an Android API level; Linux has no such rule, so the row has no EOL branch.
use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_core::app_info::APP_INFO;
use kuro_domain::release::check::Outcome;
use kuro_domain::release::github::{GithubReleases, ReleaseHttp};
use kuro_domain::release::{Release, ReleaseError};
use kuro_i18n::{tr, MsgKey};

use crate::app_update;
use crate::pref_model::debug_info::{app_rows, device_rows, model_text};
use crate::pref_model::model::{Preference, PreferenceItem};
use crate::settings_ui::changelog;
use crate::settings_ui::clipboard::copy_to_clipboard;
use crate::settings_ui::rows::{item, tap_item, text, TapRow};
use crate::ui::{DialogButton, Role};

pub fn version_title() -> String {
    tr(MsgKey::ext_info_version)
}
pub fn build_time_title() -> String {
    tr(MsgKey::about_build_time_title)
}
pub fn model_title() -> String {
    tr(MsgKey::about_model_title)
}

/// The Version subtitle: the build's version line (`debug_info::current_version_text`) and its build time in brackets.
pub fn version_subtitle() -> String {
    format!("{} ({})", crate::pref_model::debug_info::current_version_text(), APP_INFO.build_time)
}

/// The debug information block the Version row copies: one `Title: value` line per row, app rows first.
pub fn debug_block(app: &[(String, String)], device: &[(String, String)]) -> String {
    app.iter().chain(device).map(|(title, value)| format!("{title}: {value}")).collect::<Vec<_>>().join("\n")
}

/// The debug information as the About page copies it, read now.
pub fn current_debug_block() -> String {
    let read = |path: &str| std::fs::read_to_string(path).ok();
    let model = model_text(read("/sys/class/dmi/id/sys_vendor").as_deref(), read("/sys/class/dmi/id/product_name").as_deref());
    let gtk_version = format!("{}.{}.{}", gtk::major_version(), gtk::minor_version(), gtk::micro_version());
    let app = app_rows(&version_title(), &build_time_title());
    let device = device_rows(&model_title(), model, &kuro_platform::system::facts(), gtk_version);
    debug_block(&app, &device)
}

/// The Version row: the build subtitle, and a tap copies the debug information.
pub fn version_row() -> PreferenceItem {
    let mut row = text(&version_title(), Some(&version_subtitle()), true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(std::rc::Rc::new(|| copy_to_clipboard(&current_debug_block())));
    }
    row
}

/// The Version row of an AppImage that updates itself: the build, then how the update goes (checking, a download with its
/// bar, installed, failed) and the buttons at the end: check now, a spinner while checking, Pause (then Resume) and
/// Cancel while a download runs, Restart once installed. A tap on the text copies the debug information, as on the plain
/// row.
fn update_center_row() -> PreferenceItem {
    use crate::appimage_update::{self as update, Status};
    use crate::pref_model::widgets::row::{self, RowSpec};
    PreferenceItem::Custom {
        title: version_title().to_owned(),
        build: Rc::new(|| {
            let button = gtk::Button::new();
            button.set_valign(gtk::Align::Center);
            let spinner = crate::ui::loading(crate::ui::LoadSize::Inline);
            let pause = crate::ui::icon_button("media-playback-pause-symbolic", &tr(MsgKey::action_pause));
            let cancel = crate::ui::icon_button("window-close-symbolic", &tr(MsgKey::action_cancel));
            for download_button in [&pause, &cancel] {
                download_button.set_valign(gtk::Align::Center);
                download_button.add_css_class("flat");
            }
            let end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            end.append(&spinner);
            end.append(&pause);
            end.append(&cancel);
            end.append(&button);
            let subtitle = version_subtitle();
            let built = row::build(RowSpec { title: &version_title(), subtitle: Some(&subtitle), icon: None, trailing: Some(end.upcast_ref()), on_click: None });
            let content = built.row.child().and_downcast::<gtk::Box>().expect("the row content");
            built.row.set_child(None::<&gtk::Widget>);
            let state = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(built.subtitle.css_classes()).build();
            // the state line with the download's spinner right after its text, then the bar
            let bar_spinner = crate::ui::loading(crate::ui::LoadSize::Inline);
            let state_row = gtk::Box::builder().spacing(6).build();
            state_row.append(&state);
            state_row.append(&bar_spinner);
            let bar = gtk::ProgressBar::builder().margin_top(6).build();
            if let Some(column) = built.subtitle.parent().and_downcast::<gtk::Box>() {
                column.append(&state_row);
                column.append(&bar);
            }
            let copy = gtk::GestureClick::new();
            copy.connect_released(|_, _, _, _| copy_to_clipboard(&current_debug_block()));
            content.add_controller(copy);
            button.connect_clicked(|_| match update::status() {
                Status::Installed(_) => crate::whats_new::restart(),
                _ => update::check(),
            });
            pause.connect_clicked(|_| match update::status() {
                Status::Paused { .. } => update::resume(),
                _ => update::pause(),
            });
            cancel.connect_clicked(|_| update::cancel());
            let weak = (button.downgrade(), spinner.downgrade(), state.downgrade(), bar.downgrade(), bar_spinner.downgrade(), pause.downgrade(), cancel.downgrade());
            update::watch(move |status| {
                let (Some(button), Some(spinner), Some(state), Some(bar), Some(bar_spinner), Some(pause), Some(cancel)) =
                    (weak.0.upgrade(), weak.1.upgrade(), weak.2.upgrade(), weak.3.upgrade(), weak.4.upgrade(), weak.5.upgrade(), weak.6.upgrade())
                else {
                    return false;
                };
                let line = status.line();
                state.set_visible(line.is_some());
                state.set_label(line.as_deref().unwrap_or_default());
                bar.set_visible(status.fraction().is_some());
                bar_spinner.set_visible(matches!(status, Status::Downloading { .. }));
                bar.set_fraction(status.fraction().unwrap_or(0.0));
                // Pause turns into Resume while paused
                let downloading = status.fraction().is_some();
                let paused = matches!(status, Status::Paused { .. });
                pause.set_visible(downloading);
                cancel.set_visible(downloading);
                let (icon, tip) = if paused { ("media-playback-start-symbolic", tr(MsgKey::action_resume)) } else { ("media-playback-pause-symbolic", tr(MsgKey::action_pause)) };
                pause.set_icon_name(icon);
                pause.set_tooltip_text(Some(&tip));
                pause.update_property(&[gtk::accessible::Property::Label(&tip)]);
                // the end spinner while checking; during a download the bar's own spinner
                spinner.set_visible(status.busy() && status.fraction().is_none());
                button.set_visible(!status.busy());
                let installed = matches!(status, Status::Installed(_));
                if installed {
                    button.set_icon_name("");
                    button.set_label(&crate::whats_new::action_label());
                    button.set_tooltip_text(None);
                    button.remove_css_class("flat");
                    button.add_css_class("suggested-action");
                } else {
                    button.set_icon_name("view-refresh-symbolic");
                    button.set_tooltip_text(Some(&app_update::check_row_title()));
                    button.add_css_class("flat");
                }
                true
            });
            content.upcast()
        }),
    }
}

/// The switch that lets the app look for updates at start (`check_app_updates`, on by default).
fn startup_check_row() -> PreferenceItem {
    let trailing: Rc<dyn Fn() -> gtk::Widget> = Rc::new(|| {
        let switch = gtk::Switch::builder().valign(gtk::Align::Center).active(crate::appimage_update::check_at_startup()).build();
        switch.connect_active_notify(|switch| {
            if crate::data::is_installed()
                && let Err(error) = crate::data::app().base.check_app_updates.set(switch.is_active())
            {
                tracing::warn!(%error, "the update switch could not be stored");
            }
        });
        switch.upcast()
    });
    let on_tap: Rc<dyn Fn(&TapRow)> = Rc::new(|row| {
        if let Some(switch) = row.trailing.as_ref().and_then(|widget| widget.downcast_ref::<gtk::Switch>()) {
            switch.set_active(!switch.is_active());
        }
    });
    tap_item(&startup_check_title(), Some(&startup_check_subtitle()), true, Some(trailing), None, on_tap)
}

/// The switch that adds the AppImage to the applications menu or takes it out (`appimage_integrate`). When an AppImage
/// manager already lists it, the row says so and has no switch.
fn applications_menu_row() -> PreferenceItem {
    if crate::appimage_integrate::managed_elsewhere() {
        return text(&menu_title(), Some(&tr(MsgKey::ui_your_appimage_manager_already_lists_it_there)), false);
    }
    let trailing: Rc<dyn Fn() -> gtk::Widget> = Rc::new(|| {
        let switch = gtk::Switch::builder().valign(gtk::Align::Center).active(crate::appimage_integrate::in_menu()).build();
        switch.connect_active_notify(|switch| {
            crate::appimage_integrate::set_removed_by_user(!switch.is_active());
            if switch.is_active() {
                crate::appimage_integrate::add();
            } else {
                crate::appimage_integrate::remove();
            }
        });
        switch.upcast()
    });
    let on_tap: Rc<dyn Fn(&TapRow)> = Rc::new(|row| {
        if let Some(switch) = row.trailing.as_ref().and_then(|widget| widget.downcast_ref::<gtk::Switch>()) {
            switch.set_active(!switch.is_active());
        }
    });
    tap_item(&menu_title(), Some(&tr(MsgKey::ui_with_its_icon_like_an_installed_app)), true, Some(trailing), None, on_tap)
}

pub fn menu_title() -> String {
    tr(MsgKey::about_menu_title)
}

pub fn startup_check_title() -> String {
    tr(MsgKey::about_startup_check_title)
}
fn startup_check_subtitle() -> String {
    tr(MsgKey::about_startup_check_subtitle)
}

/// Whether the release check exists in this build (the updater feature and a repository).
pub fn check_available() -> bool {
    app_update::enabled(APP_INFO.updater_enabled, app_update::UPDATE_REPOSITORY)
}

/// What a finished check shows.
#[derive(Debug, PartialEq, Eq)]
pub enum Report {
    /// A toast with this text.
    Toast(String),
    /// The new-version dialog for this release.
    Dialog(Release),
}

/// The message of a finished manual check: the error text when the lookup failed, the dialog for a newer release.
pub fn report(result: Result<Outcome, ReleaseError>) -> Report {
    match result {
        Ok(Outcome::NoNewUpdate) => Report::Toast(tr(MsgKey::update_check_no_new_updates)),
        Ok(Outcome::NewUpdate(release)) => Report::Dialog(release),
        Err(e) => Report::Toast(e.to_string()),
    }
}

/// One check at a time: a second tap while a check runs does nothing.
#[derive(Default)]
pub struct InFlight(Cell<bool>);

impl InFlight {
    /// True when the caller may start a check (and now holds the flag).
    pub fn begin(&self) -> bool {
        !self.0.replace(true)
    }

    pub fn end(&self) {
        self.0.set(false);
    }
}

thread_local! {
    static CHECKING: InFlight = InFlight::default();
}

/// The GET behind [`GithubReleases`], over a blocking client built on the worker thread.
struct Http(reqwest::blocking::Client);

impl ReleaseHttp for Http {
    fn get(&self, url: &str, user_agent: &str) -> Result<(u16, Vec<u8>), String> {
        let response = self.0.get(url).header(reqwest::header::USER_AGENT, user_agent).send().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response.bytes().map_err(|e| e.to_string())?;
        Ok((status, body.to_vec()))
    }
}

/// The forced check, off the UI thread: `done` runs on the main context with the result.
fn run_check(done: impl FnOnce(Result<Outcome, ReleaseError>) + 'static) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map(Http)
            .map_err(|e| ReleaseError::Transport(e.to_string()))
            .and_then(|http| {
                let service = GithubReleases::new(http, std::env::consts::ARCH);
                app_update::check(&service, app_update::UPDATE_REPOSITORY, env!("CARGO_PKG_VERSION"), chrono::Utc::now().timestamp_millis(), true)
            });
        let _ = tx.send_blocking(result);
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(result) = rx.recv().await {
            done(result);
        }
    });
}

/// A release-notes dialog (the update page, T-47ba-030, ui-parity 3.5, and What's new): a 48 px icon, the heading, the
/// version, the notes (markdown) in a scrolling body, the release page when there is one, and the buttons: `primary` after
/// Not now, or Close alone. Escape closes it.
pub fn notes_dialog(heading: &str, version: &str, notes_markdown: &str, link: Option<&str>, primary: Option<DialogButton>) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let icon = gtk::Image::builder().icon_name("software-update-available-symbolic").pixel_size(48).halign(gtk::Align::Center).build();
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    column.append(&icon);
    column.append(&gtk::Label::builder().label(heading).css_classes(["title-1"]).wrap(true).build());
    column.append(&gtk::Label::builder().label(version).css_classes(["dim-label"]).build());

    let notes = gtk::Label::builder().use_markup(true).wrap(true).xalign(0.0).yalign(0.0).selectable(true).build();
    notes.set_markup(&changelog::to_pango(notes_markdown));
    notes.connect_activate_link(|_, url| {
        crate::ui::open_link(url, None);
        gtk::glib::Propagation::Stop
    });
    let scroll = gtk::ScrolledWindow::builder().child(&notes).propagate_natural_height(true).max_content_height(360).hscrollbar_policy(gtk::PolicyType::Never).build();
    column.append(&scroll);

    if let Some(link) = link.filter(|link| changelog::link_visible(link)) {
        let open = gtk::Button::builder().label(tr(MsgKey::update_check_open)).icon_name("web-browser-symbolic").halign(gtk::Align::Start).build();
        open.add_css_class("flat");
        let link = link.to_owned();
        open.connect_clicked(move |_| crate::ui::open_link(&link, None));
        column.append(&open);
    }

    let buttons = match primary {
        Some(primary) => vec![DialogButton::cancel(tr(MsgKey::action_not_now)), primary],
        None => vec![DialogButton::cancel(&tr(MsgKey::action_close))],
    };
    crate::ui::dialog(heading, &column, 520, buttons).present();
}

/// The update page of a newer stable release. Decision (T-47ba-032): there is no in-place install for a stable build
/// (T-35a-056 drops it). Download opens the release's file link with the desktop's handler and closes the page. Offline,
/// the handler fails and a toast says so.
pub fn show_update(release: &Release) {
    let file = release.download_link.clone();
    let download = DialogButton::new(tr(MsgKey::update_check_confirm), Role::Suggested, move || {
        crate::ui::open_link_or(&file, None, |_| crate::toast::notify(tr(MsgKey::update_check_notification_download_error)));
    });
    notes_dialog(&tr(MsgKey::update_check_notification_update_available), &release.version, &release.info, Some(&release.release_link), Some(download));
}

/// A tap on the row: starts a check unless one runs, shows the spinner meanwhile, then the report.
fn check_tapped(row: &TapRow) {
    if !CHECKING.with(InFlight::begin) {
        return;
    }
    let spinner = row.trailing.clone().and_downcast::<gtk::Spinner>();
    if let Some(spinner) = &spinner {
        spinner.set_visible(true);
        spinner.start();
    }
    run_check(move |result| {
        if let Err(e) = &result {
            tracing::warn!(error = %e, "the update check failed");
        }
        match report(result) {
            Report::Toast(text) => crate::toast::notify(text),
            Report::Dialog(release) => show_update(&release),
        }
        if let Some(spinner) = &spinner {
            spinner.stop();
            spinner.set_visible(false);
        }
        CHECKING.with(InFlight::end);
    });
}

/// The Check for updates row. Where the build has no updater the row is a disabled twin, which drops the spec row.
pub fn check_row() -> PreferenceItem {
    if !check_available() {
        return text(&app_update::check_row_title(), None, false);
    }
    let spinner: Rc<dyn Fn() -> gtk::Widget> = Rc::new(|| {
        let spinner = crate::ui::loading(crate::ui::LoadSize::Inline);
        spinner.set_visible(false);
        spinner.upcast()
    });
    tap_item(&app_update::check_row_title(), None, true, Some(spinner), None, Rc::new(check_tapped))
}

/// This app's project page. Aniyomi's website, Discord and privacy page describe the Android app, so none is linked.
pub const PROJECT_URL: &str = "https://github.com/kurobayomi/kurobayomi";
/// Where bugs are reported.
pub const ISSUES_URL: &str = "https://github.com/kurobayomi/kurobayomi/issues";
/// What's new: the waiting update when there is one (its subtitle says which), else the notes of this build
/// (`whats_new`). Off in a local build, which has no release to show.
fn whats_new_row() -> PreferenceItem {
    let mut row = text(&tr(MsgKey::whats_new), crate::whats_new::row_subtitle().as_deref(), crate::whats_new::row_enabled());
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(crate::whats_new::open));
    }
    row
}

/// The bottom row of links: the project page and the issue tracker.
fn links_row() -> PreferenceItem {
    let build: Rc<dyn Fn() -> gtk::Widget> = Rc::new(|| {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bar.set_halign(gtk::Align::Center);
        for (label, url) in [("GitHub", PROJECT_URL), (&tr(MsgKey::ui_report_an_issue), ISSUES_URL)] {
            let button = gtk::Button::with_label(label);
            button.add_css_class("flat");
            button.connect_clicked(move |_| crate::ui::open_link(url, None));
            bar.append(&button);
        }
        bar.upcast()
    });
    PreferenceItem::Custom { title: tr(MsgKey::ui_links).to_owned(), build }
}

/// The typed rows of the About page, merged over the spec rows of the same titles. The links row has no spec twin and
/// is kept at the end.
pub fn page_rows() -> Vec<Preference> {
    // an AppImage that updates itself shows it all on the Version row; another build keeps the plain row and the check row
    // (the disabled twin of the check row drops the spec's row of that title)
    let (version, check) = if crate::appimage_update::can_update() {
        (update_center_row(), text(&app_update::check_row_title(), None, false))
    } else {
        (version_row(), check_row())
    };
    let startup = (crate::appimage_update::can_update() || check_available()).then(startup_check_row);
    let menu = crate::appimage_integrate::available().then(applications_menu_row);
    [Some(version), Some(check), startup, menu, Some(whats_new_row())]
        .into_iter()
        .flatten()
        .map(item)
        .chain([
        item(crate::settings_ui::maintenance::share_crash_logs_row()),
        item(links_row()),
    ])
    .collect()
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn the_links_lead_to_this_apps_project_and_never_to_aniyomi() {
        let urls = [PROJECT_URL, ISSUES_URL];
        for url in urls {
            assert!(url.starts_with("https://github.com/kurobayomi/kurobayomi"), "{url}");
        }
        assert!(!urls.iter().any(|u| u.contains("aniyomi")));
    }

    #[test]
    fn the_page_has_no_privacy_row_and_the_link_bar_is_the_last_row() {
        let rows = page_rows();
        let titles: Vec<&str> = rows.iter().map(|r| match r { Preference::Item(i) => i.title(), Preference::Group(g) => g.title.as_str() }).collect();
        assert!(!titles.contains(&"Privacy policy"));
        assert_eq!(titles.last(), Some(&"Links"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_debug_block_lists_the_app_rows_then_the_device_rows() {
        let app = vec![("Version".to_owned(), "Stable 1.0".to_owned())];
        let device = vec![("GTK".to_owned(), "4.16.0".to_owned())];
        assert_eq!(debug_block(&app, &device), "Version: Stable 1.0\nGTK: 4.16.0");
    }

    #[test]
    fn the_version_subtitle_ends_with_the_build_time_in_brackets() {
        let subtitle = version_subtitle();
        assert!(subtitle.ends_with(&format!("({})", APP_INFO.build_time)), "{subtitle}");
    }

    #[test]
    fn the_check_row_is_a_dropped_twin_where_the_updater_is_not_built() {
        let row = check_row();
        assert_eq!(row.title(), app_update::check_row_title());
        assert_eq!(matches!(row, PreferenceItem::Custom { .. }), check_available());
        assert!(row.enabled() || !check_available());
    }

    #[test]
    fn a_second_tap_while_a_check_runs_is_refused_until_it_ends() {
        let flight = InFlight::default();
        assert!(flight.begin());
        assert!(!flight.begin());
        flight.end();
        assert!(flight.begin());
    }

    #[test]
    fn a_finished_check_reports_the_error_text_the_no_update_toast_or_the_release() {
        assert_eq!(report(Ok(Outcome::NoNewUpdate)), Report::Toast(tr(MsgKey::update_check_no_new_updates)));
        let release = Release { version: "v2".into(), info: "notes".into(), release_link: "https://x".into(), download_link: "https://y".into() };
        assert_eq!(report(Ok(Outcome::NewUpdate(release.clone()))), Report::Dialog(release));
        let err = ReleaseError::Transport("offline".into());
        assert_eq!(report(Err(err.clone())), Report::Toast(err.to_string()));
    }

    #[test]
    fn the_version_row_carries_the_build_subtitle_and_copies_on_tap() {
        let PreferenceItem::Text { common, on_click, .. } = version_row() else { panic!("a text row") };
        assert_eq!(common.title, version_title());
        assert_eq!(common.subtitle.as_deref(), Some(version_subtitle().as_str()));
        assert!(on_click.is_some());
    }
}
