//! The library's view options (user decision 2026-10-12), on the shared menu button of `widgets::view_options`:
//! "Show only" checks, "Sort by" radios with "Reverse order", and "View" radios Grid / List. Every change is written at
//! once; the button shows a dot while a Show only filter is on. A filter is include-or-off: there is no exclude state
//! (`load::include_only`).

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use gtk::glib;
use kuro_platform::kind::Kind;
use kuro_platform::sort::{Direction, LibrarySort, SortType};
use kuro_i18n::{MsgKey, tr};
use kuro_prefs::library::{LibraryDisplayMode, LibraryPrefs};
use kuro_prefs::pref::Pref;
use kuro_prefs::tristate::TriState;

use crate::library::load::include_only;
use crate::misc::diet_defaults as diet;
use crate::widgets::view_options::{Item, Pick, Section, ViewOptions};

use super::{Inner, active_index, submit};

/// One "Show only" check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Downloaded,
    /// Unread for manga, Unwatched for anime.
    Unseen,
    Completed,
    /// On any logged-in tracker; listed only while one is logged in.
    Tracked,
}

impl Show {
    pub const ALL: [Show; 4] = [Show::Downloaded, Show::Unseen, Show::Completed, Show::Tracked];

    fn action(self) -> &'static str {
        match self {
            Show::Downloaded => "downloaded",
            Show::Unseen => "unseen",
            Show::Completed => "completed",
            Show::Tracked => "tracked",
        }
    }

    pub fn label(self, kind: Kind) -> String {
        match (self, kind) {
            (Show::Downloaded, _) => tr(MsgKey::label_downloaded),
            (Show::Unseen, Kind::Manga) => tr(MsgKey::action_filter_unread),
            (Show::Unseen, Kind::Anime) => tr(MsgKey::action_filter_unwatched),
            (Show::Completed, _) => tr(MsgKey::completed),
            (Show::Tracked, _) => tr(MsgKey::action_filter_tracked),
        }
    }
}

/// The stored keys behind one check: one key, or one per logged-in tracker for Tracked.
fn keys(prefs: &LibraryPrefs, kind: Kind, show: Show, logged_in: &BTreeSet<i64>) -> Vec<Pref<TriState>> {
    match show {
        Show::Downloaded => vec![prefs.filter_downloaded(kind).clone()],
        Show::Unseen => vec![prefs.filter_unread(kind).clone()],
        Show::Completed => vec![prefs.filter_completed(kind).clone()],
        Show::Tracked => logged_in.iter().map(|id| prefs.filter_tracked(kind, *id as i32)).collect(),
    }
}

/// Whether a check is on. The global downloaded-only switch forces Downloaded on; an old exclude value reads as off.
pub fn is_on(prefs: &LibraryPrefs, kind: Kind, show: Show, logged_in: &BTreeSet<i64>, downloaded_only: bool) -> bool {
    (show == Show::Downloaded && downloaded_only)
        || keys(prefs, kind, show, logged_in).iter().any(|p| include_only(p.get()) == TriState::ENABLED_IS)
}

/// Writes a check: on is include, off is off. Tracked writes the key of every logged-in tracker.
pub fn set(prefs: &LibraryPrefs, kind: Kind, show: Show, on: bool, logged_in: &BTreeSet<i64>) -> Result<(), kuro_prefs::PrefError> {
    let state = if on { TriState::ENABLED_IS } else { TriState::DISABLED };
    keys(prefs, kind, show, logged_in).iter().try_for_each(|p| p.set(state))
}

/// Sort types the menu does not offer: HIDDEN ON PURPOSE (user, 2026-10-14: "useless"). Only the menu rows are gone. The
/// sort types, their labels and the stored values still exist, so a sort saved earlier keeps working; to bring one back,
/// remove it from this list. Noted in CLAUDE.md.
const HIDDEN_SORTS: [SortType; 4] = [SortType::TotalItems, SortType::ItemFetchDate, SortType::AiringTime, SortType::Random];

/// Whether the menu offers the "Reverse order" check. HIDDEN ON PURPOSE (user, 2026-10-14), like [`HIDDEN_SORTS`]: the
/// direction a sort has is kept, and picking another sort keeps it.
const OFFER_REVERSE: bool = false;

/// The "Sort by" radios of a kind, in its declaration order, without the [`HIDDEN_SORTS`]. Tracker score is offered only
/// with a tracker logged in.
pub fn sort_rows(kind: Kind, has_tracker: bool) -> Vec<SortType> {
    SortType::types(kind).iter().copied().filter(|ty| !HIDDEN_SORTS.contains(ty) && (has_tracker || *ty != SortType::TrackerMean)).collect()
}

/// The radio title from the string table (the labels differ between manga and anime).
pub fn sort_label(kind: Kind, ty: SortType) -> MsgKey {
    use Kind::{Anime, Manga};
    match (kind, ty) {
        (_, SortType::Alphabetical) => MsgKey::action_sort_alpha,
        (Manga, SortType::LastActivity) => MsgKey::action_sort_last_read,
        (Anime, SortType::LastActivity) => MsgKey::action_sort_last_seen,
        (Manga, SortType::LastUpdate) => MsgKey::action_sort_last_manga_update,
        (Anime, SortType::LastUpdate) => MsgKey::action_sort_last_anime_update,
        (Manga, SortType::UnreadCount) => MsgKey::action_sort_unread_count,
        (Anime, SortType::UnreadCount) => MsgKey::action_sort_unseen_count,
        (Manga, SortType::TotalItems) => MsgKey::action_sort_total,
        (Anime, SortType::TotalItems) => MsgKey::action_sort_total_episodes,
        (Manga, SortType::LatestItem) => MsgKey::action_sort_latest_chapter,
        (Anime, SortType::LatestItem) => MsgKey::action_sort_latest_episode,
        (Manga, SortType::ItemFetchDate) => MsgKey::action_sort_chapter_fetch_date,
        (Anime, SortType::ItemFetchDate) => MsgKey::action_sort_episode_fetch_date,
        (_, SortType::DateAdded) => MsgKey::action_sort_date_added,
        (_, SortType::TrackerMean) => MsgKey::action_sort_tracker_score,
        (_, SortType::AiringTime) => MsgKey::action_sort_airing_time,
        (_, SortType::Random) => MsgKey::action_sort_random,
    }
}

/// Picking a sort key keeps the direction; Random has no order to reverse, so it is always ascending.
pub fn pick(current: LibrarySort, ty: SortType) -> LibrarySort {
    let dir = if ty == SortType::Random { Direction::Ascending } else { current.dir };
    LibrarySort { ty, dir }
}

/// "Reverse order" is the descending direction.
pub fn reversed(sort: LibrarySort) -> bool {
    sort.ty != SortType::Random && sort.dir == Direction::Descending
}

pub fn reverse(sort: LibrarySort, on: bool) -> LibrarySort {
    LibrarySort { dir: if on { Direction::Descending } else { Direction::Ascending }, ..sort }
}

/// The display mode the Grid / List radios write. Grid keeps a stored grid style and turns List into the default grid.
pub fn mode(list: bool, current: LibraryDisplayMode) -> LibraryDisplayMode {
    match (list, current) {
        (true, _) => LibraryDisplayMode::List,
        (false, LibraryDisplayMode::List) => LibraryDisplayMode::DEFAULT,
        (false, grid) => grid,
    }
}

fn mode_target(mode: LibraryDisplayMode) -> &'static str {
    if mode == LibraryDisplayMode::List { "list" } else { "grid" }
}

/// The three grid styles of the Grid style radios: the title on the cover, under it, or no title.
fn grid_styles() -> [(&'static str, String, LibraryDisplayMode); 3] {
    [
        ("compact", tr(MsgKey::ui_compact), LibraryDisplayMode::CompactGrid),
        ("comfortable", tr(MsgKey::ui_comfortable), LibraryDisplayMode::ComfortableGrid),
        ("cover", tr(MsgKey::ui_cover_only), LibraryDisplayMode::CoverOnlyGrid),
    ]
}

/// The Grid style radio of `mode`; none in the list view.
fn style_target(mode: LibraryDisplayMode) -> &'static str {
    grid_styles().into_iter().find(|(_, _, m)| *m == mode).map_or("", |(token, _, _)| token)
}

/// The grid a Grid style radio picks (from the list view too).
fn style_mode(token: &str) -> Option<LibraryDisplayMode> {
    grid_styles().into_iter().find(|(t, _, _)| *t == token).map(|(_, _, m)| m)
}

/// The sort of the active page and the category it is stored on (`None`: the global sort).
fn current_sort(st: &Inner) -> (Option<i64>, LibrarySort) {
    let page = st.built.as_ref().and_then(|b| b.pages.get(active_index(st)));
    let categorized = diet::LIBRARY_CATEGORISED_DISPLAY;
    let category = page.map(|p| p.category.id).filter(|_| categorized);
    let sort = match page {
        Some(p) if categorized => p.category.sort(st.deps.kind),
        _ => st.deps.categories.global_sort(),
    };
    (category, sort)
}

/// The library's menu on the shared view-options button (`widgets::view_options`). Cheap to clone.
#[derive(Clone)]
pub(super) struct Options {
    /// What the page header shows: the menu button with the dot over it.
    pub widget: gtk::Overlay,
    menu: ViewOptions,
    /// Whether the menu lists the tracker items; the menu is rebuilt when this changes.
    has_tracker: Rc<Cell<bool>>,
}

/// The menu: four titled sections (Grid style picks how the grid shows titles). The Show only checks are named by `Show::action`, the sort radios carry their
/// token, and the View radios "grid" or "list".
fn sections(kind: Kind, has_tracker: bool) -> Vec<Section> {
    let show = Show::ALL.into_iter().filter(|s| has_tracker || *s != Show::Tracked).map(|s| Item::toggle(s.action(), s.label(kind))).collect();
    let mut sort: Vec<Item> =
        sort_rows(kind, has_tracker).into_iter().filter_map(|ty| ty.token(kind).map(|token| Item::radio("sort", token, tr(sort_label(kind, ty))))).collect();
    if OFFER_REVERSE {
        sort.push(Item::toggle("reverse", tr(MsgKey::ui_reverse_order)));
    }
    vec![
        Section::filter(tr(MsgKey::ui_show_only), show),
        Section::new(tr(MsgKey::ui_sort_by), sort),
        Section::new("View", vec![Item::radio("mode", "grid", "Grid"), Item::radio("mode", "list", tr(MsgKey::action_display_list))]),
        Section::new(tr(MsgKey::ui_grid_style), grid_styles().iter().map(|(token, label, _)| Item::radio("style", *token, label)).collect()),
        Section::new(tr(MsgKey::pref_cover_size), vec![Item::custom(COVER_SIZE)]),
    ]
}

/// The custom slot of the cover size slider.
const COVER_SIZE: &str = "cover_size";

/// The cover size slider: the column width of the Library and Browse grids. A move is stored at once, and the grids
/// follow the stored value, so the covers resize while it moves.
fn cover_size_slider(pref: &Pref<i32>) -> gtk::Widget {
    use gtk::prelude::*;
    use kuro_domain::library::columns::{COVER_WIDTH_MAX_PX, COVER_WIDTH_MIN_PX, COVER_WIDTH_STEP_PX, column_width, snap_cover_width};
    let scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(COVER_WIDTH_MIN_PX),
        f64::from(COVER_WIDTH_MAX_PX),
        f64::from(COVER_WIDTH_STEP_PX),
    );
    scale.set_value(f64::from(column_width(pref.get())));
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_size_request(200, -1);
    scale.update_property(&[gtk::accessible::Property::Label(&tr(MsgKey::pref_cover_size))]);
    let pref = pref.clone();
    scale.connect_value_changed(move |scale| {
        let width = snap_cover_width(scale.value());
        if width != pref.get()
            && let Err(e) = pref.set(width)
        {
            tracing::warn!(error = %e, "the cover size could not be saved");
        }
    });
    scale.upcast()
}

impl Options {
    /// The button and its menu; `wire` connects the picks once the view exists.
    pub fn new(kind: Kind) -> Self {
        let menu = ViewOptions::new(&tr(MsgKey::ui_view_options), &sections(kind, false));
        Self { widget: menu.widget.clone(), menu, has_tracker: Rc::new(Cell::new(false)) }
    }

    /// Connects the picks. A filter or view write reloads through the view's preference watch; a sort write goes
    /// through `CategoryService::set_sort_mode` (which draws a new Random seed) and reloads when it is done.
    pub fn wire(&self, weak: &Weak<RefCell<Inner>>) {
        if let Some(inner) = weak.upgrade() {
            let pref = inner.borrow().deps.prefs.cover_size.clone();
            self.menu.set_custom(COVER_SIZE, move || cover_size_slider(&pref));
        }
        let weak = weak.clone();
        self.menu.connect(move |choice| {
            let Some(inner) = weak.upgrade() else { return };
            match choice {
                Pick::Toggle("reverse", on) => {
                    let (category, current) = current_sort(&inner.borrow());
                    write_sort(&inner, category, reverse(current, on));
                }
                Pick::Toggle(name, on) => {
                    let Some(show) = Show::ALL.into_iter().find(|s| s.action() == name) else { return };
                    let (prefs, kind, logged_in) = {
                        let st = inner.borrow();
                        (st.deps.prefs.clone(), st.deps.kind, (st.deps.logged_in)())
                    };
                    if let Err(e) = set(&prefs, kind, show, on, &logged_in) {
                        tracing::error!(error = %e, ?show, "saving a library filter failed");
                    }
                }
                Pick::Radio("sort", token) => {
                    let kind = inner.borrow().deps.kind;
                    let Some(ty) = SortType::from_token(kind, &token) else { return };
                    let (category, current) = current_sort(&inner.borrow());
                    write_sort(&inner, category, pick(current, ty));
                }
                Pick::Radio("mode", target) => {
                    let prefs = inner.borrow().deps.prefs.clone();
                    let next = mode(target == "list", prefs.display_mode.get());
                    kuro_domain::category::service::set_display_mode(&prefs, next);
                }
                Pick::Radio("style", token) => {
                    let Some(next) = style_mode(&token) else { return };
                    let prefs = inner.borrow().deps.prefs.clone();
                    kuro_domain::category::service::set_display_mode(&prefs, next);
                }
                Pick::Radio(..) | Pick::Action(_) => {}
            }
        });
    }

    /// Brings the checks, radios and the dot in line with what is stored, and lists the tracker items only while a
    /// tracker is logged in. Runs on every render, without a borrow of the view held.
    pub fn sync(inner: &Rc<RefCell<Inner>>) {
        let (options, kind, prefs, logged_in, downloaded_only, sort) = {
            let st = inner.borrow();
            (st.options.clone(), st.deps.kind, st.deps.prefs.clone(), st.logged_in.clone(), (st.deps.downloaded_only)(), current_sort(&st).1)
        };
        let menu = &options.menu;
        let has_tracker = !logged_in.is_empty();
        if options.has_tracker.replace(has_tracker) != has_tracker {
            menu.set_sections(&sections(kind, has_tracker));
        }
        for show in Show::ALL {
            menu.set_toggle(show.action(), is_on(&prefs, kind, show, &logged_in, downloaded_only));
            // the global downloaded-only switch holds the Downloaded check on
            menu.set_enabled(show.action(), !(show == Show::Downloaded && downloaded_only));
        }
        menu.set_radio("sort", sort.ty.token(kind).unwrap_or_default());
        menu.set_toggle("reverse", reversed(sort));
        menu.set_enabled("reverse", sort.ty != SortType::Random);
        menu.set_radio("mode", mode_target(prefs.display_mode.get()));
        menu.set_radio("style", style_target(prefs.display_mode.get()));
    }

    #[cfg(test)]
    pub fn group(&self) -> &gtk::gio::SimpleActionGroup {
        self.menu.group()
    }

    #[cfg(test)]
    pub fn dot_visible(&self) -> bool {
        self.menu.dot_visible()
    }
}

fn write_sort(inner: &Rc<RefCell<Inner>>, category: Option<i64>, sort: LibrarySort) {
    let (categories, weak) = (inner.borrow().deps.categories.clone(), Rc::downgrade(inner));
    glib::spawn_future_local(async move {
        if let Err(e) = categories.set_sort_mode(category, sort, kuro_domain::library::order::new_seed).await {
            tracing::error!(error = %e, "saving the library sort failed");
        }
        if let Some(inner) = weak.upgrade() {
            submit(&inner);
        }
    });
}

#[cfg(test)]
mod tests {
    use kuro_prefs::store::PrefStore;

    use super::*;

    #[test]
    fn each_grid_style_picks_its_grid_and_the_list_view_marks_none() {
        for (token, _, mode) in grid_styles() {
            assert_eq!(style_mode(token), Some(mode));
            assert_eq!(style_target(mode), token);
        }
        assert_eq!(style_target(LibraryDisplayMode::List), "", "no style is marked in the list view");
        assert_eq!(style_mode("list"), None);
    }

    fn prefs() -> LibraryPrefs {
        LibraryPrefs::new(PrefStore::in_memory())
    }

    #[test]
    fn a_check_writes_include_or_off_and_reads_an_old_exclude_as_off() {
        let p = prefs();
        let none = BTreeSet::new();
        assert!(!is_on(&p, Kind::Manga, Show::Unseen, &none, false));
        set(&p, Kind::Manga, Show::Unseen, true, &none).expect("write");
        assert_eq!(p.filter_unread.get(), TriState::ENABLED_IS);
        assert!(is_on(&p, Kind::Manga, Show::Unseen, &none, false));
        assert!(!is_on(&p, Kind::Anime, Show::Unseen, &none, false), "anime and manga keep separate keys");
        set(&p, Kind::Manga, Show::Unseen, false, &none).expect("write");
        assert_eq!(p.filter_unread.get(), TriState::DISABLED);
        p.filter_completed_anime.set(TriState::ENABLED_NOT).expect("write");
        assert!(!is_on(&p, Kind::Anime, Show::Completed, &none, false));
    }

    #[test]
    fn downloaded_only_holds_the_downloaded_check_on() {
        let p = prefs();
        assert!(is_on(&p, Kind::Anime, Show::Downloaded, &BTreeSet::new(), true));
        assert!(!is_on(&p, Kind::Anime, Show::Completed, &BTreeSet::new(), true));
    }

    #[test]
    fn tracked_is_one_check_over_every_logged_in_tracker() {
        let p = prefs();
        let logged = BTreeSet::from([1, 2]);
        assert!(!is_on(&p, Kind::Anime, Show::Tracked, &logged, false));
        set(&p, Kind::Anime, Show::Tracked, true, &logged).expect("write");
        assert_eq!(p.filter_tracked_anime(1).get(), TriState::ENABLED_IS);
        assert_eq!(p.filter_tracked_anime(2).get(), TriState::ENABLED_IS);
        assert!(is_on(&p, Kind::Anime, Show::Tracked, &BTreeSet::from([2]), false));
        set(&p, Kind::Anime, Show::Tracked, false, &logged).expect("write");
        assert!(!is_on(&p, Kind::Anime, Show::Tracked, &logged, false));
    }

    #[test]
    fn the_labels_follow_the_kind() {
        assert_eq!(Show::Unseen.label(Kind::Manga), "Unread");
        assert_eq!(Show::Unseen.label(Kind::Anime), "Unwatched");
        assert_eq!(Show::Tracked.label(Kind::Anime), "Tracked");
        for kind in [Kind::Manga, Kind::Anime] {
            for ty in sort_rows(kind, true) {
                assert!(!tr(sort_label(kind, ty)).is_empty(), "{kind:?} {ty:?}");
                assert!(ty.token(kind).is_some(), "every radio has a target");
            }
        }
        assert_eq!(tr(sort_label(Kind::Anime, SortType::LastActivity)), "Last seen");
    }

    #[test]
    fn tracker_score_needs_a_tracker_and_the_hidden_sorts_are_never_offered() {
        assert!(!sort_rows(Kind::Manga, false).contains(&SortType::TrackerMean));
        assert!(sort_rows(Kind::Manga, true).contains(&SortType::TrackerMean));
        for kind in [Kind::Manga, Kind::Anime] {
            for hidden in HIDDEN_SORTS {
                assert!(!sort_rows(kind, true).contains(&hidden), "{kind:?} {hidden:?} is hidden on purpose");
            }
        }
    }

    #[test]
    fn a_sort_key_keeps_the_direction_and_reverse_is_descending() {
        let desc = LibrarySort { ty: SortType::Alphabetical, dir: Direction::Descending };
        assert!(reversed(desc));
        assert_eq!(pick(desc, SortType::DateAdded), LibrarySort { ty: SortType::DateAdded, dir: Direction::Descending });
        let random = pick(desc, SortType::Random);
        assert_eq!(random.dir, Direction::Ascending);
        assert!(!reversed(random), "Random has no order to reverse");
        assert_eq!(reverse(LibrarySort::DEFAULT, true).dir, Direction::Descending);
        assert_eq!(reverse(desc, false).dir, Direction::Ascending);
        assert!(!reversed(LibrarySort::DEFAULT));
    }

    #[test]
    fn grid_keeps_a_grid_style_and_list_is_list() {
        assert_eq!(mode(true, LibraryDisplayMode::ComfortableGrid), LibraryDisplayMode::List);
        assert_eq!(mode(false, LibraryDisplayMode::List), LibraryDisplayMode::DEFAULT);
        assert_eq!(mode(false, LibraryDisplayMode::CoverOnlyGrid), LibraryDisplayMode::CoverOnlyGrid);
        assert_eq!(mode_target(LibraryDisplayMode::CoverOnlyGrid), "grid");
        assert_eq!(mode_target(LibraryDisplayMode::List), "list");
    }
}
