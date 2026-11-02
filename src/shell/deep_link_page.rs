//! Deep-link loading page (T-53a-047 anime, T-53a-049 manga). A link or a searched address is resolved in the background
//! while this page shows Loading. The states, what each shows, and where a state navigates are pure here. Navigation
//! runs once per state change, from [`transition`], never from the render function (defect E-10). The resolver (kuro-core,
//! T-53a-048 and T-53a-050) supplies the [`State`] values.

use std::cell::RefCell;
use std::rc::Rc;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;

use gtk::prelude::*;


#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// What a resolved link points to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The entry itself.
    Entry(i64),
    /// An episode of an anime (opens the player) or a chapter of a manga (opens the reader).
    Child { entry_id: i64, child_id: i64 },
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// The page state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Loading,
    NoResults { query: String },
    Found(Found),
    /// The resolve failed; `reason` is shown and the page offers Retry and Search instead (E-07).
    Failed { reason: String },
}

/// Where the app goes for a state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    /// The global search of this kind, with the query in its box.
    GlobalSearch { kind: Kind, query: String },
    Entry { kind: Kind, id: i64 },
    /// The player for an anime episode, the reader for a manga chapter.
    Child { kind: Kind, entry_id: i64, child_id: i64 },
}

/// What the page shows in one state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub message: Option<String>,
    pub spinner: bool,
    pub retry: bool,
    pub search_instead: bool,
}

/// The view of a state. Loading has the spinner; a failure has the reason and the two buttons.
pub fn view(state: &State) -> View {
    match state {
        State::Loading => View { title: "Search…".into(), message: None, spinner: true, retry: false, search_instead: false },
        State::NoResults { .. } => View { title: tr(MsgKey::ui_no_results).into(), message: None, spinner: false, retry: false, search_instead: false },
        State::Found(_) => View { title: "Opening…".into(), message: None, spinner: true, retry: false, search_instead: false },
        State::Failed { reason } => View {
            title: tr(MsgKey::ui_couldn_t_open_this_link).into(),
            message: Some(reason.clone()),
            spinner: false,
            retry: true,
            search_instead: true,
        },
    }
}

/// Where a state navigates. Loading and a failure stay on the page.
pub fn navigation(kind: Kind, state: &State) -> Option<Nav> {
    match state {
        State::Loading | State::Failed { .. } => None,
        State::NoResults { query } => Some(Nav::GlobalSearch { kind, query: query.clone() }),
        State::Found(Found::Entry(id)) => Some(Nav::Entry { kind, id: *id }),
        State::Found(Found::Child { entry_id, child_id }) => Some(Nav::Child { kind, entry_id: *entry_id, child_id: *child_id }),
    }
}

/// The navigation for a change from `previous` to `next`: only a state that differs from the one before navigates, so the
/// same state rendered again does nothing.
pub fn transition(kind: Kind, previous: &State, next: &State) -> Option<Nav> {
    if previous == next { None } else { navigation(kind, next) }
}

/// The page widget for a state: the title with the page spinner while it works, the empty state when nothing was found,
/// or the error state with its buttons. The buttons run `on_retry` and `on_search_instead`. Navigation is not done here
/// (see [`transition`]).
pub fn widget(state: &State, on_retry: impl Fn() + 'static, on_search_instead: impl Fn() + 'static) -> gtk::Widget {
    use kuro_i18n::{tr, MsgKey};
    let shown = view(state);
    if shown.retry || shown.search_instead {
        let mut buttons = Vec::new();
        if shown.retry {
            let retry = gtk::Button::with_label(&tr(MsgKey::action_retry));
            retry.connect_clicked(move |_| on_retry());
            buttons.push(retry);
        }
        if shown.search_instead {
            let search = gtk::Button::with_label(&tr(MsgKey::ui_search_instead));
            search.connect_clicked(move |_| on_search_instead());
            buttons.push(search);
        }
        return crate::widgets::status::error_with(&shown.title, shown.message.as_deref(), &buttons).upcast();
    }
    if !shown.spinner {
        return crate::widgets::status::empty("system-search-symbolic", &shown.title, None).upcast();
    }
    crate::widgets::status::loading(Some(&shown.title))
}

/// Delivers the result of one resolve. It must run on the GTK thread.
pub type Reply = Box<dyn FnOnce(State)>;
/// Resolves a link or an address in the background and calls the reply with `Found`, `NoResults` or `Failed`.
pub type Resolver = Rc<dyn Fn(String, Reply)>;

thread_local! {
    static RESOLVERS: RefCell<[Option<Resolver>; 2]> = const { RefCell::new([None, None]) };
}

fn slot(kind: Kind) -> usize {
    match kind {
        Kind::Anime => 0,
        Kind::Manga => 1,
    }
}

#[cfg(test)]
/// Installs the resolver of one catalogue (anime T-53a-048, manga T-53a-050). Without one, [`open`] goes straight to the
/// global search with the address as text.
pub fn install_resolver(kind: Kind, resolver: Resolver) {
    RESOLVERS.with(|r| r.borrow_mut()[slot(kind)] = Some(resolver));
}

struct Page {
    kind: Kind,
    link: String,
    state: RefCell<State>,
    root: gtk::Box,
    resolver: Resolver,
}

/// Opens the loading page for `link` and resolves it. Each state change draws the page and navigates once, through
/// [`transition`]; a page the user already left does not navigate.
pub fn open(kind: Kind, link: &str) {
    let Some(resolver) = RESOLVERS.with(|r| r.borrow()[slot(kind)].clone()) else {
        run(&Nav::GlobalSearch { kind, query: link.to_owned() });
        return;
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let page = Rc::new(Page { kind, link: link.to_owned(), state: RefCell::new(State::Loading), root, resolver });
    draw(&page);
    // its own header, so Back and the window buttons show while the link resolves
    crate::ui::push(&crate::ui::page("", &page.root, Vec::new(), true));
    resolve(&page);
}

fn resolve(page: &Rc<Page>) {
    let weak = Rc::downgrade(page);
    (page.resolver)(
        page.link.clone(),
        Box::new(move |next| {
            if let Some(page) = weak.upgrade() {
                change(&page, next);
            }
        }),
    );
}

fn draw(page: &Rc<Page>) {
    while let Some(child) = page.root.first_child() {
        page.root.remove(&child);
    }
    let (retry, instead) = (Rc::downgrade(page), Rc::downgrade(page));
    let content = widget(
        &page.state.borrow(),
        move || {
            if let Some(page) = retry.upgrade() {
                *page.state.borrow_mut() = State::Loading;
                draw(&page);
                resolve(&page);
            }
        },
        move || {
            if let Some(page) = instead.upgrade() {
                leave(&page, &Nav::GlobalSearch { kind: page.kind, query: page.link.clone() });
            }
        },
    );
    content.set_vexpand(true);
    page.root.append(&content);
}

fn change(page: &Rc<Page>, next: State) {
    let nav = transition(page.kind, &page.state.borrow(), &next);
    *page.state.borrow_mut() = next;
    draw(page);
    if let Some(nav) = nav {
        leave(page, &nav);
    }
}

/// Replaces the loading page by the destination, unless the user already went back.
fn leave(page: &Page, nav: &Nav) {
    if !page.root.is_mapped() {
        return;
    }
    crate::ui::pop();
    run(nav);
}

/// Opens the global search of `kind` with `query` in its box.
pub fn open_search(kind: Kind, query: &str) {
    crate::browse::pending::open(crate::browse::global_search::deep_link_fallback_for(kind, query));
}

fn run(nav: &Nav) {
    match nav {
        Nav::GlobalSearch { kind, query } => open_search(*kind, query),
        Nav::Entry { kind: Kind::Anime, id } | Nav::Child { kind: Kind::Anime, entry_id: id, .. } => crate::entry_core::details::route::open(*id),
        Nav::Entry { kind: Kind::Manga, id } => crate::entry_core::details::route::open_kind(Kind::Manga, *id),
        Nav::Child { kind: Kind::Manga, entry_id, child_id } => crate::ui::push(&crate::reader::launch::open_chapter(*entry_id, *child_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loading() -> State {
        State::Loading
    }

    #[test]
    fn loading_shows_the_search_title_and_a_spinner_and_stays_put() {
        let v = view(&loading());
        assert_eq!(v.title, "Search…");
        assert!(v.spinner);
        assert!(!v.retry && !v.search_instead);
        assert_eq!(navigation(Kind::Anime, &loading()), None);
    }

    #[test]
    fn no_results_opens_the_global_search_of_the_kind_with_the_query() {
        let state = State::NoResults { query: "frieren".into() };
        assert_eq!(navigation(Kind::Anime, &state), Some(Nav::GlobalSearch { kind: Kind::Anime, query: "frieren".into() }));
        assert_eq!(navigation(Kind::Manga, &state), Some(Nav::GlobalSearch { kind: Kind::Manga, query: "frieren".into() }));
    }

    #[test]
    fn a_found_entry_opens_the_entry_and_a_found_child_opens_the_player_or_the_reader() {
        assert_eq!(navigation(Kind::Anime, &State::Found(Found::Entry(3))), Some(Nav::Entry { kind: Kind::Anime, id: 3 }));
        let child = State::Found(Found::Child { entry_id: 3, child_id: 9 });
        assert_eq!(navigation(Kind::Anime, &child), Some(Nav::Child { kind: Kind::Anime, entry_id: 3, child_id: 9 }));
        assert_eq!(navigation(Kind::Manga, &child), Some(Nav::Child { kind: Kind::Manga, entry_id: 3, child_id: 9 }));
    }

    #[test]
    fn a_failure_shows_the_reason_with_retry_and_search_instead() {
        let v = view(&State::Failed { reason: "timeout".into() });
        assert_eq!(v.message.as_deref(), Some("timeout"));
        assert!(v.retry && v.search_instead);
        assert!(!v.spinner);
        assert_eq!(navigation(Kind::Anime, &State::Failed { reason: "x".into() }), None, "a failure does not navigate");
    }

    #[test]
    fn without_a_resolver_the_link_goes_to_the_search_of_its_kind() {
        let nav = Nav::GlobalSearch { kind: Kind::Manga, query: "https://x.test/m/1".into() };
        assert_eq!(navigation(Kind::Manga, &State::NoResults { query: "https://x.test/m/1".into() }), Some(nav));
        assert_eq!(slot(Kind::Anime), 0);
        assert_eq!(slot(Kind::Manga), 1);
    }

    #[test]
    fn navigation_happens_once_per_state_change() {
        let found = State::Found(Found::Entry(1));
        assert_eq!(transition(Kind::Anime, &loading(), &found), Some(Nav::Entry { kind: Kind::Anime, id: 1 }));
        assert_eq!(transition(Kind::Anime, &found, &found), None, "the same state again does not navigate");
        assert_eq!(transition(Kind::Anime, &loading(), &loading()), None);
    }
}
