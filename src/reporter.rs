//! The `--crash-report <file>` window. It shows a report written by the panic hook and can copy it,
//! reveal it, or restart the app. It opens no database, settings, runtime or stylesheet.

use kuro_i18n::{MsgKey, tr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{Align, Justification, Orientation as O, gio, glib, pango};

const APP_ID: &str = crate::channel::APP_ID;
const REPORTER_ID: &str = "io.github.anas1412.Kurobayomi.CrashReporter";
fn no_details() -> String {
    tr(MsgKey::crash_no_details)
}
fn subtitle() -> String {
    tr(MsgKey::crash_subtitle)
}

/// Shows the report at `path` and returns the exit code.
pub fn run(path: PathBuf) -> glib::ExitCode {
    // `KUROBAYOMI_REPORTER_DRY=1`: print the report path and exit, no window (T-01a-048)
    let env = |name: &str| std::env::var(name).ok();
    if kuro_core::crash::reporter_dry_run(env(kuro_core::crash::REPORTER_DRY_ENV).as_deref(), env(kuro_core::crash::ALLOW_TEST_HOOKS_ENV).as_deref(), kuro_core::crash::RELEASE_BUILD) {
        println!("{}", path.display());
        return glib::ExitCode::SUCCESS;
    }
    if gtk::init().is_err() {
        eprintln!("kurobayomi: no display for the crash reporter; crash report: {}", path.display());
        return glib::ExitCode::FAILURE;
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("Wasn't able to retrieve crash report: {e}");
            None
        }
    };
    let app = gtk::Application::builder().application_id(REPORTER_ID).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| build(app, &path, text.as_deref()));
    app.run_with_args::<&str>(&[])
}

fn build(app: &gtk::Application, path: &Path, text: Option<&str>) {
    let icon = bug_glyph();
    icon.add_css_class("accent");

    let heading = gtk::Label::builder().label(&tr(MsgKey::crash_screen_title)).css_classes(["title-2"]).halign(Align::Center).build();
    let subtitle = gtk::Label::builder().label(subtitle()).wrap(true).justify(Justification::Center).css_classes(["dim-label"]).build();

    let attrs = pango::AttrList::new();
    attrs.insert(pango::AttrFontDesc::new(&pango::FontDescription::from_string("monospace")));
    let details = gtk::Label::builder().label(text.unwrap_or(&no_details())).selectable(true).xalign(0.0).yalign(0.0).build();
    details.set_attributes(Some(&attrs));
    details.set_margin_start(12);
    details.set_margin_end(12);
    details.set_margin_top(12);
    details.set_margin_bottom(12);
    let scroll = gtk::ScrolledWindow::builder().child(&details).min_content_height(160).vexpand(true).build();
    let well = gtk::Box::new(O::Vertical, 0);
    well.add_css_class("card");
    well.append(&scroll);

    let share = gtk::Button::with_label(&tr(MsgKey::pref_dump_crash_logs));
    share.set_sensitive(text.is_some());
    let text = text.map(str::to_owned);
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    share.connect_clicked(move |_| {
        if let (Some(display), Some(text)) = (gtk::gdk::Display::default(), &text) {
            display.clipboard().set_text(text);
        }
        if let Err(e) = kuro_platform::opener::Opener::reveal(&kuro_platform::opener::SystemOpener, &dir.join("report")) {
            eprintln!("kurobayomi: could not reveal the crash report folder: {e}");
        }
    });

    let restart = gtk::Button::with_label(&tr(MsgKey::crash_screen_restart_application));
    restart.add_css_class("suggested-action");
    let quit = app.clone();
    restart.connect_clicked(move |_| {
        wait_until_app_exits(Duration::from_secs(2));
        match std::env::current_exe().map(|exe| kuro_platform::process::spawn_detached(&exe, &[])) {
            Ok(Ok(())) => {}
            Ok(Err(e)) | Err(e) => eprintln!("kurobayomi: could not restart the application: {e}"),
        }
        quit.quit();
    });

    let body = gtk::Box::new(O::Vertical, 12);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.set_margin_top(24);
    body.set_margin_bottom(24);
    body.append(&icon);
    body.append(&heading);
    body.append(&subtitle);
    body.append(&well);
    body.append(&gtk::Separator::new(O::Horizontal));
    body.append(&share);
    body.append(&restart);

    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Kurobayomi")
        .default_width(520)
        .default_height(440)
        .child(&body)
        .build();
    let keys = gtk::EventControllerKey::new();
    let close = win.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            close.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    win.add_controller(keys);
    win.present();
}

/// A 48 px bug glyph drawn in the widget's CSS colour, so the `.accent` class colours it. Icon
/// themes differ (Tela has no symbolic bug icon), so the glyph is not looked up by name.
fn bug_glyph() -> gtk::DrawingArea {
    let glyph = gtk::DrawingArea::new();
    glyph.set_content_width(48);
    glyph.set_content_height(48);
    glyph.set_halign(Align::Center);
    glyph.set_draw_func(|glyph, cr, w, h| {
        let c = glyph.color();
        cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), f64::from(c.alpha()));
        // drawn on a 16 by 16 grid
        cr.scale(f64::from(w) / 16.0, f64::from(h) / 16.0);
        cr.arc(8.0, 3.2, 2.0, 0.0, std::f64::consts::TAU);
        cr.new_sub_path();
        let _ = cr.save();
        cr.translate(8.0, 9.6);
        cr.scale(4.2, 4.6);
        cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
        let _ = cr.restore();
        for (x, y) in [(1.2, 5.8), (12.2, 5.8), (1.2, 9.2), (12.2, 9.2), (1.2, 12.4), (12.2, 12.4)] {
            cr.rectangle(x, y, 2.6, 1.6);
        }
        let _ = cr.fill();
        cr.set_line_width(1.1);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.move_to(6.2, 1.4);
        cr.line_to(4.4, 0.4);
        cr.move_to(9.8, 1.4);
        cr.line_to(11.6, 0.4);
        let _ = cr.stroke();
    });
    glyph
}

/// Waits at most `limit` for the crashed app to release its bus name. The name is held until that
/// process exits, and starting the app before then would hand off to the dying process.
fn wait_until_app_exits(limit: Duration) {
    let start = Instant::now();
    while app_is_running() && start.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// True when the session bus says someone owns the app's id. A missing bus counts as not running.
fn app_is_running() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) else {
        return false;
    };
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(APP_ID,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        500,
        None::<&gio::Cancellable>,
    )
    .ok()
    .and_then(|reply| reply.get::<(bool,)>())
    .is_some_and(|(owned,)| owned)
}
