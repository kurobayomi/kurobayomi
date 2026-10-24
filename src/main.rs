// a release build on Windows opens no console window next to the app (a debug build keeps it, for the logs)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_update;
mod appimage_update;
mod channel;
mod whats_new;
mod appimage_integrate;
mod backup_host;
mod backup_run;
mod backup_schedule;
mod bind;
mod cache_limit;
mod category_picker;
mod duplicate_dialog;
mod clamp;
mod cli;
// the thumbnail renderer is called by the cover loader, which does not exist yet
mod feed;
// No WebView host shows the window yet (T-12a-017 needs WebKitGTK).
mod data;
mod downloads;
mod entry;
mod entry_core;
mod library;
mod library_update;
mod misc;
// no screen calls the client yet; the extension host and the trackers take it from here
mod net_client;
mod local_flow;
mod live;
mod gio_notifier;
mod incognito;
mod open_link;
mod player_view;
mod reader;
mod recipe_sources;
mod reporter;
mod screens;
mod selection;
mod settings;
mod settings_ui;
mod source_bridge;
mod source_gate;
mod source_switch;
mod toast;
mod tri_check;
mod spec;
mod startup;
mod theme;
mod widgets;
mod ui;
mod watch;
mod window_size;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod i18n_guard;
#[cfg(test)]
mod header_guard;
mod updates;
mod pref_model;
mod browse;
mod shell;
mod util;
#[cfg(test)]
mod dropped;

use gtk::prelude::*;
use kuro_platform::paths::Paths as _;
use gtk::{glib, Box as GBox, Orientation as O};

fn main() -> glib::ExitCode {
    // before GTK is initialised, so this works without a display
    if std::env::args().skip(1).any(|a| a == "--version") {
        println!("{}", kuro_core::app_info::version_line());
        return glib::ExitCode::SUCCESS;
    }
    // the name desktop notifications show: without it they fall back to the binary, "AppRun.wrapped" inside the AppImage
    glib::set_application_name(channel::APP_NAME);
    // `--updates-json`: the desktop applets read the updates feed here, before any GTK call (works without a display)
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if cli::wants_updates_json(&args) {
            return glib::ExitCode::from(cli::run(&args));
        }
    }
    // `--run-backup`: one automatic backup from the systemd timer, no window and no GTK
    if std::env::args().skip(1).any(|a| a == "--run-backup") {
        return glib::ExitCode::from(backup_run::run_backup_cli());
    }
    // `--library-update=auto|manual`: the timer's headless pass, or a request handed to the running app; no window
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match misc::headless::parse_mode(&args) {
            Ok(Some(mode)) => return glib::ExitCode::from(misc::headless::run(mode)),
            Ok(None) => {}
            Err(e) => {
                eprintln!("kurobayomi: {e}");
                return glib::ExitCode::from(kuro_platform::deep_link::USAGE_EXIT_CODE as u8);
            }
        }
    }
    // the detached crash reporter started by the panic hook: a window over the report file, none of the app state below
    let mut args = std::env::args().skip(1);
    if args.any(|a| a == "--crash-report") {
        return reporter::run(args.next().unwrap_or_default().into());
    }
    // the command line is checked before GTK starts, so `--help` and a bad option work without a display; a second
    // launch hands the same arguments to the running instance through GApplication
    let args: Vec<String> = std::env::args().skip(1).collect();
    if shell::argv::wants_help(&args) {
        print!("{}", shell::argv::USAGE);
        return glib::ExitCode::SUCCESS;
    }
    if let Err(e) = shell::argv::parse_launch(&args) {
        eprintln!("kurobayomi: {e}");
        return glib::ExitCode::from(kuro_platform::deep_link::USAGE_EXIT_CODE as u8);
    }
    let paths = kuro_platform::system_paths::SystemPaths::system().expect("resolve app directories");
    // the hook goes in first so a panic during the rest of startup is reported too
    kuro_core::crash::install(paths.crash_dir());
    let _log = match kuro_core::logging::init(cfg!(debug_assertions), &paths.log_dir()) {
        Ok(log) => Some(log),
        Err(e) => {
            eprintln!("kurobayomi: file logging is off: {e}");
            None
        }
    };
    if let Err(e) = kuro_core::crash::prune(&paths.crash_dir(), kuro_core::crash::now_ms()) {
        eprintln!("kurobayomi: could not prune crash reports: {e}");
    }
    let rt = kuro_core::runtime::build().expect("tokio runtime");
    // the one runtime, reachable from the shell for the whole run
    let _shell = shell::Shell::install(rt.handle().clone(), &paths);

    // a second launch is forwarded to the running instance by GApplication; dev variables run a window of their own
    let unique = kuro_platform::single_instance::is_unique(|v| std::env::var(v).is_ok());
    let app = gtk::Application::builder().application_id(channel::APP_ID).flags(shell::launch::app_flags(unique)).build();
    shell::launch::connect(&app);
    // the preferences get up to 2 s to reach the disk when the application shuts down (T-02a-038)
    app.connect_shutdown(|_| {
        if data::is_installed() {
            data::flush_on_shutdown(data::app().base.store());
        }
        // a downloaded Windows setup installs the update now
        appimage_update::on_quit();
    });
    app.connect_activate(|app| {
        // a later launch only raises the window; the first one builds it
        if let Some(win) = app.windows().first() {
            win.present();
            return;
        }
        build_ui(app);
    });
    // live queries of the entry screens are spawned on the runtime, so the GTK thread enters it while the app runs
    let code = {
        let _entered = rt.enter();
        app.run()
    };
    net_client::flush_jars();
    // detached history flushes get up to 2 s to finish before the runtime goes away
    let unfinished = kuro_domain::history::repo::join_pending_flushes(std::time::Duration::from_secs(2));
    if unfinished > 0 {
        eprintln!("kurobayomi: {unfinished} history flush(es) were still running at exit");
    }
    // WAL checkpoint and PRAGMA optimize on the writer (T-03a-028)
    if data::is_installed() {
        if let Err(e) = data::app().db.close() {
            eprintln!("kurobayomi: closing the database failed: {e}");
        }
    }
    kuro_core::runtime::shutdown(rt);
    code
}

thread_local! {
    /// True while the databases open, so a second activation does not start a second open.
    static OPENING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Opens the data off the GTK thread (a splash shows if that takes over 300 ms), then builds and presents the window.
fn build_ui(app: &gtk::Application) {
    if data::is_installed() {
        build_window(app);
        return;
    }
    if OPENING.with(|o| o.replace(true)) {
        return;
    }
    // the window does not exist yet, so the application is held until the data is open
    let hold = app.hold();
    let (shown_app, app) = (app.clone(), app.clone());
    startup::run_with_splash(
        data::open_blocking,
        startup::SPLASH_DELAY,
        move || startup::splash_window(&shown_app),
        move |opened, splash| {
            match opened.and_then(data::install) {
                Ok(()) => build_window(&app),
                Err(e) => {
                    eprintln!("kurobayomi: cannot open the library databases: {e}");
                    app.quit();
                }
            }
            // the splash goes after the main window is presented, so focus passes straight to the main window
            if let Some(splash) = splash {
                splash.destroy();
            }
            OPENING.with(|o| o.set(false));
            drop(hold);
        },
    );
}

fn build_window(app: &gtk::Application) {
    // the theme is applied before the first widget is built (T-36a-061)
    shell::Shell::build(theme::startup).build_widget().expect("the theme was applied by Shell::build");
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title(channel::APP_NAME)
        .default_width(1180)
        .default_height(780)
        .build();
    // dev hook: KUROBAYOMI_SIZE=WxH opens at that size, set before any page is built so they lay out for it
    if let Some((w, h)) = std::env::var("KUROBAYOMI_SIZE").ok().and_then(|v| v.split_once('x').map(|(w, h)| (w.parse().unwrap_or(1180), h.parse().unwrap_or(780)))) {
        win.set_default_size(w, h);
    }
    // each page brings its own header bar, so the window titlebar stays empty. It is hidden, not just empty: GTK gives a
    // visible titlebar widget the theme's titlebar height (the built-in `.titlebar:not(headerbar)` min-height), which
    // shows as an empty band above the app's header. A hidden one takes no space and GTK does not add a default one.
    let titlebar = GBox::new(O::Horizontal, 0);
    titlebar.set_visible(false);
    win.set_titlebar(Some(&titlebar));

    // [sidebar | content nav stack], plus a bottom tab bar that replaces the sidebar when narrow
    let nav = gtk::Stack::new();
    nav.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    nav.set_vexpand(true);
    nav.set_hexpand(true);
    nav.set_hhomogeneous(false);
    ui::nav_init(&nav, &win);
    // the download managers start before the tab pages, so the Downloads page shows the live queue
    let host = shell::Shell::get().expect("the shell is installed in main");
    host.migration_done();
    host.start_downloads(std::sync::Arc::new(gio_notifier::GioNotifier::new(app)));
    // `app.incognito` exists before the panel's app menu and title icon look it up
    let base = &data::app().base;
    incognito::attach(app, base.incognito_mode.clone());
    let tabs = tabs();
    nav.add_child(&tabs);
    let sidebar = shell::sidebar::build(&tabs);
    let prefs = settings_ui::route::prefs();
    settings_ui::appearance::open_start_tab(&tabs, &prefs);
    // one selection for every layout: the sidebar and the bottom bar show the same tab (T-36a-004)
    let home = host.home_tabs(prefs.ui.start_screen.get());
    shell::tabs::bind(&tabs, &home);
    let sep = gtk::Separator::new(O::Vertical);
    let row = GBox::new(O::Horizontal, 0);
    row.append(&sidebar);
    row.append(&sep);
    row.append(&nav);
    let bottom = shell::sidebar::bottom_bar(&tabs);
    let outer = GBox::new(O::Vertical, 0);
    row.set_vexpand(true);
    outer.append(&row);
    outer.append(&bottom);
    // the toast layer sits over the whole window (T-12a-014)
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&outer));
    net_client::set_toaster(toast::attach(&overlay));
    win.set_child(Some(&overlay));
    // the app-state banners above everything: indexing and downloaded only (T-36a-018)
    let banners = std::rc::Rc::new(shell::banners::Banners::new());
    outer.prepend(&banners.widget);
    {
        let (index, downloaded_only) = (host.download_service().map(|d| d.index().clone()), base.downloaded_only.clone());
        shell::banners::watch(banners, move || shell::banners::BannerState {
            indexing: index.as_ref().is_some_and(|index| index.checking_indicator_visible()),
            downloaded_only: downloaded_only.get(),
        });
    }
    // the Local folders, which the local source and every local path read (from any thread)
    local_flow::load_roots();
    // the sources the screens read (repository sources), then the source gate they wait on
    startup::start_sources(base.store());
    // the library update runner behind the refresh button and the automatic schedule
    misc::start();
    // the restore banner, the Create and Restore hosts, and the close dialog of a running restore
    outer.prepend(&backup_host::attach(app, &win));
    // closing while a download runs asks first (the downloads pause)
    shell::Shell::guard_close(&win);
    // the automatic backup schedule follows the interval setting (T-02a-024)
    backup_schedule::start(app);
    open_link::register(app, &win);

    // reader/player are immersive: no sidebar or tab bar while they're on screen
    let refresh = {
        let (win, nav, tabs) = (win.clone(), nav.clone(), tabs.clone());
        move || {
            let immersive = nav.visible_child().is_some_and(|c| matches!(c.widget_name().as_str(), "screen:reader" | "screen:player"));
            let (width, _) = window_size::size_of(&win);
            let chrome = shell::layout::chrome_for(width, win.is_maximized());
            let shown = shell::layout::visibility(chrome, immersive);
            sidebar.set_visible(shown.sidebar);
            // a narrow window packs its header buttons tighter (`style.css`, `.kb-narrow`)
            if shown.bottom_bar { win.add_css_class("kb-narrow") } else { win.remove_css_class("kb-narrow") }
            // the top page's search and ⋮ menu sit in the sidebar's header while it shows: the open tab, or the page pushed
            // over the tabs
            let on_tabs = nav.visible_child().as_ref() == Some(tabs.upcast_ref());
            let top = if on_tabs { tabs.visible_child() } else { nav.visible_child() };
            shell::lead::place(top.as_ref(), shown.sidebar);
            sep.set_visible(shown.sidebar);
            bottom.set_visible(shown.bottom_bar);
        }
    };
    let r = refresh.clone();
    nav.connect_visible_child_notify(move |_| r());
    let r = refresh.clone();
    tabs.connect_visible_child_name_notify(move |_| r());
    // a search belongs to its tab: leaving the tab closes it
    let left = std::rc::Rc::new(std::cell::RefCell::new(tabs.visible_child()));
    tabs.connect_visible_child_notify(move |tabs| {
        if let Some(page) = left.replace(tabs.visible_child()) {
            ui::close_searches(&page);
        }
    });
    let r = refresh.clone();
    win.connect_default_width_notify(move |_| r());
    let r = refresh.clone();
    win.connect_default_height_notify(move |_| r());
    let r = refresh.clone();
    win.connect_maximized_notify(move |_| r());
    window_size::watch(&win);
    theme::attach_window(&win);
    local_flow::ui::attach(&win);
    refresh();
    win.present();
    schedule_crash_test();
    startup::publish_launcher_badge(&app.application_id().unwrap_or_default());
    // background work that waits for the window: the chapter cache clear and the opt-in release check
    if let Ok(paths) = kuro_platform::system_paths::SystemPaths::system() {
        startup::clear_chapter_cache_on_launch(crate::misc::diet_defaults::LIBRARY_AUTO_CLEAR_CHAPTER_CACHE, kuro_images::platform::chapter_cache_dir(&paths));
        cache_limit::start(&kuro_prefs::backup::StoragePrefs::new(data::app().base.store().clone()), cache_limit::dirs(&paths));
    }
    app_update::startup();
    appimage_update::startup();
    appimage_integrate::startup();
    whats_new::startup();
    // requests that arrived before the window: --action, --search, --open and opened files
    shell::launch::window_built(app, &win);
    // the desktop search provider serves library titles while the app runs (T-53a-066)
    shell::library_catalogue::serve(app, data::app().db.clone());
    // no keyboard focus ring on the first header button at startup
    GtkWindowExt::set_focus(&win, None::<&gtk::Widget>);

    // Escape / Alt+Left goes back, like Aniyomi's back gesture. On the root page, back returns to the start tab.
    let go_back: std::rc::Rc<dyn Fn()> = {
        let (tabs, home) = (tabs.clone(), home.clone());
        std::rc::Rc::new(move || {
            let (_, shown) = ui::page_stack();
            if shown == 0 && shell::tabs::go_back(&tabs, &home) {
                return;
            }
            pref_model::shell::go_back();
        })
    };
    let keys = gtk::EventControllerKey::new();
    let back_key = go_back.clone();
    keys.connect_key_pressed(move |_, key, _, state| {
        let back = key == gtk::gdk::Key::Escape || (key == gtk::gdk::Key::Left && state.contains(gtk::gdk::ModifierType::ALT_MASK));
        if back {
            back_key();
        }
        if back { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    win.add_controller(keys);
    // the mouse back button (button 8) is the same action
    let mouse_back = gtk::GestureClick::builder().button(8).build();
    mouse_back.connect_pressed(move |_, _, _, _| go_back());
    win.add_controller(mouse_back);

    shell::layout::apply_minimum_size(&win);
    // dev hooks: KUROBAYOMI_OPEN=<screen id> starts on that screen; KUROBAYOMI_SMOKE=1 opens every
    // screen, sheet and dialog once and quits (a panic or GTK critical shows up in the output)
    if let Ok(id) = std::env::var("KUROBAYOMI_OPEN") {
        ui::open(&id);
        if let Some(sh) = std::env::var("KUROBAYOMI_SHEET").ok().and_then(|n| ui::screen(&id).sheets.into_iter().find(|s| s.id == n)) {
            ui::show_sheet(&sh);
        }
    }
    // KUROBAYOMI_SHOT=file.png renders the window to a PNG and quits (no focus-dependent screenshots)
    if let Ok(path) = std::env::var("KUROBAYOMI_SHOT") {
        let (w, app) = (win.clone(), app.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            let snap = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&w)).snapshot(&snap, w.width() as f64, w.height() as f64);
            if let (Some(node), Some(r)) = (snap.to_node(), w.renderer()) {
                r.render_texture(&node, None).save_to_png(&path).expect("save png");
            }
            app.quit();
        });
    }
    // KUROBAYOMI_SHOT_DIR=dir: creating the file dir/take saves every open window as dir/shot-N.png (dialogs as
    // shot-N-K.png) and deletes the file again (screenshots for the docs, taken while the app is driven by hand)
    if let Ok(dir) = std::env::var("KUROBAYOMI_SHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let mut count = 0u32;
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            if std::fs::remove_file(dir.join("take")).is_ok() {
                count += 1;
                let windows = gtk::Window::list_toplevels().into_iter().filter_map(|w| w.downcast::<gtk::Window>().ok()).filter(|w| w.is_visible());
                for (k, w) in windows.enumerate() {
                    let name = if k == 0 { format!("shot-{count}.png") } else { format!("shot-{count}-{k}.png") };
                    let snap = gtk::Snapshot::new();
                    gtk::WidgetPaintable::new(Some(&w)).snapshot(&snap, w.width() as f64, w.height() as f64);
                    if let (Some(node), Some(r)) = (snap.to_node(), w.renderer()) {
                        if let Err(e) = r.render_texture(&node, None).save_to_png(dir.join(name)) {
                            tracing::warn!(error = %e, "screenshot not saved");
                        }
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }
    if std::env::var("KUROBAYOMI_SMOKE").is_ok() {
        let mut ids: Vec<String> = ui::with_spec(|s| s.screens.keys().cloned().collect());
        ids.sort();
        let (mut sheets, mut dialogs) = (0, 0);
        for id in &ids {
            ui::open(id);
            let s = ui::screen(id);
            for sh in &s.sheets {
                ui::show_sheet(sh);
                sheets += 1;
            }
            for d in &s.dialogs {
                ui::show_dialog(d);
                dialogs += 1;
            }
            for w in gtk::Window::list_toplevels() {
                if let Ok(w) = w.downcast::<gtk::Window>() {
                    if w != *win.upcast_ref::<gtk::Window>() {
                        w.destroy();
                    }
                }
            }
        }
        println!("smoke: {} screens, {sheets} sheets, {dialogs} dialogs opened", ids.len());
        app.quit();
    }
}

/// The crash-reporter test hooks (T-01a-048): `KUROBAYOMI_CRASH_TEST=1` panics on the main thread 500 ms after the
/// window is presented, `=worker` panics inside a tokio task. Release builds ignore both unless
/// `KUROBAYOMI_ALLOW_TEST_HOOKS=1`.
fn schedule_crash_test() {
    use kuro_core::crash::{self, CrashTest};
    let env = |name: &str| std::env::var(name).ok();
    match crash::crash_test_hook(env(crash::CRASH_TEST_ENV).as_deref(), env(crash::ALLOW_TEST_HOOKS_ENV).as_deref(), crash::RELEASE_BUILD) {
        Some(CrashTest::MainWindow) => {
            glib::timeout_add_local_once(std::time::Duration::from_millis(crash::CRASH_TEST_DELAY_MS), || crash::crash_test_now());
        }
        // the GTK thread is inside the runtime while the app runs, see `main`
        Some(CrashTest::Worker) => {
            tokio::spawn(crash::crash_test_worker());
        }
        None => {}
    }
}

/// One page per Aniyomi nav destination, each with its own header (title + that tab's actions).
fn tabs() -> gtk::Stack {
    let tabs = gtk::Stack::new();
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_hhomogeneous(false);
    for n in ui::with_spec(|s| s.nav.clone()) {
        let Some(b) = screens::body(&n.id) else { continue };
        // a page that starts with its own one-row header (Library, Updates, History, Browse) is used as it is; the
        // sidebar already names the tab, so it carries no title
        let own_header = b.content.first_child().is_some_and(|w| w.is::<gtk::HeaderBar>());
        let page: gtk::Widget = if own_header { b.content } else { ui::page(&spec::shown(&ui::screen(&n.id).title), &b.content, b.end, false).upcast() };
        tabs.add_titled(&page, Some(&n.id), &spec::shown(&n.title)).set_icon_name(&n.icon);
    }
    tabs
}
