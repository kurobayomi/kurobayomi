//! A title's reviews on AniList (docs/design/comments.md): a page opened from the title page's Reviews button, the 50
//! most helpful first, each with its writer, its score and how many readers found it helpful, then its summary. A review
//! opens its whole text on a page of its own. "Open on AniList" in the header goes to the title there.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_platform::kind::Kind;
use kuro_track::tracker::anilist::comments::{self as api, Review};
use kuro_track::tracker::anilist::error::AniListError;

use super::view;
use crate::ui;

/// The Reviews button's icon.
pub const ICON: &str = "starred-symbolic";

/// What the line under a writer's name says: the score and how many found it helpful.
pub fn details(review: &Review) -> String {
    let score = trf(MsgKey::reviews_score, &[("score", Arg::Int(review.score))]);
    let helpful = trf(MsgKey::reviews_helpful, &[("rating", Arg::Int(review.rating)), ("total", Arg::Int(review.rating_amount))]);
    format!("{score} \u{b7} {helpful}")
}

/// Pushes the Reviews page of `entry_id` and fills it once AniList answers.
pub fn open(entry_id: i64, kind: Kind) {
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    let loading = crate::widgets::status::spinner();
    loading.set_halign(gtk::Align::Center);
    loading.set_valign(gtk::Align::Center);
    stack.add_named(&loading, Some("loading"));
    let link: Rc<RefCell<Option<String>>> = Rc::default();
    let open_link = ui::icon_button("web-browser-symbolic", &tr(MsgKey::ui_open_on_anilist));
    open_link.set_visible(false);
    {
        let link = Rc::clone(&link);
        open_link.connect_clicked(move |_| {
            if let Some(link) = link.borrow().as_deref() {
                crate::entry_core::overflow_menu::open_in_browser(link);
            }
        });
    }
    ui::push(&ui::page(&tr(MsgKey::reviews_title), &stack, vec![open_link.clone().upcast()], true));
    load(entry_id, kind, &stack, &open_link, &link);
}

/// Asks AniList, or shows why it cannot.
fn load(entry_id: i64, kind: Kind, stack: &gtk::Stack, open_link: &gtk::Button, link: &Rc<RefCell<Option<String>>>) {
    stack.set_visible_child_name("loading");
    if super::blocked() {
        return show_status(stack, &tr(MsgKey::comments_rate_limited), Some((entry_id, kind, open_link, link)));
    }
    let Some(runtime) = super::runtime() else { return show_status(stack, &tr(MsgKey::reviews_failed), Some((entry_id, kind, open_link, link))) };
    let ids = super::ids_of(entry_id);
    let known = super::remembered_media(entry_id);
    let (stack, open_link, link) = (stack.downgrade(), open_link.downgrade(), Rc::clone(link));
    crate::background::run_async_on(
        &runtime,
        async move {
            let transport = super::transport()?;
            let media = match known {
                Some(media) => media,
                None => super::resolve_media(&transport, ids, kind).await?,
            };
            let reviews = match media {
                Some(id) => api::reviews(&transport, id).await?,
                None => Vec::new(),
            };
            Ok::<_, AniListError>((media, reviews))
        },
        move |answer| {
            let (Some(stack), Some(open_link)) = (stack.upgrade(), open_link.upgrade()) else { return };
            match answer {
                Some(Ok((media, reviews))) => {
                    super::remember_media(entry_id, media);
                    let Some(media) = media else { return show_status(&stack, &tr(MsgKey::comments_not_on_anilist), None) };
                    *link.borrow_mut() = Some(api::media_url(media, kind));
                    open_link.set_visible(true);
                    if reviews.is_empty() {
                        return show_status(&stack, &tr(MsgKey::reviews_none), None);
                    }
                    show_reviews(&stack, reviews);
                }
                Some(Err(error)) => {
                    tracing::warn!(%error, entry_id, "reviews: AniList did not answer");
                    super::note_error(&error);
                    show_status(&stack, &super::error_text(Some(&error), MsgKey::reviews_failed), Some((entry_id, kind, &open_link, &link)));
                }
                None => show_status(&stack, &tr(MsgKey::reviews_failed), Some((entry_id, kind, &open_link, &link))),
            }
        },
    );
}

type Retry<'a> = (i64, Kind, &'a gtk::Button, &'a Rc<RefCell<Option<String>>>);

/// `text` in place of the list; with `retry`, a Retry button that asks again.
fn show_status(stack: &gtk::Stack, text: &str, retry: Option<Retry<'_>>) {
    if let Some(old) = stack.child_by_name("status") {
        stack.remove(&old);
    }
    let page = match retry {
        Some((entry_id, kind, open_link, link)) => {
            let (weak_stack, weak_link_button, link) = (stack.downgrade(), open_link.downgrade(), Rc::clone(link));
            crate::widgets::status::error(text, None, Some(move || {
                if let (Some(stack), Some(open_link)) = (weak_stack.upgrade(), weak_link_button.upgrade()) {
                    load(entry_id, kind, &stack, &open_link, &link);
                }
            }))
        }
        None => crate::widgets::status::empty(ICON, text, None),
    };
    stack.add_named(&page, Some("status"));
    stack.set_visible_child_name("status");
}

/// The reviews as rows; a row opens the whole review.
fn show_reviews(stack: &gtk::Stack, reviews: Vec<Review>) {
    if let Some(old) = stack.child_by_name("list") {
        stack.remove(&old);
    }
    let list = ui::boxed_list();
    for review in &reviews {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_margin_top(10);
        column.set_margin_bottom(10);
        column.set_margin_start(12);
        column.set_margin_end(12);
        column.append(&view::meta_line(&review.user, &details(review)));
        let summary = gtk::Label::builder().label(&review.summary).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).build();
        column.append(&summary);
        let row = gtk::ListBoxRow::builder().child(&column).activatable(true).build();
        list.append(&row);
    }
    let reviews = Rc::new(reviews);
    list.connect_row_activated(move |_, row| {
        if let Some(review) = usize::try_from(row.index()).ok().and_then(|i| reviews.get(i)) {
            open_review(review);
        }
    });
    let column = ui::column();
    column.append(&list);
    stack.add_named(&ui::scrolled(&column), Some("list"));
    stack.set_visible_child_name("list");
}

/// The whole text of `review` on a page of its own, titled with its writer's name.
fn open_review(review: &Review) {
    let column = ui::column();
    column.append(&view::meta_line(&review.user, &details(review)));
    let summary = gtk::Label::builder().label(&review.summary).xalign(0.0).wrap(true).css_classes(["title-4"]).build();
    column.append(&summary);
    column.append(&view::rich_text(&review.html, false));
    let open_link = ui::icon_button("web-browser-symbolic", &tr(MsgKey::ui_open_on_anilist));
    let url = review.url.clone();
    open_link.connect_clicked(move |_| crate::entry_core::overflow_menu::open_in_browser(&url));
    ui::push(&ui::page(&review.user, &ui::scrolled(&column), vec![open_link.upcast()], true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_under_a_name_has_the_score_and_the_helpful_count() {
        let review = Review { id: 1, user: "befalt".into(), summary: String::new(), score: 100, rating: 336, rating_amount: 385, html: String::new(), url: String::new() };
        assert_eq!(details(&review), "100/100 \u{b7} 336 of 385 found it helpful");
    }
}
