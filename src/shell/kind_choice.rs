//! The one Anime | Manga choice of the app. Every page with a kind switch (Library, Browse, Updates, History, Downloads,
//! Categories) follows it: a switch on one page moves them all, and the choice is kept across runs (`last_kind`).
//!
//! Show (Settings > Appearance, `base.kinds_shown`, user 2026-11-21) can keep the app to one kind. The pages that follow
//! it ([`Lock::Shown`]) then show only that kind and their switch ([`register_switch`]) hides; a page that edits a
//! given kind (Categories, opened from Settings for either kind) keeps both ([`Lock::Free`]). The kept choice is left as
//! it was, so showing both kinds again lands where the user was.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_prefs::base::KindsShown;

/// Whether a following stack keeps to the kind Show allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// It shows only the allowed kind while one is set.
    Shown,
    /// It may show either kind (it still moves with the shared choice to a kind that is allowed).
    Free,
}

thread_local! {
    /// Every following stack, with its page names for anime and for manga, and whether Show holds it.
    static STACKS: RefCell<Vec<(glib::WeakRef<gtk::Stack>, [&'static str; 2], Lock)>> = const { RefCell::new(Vec::new()) };
    /// The switches of the stacks Show holds: hidden while one kind is shown alone.
    static SWITCHES: RefCell<Vec<glib::WeakRef<gtk::StackSwitcher>>> = const { RefCell::new(Vec::new()) };
    /// Set while the other stacks are being moved, so their own change is not taken as a new choice.
    static MOVING: Cell<bool> = const { Cell::new(false) };
    /// The kind shown alone, read once from the setting and kept up to date by [`start`] (a narrow window's switch row
    /// asks every frame); `None` until read.
    static ONLY: Cell<Option<Option<Kind>>> = const { Cell::new(None) };
    /// What redraws when Show changes ([`on_change`]), called after [`ONLY`] holds the new value.
    static LISTENERS: RefCell<Vec<Box<dyn Fn()>>> = RefCell::new(Vec::new());
}

/// The page name of `kind` among `names` (anime first).
fn name_of(kind: Kind, names: [&'static str; 2]) -> &'static str {
    if kind == Kind::Anime { names[0] } else { names[1] }
}

/// The kind a page name stands for.
fn kind_of(name: &str, names: [&'static str; 2]) -> Option<Kind> {
    [Kind::Anime, Kind::Manga].into_iter().find(|&kind| name_of(kind, names) == name)
}

/// The kind a Show setting keeps the app to, or `None` for both.
pub fn only_of(shown: KindsShown) -> Option<Kind> {
    match shown {
        KindsShown::BOTH => None,
        KindsShown::ANIME => Some(Kind::Anime),
        KindsShown::MANGA => Some(Kind::Manga),
    }
}

/// The kind the app shows alone, or `None` while it shows both (and before the app data is open).
pub fn only() -> Option<Kind> {
    if let Some(known) = ONLY.get() {
        return known;
    }
    let read = crate::data::is_installed().then(|| only_of(crate::data::app().base.kinds_shown.get())).flatten();
    if crate::data::is_installed() {
        ONLY.set(Some(read));
    }
    read
}

/// Whether the pages show `kind`.
pub fn is_shown(kind: Kind) -> bool {
    only().is_none_or(|only| only == kind)
}

/// The kind a page shows: the one Show allows alone, else the kept choice.
pub fn resolve(only: Option<Kind>, kept: Kind) -> Kind {
    only.unwrap_or(kept)
}

/// The kept choice; anime before the app data is open.
fn kept() -> Kind {
    if crate::data::is_installed() && crate::data::app().base.last_kind.get() == "manga" { Kind::Manga } else { Kind::Anime }
}

/// The kind the pages show now: the one Show allows alone, else the kept choice.
pub fn current() -> Kind {
    resolve(only(), kept())
}

fn keep(kind: Kind) {
    if crate::data::is_installed() {
        let value = if kind == Kind::Manga { "manga" } else { "anime" };
        if let Err(error) = crate::data::app().base.last_kind.set(value.to_owned()) {
            tracing::warn!(%error, "the Anime | Manga choice could not be kept");
        }
    }
}

/// Moves `stack` to `kind` without taking it as a choice, and closes the search of the kind it leaves.
fn move_quietly(stack: &gtk::Stack, names: [&'static str; 2], kind: Kind) {
    if stack.visible_child_name().as_deref() == Some(name_of(kind, names)) {
        return;
    }
    let was = MOVING.replace(true);
    stack.set_visible_child_name(name_of(kind, names));
    if let Some(page) = stack.parent() {
        crate::ui::close_searches(&page);
    }
    MOVING.set(was);
}

/// Makes `stack` (already holding both pages, named `names`, anime first) show the shared choice and follow it. A
/// [`Lock::Shown`] stack never shows a kind Show hides: a move there (a shortcut, a link) is turned back.
pub fn follow(stack: &gtk::Stack, names: [&'static str; 2], lock: Lock) {
    stack.set_visible_child_name(name_of(current(), names));
    STACKS.with(|stacks| stacks.borrow_mut().push((stack.downgrade(), names, lock)));
    stack.connect_visible_child_name_notify(move |stack| {
        if MOVING.get() {
            return;
        }
        let Some(kind) = stack.visible_child_name().and_then(|name| kind_of(&name, names)) else { return };
        if lock == Lock::Shown && !is_shown(kind) {
            move_quietly(stack, names, current());
            return;
        }
        keep(kind);
        // a search belongs to the kind it was made in
        if let Some(page) = stack.parent() {
            crate::ui::close_searches(&page);
        }
        STACKS.with(|stacks| {
            stacks.borrow_mut().retain(|(weak, _, _)| weak.upgrade().is_some());
            for (other, other_names, other_lock) in stacks.borrow().iter() {
                let Some(other) = other.upgrade().filter(|other| other != stack) else { continue };
                // a page Show holds stays on its kind when the choice is one it hides (Categories opened for it)
                if *other_lock == Lock::Free || is_shown(kind) {
                    move_quietly(&other, *other_names, kind);
                }
            }
        });
    });
}

/// Keeps `switch` (the Anime | Manga switch of a page Show holds) hidden while one kind is shown alone.
pub fn register_switch(switch: &gtk::StackSwitcher) {
    switch.set_visible(only().is_none());
    SWITCHES.with(|switches| switches.borrow_mut().push(switch.downgrade()));
}

/// Whether `switch` stays hidden now: it is one of [`register_switch`]'s and a kind is shown alone.
pub fn switch_hidden(switch: &gtk::StackSwitcher) -> bool {
    only().is_some() && SWITCHES.with(|switches| switches.borrow().iter().any(|known| known.upgrade().is_some_and(|known| &known == switch)))
}

/// Takes a new Show setting: every held stack moves to the kind it may show, and every registered switch hides or comes
/// back (a narrow window's switch row follows on its next frame, `ui::switch_with_narrow_row`).
pub(crate) fn set_only(only: Option<Kind>) {
    ONLY.set(Some(only));
    let kind = resolve(only, kept());
    STACKS.with(|stacks| {
        stacks.borrow_mut().retain(|(weak, _, _)| weak.upgrade().is_some());
        for (stack, names, lock) in stacks.borrow().iter() {
            if let Some(stack) = stack.upgrade().filter(|_| *lock == Lock::Shown) {
                move_quietly(&stack, *names, kind);
            }
        }
    });
    SWITCHES.with(|switches| {
        switches.borrow_mut().retain(|weak| weak.upgrade().is_some());
        for switch in switches.borrow().iter().filter_map(glib::WeakRef::upgrade) {
            switch.set_visible(only.is_none());
        }
    });
    LISTENERS.with(|listeners| {
        for listener in listeners.borrow().iter() {
            listener();
        }
    });
}

/// Calls `redraw` after every change of Show, once [`only`] and [`is_shown`] answer with the new value (a page that
/// filters by kind: Home, the launcher badge). A listener whose page is gone should do nothing.
pub fn on_change(redraw: impl Fn() + 'static) {
    LISTENERS.with(|listeners| listeners.borrow_mut().push(Box::new(redraw)));
}

/// Follows the Show setting for the app's run, wherever it changes (the Appearance page, a backup restore).
pub fn start() {
    if !crate::data::is_installed() {
        return;
    }
    crate::pref_model::bind::follow(&crate::data::app().base.kinds_shown, || true, |shown: KindsShown| set_only(only_of(shown)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(names: [&'static str; 2]) -> gtk::Stack {
        let stack = gtk::Stack::new();
        for name in names {
            stack.add_named(&gtk::Label::new(Some(name)), Some(name));
        }
        stack
    }

    #[test]
    fn a_switch_on_one_page_moves_every_page() {
        crate::testing::with_display(|| {
            let (library, browse) = (stack(["anime", "manga"]), stack(["0", "1"]));
            follow(&library, ["anime", "manga"], Lock::Shown);
            follow(&browse, ["0", "1"], Lock::Shown);
            library.set_visible_child_name("manga");
            assert_eq!(browse.visible_child_name().as_deref(), Some("1"), "browse follows the library");
            browse.set_visible_child_name("0");
            assert_eq!(library.visible_child_name().as_deref(), Some("anime"), "and the other way");
        });
    }

    #[test]
    fn switching_the_kind_closes_an_open_search() {
        crate::testing::with_display(|| {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let bar = gtk::SearchBar::new();
            let kinds = stack(["anime", "manga"]);
            page.append(&bar);
            page.append(&kinds);
            follow(&kinds, ["anime", "manga"], Lock::Shown);
            bar.set_search_mode(true);
            kinds.set_visible_child_name(if kinds.visible_child_name().as_deref() == Some("anime") { "manga" } else { "anime" });
            assert!(!bar.is_search_mode(), "the search closed with the switch");
        });
    }

    #[test]
    fn show_keeps_held_pages_on_its_kind_hides_their_switch_and_gives_both_back() {
        crate::testing::with_display(|| {
            let (library, categories) = (stack(["anime", "manga"]), stack(["anime", "manga"]));
            follow(&library, ["anime", "manga"], Lock::Shown);
            follow(&categories, ["anime", "manga"], Lock::Free);
            let switch = gtk::StackSwitcher::builder().stack(&library).build();
            register_switch(&switch);
            library.set_visible_child_name("anime");
            set_only(Some(Kind::Manga));
            assert_eq!(library.visible_child_name().as_deref(), Some("manga"), "the held page moves to the kind shown");
            assert!(!switch.is_visible() && switch_hidden(&switch), "its switch hides");
            library.set_visible_child_name("anime");
            assert_eq!(library.visible_child_name().as_deref(), Some("manga"), "a move to the hidden kind is turned back");
            categories.set_visible_child_name("manga");
            categories.set_visible_child_name("anime");
            assert_eq!(categories.visible_child_name().as_deref(), Some("anime"), "a free page opens on the hidden kind");
            assert_eq!(library.visible_child_name().as_deref(), Some("manga"), "and the held page stays");
            set_only(None);
            assert!(switch.is_visible() && !switch_hidden(&switch), "both kinds again: the switch is back");
        });
    }

    #[test]
    fn show_resolves_to_the_kind_alone_or_the_kept_choice() {
        assert_eq!(resolve(None, Kind::Manga), Kind::Manga, "both kinds: the kept choice");
        assert_eq!(resolve(Some(Kind::Anime), Kind::Manga), Kind::Anime, "one kind: that kind");
        assert_eq!(only_of(KindsShown::BOTH), None);
        assert_eq!(only_of(KindsShown::ANIME), Some(Kind::Anime));
        assert_eq!(only_of(KindsShown::MANGA), Some(Kind::Manga));
    }

    #[test]
    fn page_names_map_to_kinds_anime_first() {
        assert_eq!(kind_of("1", ["0", "1"]), Some(Kind::Manga));
        assert_eq!(kind_of("anime", ["anime", "manga"]), Some(Kind::Anime));
        assert_eq!(kind_of("spinner", ["anime", "manga"]), None);
    }
}
