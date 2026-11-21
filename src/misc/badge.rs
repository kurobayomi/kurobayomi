//! The badge of the "Updates" entry in the sidebar (T-26a-101): the new manga updates plus the new anime updates the
//! update passes counted, hidden at zero and when the "show updates count" switch is off. With one kind shown alone
//! (Show, Settings > Appearance) only its count is. The Updates page resets the counters when it opens; this only shows
//! them.
//!
//! The label keeps its place when hidden (its opacity goes to zero), so the rows do not shift.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_prefs::library::LibraryPrefs;

use crate::bind;
use crate::misc::diet_defaults as diet;

/// The badge text for the two counters, or `None` when the badge is hidden: the counters of the kinds shown at zero, or
/// the switch off. `only` is the kind shown alone, `None` for both.
pub fn text(manga: i32, anime: i32, show: bool, only: Option<Kind>) -> Option<String> {
    let counted = |kind: Kind, count: i32| if only.is_none_or(|only| only == kind) { i64::from(count.max(0)) } else { 0 };
    let total = counted(Kind::Manga, manga) + counted(Kind::Anime, anime);
    (show && total > 0).then(|| total.to_string())
}

/// What the badge draws from: the manga and anime counters, the switch, and the kind shown alone.
#[derive(Clone, Copy)]
struct State {
    manga: i32,
    anime: i32,
    show: bool,
    only: Option<Kind>,
}

fn paint(label: &gtk::Label, state: State) {
    let shown = text(state.manga, state.anime, state.show, state.only);
    label.set_label(shown.as_deref().unwrap_or(""));
    label.set_opacity(if shown.is_some() { 1.0 } else { 0.0 });
}

/// Shows the live count in `label`, now and on every change of the counters, of the switch or of Show.
pub fn bind(label: &gtk::Label, library: &LibraryPrefs) {
    let state = Rc::new(Cell::new(State { manga: 0, anime: 0, show: diet::LIBRARY_UPDATES_COUNT, only: crate::shell::kind_choice::only() }));
    let (manga, anime, kinds) = (state.clone(), state.clone(), state.clone());
    bind::bind(label, &library.new_manga_updates_count, move |l, count| {
        manga.set(State { manga: count, ..manga.get() });
        paint(l, manga.get());
    });
    bind::bind(label, &library.new_anime_updates_count, move |l, count| {
        anime.set(State { anime: count, ..anime.get() });
        paint(l, anime.get());
    });
    if crate::data::is_installed() {
        bind::bind(label, &crate::data::app().base.kinds_shown, move |l, shown| {
            kinds.set(State { only: crate::shell::kind_choice::only_of(shown), ..kinds.get() });
            paint(l, kinds.get());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_is_the_manga_count_plus_the_anime_count() {
        assert_eq!(text(3, 4, true, None).as_deref(), Some("7"));
        assert_eq!(text(5, 0, true, None).as_deref(), Some("5"));
        assert_eq!(text(0, 2, true, None).as_deref(), Some("2"));
    }

    #[test]
    fn with_one_kind_shown_alone_only_its_count_shows() {
        assert_eq!(text(3, 4, true, Some(Kind::Anime)).as_deref(), Some("4"));
        assert_eq!(text(3, 4, true, Some(Kind::Manga)).as_deref(), Some("3"));
        assert_eq!(text(3, 0, true, Some(Kind::Anime)), None, "the hidden kind's updates alone show nothing");
    }

    #[test]
    fn the_badge_is_hidden_at_zero_and_when_the_switch_is_off() {
        assert_eq!(text(0, 0, true, None), None);
        assert_eq!(text(3, 4, false, None), None);
        assert_eq!(text(-2, 0, true, None), None, "a stray negative counter never shows");
    }

    #[test]
    fn the_placeholder_is_gone_from_the_sidebar() {
        // T-26a-101: no hard-coded count remains in the sidebar's badge table.
        let main = include_str!("../main.rs");
        assert!(!main.contains("\"updates\" => \"8\""));
        assert!(!main.contains("\"updates\" => \"0\"") && !main.contains("\"browse\" => \"1\""));
    }
}
