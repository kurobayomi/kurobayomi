//! The widgets the panel and the Reviews page share: AniList text in a label ([`rich_text`]), a comment with its replies
//! ([`comment`]), and the line under a name ([`meta_line`]).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use kuro_i18n::date::DateFormat;
use kuro_i18n::number::compact_count;
use kuro_i18n::time::{DateStyle, relative_time_span};
use kuro_i18n::{MsgKey, tr};
use kuro_track::tracker::anilist::comments::Comment;

use super::text::{self, LONG_LINES};

/// How far a reply sits in from its comment, in px.
const REPLY_INDENT: i32 = 16;

/// AniList's HTML in a wrapping label. A tagged spoiler opens where it is when clicked; a web link opens in the browser.
/// With `cut`, a long text stops at [`LONG_LINES`] lines with "Show more", which shows the rest in place (a page that
/// shows one whole review passes `false`).
pub fn rich_text(html: &str, cut: bool) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let revealed: Rc<RefCell<HashSet<usize>>> = Rc::default();
    let rendered = text::render(html, &revealed.borrow());
    let label = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .build();
    label.set_markup(&rendered.markup);
    column.append(&label);
    if cut && rendered.long {
        label.set_lines(LONG_LINES as i32);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let more = gtk::Button::builder().label(tr(MsgKey::comments_show_more)).css_classes(["flat", "caption"]).halign(gtk::Align::Start).build();
        let shown = label.downgrade();
        more.connect_clicked(move |more| {
            if let Some(label) = shown.upgrade() {
                label.set_lines(-1);
                label.set_ellipsize(gtk::pango::EllipsizeMode::None);
            }
            more.set_visible(false);
        });
        column.append(&more);
    }
    let html = html.to_owned();
    label.connect_activate_link(move |label, uri| {
        match text::spoiler_index(uri) {
            Some(index) => {
                revealed.borrow_mut().insert(index);
                label.set_markup(&text::render(&html, &revealed.borrow()).markup);
            }
            None => crate::entry_core::overflow_menu::open_in_browser(uri),
        }
        glib::Propagation::Stop
    });
    column
}

/// How long ago `unix_seconds` was, as the app writes times: relative within a week, else the date.
fn age(unix_seconds: i64) -> String {
    let style = DateStyle { relative: true, format: DateFormat::LocaleShort, locale: kuro_i18n::current_locale().tag };
    relative_time_span(unix_seconds.saturating_mul(1000), kuro_data::time::now_millis(), style)
}

/// A name in bold over nothing else, then its details dimmed: `SamuraiGinpachi  2 years ago · ♥ 83`.
pub fn meta_line(name: &str, details: &str) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::builder().label(name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build();
    let details = gtk::Label::builder().label(details).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label"]).build();
    line.append(&name);
    line.append(&details);
    line
}

/// The details of a comment: its age and its likes.
fn comment_details(comment: &Comment) -> String {
    format!("{} \u{b7} \u{2665} {}", age(comment.created_at), compact_count(comment.likes))
}

/// A comment, its text, and its replies under it, each set in a step.
pub fn comment(comment: &Comment) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.append(&meta_line(&comment.user, &comment_details(comment)));
    column.append(&rich_text(&comment.html, true));
    for reply in &comment.replies {
        let reply = self::comment(reply);
        reply.set_margin_start(REPLY_INDENT);
        reply.set_margin_top(6);
        column.append(&reply);
    }
    column
}
