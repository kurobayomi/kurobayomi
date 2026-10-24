//! The player page (areas 45 and 46): a GL area that shows libmpv's render output, a control bar and keyboard
//! shortcuts. It opens inside the main window (docs/design/player-reader.md). It is opened through
//! [`open_episode`] for a file, or [`open_source_episode`] for an episode of an online source. Logic lives in `playback` (mpv), `source` (the source's video) and `store` (watch progress); this
//! file is widget wiring. Main thread only.

use kuro_i18n::{MsgKey, tr};

mod after_seen;
mod auto_hide;
mod controls;
mod hls_proxy;
mod menu;
mod neighbours;
mod playback;
mod sheets;
pub(crate) use sheets::quality::default_or_best;
mod skip_button;

/// "1080p": how the player and its settings name a video height.
pub fn quality_label(height: i32) -> String {
    sheets::quality::height_label(height)
}
mod source;
pub mod sub_cache;
mod store;
#[cfg(test)]
mod tests;

pub use source::open_source_episode;

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gtk::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, glib};
use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;
use kuro_platform::system_paths::SystemPaths;
use kuro_platform::paths::Paths as _;
use kuro_player::keys::Modifiers;
use kuro_player::mpv::MpvHandle;
use kuro_player::observer::MpvEvent;
use kuro_player::options::{PlayerPrefs, startup_settings};
use kuro_player::seekbar_model::{drags_enabled, duration_label, fraction, position_label};
use kuro_player::session::prepare_mpv_dir;
use kuro_player::shortcuts::{Action, action_for};

use neighbours::{Step, neighbour};
use playback::{Playback, VOLUME_MAX, seek_delta_ms, seek_target_ms, volume_after_steps};
pub use store::start_ms;
use store::Tracker;

use crate::widgets::context_menu::{self, Item};
use crate::widgets::page_header;

/// How often the page reads mpv, updates the controls and saves progress when one is due.
const TICK: Duration = Duration::from_millis(250);

/// What the player plays: a file or a source's stream with its request headers and side tracks.
struct Media {
    /// The path or URL `loadfile` gets.
    target: String,
    /// The anime over the episode, as the top bar shows it.
    title: page_header::MediaTitle,
    /// The `http-header-fields` value; `None` for a file.
    headers: Option<String>,
    /// The stream's request headers, which a side subtitle file is fetched with into the subtitle cache.
    track_headers: Vec<(String, String)>,
    /// `audio-add` and `sub-add` commands, sent once mpv has loaded the stream.
    tracks: Vec<Vec<String>>,
}

/// Opens `path` in the player for the anime episode `episode_id`.
///
/// `start_ms` above zero starts there. Zero or less resumes from the saved progress: an unseen episode resumes at
/// its last second, a seen one restarts at zero. Progress is saved every five seconds while playing, on pause, at
/// the end of the file and on close. An episode missing from the database plays without saving.
pub fn open_episode(path: &Path, episode_id: i64, start_ms: i64) {
    let Some(target) = path.to_str() else {
        return tracing::error!(path = %path.display(), "player: the path is not UTF-8; not opened");
    };
    let title = title_or_file(episode_id, path);
    open_media(Media { target: target.to_owned(), title, headers: None, track_headers: Vec::new(), tracks: Vec::new() }, episode_id, start_ms);
}

thread_local! {
    /// The player that was opened last, held weakly: it is open while it lives and has not been closed.
    static CURRENT: RefCell<std::rc::Weak<RefCell<Player>>> = RefCell::new(std::rc::Weak::new());
}

/// A player page is open. A second click on an episode then opens nothing more.
pub fn is_open() -> bool {
    CURRENT.with(|c| c.borrow().upgrade().is_some_and(|p| p.try_borrow().map_or(true, |p| !p.closed)))
}

fn open_media(media: Media, episode_id: i64, start_ms: i64) {
    if is_open() {
        return;
    }
    match SystemPaths::system() {
        Ok(paths) => {
            if open_in(paths.data_dir(), media, episode_id, start_ms).is_none() {
                crate::toast::notify(tr(MsgKey::player_hoster_failed));
            }
        }
        Err(e) => tracing::error!(error = %e, "player: no app directories; not opened"),
    }
}

/// Opens the player with mpv's config files under `data_dir`. Returns the state the page shares, or `None` when libmpv
/// did not start.
fn open_in(data_dir: std::path::PathBuf, media: Media, episode_id: i64, start_ms: i64) -> Option<Shared> {
    let mut prefs = PlayerPrefs {
        data_dir,
        ..PlayerPrefs::default()
    };
    // the kept preferences: the preferred subtitle and audio languages (mpv's `slang` and `alang`)
    prefs.sub.lang = stored_text("pref_subtitle_lang");
    prefs.audio.lang = stored_text("pref_audio_lang");
    prepare_mpv_dir(&prefs);
    kuro_platform::locale::use_c_numeric_locale();
    let mpv = match MpvHandle::create(&startup_settings(&prefs)) {
        Ok(mpv) => mpv,
        Err(e) => {
            tracing::error!(error = %e, "player: libmpv did not start; not opened");
            return None;
        }
    };
    let (tracker, start) = if crate::data::is_installed() {
        let app = crate::data::app();
        let tracker = Tracker::open_live(app.db.clone(), episode_id);
        let start = tracker.as_ref().map_or(start_ms.max(0), |t| t.start_ms(start_ms));
        (tracker, start)
    } else {
        (None, start_ms.max(0))
    };

    let shared = Rc::new(RefCell::new(Player {
        playback: Playback::new(mpv),
        tracker,
        finished_saved: false,
        ahead_done: false,
        closed: false,
        media,
        episode_id,
        resolving: false,
        skip: skip_button::State::default(),
        awake: crate::reader::keep_awake::KeepAwake::default(),
    }));
    CURRENT.with(|c| *c.borrow_mut() = Rc::downgrade(&shared));
    crate::ui::push(&build_page(&shared, start));
    Some(shared)
}

/// Everything the page reads and writes on mpv and the database. Shared by the GTK closures; a `RefCell` borrow
/// never spans a GTK call, so a re-entrant render cannot panic.
struct Player {
    playback: Playback,
    tracker: Option<Tracker>,
    media: Media,
    /// The episode row that plays now; its neighbours come from it.
    episode_id: i64,
    /// A source episode is resolving for the previous or next button (or autoplay); a second one waits for it.
    resolving: bool,
    /// The end-of-file save has run, so it runs once.
    finished_saved: bool,
    /// The next episode was queued for download for the playing one (once per episode).
    ahead_done: bool,
    /// Playback was stopped and saved for good (Back, or the page or its window went away).
    closed: bool,
    /// The skip button's state for the playing file.
    skip: skip_button::State,
    /// The screen stays on while the video plays (released on pause and when the player closes).
    awake: crate::reader::keep_awake::KeepAwake,
}

type Shared = Rc<RefCell<Player>>;

impl Player {
    /// Saves the position if there is one. At the end of the file the position is the duration.
    fn save(&mut self, forced: bool) {
        let Some(tracker) = self.tracker.as_mut() else {
            return;
        };
        let duration = self.playback.duration_ms().unwrap_or(0);
        let position = if self.playback.eof_reached() {
            Some(duration)
        } else {
            self.playback.position_ms()
        };
        if let Some(position) = position {
            tracker.save(now_ms(), position, duration, forced);
        }
    }

    /// Plays `media` as episode row `episode_id` in this window. The episode that was playing is saved first, then the
    /// new one starts at its saved position (a seen one at zero).
    fn play(&mut self, media: Media, episode_id: i64) {
        self.save(true);
        self.tracker = crate::data::is_installed()
            .then(|| {
                let app = crate::data::app();
                Tracker::open_live(app.db.clone(), episode_id)
            })
            .flatten();
        let start = self.tracker.as_ref().map_or(0, |t| t.start_ms(0));
        self.episode_id = episode_id;
        self.finished_saved = false;
        self.ahead_done = false;
        self.media = media;
        if let Err(e) = self.playback.load(&self.media.target, self.media.headers.as_deref(), start) {
            tracing::error!(error = %e, "player: could not load the episode");
            crate::toast::notify(tr(MsgKey::player_hoster_failed));
        }
    }

    /// Closing: saves the progress and stops playback, once.
    fn finish(&mut self) {
        if !self.closed {
            self.closed = true;
            self.save(true);
            self.playback.stop();
            self.awake.sync_for(false, &tr(MsgKey::ui_playing_video));
            after_seen::flush();
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Toggles pause. Pausing saves progress at once.
fn toggle_pause(shared: &Shared) {
    let mut player = shared.borrow_mut();
    match player.playback.toggle_pause() {
        Ok(true) => player.save(true),
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "player: pause failed"),
    }
}

/// The window that holds the page: the main window.
fn window_of(page: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    page.as_ref().root().and_downcast::<gtk::Window>()
}

fn toggle_fullscreen(page: &gtk::Box) {
    let Some(window) = window_of(page) else { return };
    if window.is_fullscreen() {
        window.unfullscreen();
    } else {
        window.fullscreen();
    }
}

/// Runs one shortcut. `false` when the page does nothing with it, so the key goes on to GTK.
fn apply_action(shared: &Shared, page: &gtk::Box, action: Action) -> bool {
    let fullscreen = window_of(page).is_some_and(|w| w.is_fullscreen());
    match action {
        Action::TogglePause => toggle_pause(shared),
        Action::Seek { forward, long } => {
            if let Err(e) = shared.borrow().playback.seek_by_ms(seek_delta_ms(forward, long)) {
                tracing::warn!(error = %e, "player: seek failed");
            }
        }
        Action::VolumeStep(steps) => {
            let player = shared.borrow();
            let volume = volume_after_steps(player.playback.volume(), steps);
            if let Err(e) = player.playback.set_volume(volume) {
                tracing::warn!(error = %e, "player: volume change failed");
            }
            remember_volume(&player.playback);
        }
        Action::ToggleFullscreen => toggle_fullscreen(page),
        Action::Escape if fullscreen => toggle_fullscreen(page),
        _ => return false,
    }
    true
}

/// The player page: a slim header (Back, title, the ⋮ menu of tracks and servers) over the video and its control bar (with the quality
/// menu). The
/// page is named `screen:player`, so the main window hides its sidebar while it shows. The caller pushes it.
fn build_page(shared: &Shared, start: i64) -> gtk::Box {
    let area = gtk::GLArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_auto_render(false);

    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    let previous = crate::ui::icon_button("media-skip-backward-symbolic", &tr(MsgKey::action_previous_episode));
    let next = crate::ui::icon_button("media-skip-forward-symbolic", &tr(MsgKey::action_next_episode));
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.001);
    seek.set_hexpand(true);
    seek.set_draw_value(false);
    let time = gtk::Label::new(None);
    time.add_css_class("numeric");
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, VOLUME_MAX, 1.0);
    volume.set_value(VOLUME_MAX);
    volume.set_width_request(120);
    volume.set_draw_value(false);
    let fullscreen = gtk::Button::from_icon_name("view-fullscreen-symbolic");
    let speed = sheets::speed::button(shared);
    let chapters = sheets::chapters::button(shared);
    let servers = sheets::quality::Servers::default();
    let quality = sheets::quality::button(shared, &servers);
    let skip = skip_button::Ui::new(shared);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("osd");
    bar.add_css_class("toolbar");
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_margin_bottom(12);
    bar.set_valign(gtk::Align::End);
    for widget in [previous.upcast_ref::<gtk::Widget>(), play.upcast_ref(), next.upcast_ref(), seek.upcast_ref(), time.upcast_ref(), quality.upcast_ref(), speed.upcast_ref(), chapters.upcast_ref(), volume.upcast_ref(), fullscreen.upcast_ref()] {
        bar.append(widget);
    }

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(skip.widget());
    // shown from the first frame until mpv knows the duration: the stream, its audio and its subtitles take a moment
    let loading = gtk::Spinner::builder().spinning(true).halign(gtk::Align::Center).valign(gtk::Align::Center).width_request(48).height_request(48).can_target(false).build();
    overlay.add_overlay(&loading);
    overlay.set_vexpand(true);

    let title = shared.borrow().media.title.clone();

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.set_widget_name("screen:player");
    page.set_focusable(true);
    // the shared page header: Back stops and saves at once, then leaves the page
    // the reader's top bar (`media_header`): the anime over the episode
    let (header, two_lines) = page_header::media_header(&title, {
        let (shared, page) = (Rc::clone(shared), page.downgrade());
        move || {
            shared.borrow_mut().finish();
            if let Some(window) = page.upgrade().and_then(|p| window_of(&p)).filter(|w| w.is_fullscreen()) {
                window.unfullscreen();
            }
            crate::ui::pop();
        }
    });
    let menu = menu::tracks_menu(shared, &servers);
    header.pack_end(&menu);
    // the buttons packed here came after page_header::header made its pass, so they need their own
    crate::ui::no_sticky_focus(&header);
    // the bars hide by themselves while the video plays (`auto_hide`); with the setting off the header sits above it
    let enabled = crate::data::is_installed() && crate::data::app().base.auto_hide_bars.get();
    let hiding = auto_hide::AutoHide::attach(enabled, &page, &overlay, &header, &bar, &area);
    page.append(&overlay);
    watch_fullscreen(&page, &header);

    wire_gl(&area, shared, start);
    watch_left(&page, shared, &area);
    {
        let (shared, page) = (Rc::clone(shared), page.downgrade());
        context_menu::attach(&area, move |_, _| page.upgrade().map_or_else(Vec::new, |page| video_menu(&shared, &page, &servers)));
    }

    {
        let shared = Rc::clone(shared);
        play.connect_clicked(move |_| toggle_pause(&shared));
    }
    {
        let shared = Rc::clone(shared);
        previous.connect_clicked(move |_| play_adjacent(&shared, Step::Previous, true));
    }
    {
        let shared = Rc::clone(shared);
        next.connect_clicked(move |_| play_adjacent(&shared, Step::Next, true));
    }
    {
        let shared = Rc::clone(shared);
        seek.connect_change_value(move |_, _, value| {
            let player = shared.borrow();
            if let Some(duration) = player.playback.duration_ms() {
                if let Err(e) = player.playback.seek_to_ms(seek_target_ms(value, duration)) {
                    tracing::warn!(error = %e, "player: seek failed");
                }
            }
            glib::Propagation::Proceed
        });
    }
    {
        let shared = Rc::clone(shared);
        volume.connect_change_value(move |_, _, value| {
            let player = shared.borrow();
            if let Err(e) = player.playback.set_volume(value) {
                tracing::warn!(error = %e, "player: volume change failed");
            }
            remember_volume(&player.playback);
            glib::Propagation::Proceed
        });
    }
    {
        let page = page.downgrade();
        fullscreen.connect_clicked(move |_| {
            if let Some(page) = page.upgrade() {
                toggle_fullscreen(&page);
            }
        });
    }

    // capture phase: the shortcuts work wherever the focus is in the page, before a focused button sees the key
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (shared, page, menu) = (Rc::clone(shared), page.downgrade(), menu.downgrade());
        let popovers = [quality.downgrade(), speed.downgrade(), chapters.downgrade()];
        keys.connect_key_pressed(move |_, keyval, _, state| {
            let Some(page) = page.upgrade() else {
                return glib::Propagation::Proceed;
            };
            // an open menu (tracks, quality, speed, chapters) keeps its own keys
            let popover_open = popovers.iter().filter_map(|b| b.upgrade()).any(|b| b.popover().is_some_and(|p| p.is_visible()));
            if popover_open || menu.upgrade().is_some_and(|m| m.is_active()) {
                return glib::Propagation::Proceed;
            }
            let Some(action) = action_for(keyval.into_glib(), modifiers(state), false) else {
                return glib::Propagation::Proceed;
            };
            if apply_action(&shared, &page, action) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    page.add_controller(keys);
    page.connect_map(|page| {
        page.grab_focus();
    });

    controls::wire(
        controls::Parts { page: &page, area: &area, bar: &bar, previous: &previous, next: &next, quality: &quality, speed: &speed, chapters: &chapters },
        shared,
    );
    start_ticks(&page, &two_lines, &hiding, &play, &seek, &time, &volume, &loading, &skip, shared);
    page
}

/// The video's right-click menu: Audio, Subtitles and Quality (the lists of the header's menus), Speed (the speed menu's
/// presets) and Fullscreen. Each list marks what plays now; the rows run the menus' own picks.
fn video_menu(shared: &Shared, page: &gtk::Box, servers: &sheets::quality::Servers) -> Vec<Item> {
    let mut items: Vec<Item> = menu::track_sections(shared)
        .into_iter()
        .map(|(heading, choices)| Item::submenu(heading, 0, menu_items(choices)))
        .filter(|item| !item.children.is_empty())
        .collect();
    if let Some(sections) = sheets::quality::menu_sections(shared, servers) {
        // the servers load on the first ask; the next right-click lists them: Quality, then Server
        if sections.is_empty() {
            items.push(Item::new(tr(MsgKey::ui_quality_loading), 0, || {}));
        }
        for (heading, choices) in sections {
            let choices: Vec<sheets::Choice> = choices.into_iter().filter(|choice| choice.enabled).collect();
            if !choices.is_empty() {
                items.push(Item::submenu(heading.unwrap_or_default(), 0, menu_items(choices)));
            }
        }
    }
    items.push(Item::submenu(tr(MsgKey::player_speed_menu), 0, menu_items(sheets::speed::choices(shared))));
    let fullscreen = window_of(page).is_some_and(|w| w.is_fullscreen());
    let page = page.downgrade();
    let toggle = move || {
        if let Some(page) = page.upgrade() {
            toggle_fullscreen(&page);
        }
    };
    items.push(Item::new(tr(MsgKey::pref_fullscreen), 1, toggle).checked(fullscreen));
    items
}

/// The rows of a player menu as right-click entries, the current one checked. A row that cannot be chosen is left out.
fn menu_items(choices: Vec<sheets::Choice>) -> Vec<Item> {
    choices.into_iter().filter(|choice| choice.enabled).map(|choice| Item::new(choice.title, 0, choice.pick).checked(choice.current)).collect()
}

/// Stops playback at once when the page leaves the main stack by a route the page does not see (Escape, Alt+Left, the
/// mouse back button): the stack removes the page only after its 400 ms transition. The handler goes with the GL area.
fn watch_left(page: &gtk::Box, shared: &Shared, area: &gtk::GLArea) {
    let stack = crate::ui::stack();
    let (page_ref, shared_ref) = (page.downgrade(), Rc::downgrade(shared));
    let id = stack.connect_visible_child_notify(move |stack| {
        let (Some(page), Some(shared)) = (page_ref.upgrade(), shared_ref.upgrade()) else { return };
        let left = page.parent().as_ref() == Some(stack.upcast_ref::<gtk::Widget>())
            && stack.visible_child().as_ref() != Some(page.upcast_ref::<gtk::Widget>());
        if left {
            shared.borrow_mut().finish();
            // leaving the page leaves fullscreen first, so the page below is not shown full screen
            if let Some(window) = window_of(&page).filter(|w| w.is_fullscreen()) {
                window.unfullscreen();
            }
        }
    });
    let id = Cell::new(Some(id));
    area.connect_unrealize(move |_| {
        if let Some(id) = id.take() {
            stack.disconnect(id);
        }
    });
}

/// Fullscreen shows only the video: the header follows the window's fullscreen state, so it hides on entering and is
/// back on leaving (Escape, the fullscreen button, the window manager). The window is known only once the page is
/// mapped, so the connection is made then, once per mapping, and dropped on unmap.
fn watch_fullscreen(page: &gtk::Box, header: &gtk::HeaderBar) {
    let link: Rc<RefCell<Option<(gtk::Window, glib::SignalHandlerId)>>> = Rc::default();
    let header = header.downgrade();
    {
        let link = Rc::clone(&link);
        page.connect_map(move |page| {
            let (Some(window), Some(header), false) = (window_of(page), header.upgrade(), link.borrow().is_some()) else {
                return;
            };
            header.set_visible(!window.is_fullscreen());
            let weak = header.downgrade();
            let id = window.connect_fullscreened_notify(move |window| {
                if let Some(header) = weak.upgrade() {
                    header.set_visible(!window.is_fullscreen());
                }
            });
            *link.borrow_mut() = Some((window, id));
        });
    }
    page.connect_unmap(move |_| {
        if let Some((window, id)) = link.borrow_mut().take() {
            window.disconnect(id);
        }
    });
}

/// `GL_FRAMEBUFFER_BINDING`.
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

/// The framebuffer the GL area draws into this frame. GTK binds its own framebuffer, so this is not 0.
/// Falls back to 0 when `glGetIntegerv` cannot be resolved.
fn current_framebuffer() -> i32 {
    kuro_platform::gl::get_integer(GL_FRAMEBUFFER_BINDING).unwrap_or(0)
}

/// Connects the GL area: the render context is made on realize, frames are drawn on render, and the context is
/// freed on unrealize while the GL context is still current. An unrealize closes the player (the page was popped, its
/// window closed, the app quit): progress is saved and playback stops.
fn wire_gl(area: &gtk::GLArea, shared: &Shared, start: i64) {
    let loaded = Rc::new(Cell::new(false));
    {
        let shared = Rc::clone(shared);
        area.connect_realize(move |area| realize(area, &shared, start, &loaded));
    }
    {
        let shared = Rc::clone(shared);
        area.connect_unrealize(move |area| {
            {
                let mut player = shared.borrow_mut();
                player.finish();
                area.make_current();
                player.playback.mpv_mut().destroy_render_context();
            }
            // however the page left (Back, Escape, Alt+Left, the mouse back button), the window that held it leaves
            // fullscreen
            if let Some(window) = window_of(area).filter(|w| w.is_fullscreen()) {
                window.unfullscreen();
            }
        });
    }
    {
        let shared = Rc::clone(shared);
        area.connect_render(move |area, _| {
            let scale = area.scale_factor();
            let player = shared.borrow();
            if let Err(e) = player.playback.mpv().render(current_framebuffer(), area.width() * scale, area.height() * scale) {
                tracing::debug!(error = %e, "player: frame not drawn");
            }
            glib::Propagation::Stop
        });
    }
}

fn realize(area: &gtk::GLArea, shared: &Shared, start: i64, loaded: &Cell<bool>) {
    area.make_current();
    if let Some(error) = area.error() {
        return tracing::error!(error = %error, "player: no GL context; the video cannot be drawn");
    }
    // mpv asks for a redraw from its own thread, so the request crosses to the main loop over a channel
    let (wake_tx, wake_rx) = async_channel::unbounded::<()>();
    let weak = area.downgrade();
    glib::spawn_future_local(async move {
        while wake_rx.recv().await.is_ok() {
            match weak.upgrade() {
                Some(area) => area.queue_render(),
                None => break,
            }
        }
    });
    let mut player = shared.borrow_mut();
    if let Err(e) = player.playback.mpv_mut().create_render_context(kuro_platform::gl::proc_address) {
        return tracing::error!(error = %e, "player: render context failed");
    }
    if let Err(e) = player.playback.mpv_mut().set_render_update_callback(move || {
        let _ = wake_tx.try_send(());
    }) {
        tracing::error!(error = %e, "player: render updates are not wired");
    }
    if !loaded.replace(true) {
        restore_volume(&player.playback);
        if let Err(e) = player.playback.load(&player.media.target, player.media.headers.as_deref(), start) {
            tracing::error!(error = %e, "player: could not load the file");
            crate::toast::notify(tr(MsgKey::player_hoster_failed));
        }
    }
}

/// The preference sections of the player, or `None` before the app data is installed.
fn player_prefs() -> Option<kuro_prefs::player::PlayerPrefs> {
    crate::data::is_installed().then(|| kuro_prefs::player::PlayerPrefs::new(crate::data::app().base.store().clone()))
}

/// The default quality of the settings (`pref_player_default_quality`): 1080 by default and before the app data exists.
fn default_quality() -> i32 {
    player_prefs().map_or(1080, |p| p.default_quality.get())
}

/// A text preference as stored, or empty before the app data is installed.
fn stored_text(key: &'static str) -> String {
    if !crate::data::is_installed() {
        return String::new();
    }
    kuro_prefs::pref::Pref::new(crate::data::app().base.store().clone(), key, String::new()).get()
}

/// Whether the next episode starts by itself when one ends (`pref_auto_play_enabled`); off before the app data exists.
fn autoplay_next() -> bool {
    player_prefs().is_some_and(|p| p.autoplay_enabled.get())
}

/// The title the top bar shows for an episode: the anime over the episode.
pub fn episode_title(anime: &str, episode: &str) -> page_header::MediaTitle {
    page_header::MediaTitle::new(anime, episode)
}

/// The title of episode row `episode_id`, or the file name of `path` when the row or its entry is missing.
fn title_or_file(episode_id: i64, path: &Path) -> page_header::MediaTitle {
    row_title(episode_id).unwrap_or_else(|| page_header::MediaTitle::new(&path.file_name().map_or_else(|| "Player".to_owned(), |n| n.to_string_lossy().into_owned()), ""))
}

/// The anime and the episode of row `episode_id`; `None` before the app data exists or when the row is missing.
fn row_title(episode_id: i64) -> Option<page_header::MediaTitle> {
    if !crate::data::is_installed() {
        return None;
    }
    let db = &crate::data::app().db;
    let episode = ItemRepo::new(Arc::clone(db)).by_id(episode_id).ok()??;
    let anime = EntryRepo::new(Arc::clone(db)).by_id(episode.entry_id).ok()?;
    Some(episode_title(&anime.title, &episode.name))
}

/// Plays the episode before or after the one playing, in this window. At the ends a toast says so, unless
/// `announce_end` is off (autoplay). Nothing starts while a source episode is still resolving.
fn play_adjacent(shared: &Shared, step: Step, announce_end: bool) {
    let (current, resolving) = {
        let player = shared.borrow();
        (player.episode_id, player.resolving)
    };
    if resolving || !crate::data::is_installed() {
        return;
    }
    match neighbour(current, step) {
        Ok(Some(next)) => play_episode(shared, next),
        Ok(None) => {
            if announce_end {
                let key = match step {
                    Step::Previous => MsgKey::no_prev_episode,
                    Step::Next => MsgKey::no_next_episode,
                };
                crate::toast::notify(tr(key));
            }
        }
        Err(e) => tracing::error!(error = %e, "player: the episode list could not be read"),
    }
}

/// The file of a downloaded episode of an online source. Such an episode plays from its file, online or not. The disk is
/// asked once (a folder lookup), on the GTK thread, as the local episodes' open is.
fn downloaded_media(episode_id: i64) -> Option<Media> {
    if !crate::data::is_installed() {
        return None;
    }
    let db = crate::data::app().db.clone();
    let episode = ItemRepo::new(Arc::clone(&db)).by_id(episode_id).ok().flatten()?;
    let (entry, source) = crate::downloads::directory::AppDirectory::new(db).entry_context(kuro_platform::kind::Kind::Anime, episode.entry_id)?;
    let service = crate::shell::host::Shell::get()?.download_service()?;
    let path = service.locate(&entry, &source, &kuro_downloads::unit::UnitRef::from(&episode))?;
    Some(Media { target: path.to_string_lossy().into_owned(), title: title_or_file(episode_id, &path), headers: None, track_headers: Vec::new(), tracks: Vec::new() })
}

/// Plays episode row `episode_id` from its download, for an episode whose source is not loaded (turned off). False when
/// it has no download or a player is already open.
pub fn open_downloaded(episode_id: i64) -> bool {
    if is_open() {
        return false;
    }
    let Some(media) = downloaded_media(episode_id) else { return false };
    open_media(media, episode_id, 0);
    true
}

/// Starts episode row `episode_id` in this window: a local file at once, a source's stream once it resolves.
fn play_episode(shared: &Shared, episode_id: i64) {
    let db = crate::data::app().db.clone();
    match crate::local_flow::open::opening_by_id(&db, episode_id) {
        Ok(Some(found)) => {
            let title = title_or_file(episode_id, Path::new(&found.path));
            let media = Media { target: found.path, title, headers: None, track_headers: Vec::new(), tracks: Vec::new() };
            shared.borrow_mut().play(media, episode_id);
        }
        Ok(None) => play_source_episode(shared, episode_id),
        Err(e) => tracing::error!(error = %e, "player: the episode lookup failed"),
    }
}

/// A source episode in this window: its stream resolves in the background, then the page plays it.
fn play_source_episode(shared: &Shared, episode_id: i64) {
    if let Some(media) = downloaded_media(episode_id) {
        shared.borrow_mut().play(media, episode_id);
        return;
    }
    let Some(found) = source::source_episode(&crate::screens::anime_sources(), episode_id) else {
        return crate::toast::notify(tr(MsgKey::episode_not_found));
    };
    shared.borrow_mut().resolving = true;
    let page = Rc::clone(shared);
    let started = source::resolve_episode(found, move |answer| {
        page.borrow_mut().resolving = false;
        match answer {
            Ok(media) => {
                let mut player = page.borrow_mut();
                if !player.closed {
                    player.play(media, episode_id);
                }
            }
            Err(text) => crate::toast::notify(text),
        }
    });
    if !started {
        shared.borrow_mut().resolving = false;
        crate::toast::notify(tr(MsgKey::player_hoster_failed));
    }
}

/// Session start (T-45b-053): the remembered volume is written to mpv when the stored value is not negative.
fn restore_volume(playback: &Playback) {
    let Some(prefs) = player_prefs() else { return };
    if let Some(volume) = kuro_player::remember::value_to_restore(prefs.player_volume_value.get())
        && let Err(e) = playback.set_volume(f64::from(volume))
    {
        tracing::warn!(error = %e, "player: the remembered volume was not applied");
    }
}

/// A volume change: stored under `player_volume_value`.
fn remember_volume(playback: &Playback) {
    let Some(prefs) = player_prefs() else { return };
    if let Some(value) = kuro_player::remember::value_to_store(playback.volume() as f32) {
        if let Err(e) = prefs.player_volume_value.set(value) {
            tracing::warn!(error = %e, "player: the volume was not stored");
        }
    }
}

/// Reads mpv on a timer. Each tick drains mpv's events, saves progress when due and refreshes the controls and the skip
/// button. A failed file is a toast, and the end of an episode plays the next one when autoplay is on. The header hides
/// only in fullscreen (`watch_fullscreen`). The timer stops once the page is gone or closed.
fn start_ticks(page: &gtk::Box, title: &page_header::TwoLineTitle, hiding: &Rc<auto_hide::AutoHide>, play: &gtk::Button, seek: &gtk::Scale, time: &gtk::Label, volume: &gtk::Scale, loading: &gtk::Spinner, skip: &skip_button::Ui, shared: &Shared) {
    let loading = loading.clone();
    let opened = std::time::Instant::now();
    let page = page.downgrade();
    // the title follows the episode (the next one plays in place)
    let (title_bar, shown_title) = (title.clone(), RefCell::new(shared.borrow().media.title.clone()));
    let hiding = Rc::clone(hiding);
    let play = play.clone();
    let seek = seek.clone();
    let time = time.clone();
    let volume = volume.clone();
    let skip = skip.clone();
    let shared = Rc::clone(shared);
    glib::timeout_add_local(TICK, move || {
        if page.upgrade().is_none() || shared.borrow().closed {
            return glib::ControlFlow::Break;
        }
        let (snapshot, failed, ended) = {
            let mut player = shared.borrow_mut();
            let mut failed = false;
            for event in player.playback.mpv_mut().drain_events() {
                match event {
                    MpvEvent::EndFile { error: Some(_), .. } => {
                        tracing::warn!("player: playback stopped with an error");
                        failed = true;
                    }
                    MpvEvent::FileLoaded => {
                        let tracks = std::mem::take(&mut player.media.tracks);
                        let prepare = sub_cache::preparer(player.episode_id, &player.media.track_headers);
                        player.playback.add_tracks(&tracks, prepare);
                        player.skip.file_loaded();
                    }
                    _ => {}
                }
            }
            if !player.ahead_done && !player.playback.is_paused() {
                let (position, duration) = (player.playback.position_ms().unwrap_or(0) / 1000, player.playback.duration_ms().unwrap_or(0) / 1000);
                if duration > 0 && position * 4 > duration {
                    player.ahead_done = true;
                    after_seen::download_ahead(player.episode_id, position, duration);
                }
            }
            let ended = player.playback.eof_reached() && !player.finished_saved;
            if ended {
                player.finished_saved = true;
                player.save(true);
            } else if !player.playback.is_paused() {
                player.save(false);
            }
            let snapshot = Snapshot {
                paused: player.playback.is_paused(),
                position_s: player.playback.position_ms().unwrap_or(0) as f32 / 1000.0,
                duration_s: player.playback.duration_ms().unwrap_or(0) as f32 / 1000.0,
                volume: player.playback.volume(),
            };
            (snapshot, failed, ended)
        };
        shared.borrow().awake.sync_for(!snapshot.paused && !ended, &tr(MsgKey::ui_playing_video));
        skip_button::tick(&shared, &skip);
        if failed {
            crate::toast::notify(tr(MsgKey::player_hoster_failed));
        }
        if ended && autoplay_next() {
            play_adjacent(&shared, Step::Next, false);
        }
        let title = shared.borrow().media.title.clone();
        if *shown_title.borrow() != title {
            title_bar.set_title(&title);
            *shown_title.borrow_mut() = title;
        }
        play.set_icon_name(if snapshot.paused { "media-playback-start-symbolic" } else { "media-playback-pause-symbolic" });
        hiding.tick(snapshot.paused);
        // loading until mpv knows the duration; an idle mpv after the first seconds means the load failed, so stop
        let idle = shared.borrow().playback.mpv().get_flag("idle-active") == Some(true);
        let waiting = snapshot.duration_s <= 0.0 && !(idle && opened.elapsed() > std::time::Duration::from_secs(3));
        // a quality or server switch shows the spinner while its stream resolves and loads
        let waiting = waiting || shared.borrow().resolving;
        loading.set_visible(waiting);
        loading.set_spinning(waiting);
        seek.set_sensitive(drags_enabled(snapshot.duration_s));
        seek.set_value(f64::from(fraction(snapshot.position_s, snapshot.duration_s)));
        time.set_label(&format!(
            "{} / {}",
            position_label(snapshot.position_s),
            duration_label(snapshot.position_s, snapshot.duration_s, false)
        ));
        if (volume.value() - snapshot.volume).abs() > 0.5 {
            volume.set_value(snapshot.volume);
        }
        glib::ControlFlow::Continue
    });
}

struct Snapshot {
    paused: bool,
    position_s: f32,
    duration_s: f32,
    volume: f64,
}

fn modifiers(state: gdk::ModifierType) -> Modifiers {
    Modifiers {
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
        alt: state.contains(gdk::ModifierType::ALT_MASK),
        meta: state.intersects(gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK),
    }
}
