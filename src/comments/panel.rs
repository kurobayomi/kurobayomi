//! The comments of the episode playing, in a panel over the right side of the video (docs/design/comments.md): the
//! same kind and place as the episode list (`reader::chapter_panel`), opened by the header's Comments button and C, shut
//! by its close button and Esc. It follows the episode: a new one reloads it. The video keeps playing.
//!
//! What it shows comes from AniList's discussion thread of the episode: at most 50 comments (the oldest, the week the
//! episode aired), Top (most liked first) or New, each with its replies, and "Open on AniList". Spoilers show at once
//! (user, 2026-11-21); only text the author tagged as a spoiler waits for a click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use kuro_data::item::ItemRepo;
use kuro_i18n::number::compact_count;
use kuro_i18n::{Arg, MsgKey, tr, tr_plural};
use kuro_platform::kind::Kind;
use kuro_track::tracker::anilist::comments::{self as api, Comment, LIMIT, Thread};
use kuro_track::tracker::anilist::error::AniListError;

use super::{Ids, view};
use crate::reader::chapter_panel::WIDTH;

/// How often an open panel checks whether the episode changed.
const FOLLOW_EVERY: Duration = Duration::from_millis(500);
/// The comments icon, the header button's and the empty state's.
pub const ICON: &str = "chat-message-new-symbolic";

/// The order the comments are shown in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// Most liked first.
    #[default]
    Top,
    /// Newest first.
    New,
}

/// `comments` in `sort` order. Equal ones keep AniList's order (oldest first).
pub fn sorted(comments: &[Comment], sort: Sort) -> Vec<&Comment> {
    let mut out: Vec<&Comment> = comments.iter().collect();
    match sort {
        Sort::Top => out.sort_by_key(|c| std::cmp::Reverse(c.likes)),
        Sort::New => out.sort_by_key(|c| std::cmp::Reverse(c.created_at)),
    }
    out
}

/// What one load found.
struct Loaded {
    /// The title's AniList id; `None` when AniList does not know it.
    media: Option<i64>,
    thread: Option<Thread>,
    comments: Vec<Comment>,
}

/// What the panel shows now.
#[derive(Default)]
struct State {
    episode_id: i64,
    episode_name: String,
    thread: Option<Thread>,
    comments: Vec<Comment>,
    sort: Sort,
    /// Where "Open on AniList" goes.
    link: Option<String>,
}

pub struct CommentsPanel {
    revealer: gtk::Revealer,
    subtitle: gtk::Label,
    stack: gtk::Stack,
    status: gtk::Box,
    list: gtk::Box,
    scroller: gtk::ScrolledWindow,
    open_link: gtk::Button,
    state: RefCell<State>,
    /// Only the latest load may show anything.
    generation: Cell<u64>,
    /// The episode playing now, asked while the panel is open.
    current: RefCell<Option<Rc<dyn Fn() -> Option<i64>>>>,
    /// The check that follows the episode runs (one at a time, even when the panel is shut and opened quickly).
    following: Cell<bool>,
    on_closed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl CommentsPanel {
    pub fn new() -> Rc<Self> {
        let title = gtk::Label::builder().label(tr(MsgKey::comments_title)).xalign(0.0).hexpand(true).css_classes(["heading"]).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat"]).tooltip_text(tr(MsgKey::action_close)).build();
        let top = gtk::ToggleButton::builder().label(tr(MsgKey::comments_sort_top)).active(true).build();
        let new = gtk::ToggleButton::builder().label(tr(MsgKey::ui_new)).group(&top).build();
        let sorts = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sorts.add_css_class("linked");
        sorts.append(&top);
        sorts.append(&new);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&title);
        head.append(&sorts);
        head.append(&close);
        let subtitle = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label"]).build();

        let list = gtk::Box::new(gtk::Orientation::Vertical, 14);
        list.set_margin_start(12);
        list.set_margin_end(12);
        list.set_margin_top(4);
        list.set_margin_bottom(12);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
        let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
        status.set_vexpand(true);
        let loading = crate::widgets::status::spinner();
        loading.set_halign(gtk::Align::Center);
        loading.set_valign(gtk::Align::Center);
        loading.set_vexpand(true);
        let stack = gtk::Stack::new();
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&status, Some("status"));
        stack.add_named(&scroller, Some("list"));
        stack.set_vexpand(true);
        let open_link = gtk::Button::builder().label(tr(MsgKey::ui_open_on_anilist)).css_classes(["flat"]).build();

        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.add_css_class("background");
        column.add_css_class("reader-chapter-panel");
        column.set_width_request(WIDTH);
        let top_part = gtk::Box::new(gtk::Orientation::Vertical, 2);
        top_part.set_margin_top(8);
        top_part.set_margin_start(12);
        top_part.set_margin_end(8);
        top_part.append(&head);
        top_part.append(&subtitle);
        column.append(&top_part);
        column.append(&stack);
        column.append(&open_link);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .transition_duration(200)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Fill)
            .child(&column)
            .build();
        // no room taken while shut, so a click on the video behind it still reaches the video
        revealer.set_visible(false);

        let panel = Rc::new(Self {
            revealer,
            subtitle,
            stack,
            status,
            list,
            scroller,
            open_link: open_link.clone(),
            state: RefCell::default(),
            generation: Cell::new(0),
            current: RefCell::new(None),
            following: Cell::new(false),
            on_closed: RefCell::new(None),
        });
        let weak = Rc::downgrade(&panel);
        close.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.close();
            }
        });
        for (button, sort) in [(top, Sort::Top), (new, Sort::New)] {
            let weak = Rc::downgrade(&panel);
            button.connect_toggled(move |button| {
                let Some(panel) = weak.upgrade().filter(|_| button.is_active()) else { return };
                panel.state.borrow_mut().sort = sort;
                panel.show_list();
            });
        }
        let weak = Rc::downgrade(&panel);
        open_link.connect_clicked(move |_| {
            let link = weak.upgrade().and_then(|panel| panel.state.borrow().link.clone());
            if let Some(link) = link {
                crate::entry_core::overflow_menu::open_in_browser(&link);
            }
        });
        panel
    }

    /// The widget to put over the video, aligned to its right edge.
    pub fn root(&self) -> &gtk::Revealer {
        &self.revealer
    }

    pub fn is_open(&self) -> bool {
        self.revealer.reveals_child()
    }

    /// Called after the panel closed, so the player takes the keys back.
    pub fn connect_closed(&self, f: impl Fn() + 'static) {
        *self.on_closed.borrow_mut() = Some(Rc::new(f));
    }

    /// Opens the panel on the episode `current` names, and follows it while open. `current` answers `None` while it
    /// cannot tell; the panel then keeps what it shows.
    pub fn open(self: &Rc<Self>, current: Rc<dyn Fn() -> Option<i64>>) {
        let Some(episode_id) = current() else { return };
        *self.current.borrow_mut() = Some(current);
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        if episode_id != self.state.borrow().episode_id || self.state.borrow().link.is_none() {
            self.load(episode_id);
        }
        if self.following.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(FOLLOW_EVERY, move || {
            let Some(panel) = weak.upgrade() else { return glib::ControlFlow::Break };
            if !panel.is_open() {
                panel.following.set(false);
                return glib::ControlFlow::Break;
            }
            let now = panel.current.borrow().as_ref().and_then(|current| current());
            if let Some(now) = now.filter(|now| *now != panel.state.borrow().episode_id) {
                panel.load(now);
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn close(&self) {
        if !self.is_open() {
            return;
        }
        self.revealer.set_reveal_child(false);
        self.revealer.set_visible(false);
        let closed = self.on_closed.borrow().clone();
        if let Some(closed) = closed {
            closed();
        }
    }

    /// Looks up the comments of `episode_id`: what is remembered first, then AniList.
    fn load(self: &Rc<Self>, episode_id: i64) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        {
            let mut state = self.state.borrow_mut();
            *state = State { episode_id, sort: state.sort, ..State::default() };
        }
        self.subtitle.set_text("");
        let item = crate::data::is_installed().then(|| ItemRepo::new(crate::data::app().db.clone()).by_id(episode_id).ok().flatten()).flatten();
        let Some(item) = item else { return self.show_status(&tr(MsgKey::comments_no_thread), false) };
        self.subtitle.set_text(&item.name);
        self.state.borrow_mut().episode_name = item.name.clone();
        let Some(episode) = api::whole_episode(item.number) else { return self.show_status(&tr(MsgKey::comments_no_thread), false) };
        if super::blocked() {
            return self.show_status(&tr(MsgKey::comments_rate_limited), true);
        }
        let ids: Ids = super::ids_of(item.entry_id);
        let known_media = super::remembered_media(item.entry_id);
        let known_thread = known_media.flatten().and_then(|media| super::remembered_thread(media, episode));
        let Some(runtime) = super::runtime() else { return self.show_status(&tr(MsgKey::comments_failed), true) };
        self.stack.set_visible_child_name("loading");
        let entry_id = item.entry_id;
        let weak = Rc::downgrade(self);
        crate::background::run_async_on(
            &runtime,
            async move {
                let transport = super::transport()?;
                let media = match known_media {
                    Some(media) => media,
                    None => super::resolve_media(&transport, ids, Kind::Anime).await?,
                };
                let Some(id) = media else { return Ok(Loaded { media: None, thread: None, comments: Vec::new() }) };
                let thread = match known_thread {
                    Some(thread) => thread,
                    None => api::find_thread(&transport, id, episode).await?,
                };
                let comments = match &thread {
                    Some(thread) => api::comments(&transport, thread.id).await?,
                    None => Vec::new(),
                };
                Ok::<Loaded, AniListError>(Loaded { media, thread, comments })
            },
            move |answer| {
                let Some(panel) = weak.upgrade().filter(|p| p.generation.get() == generation) else { return };
                match answer {
                    Some(Ok(loaded)) => {
                        super::remember_media(entry_id, loaded.media);
                        if let Some(media) = loaded.media {
                            super::remember_thread(media, episode, loaded.thread.clone());
                        }
                        panel.show(loaded);
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, episode_id, "comments: AniList did not answer");
                        super::note_error(&error);
                        panel.show_status(&super::error_text(Some(&error), MsgKey::comments_failed), true);
                    }
                    None => panel.show_status(&tr(MsgKey::comments_failed), true),
                }
            },
        );
    }

    fn show(self: &Rc<Self>, loaded: Loaded) {
        let Some(media) = loaded.media else {
            self.state.borrow_mut().link = None;
            return self.show_status(&tr(MsgKey::comments_not_on_anilist), false);
        };
        let Some(thread) = loaded.thread else {
            self.state.borrow_mut().link = Some(api::media_url(media, Kind::Anime));
            self.open_link.set_visible(true);
            return self.show_status(&tr(MsgKey::comments_no_thread), false);
        };
        let replies = tr_plural(MsgKey::comments_replies, thread.replies, &[("shown", Arg::Str(&compact_count(thread.replies)))]);
        let name = self.state.borrow().episode_name.clone();
        self.subtitle.set_text(&format!("{name} \u{b7} {replies}"));
        {
            let mut state = self.state.borrow_mut();
            state.link = Some(thread.url.clone());
            state.thread = Some(thread);
            state.comments = loaded.comments;
        }
        if self.state.borrow().comments.is_empty() {
            self.open_link.set_visible(true);
            return self.show_status(&tr(MsgKey::comments_empty), false);
        }
        self.show_list();
    }

    /// The comments in the chosen order, and a last row to the rest when 50 were shown.
    fn show_list(self: &Rc<Self>) {
        let state = self.state.borrow();
        if state.comments.is_empty() {
            return;
        }
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for comment in sorted(&state.comments, state.sort) {
            self.list.append(&view::comment(comment));
        }
        if state.comments.len() >= LIMIT {
            let more = gtk::Button::builder().label(tr(MsgKey::comments_more_on_anilist)).css_classes(["flat"]).build();
            let link = state.link.clone();
            more.connect_clicked(move |_| {
                if let Some(link) = &link {
                    crate::entry_core::overflow_menu::open_in_browser(link);
                }
            });
            self.list.append(&more);
        }
        self.open_link.set_visible(state.link.is_some());
        self.scroller.vadjustment().set_value(0.0);
        self.stack.set_visible_child_name("list");
    }

    /// A status in place of the list: `text`, with Retry when asking again may help.
    fn show_status(self: &Rc<Self>, text: &str, retry: bool) {
        while let Some(child) = self.status.first_child() {
            self.status.remove(&child);
        }
        let page = if retry {
            let weak = Rc::downgrade(self);
            crate::widgets::status::error(text, None, Some(move || {
                if let Some(panel) = weak.upgrade() {
                    let episode_id = panel.state.borrow().episode_id;
                    panel.load(episode_id);
                }
            }))
        } else {
            crate::widgets::status::empty(ICON, text, None)
        };
        page.add_css_class("compact");
        page.set_vexpand(true);
        self.status.append(&page);
        self.open_link.set_visible(self.state.borrow().link.is_some());
        self.stack.set_visible_child_name("status");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(id: i64, likes: i64, created_at: i64) -> Comment {
        Comment { id, user: String::new(), likes, created_at, html: String::new(), replies: Vec::new() }
    }

    #[test]
    fn top_puts_the_most_liked_first_and_new_the_newest() {
        let comments = [comment(1, 5, 100), comment(2, 83, 50), comment(3, 5, 300)];
        let ids = |sort| sorted(&comments, sort).iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(Sort::Top), [2, 1, 3], "ties keep AniList's order");
        assert_eq!(ids(Sort::New), [3, 1, 2]);
    }
}
