//! The player's control bar and its behaviour. Hiding the bars is `auto_hide`'s alone (one timer for both bars and the
//! pointer). A click on the video toggles pause and a double click toggles fullscreen. Previous and next follow the
//! playing episode: enabled only with a neighbour. The Quality button shows for an online episode, whose servers load
//! once its file is loaded, and names the playing height; the Speed button names the playing speed; the Subtitles button shows
//! when the file has subtitles. Main thread only; the shared logic is in the parent module (`toggle_pause`, `toggle_fullscreen`, `neighbour`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use super::neighbours::{Step, neighbour};
use super::sheets;
use super::{Shared, toggle_fullscreen, toggle_pause};

/// How often the buttons are checked.
const TICK: Duration = Duration::from_millis(250);

/// A second press within this time is a double click (GTK's default).
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The widgets the controls wire up and keep up to date.
pub struct Parts<'a> {
    pub page: &'a gtk::Box,
    pub area: &'a gtk::GLArea,
    pub previous: &'a gtk::Button,
    pub next: &'a gtk::Button,
    pub subtitles: &'a gtk::MenuButton,
    pub speed: &'a gtk::MenuButton,
    pub quality: &'a gtk::MenuButton,
    pub servers: &'a sheets::quality::Servers,
}

/// The widgets the timer keeps up to date. Clones of the page's own widgets, so no closure holds the page.
struct Live {
    previous: gtk::Button,
    next: gtk::Button,
    subtitles: gtk::MenuButton,
    speed: gtk::MenuButton,
    quality: gtk::MenuButton,
    servers: sheets::quality::Servers,
    /// The episode row the buttons last followed. `i64::MIN` before the first check.
    episode: Cell<i64>,
    /// The episode row whose servers were asked to load ahead.
    preloaded: Cell<i64>,
}

impl Live {
    /// Follows the playing episode: previous and next are enabled only with a neighbour, and the Quality button shows
    /// only for an online episode.
    fn episode_changed(&self, shared: &Shared) {
        let episode = shared.borrow().episode_id;
        self.episode.set(episode);
        let (has_previous, has_next) = if crate::data::is_installed() {
            let has = |step| neighbour(episode, step).ok().flatten().is_some();
            (has(Step::Previous), has(Step::Next))
        } else {
            (false, false)
        };
        self.previous.set_sensitive(has_previous);
        self.next.set_sensitive(has_next);
        let online = sheets::quality::is_online(episode);
        self.quality.set_visible(online);
    }

    /// Follows the playing file: its speed, its height and whether it has subtitles. Once mpv has loaded the episode's
    /// file, its servers load for the Quality menu: after the start, so they never slow it (the file of the episode
    /// before can still report a duration for a moment, so the duration does not tell).
    fn follow_file(&self, shared: &Shared) {
        let episode = self.episode.get();
        if self.quality.is_visible() && self.preloaded.get() != episode && shared.borrow().loaded_episode == Some(episode) {
            self.preloaded.set(episode);
            sheets::quality::preload(shared, &self.servers);
        }
        sheets::speed::refresh(&self.speed, shared);
        sheets::quality::refresh(&self.quality);
        let subtitles = super::menu::has_subtitles(shared);
        if self.subtitles.is_visible() != subtitles {
            self.subtitles.set_visible(subtitles);
        }
    }
}

/// Wires the video's clicks and the buttons' state. Call once the page is built.
pub fn wire(parts: Parts, shared: &Shared) {
    {
        // one press waits out the double click time before it pauses, so a double click only goes fullscreen
        let click = gtk::GestureClick::new();
        let (shared, page) = (Rc::clone(shared), parts.page.downgrade());
        let pending = Rc::new(Cell::new(0u64));
        click.connect_pressed(move |_, presses, _, _| match presses {
            1 => {
                let mine = pending.get() + 1;
                pending.set(mine);
                let (shared, pending) = (Rc::clone(&shared), Rc::clone(&pending));
                glib::timeout_add_local_once(DOUBLE_CLICK, move || {
                    if pending.get() == mine {
                        toggle_pause(&shared);
                    }
                });
            }
            2 => {
                pending.set(pending.get() + 1);
                if let Some(page) = page.upgrade() {
                    toggle_fullscreen(&page);
                }
            }
            _ => {}
        });
        parts.area.add_controller(click);
    }

    let live = Live {
        previous: parts.previous.clone(),
        next: parts.next.clone(),
        subtitles: parts.subtitles.clone(),
        speed: parts.speed.clone(),
        quality: parts.quality.clone(),
        servers: parts.servers.clone(),
        episode: Cell::new(i64::MIN),
        preloaded: Cell::new(i64::MIN),
    };
    live.episode_changed(shared);
    live.follow_file(shared);
    let (page, shared) = (parts.page.downgrade(), Rc::clone(shared));
    glib::timeout_add_local(TICK, move || {
        // the page is gone, or the player closed
        if page.upgrade().is_none() || shared.borrow().closed {
            return glib::ControlFlow::Break;
        }
        if live.episode.get() != shared.borrow().episode_id {
            live.episode_changed(&shared);
        }
        live.follow_file(&shared);
        glib::ControlFlow::Continue
    });
}
