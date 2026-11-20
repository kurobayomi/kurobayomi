//! The tracking page of an entry (the `tracking_sheet` and `tracker_search` screens, E-25). Tracking keeps the progress on
//! the switched-on trackers (`tracking::ENABLED`: AniList, MyAnimeList and Shikimori), so the page has one tab per
//! tracker (`TAB` keeps the one shown): a Connect row while it is not logged in, a search for the title while the entry
//! is not tracked there, and once it is, the title it is tracked as over native edit rows (status, progress, score, start and finish dates where the tracker
//! keeps them, Open on the site, Stop tracking). A tracker that cannot log in yet (`Tracker::setup_message`, such as an
//! OAuth client missing from the build and from `config.toml`) has no tab; its Settings row says why. Each edit is written to that tracker at once (`actions`),
//! and a failure shows a toast. The entry is named by [`open_sheet`] before the page is pushed, and the search page reads
//! the same entry and the tracker its Search button named. A bind, an edit or a stop tells the details page, so its
//! Tracking button follows. Under the tabs, whichever one is shown, the page ends with the entry's community score (`community_page`).

use crate::ui::DialogOpen as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Label, ListBox, Orientation as O, glib};
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_track::bind::{bind_anime, bind_manga};
use kuro_track::ctx::TrackCtx;
use kuro_track::error::TrackError;
use kuro_track::login_rows::LOGIN_ROWS;
use kuro_track::messages::english;
use kuro_track::model::search::{AnimeTrackSearch, MangaTrackSearch};
use kuro_platform::kind::Kind;
use kuro_track::model::{AnimeTrack, MangaTrack, TrackerId};
use kuro_track::picker::{StatusRow, anime_status_rows, manga_status_rows, picker_day, picker_today, remove_offered};
use kuro_track::tracker::Tracker;
use kuro_track::write::{AnimeEdit, MangaEdit};
use tokio::task::JoinHandle;

use super::writes::{self, Edit};
use super::{actions, dates, pickers, remove};
use crate::screens::{built, unavailable, Built};
use crate::settings_ui::tracker_host::{self, now_ms};
use crate::settings_ui::tracking;
use crate::ui;
use crate::theme::tokens::{SPACE_M};
use crate::theme::tokens::{SPACE_S};

/// The progress spin button writes this long after its last step, so a run of clicks is one write.
const PROGRESS_SETTLE_MS: u64 = 700;

/// The title the tracking page is about.
#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub id: i64,
    pub title: String,
}

thread_local! {
    static ENTRY: RefCell<Option<Entry>> = const { RefCell::new(None) };
    /// What the details page runs when the binding changed: it reads its tracking count again.
    static ON_CHANGE: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    /// The tab the page shows, kept across its redraws: the one it opened on, then each one the user picks.
    static TAB: Cell<Option<TrackerId>> = const { Cell::new(None) };
    /// The tracker the search page searches: the one whose Search button opened it.
    static SEARCH_TRACKER: Cell<TrackerId> = const { Cell::new(TrackerId::ANILIST) };
    /// The search page's "Track privately" switch: a result tapped while it is on is bound privately.
    static SEARCH_PRIVATE: Cell<bool> = const { Cell::new(false) };
}

fn search_tracker() -> TrackerId {
    SEARCH_TRACKER.with(Cell::get)
}

/// The texts of one tracker's tab that are a sentence of their own with the tracker's name in it.
struct Texts {
    search: MsgKey,
    open: MsgKey,
}

fn texts(tracker: TrackerId) -> Texts {
    match tracker {
        TrackerId::MYANIMELIST => Texts { search: MsgKey::ui_search_myanimelist, open: MsgKey::ui_open_on_myanimelist },
        TrackerId::SHIKIMORI => Texts { search: MsgKey::ui_search_shikimori, open: MsgKey::ui_open_on_shikimori },
        _ => Texts { search: MsgKey::ui_search_anilist, open: MsgKey::ui_open_on_anilist },
    }
}

/// What one tracker's tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    /// The entry's rows there.
    Track,
    /// The Connect row.
    Connect,
    /// No tab: the tracker cannot log in yet, and its Settings row says why.
    Hidden,
}

/// The section of a tracker that is logged in or not, and ready to log in or not (`Tracker::setup_message`). Before the
/// services are built every tracker counts as ready, so the page shows its tabs at once.
fn section(logged_in: bool, ready: bool) -> Section {
    match (logged_in, ready) {
        (true, _) => Section::Track,
        (false, true) => Section::Connect,
        (false, false) => Section::Hidden,
    }
}

/// Names the entry, then pushes the tracking page for it. `on_change` runs after each bind, edit or stop.
pub fn open_sheet(entry: Entry, on_change: impl Fn() + 'static) {
    ENTRY.with(|slot| *slot.borrow_mut() = Some(entry));
    TAB.with(|tab| tab.set(None));
    ON_CHANGE.with(|slot| *slot.borrow_mut() = Some(Rc::new(on_change)));
    ui::open("tracking_sheet");
}

pub(super) fn entry() -> Option<Entry> {
    ENTRY.with(|slot| slot.borrow().clone())
}

/// Tells the details page that the binding changed. The callback is taken out of the slot before it runs.
pub(super) fn changed() {
    let on_change = ON_CHANGE.with(|slot| slot.borrow().clone());
    if let Some(on_change) = on_change {
        on_change();
    }
}

/// This entry's binding on one tracker.
#[derive(Clone)]
enum Bound {
    Anime(AnimeTrack),
    Manga(MangaTrack),
}

/// A tracker search result of the entry's kind.
#[derive(Clone)]
enum Found {
    Anime(AnimeTrackSearch),
    Manga(MangaTrackSearch),
}

impl Found {
    fn title(&self) -> &str {
        match self {
            Found::Anime(hit) => &hit.title,
            Found::Manga(hit) => &hit.title,
        }
    }
}

/// The tracking page: one tab per switched-on tracker, with the entry's rows there or the Connect row while it is not
/// logged in.
pub fn sheet() -> Option<Built> {
    let root = ui::column();
    // the services finish building after the page is shown, and a login or a bind redraws it through the same closure
    tracker_host::set_refresh(redraw(&root));
    render(&root);
    built(ui::scrolled(&root), vec![])
}

/// The search page: the search of the tracker that opened it, for the entry's title, already run. Tapping a result binds
/// the entry to it and returns to the tracking page.
pub fn search() -> Option<Built> {
    let Some(entry) = entry() else { return unavailable("tracker_search") };
    let column = ui::column();
    let field = gtk::SearchEntry::new();
    field.set_placeholder_text(Some(&tr(texts(search_tracker()).search)));
    field.set_text(&entry.title);
    let status = ui::dim("");
    let results = GBox::new(O::Vertical, 0);
    column.append(&field);
    SEARCH_PRIVATE.with(|on| on.set(false));
    let supports_private = tracker_host::host().and_then(|host| host.manager.get(search_tracker())).is_some_and(|t| t.supports_private_tracking());
    if supports_private {
        let row = crate::widgets::row::switch(&tr(MsgKey::action_toggle_private_on), None, None, false);
        row.connect_active_notify(|row| SEARCH_PRIVATE.with(|on| on.set(row.is_active())));
        let list = ui::boxed_list();
        list.append(&row);
        column.append(&list);
    }
    column.append(&status);
    column.append(&results);

    let generation = Rc::new(Cell::new(0_u64));
    let (typed, shown_status, shown_results, typed_generation) = (entry.clone(), status.clone(), results.clone(), Rc::clone(&generation));
    field.connect_activate(move |entry_field| run_search(&typed, vec![entry_field.text().to_string()], &shown_status, &shown_results, &typed_generation));
    run_search(&entry, kuro_track::auto_bind::title_queries(&entry.title), &status, &results, &generation);
    built(ui::scrolled(&column), vec![])
}

/// A closure that redraws `root` with [`render`] while the page is alive.
fn redraw(root: &GBox) -> Rc<dyn Fn()> {
    let weak = root.downgrade();
    Rc::new(move || {
        if let Some(root) = weak.upgrade() {
            render(&root);
        }
    })
}

pub(super) fn clear(root: &GBox) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
}

fn say(label: &Label, text: &str) {
    label.set_text(text);
    label.set_visible(!text.is_empty());
}

/// The spinner of a search that is running, on either search page.
pub(super) fn searching() -> gtk::Widget {
    crate::widgets::status::loading(Some("Searching…"))
}

/// What one tracker's tab holds before the entry's binding there is read.
enum Body {
    /// The entry's rows there: the tracker is logged in.
    Track(Arc<dyn Tracker>),
    /// The Connect row; `true` while a browser login waits for its redirect.
    Connect(bool),
}

/// One tab of the page: a switched-on tracker that is logged in or can log in.
struct Plan {
    id: TrackerId,
    name: String,
    body: Body,
}

/// The tab of `id`, or none while the tracker cannot log in. Before the services are built there is no tracker to ask,
/// so the name is the one of its login row.
fn plan(host: Option<&tracker_host::Host>, id: TrackerId) -> Option<Plan> {
    let tracker = host.and_then(|host| host.manager.get(id));
    let logged_in = tracker.as_ref().is_some_and(|tracker| tracker.is_logged_in());
    let ready = tracker.as_ref().is_none_or(|tracker| tracker.setup_message().is_none());
    let name = match &tracker {
        Some(tracker) => tracker.name().to_owned(),
        None => LOGIN_ROWS.iter().find(|row| row.tracker == id).map(|row| row.name).unwrap_or_default().to_owned(),
    };
    let body = match (section(logged_in, ready), tracker) {
        (Section::Hidden, _) => return None,
        (Section::Track, Some(tracker)) => Body::Track(tracker),
        _ => Body::Connect(host.is_some_and(|host| host.is_waiting(id, now_ms()))),
    };
    Some(Plan { id, name, body })
}

/// The entry's binding on one tracker as it was read, or why the read failed.
type Read = (TrackerId, Result<Option<Bound>, TrackError>);

/// What the choice of the tab to show knows about one tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tab {
    id: TrackerId,
    /// The tracker is logged in.
    connected: bool,
    /// The entry is tracked there.
    tracked: bool,
}

/// The tab the page shows: the kept one (`TAB`) while it is still there; else the first that tracks the entry, else the
/// first that is logged in, else the first.
fn opening_tab(tabs: &[Tab], kept: Option<TrackerId>) -> Option<TrackerId> {
    let kept = tabs.iter().find(|tab| Some(tab.id) == kept);
    let tracked = || tabs.iter().find(|tab| tab.tracked);
    let connected = || tabs.iter().find(|tab| tab.connected);
    kept.or_else(tracked).or_else(connected).or_else(|| tabs.first()).map(|tab| tab.id)
}

/// The name of a tab's page in its stack.
fn page_name(id: TrackerId) -> String {
    id.0.to_string()
}

/// The tab bar over one tab per switched-on tracker that can log in, then the entry's community score, which needs no
/// login and stays under whichever tab is shown. The bindings on the logged-in trackers are read off the GTK thread in
/// one go, so the page knows which tab to open on before it shows any.
fn render(root: &GBox) {
    clear(root);
    let host = tracker_host::host();
    let plans: Vec<Plan> = tracking::ENABLED.into_iter().filter_map(|id| plan(host.as_deref(), id)).collect();
    let slot = GBox::new(O::Vertical, SPACE_M);
    slot.set_visible(!plans.is_empty());
    root.append(&slot);
    let entry = entry();
    if let Some(entry) = &entry {
        super::community_page::section(root, entry);
    }
    let again = redraw(root);
    let wanted: Vec<TrackerId> = plans.iter().filter(|plan| matches!(plan.body, Body::Track(_))).map(|plan| plan.id).collect();
    let (Some(entry), Some(host)) = (entry, host) else {
        // the services are not built yet, so nothing is known about the trackers: the tab shown now is not kept
        show_tabs(&slot, None, plans, Vec::new(), again, false);
        return;
    };
    if wanted.is_empty() {
        show_tabs(&slot, Some(&entry), plans, Vec::new(), again, true);
        return;
    }
    slot.append(&crate::widgets::status::loading(None));
    let (weak, ctx, id, kind) = (slot.downgrade(), Arc::clone(&host.ctx), entry.id, entry.kind);
    crate::background::run_async(
        async move {
            let mut read: Vec<Read> = Vec::with_capacity(wanted.len());
            for tracker in wanted {
                let found = match kind {
                    Kind::Anime => ctx.anime_binding(id, tracker).await.map(|found| found.map(Bound::Anime)),
                    Kind::Manga => ctx.manga_binding(id, tracker).await.map(|found| found.map(Bound::Manga)),
                };
                read.push((tracker, found));
            }
            read
        },
        move |read| {
            let Some(slot) = weak.upgrade() else { return };
            clear(&slot);
            show_tabs(&slot, Some(&entry), plans, read, again, true);
        },
    );
}

/// Fills `slot` with the tab bar and the tabs: a tab's name is its tracker's, and a dot marks the tab of each tracker
/// that tracks the entry. The tab shown is [`opening_tab`]'s; it is kept once the trackers are known (`settled`), and
/// so is each tab the user picks after it, so a redraw comes back to the same tab.
fn show_tabs(slot: &GBox, entry: Option<&Entry>, plans: Vec<Plan>, mut read: Vec<Read>, again: Rc<dyn Fn()>, settled: bool) {
    if plans.is_empty() {
        return;
    }
    let stack = gtk::Stack::new();
    // each tab is as tall as its own rows, so the community score sits right under the one shown
    stack.set_vhomogeneous(false);
    let mut tabs = Vec::with_capacity(plans.len());
    for plan in plans {
        let body = GBox::new(O::Vertical, SPACE_M);
        let (connected, mut tracked) = (matches!(plan.body, Body::Track(_)), false);
        match plan.body {
            Body::Connect(waiting) => body.append(&connect_list(plan.id, &plan.name, waiting, Rc::clone(&again))),
            Body::Track(tracker) => {
                let at = read.iter().position(|(id, _)| *id == plan.id);
                let found = at.map(|at| read.swap_remove(at).1);
                match (found, entry) {
                    (Some(Ok(bound)), Some(entry)) => {
                        tracked = bound.is_some();
                        body.append(&tracker_list(entry, tracker.as_ref(), bound, Rc::clone(&again)));
                    }
                    (Some(Err(err)), _) => {
                        let retry = Rc::clone(&again);
                        let failed = crate::widgets::status::error(&english(&kuro_track::dialog::failure_message(&err)), None, Some(move || retry()));
                        failed.add_css_class("compact");
                        body.append(&failed);
                    }
                    _ => {}
                }
            }
        }
        let page = stack.add_titled(&body, Some(&page_name(plan.id)), &plan.name);
        page.set_needs_attention(tracked);
        tabs.push(Tab { id: plan.id, connected, tracked });
    }
    let shown = opening_tab(&tabs, TAB.with(Cell::get));
    if let Some(id) = shown {
        stack.set_visible_child_name(&page_name(id));
        if settled {
            TAB.with(|tab| tab.set(Some(id)));
        }
    }
    // connected after the first tab is shown, so only a later switch reaches it
    stack.connect_visible_child_name_notify(|stack| {
        // a switch counts while the stack is on a page: one that is being dropped says nothing about the user
        if stack.root().is_none() {
            return;
        }
        if let Some(picked) = stack.visible_child_name().and_then(|name| name.parse::<i64>().ok()) {
            TAB.with(|tab| tab.set(Some(TrackerId(picked))));
        }
    });
    slot.append(&gtk::StackSwitcher::builder().stack(&stack).build());
    slot.append(&stack);
}

/// A tracker that is not logged in: one row that says so, with a plain Connect button. While a browser login waits for
/// its redirect, the row says that under it.
fn connect_list(tracker: TrackerId, name: &str, waiting: bool, changed: Rc<dyn Fn()>) -> ListBox {
    let connect = Button::with_label(&tr(MsgKey::ui_connect));
    connect.set_valign(Align::Center);
    connect.connect_clicked(move |_| tracking::connect(tracker, Rc::clone(&changed)));
    let waiting = waiting.then(tracking::waiting_text);
    let list = ui::boxed_list();
    list.append(&ui::row(&trf(MsgKey::ui_not_connected_to, &[("tracker", Arg::Str(name))]), waiting.as_deref(), Some(connect.upcast::<gtk::Widget>())));
    list
}

/// One tracker's rows: the title the entry is tracked as over the edit rows, or a search button while it is not tracked.
fn tracker_list(entry: &Entry, tracker: &dyn Tracker, bound: Option<Bound>, redraw: Rc<dyn Fn()>) -> ListBox {
    let list = ui::boxed_list();
    match bound {
        Some(bound) => {
            let tracked_as = match &bound {
                Bound::Anime(track) => track.title.as_str(),
                Bound::Manga(track) => track.title.as_str(),
            };
            // a binding stored without its title shows the entry's own
            let title = if tracked_as.is_empty() { entry.title.as_str() } else { tracked_as };
            list.append(&ui::row(title, Some(&trf(MsgKey::ui_tracked_on, &[("tracker", Arg::Str(tracker.name()))])), None));
            if let Some(shown) = shown(tracker, &bound) {
                edit_rows(&list, entry, shown, bound, redraw);
            }
        }
        None => {
            let search = Button::with_label(&tr(MsgKey::action_search));
            search.set_valign(Align::Center);
            let id = tracker.id();
            search.connect_clicked(move |_| {
                SEARCH_TRACKER.with(|slot| slot.set(id));
                ui::open("tracker_search");
            });
            list.append(&ui::row(&tr(MsgKey::ui_not_tracked_yet), None, Some(search.upcast::<gtk::Widget>())));
        }
    }
    list
}

/// What the edit rows show, read off the binding, the same for both kinds.
struct Shown {
    tracker: TrackerId,
    name: String,
    /// `(status, label)` in the tracker's order.
    statuses: Vec<(i64, String)>,
    status: i64,
    /// The chapter or episode number, truncated as the picker shows it.
    progress: i64,
    /// `0` while the total is unknown.
    total: i64,
    /// The account's score list on the tracker, and the current score as one of its labels.
    scores: Vec<String>,
    score: String,
    start: i64,
    finish: i64,
    /// Whether the tracker keeps the start and finish dates (Shikimori does not, so it gets no date rows).
    dates: bool,
    url: String,
    deletable: bool,
    /// Whether the entry is hidden from the public list there, or `None` when the tracker has no private tracking.
    private: Option<bool>,
}

/// The text of a status label key, from the app's catalogue.
fn status_text(key: Option<&'static str>) -> String {
    key.map(|key| MsgKey::from_id(key).map_or_else(|| key.to_owned(), tr)).unwrap_or_default()
}

fn shown(tracker: &dyn Tracker, bound: &Bound) -> Option<Shown> {
    let labels = |rows: Vec<StatusRow>| -> Vec<(i64, String)> { rows.into_iter().map(|row| (row.status, status_text(row.label_key))).collect() };
    let scores = tracker.score_list().to_vec();
    Some(match bound {
        Bound::Anime(track) => {
            let api = tracker.as_anime()?;
            Shown {
                tracker: tracker.id(),
                name: tracker.name().to_owned(),
                statuses: labels(anime_status_rows(api)),
                status: track.status,
                progress: pickers::progress_picker(track.total_episodes, track.last_episode_seen).value(),
                total: track.total_episodes,
                score: api.display_score(track),
                scores,
                start: track.start_date,
                finish: track.finish_date,
                dates: tracker.supports_reading_dates(),
                url: track.remote_url.clone(),
                deletable: tracker.as_deletable_anime().is_some(),
                private: tracker.supports_private_tracking().then_some(track.private),
            }
        }
        Bound::Manga(track) => {
            let api = tracker.as_manga()?;
            Shown {
                tracker: tracker.id(),
                name: tracker.name().to_owned(),
                statuses: labels(manga_status_rows(api)),
                status: track.status,
                progress: pickers::progress_picker(track.total_chapters, track.last_chapter_read).value(),
                total: track.total_chapters,
                score: api.display_score(track),
                scores,
                start: track.start_date,
                finish: track.finish_date,
                dates: tracker.supports_reading_dates(),
                url: track.remote_url.clone(),
                deletable: tracker.as_deletable_manga().is_some(),
                private: tracker.supports_private_tracking().then_some(track.private),
            }
        }
    })
}

/// The edit rows of a tracked entry. Every control is filled before its handler is connected, so drawing writes nothing.
fn edit_rows(list: &ListBox, entry: &Entry, shown: Shown, bound: Bound, redraw: Rc<dyn Fn()>) {
    let write: Rc<dyn Fn(Edit)> = {
        let (entry, tracker, redraw) = (entry.clone(), shown.tracker, Rc::clone(&redraw));
        Rc::new(move |edit: Edit| write_edit(&entry, tracker, edit, Rc::clone(&redraw)))
    };

    let names: Vec<&str> = shown.statuses.iter().map(|(_, label)| label.as_str()).collect();
    let status = gtk::DropDown::from_strings(&names);
    if let Some(at) = shown.statuses.iter().position(|(s, _)| *s == shown.status) {
        status.set_selected(at as u32);
    }
    status.connect_selected_notify({
        let (write, statuses): (_, Vec<i64>) = (Rc::clone(&write), shown.statuses.iter().map(|(s, _)| *s).collect());
        move |drop_down| {
            if let Some(&status) = statuses.get(drop_down.selected() as usize) {
                write(Edit::Status(status));
            }
        }
    });
    list.append(&ui::row(&tr(MsgKey::status), None, Some(status.upcast())));

    list.append(&progress_row(entry.kind, &shown, Rc::clone(&write)));

    let names: Vec<&str> = shown.scores.iter().map(String::as_str).collect();
    let score = gtk::DropDown::from_strings(&names);
    score.set_selected(pickers::score_index(&shown.scores, &shown.score).map_or(gtk::INVALID_LIST_POSITION, |at| at as u32));
    score.connect_selected_notify({
        let (write, scores) = (Rc::clone(&write), shown.scores.clone());
        move |drop_down| {
            if let Some(score) = scores.get(drop_down.selected() as usize) {
                write(Edit::Score(score.clone()));
            }
        }
    });
    list.append(&ui::row(&tr(MsgKey::ui_your_score), None, Some(score.upcast())));

    if shown.dates {
        list.append(&date_row(true, shown.start, shown.finish, Rc::clone(&write)));
        list.append(&date_row(false, shown.finish, shown.start, Rc::clone(&write)));
    }

    if let Some(private) = shown.private {
        let row = crate::widgets::row::switch(&tr(MsgKey::action_toggle_private_on), None, None, private);
        row.connect_active_notify(move |row| write(Edit::Private(row.is_active())));
        list.append(&row);
    }

    if !shown.url.is_empty() {
        let url = shown.url.clone();
        list.append(&button_row(&tr(texts(shown.tracker).open), &["flat"], move || crate::ui::open_link(&url, None)));
    }
    let (deletable, name) = (shown.deletable, shown.name);
    list.append(&button_row(&tr(MsgKey::ui_stop_tracking), &["flat", "destructive-action"], move || confirm_stop(&name, &bound, deletable, Rc::clone(&redraw))));
}

/// "Episodes" or "Chapters": a spin button over `0..=total` (10000 while the total is unknown) and "of N" beside it.
fn progress_row(kind: Kind, shown: &Shown, write: Rc<dyn Fn(Edit)>) -> adw::ActionRow {
    let spin = gtk::SpinButton::with_range(0.0, pickers::progress_max(shown.total) as f64, 1.0);
    spin.set_digits(0);
    spin.set_value(shown.progress as f64);
    let pending = Rc::new(Cell::new(0_u32));
    spin.connect_value_changed(move |spin| {
        let token = pending.get().wrapping_add(1);
        pending.set(token);
        let (pending, write, value) = (Rc::clone(&pending), Rc::clone(&write), spin.value_as_int());
        glib::timeout_add_local_once(Duration::from_millis(PROGRESS_SETTLE_MS), move || {
            if pending.get() == token {
                write(Edit::Progress(i64::from(value)));
            }
        });
    });
    let total = if shown.total > 0 { shown.total.to_string() } else { "?".to_owned() };
    let suffix = GBox::new(O::Horizontal, SPACE_S);
    suffix.append(&spin);
    suffix.append(&Label::new(Some(&format!("of {total}"))));
    let title = tr(match kind {
        Kind::Anime => MsgKey::episodes,
        Kind::Manga => MsgKey::chapters,
    });
    ui::row(&title, None, Some(suffix.upcast()))
}

/// The start or finish date: a button with the day, whose popover holds a calendar, OK and, for a set date, Remove.
/// A day in the future, a start after the finish or a finish before the start is refused with a toast.
fn date_row(is_start: bool, stored: i64, other: i64, write: Rc<dyn Fn(Edit)>) -> adw::ActionRow {
    let zone = chrono::Local;
    let text = picker_day(stored, &zone).map_or_else(|| tr(MsgKey::ui_not_set).to_owned(), |day| day.format("%Y-%m-%d").to_string());
    let button = gtk::MenuButton::builder().label(text).build();
    let popover = gtk::Popover::new();
    let calendar = gtk::Calendar::new();
    let opened = dates::initial_day(stored, now_ms(), &zone);
    if let Ok(day) = glib::DateTime::from_local(opened.year(), opened.month() as i32, opened.day() as i32, 0, 0, 0.0) {
        calendar.select_day(&day);
    }
    let buttons = GBox::new(O::Horizontal, SPACE_S);
    buttons.set_halign(Align::End);
    if remove_offered(stored) {
        let clear = Button::with_label(&tr(MsgKey::action_remove));
        clear.connect_clicked({
            let (write, popover) = (Rc::clone(&write), popover.clone());
            move |_| {
                popover.popdown();
                write(writes::date_edit(is_start, writes::REMOVE_DATE));
            }
        });
        buttons.append(&clear);
    }
    let ok = Button::builder().label(tr(MsgKey::action_ok)).css_classes(["suggested-action"]).build();
    ok.connect_clicked({
        let (calendar, popover) = (calendar.clone(), popover.clone());
        move |_| {
            let picked = calendar.date();
            let Some(day) = NaiveDate::from_ymd_opt(picked.year(), picked.month() as u32, picked.day_of_month() as u32) else { return };
            let zone = chrono::Local;
            if !dates::date_selectable(is_start, day, picker_day(other, &zone), picker_today(now_ms(), &zone)) {
                crate::toast::notify(if is_start { tr(MsgKey::ui_the_start_date_can_t_be_in_the_future_or_aft) } else { tr(MsgKey::ui_the_finish_date_can_t_be_in_the_future_or_be) });
                return;
            }
            popover.popdown();
            write(writes::date_edit(is_start, dates::stored_ms(day, &zone)));
        }
    });
    buttons.append(&ok);
    let content = GBox::new(O::Vertical, SPACE_S);
    ui::margins(&content, 6);
    content.append(&calendar);
    content.append(&buttons);
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    let title = tr(if is_start { MsgKey::track_started_reading_date } else { MsgKey::track_finished_reading_date });
    ui::row(&title, None, Some(button.upcast()))
}

/// A row that is one flat button across the list, the way GNOME lists show an action.
pub(super) fn button_row(label: &str, css: &[&str], run: impl Fn() + 'static) -> gtk::ListBoxRow {
    let button = Button::builder().label(label).css_classes(css.to_vec()).build();
    button.connect_clicked(move |_| run());
    gtk::ListBoxRow::builder().child(&button).activatable(false).build()
}

/// Stop tracking asks first. The local unlink always runs; for a tracker that can delete, a checkbox (off by default)
/// also removes the entry from the tracker's site.
fn confirm_stop(name: &str, bound: &Bound, deletable: bool, redraw: Rc<dyn Fn()>) {
    let column = GBox::new(O::Vertical, SPACE_M);
    column.append(&Label::builder().label(tr(MsgKey::track_delete_text)).wrap(true).xalign(0.0).build());
    let also = gtk::CheckButton::builder().label(trf(MsgKey::track_delete_remote_text, &[("arg1", Arg::Str(name))])).active(remove::CHECKBOX_DEFAULT).build();
    also.set_visible(remove::checkbox_shown(deletable));
    column.append(&also);
    let bound = bound.clone();
    let buttons = vec![
        ui::DialogButton::cancel(tr(MsgKey::action_cancel)),
        ui::DialogButton::new(&tr(MsgKey::ui_stop_tracking), ui::Role::Destructive, move || {
            let also_remote = remove::remote_delete(deletable, also.is_active());
            match &bound {
                Bound::Anime(track) => finish(actions::remove_anime(track.clone(), also_remote), |key| english(&key), Rc::clone(&redraw)),
                Bound::Manga(track) => finish(actions::remove_manga(track.clone(), also_remote), |key| english(&key), Rc::clone(&redraw)),
            }
        }),
    ];
    ui::dialog(&trf(MsgKey::track_delete_title, &[("arg1", Arg::Str(name))]), &column, 400, buttons).open();
}

/// Writes one edit to `tracker` at once. A success tells the details page and redraws the rows; a failure is a toast.
fn write_edit(entry: &Entry, tracker: TrackerId, edit: Edit, redraw: Rc<dyn Fn()>) {
    let failure = |err: TrackError| english(&kuro_track::dialog::failure_message(&err));
    match entry.kind {
        Kind::Anime => {
            let edit = match edit {
                Edit::Status(status) => AnimeEdit::Status(status),
                Edit::Progress(n) => AnimeEdit::Progress(i32::try_from(n).unwrap_or(i32::MAX)),
                Edit::Score(label) => AnimeEdit::Score(label),
                Edit::StartDate(ms) => AnimeEdit::StartDate(ms),
                Edit::FinishDate(ms) => AnimeEdit::FinishDate(ms),
                Edit::Private(on) => AnimeEdit::Private(on),
            };
            finish(actions::edit_anime(entry.id, tracker, edit), failure, redraw);
        }
        Kind::Manga => {
            let edit = match edit {
                Edit::Status(status) => MangaEdit::Status(status),
                Edit::Progress(n) => MangaEdit::Progress(i32::try_from(n).unwrap_or(i32::MAX)),
                Edit::Score(label) => MangaEdit::Score(label),
                Edit::StartDate(ms) => MangaEdit::StartDate(ms),
                Edit::FinishDate(ms) => MangaEdit::FinishDate(ms),
                Edit::Private(on) => MangaEdit::Private(on),
            };
            finish(actions::edit_manga(entry.id, tracker, edit), failure, redraw);
        }
    }
}

/// Waits for a detached tracking job off the GTK thread. A success tells the details page and redraws the page; a
/// failure shows `failure`'s text as a toast, and the rows stay as they were. No job means the services are not up.
fn finish<T: Send + 'static, E: Send + 'static>(job: Option<JoinHandle<Result<T, E>>>, failure: impl FnOnce(E) -> String + 'static, redraw: Rc<dyn Fn()>) {
    let Some(job) = job else {
        crate::toast::notify(tracking::not_ready());
        return;
    };
    crate::background::run_async(async move { job.await }, move |result| match result {
        Ok(Ok(_)) => {
            changed();
            redraw();
        }
        Ok(Err(err)) => crate::toast::notify(failure(err)),
        Err(err) => crate::toast::notify(err.to_string()),
    });
}

/// Runs the search page's tracker search and shows its results: `queries` in order, until one finds something. Only the
/// latest search may show anything.
fn run_search(entry: &Entry, queries: Vec<String>, status: &Label, results: &GBox, generation: &Rc<Cell<u64>>) {
    let Some(host) = tracker_host::host() else {
        say(status, &tracking::not_ready());
        return;
    };
    let Some(tracker) = host.manager.get(search_tracker()) else { return };
    let queries: Vec<String> = queries.into_iter().filter(|query| !query.trim().is_empty()).collect();
    if queries.is_empty() {
        say(status, "");
        return;
    }
    let token = generation.get() + 1;
    generation.set(token);
    say(status, "");
    clear(results);
    results.append(&searching());
    let (kind, latest, shown_status, shown_results, entry, again) = (entry.kind, Rc::clone(generation), status.clone(), results.clone(), entry.clone(), (entry.clone(), queries.clone(), Rc::clone(generation)));
    crate::background::run_async(
        async move { search_first(tracker.as_ref(), kind, &queries).await },
        move |result| {
            if latest.get() != token {
                return;
            }
            clear(&shown_results);
            match result {
                Ok(found) if found.is_empty() => shown_results.append(&crate::widgets::status::empty("system-search-symbolic", &kuro_i18n::tr(kuro_i18n::MsgKey::no_results_found), None)),
                Ok(found) => shown_results.append(&hits_list(&entry, found)),
                Err(err) => {
                    let (shown, status, results) = (again.clone(), shown_status.clone(), shown_results.clone());
                    shown_results.append(&crate::widgets::status::error(
                        &english(&kuro_track::dialog::failure_message(&err)),
                        None,
                        Some(move || run_search(&shown.0, shown.1.clone(), &status, &results, &shown.2)),
                    ));
                }
            }
        },
    );
}

/// The hits of the first of `queries` that finds any. A failed search stops there: the error is shown with its Retry.
async fn search_first(tracker: &dyn Tracker, kind: Kind, queries: &[String]) -> Result<Vec<Found>, TrackError> {
    for query in queries {
        let hits = search_hits(tracker, kind, query).await?;
        if !hits.is_empty() {
            return Ok(hits);
        }
    }
    Ok(Vec::new())
}

async fn search_hits(tracker: &dyn Tracker, kind: Kind, query: &str) -> Result<Vec<Found>, TrackError> {
    Ok(match kind {
        Kind::Anime => match tracker.as_anime() {
            Some(api) => api.search(query).await?.into_iter().map(Found::Anime).collect(),
            None => Vec::new(),
        },
        Kind::Manga => match tracker.as_manga() {
            Some(api) => api.search(query).await?.into_iter().map(Found::Manga).collect(),
            None => Vec::new(),
        },
    })
}

/// One row per result. Tapping a row binds the entry to that result.
fn hits_list(entry: &Entry, found: Vec<Found>) -> ListBox {
    let list = ui::boxed_list();
    for hit in &found {
        // a plain row is not activatable on its own, and the list's activation below is what binds
        let row = ui::row(hit.title(), None, None);
        row.set_activatable(true);
        list.append(&row);
    }
    let entry = entry.clone();
    list.connect_row_activated(move |_, row| {
        if let Some(hit) = usize::try_from(row.index()).ok().and_then(|index| found.get(index)) {
            bind(entry.clone(), hit.clone());
        }
    });
    list
}

/// Binds the entry to `hit` on the search page's tracker, then returns to the tracking page, which shows the new binding.
fn bind(entry: Entry, hit: Found) {
    let Some(ctx) = tracker_host::ctx() else {
        crate::toast::notify(tracking::not_ready());
        return;
    };
    let (tracker, private) = (search_tracker(), SEARCH_PRIVATE.with(Cell::get));
    crate::background::run_async(
        async move { bind_hit(&ctx, tracker, &entry, hit, private).await },
        |result| match result {
            Ok(()) => {
                ui::pop();
                tracker_host::refresh();
                changed();
            }
            Err(err) => crate::toast::notify(english(&kuro_track::dialog::failure_message(&err))),
        },
    );
}

/// The bind of the add flow for one result: the remote bind, the stored row and the progress sync (`bind_anime`).
/// `private` is kept only by a tracker with private tracking.
async fn bind_hit(ctx: &TrackCtx, id: TrackerId, entry: &Entry, hit: Found, private: bool) -> Result<(), TrackError> {
    let tracker = ctx.trackers.get(id).ok_or(TrackError::UnknownTracker(id.0))?;
    let private = private && tracker.supports_private_tracking();
    match hit {
        Found::Anime(mut hit) => {
            let api = tracker.as_anime().ok_or(TrackError::UnknownTracker(id.0))?;
            hit.anime_id = entry.id;
            hit.private = private;
            bind_anime(ctx, entry.id, api, hit.to_track(), now_ms()).await?;
        }
        Found::Manga(mut hit) => {
            let api = tracker.as_manga().ok_or(TrackError::UnknownTracker(id.0))?;
            hit.manga_id = entry.id;
            hit.private = private;
            bind_manga(ctx, entry.id, api, hit.to_track(), now_ms()).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_label_key_reads_as_its_catalogue_text() {
        assert_eq!(status_text(Some("watching")), "Watching");
        assert_eq!(status_text(Some("plan_to_read")), "Plan to read");
        assert_eq!(status_text(Some("no_such_key_here")), "no_such_key_here", "an unknown key shows as it is");
        assert_eq!(status_text(None), "");
    }

    #[test]
    fn search_results_can_be_tapped() {
        crate::testing::with_display(|| {
            let entry = Entry { kind: Kind::Anime, id: 1, title: "Frieren".into() };
            let found = vec![Found::Anime(AnimeTrackSearch::new(TrackerId::ANILIST, 1, "Frieren", ""))];
            let list = hits_list(&entry, found);
            let row = list.row_at_index(0).expect("one row per result");
            assert!(row.is_activatable(), "a tap on a result binds it");
        });
    }

    #[test]
    fn each_tracker_has_its_own_texts() {
        let shikimori = texts(TrackerId::SHIKIMORI);
        assert_eq!(tr(shikimori.search), "Search Shikimori");
        assert_eq!(tr(shikimori.open), "Open on Shikimori");
        assert_eq!(tr(texts(TrackerId::MYANIMELIST).open), "Open on MyAnimeList");
        assert_eq!(tr(texts(TrackerId::ANILIST).search), "Search AniList");
        let named = |key| trf(key, &[("tracker", Arg::Str("Shikimori"))]);
        assert_eq!(named(MsgKey::ui_not_connected_to), "Not connected to Shikimori");
        assert_eq!(named(MsgKey::ui_tracked_on), "Tracked on Shikimori");
        assert_eq!(tr(MsgKey::ui_connect), "Connect");
        assert_eq!(tr(MsgKey::ui_your_score), "Your score", "beside the community score under the tabs, whose score it is");
    }

    #[test]
    fn the_page_opens_on_the_tracker_that_tracks_the_title() {
        let (anilist, mal, shikimori) = (TrackerId::ANILIST, TrackerId::MYANIMELIST, TrackerId::SHIKIMORI);
        let tab = |id, connected, tracked| Tab { id, connected, tracked };
        let tabs = [tab(anilist, false, false), tab(mal, true, false), tab(shikimori, true, true)];
        assert_eq!(opening_tab(&tabs, None), Some(shikimori), "the first that tracks the title");
        assert_eq!(opening_tab(&tabs, Some(anilist)), Some(anilist), "the kept tab wins: a redraw stays where the user is");
        assert_eq!(opening_tab(&tabs[..2], None), Some(mal), "nothing tracks it: the first that is logged in");
        assert_eq!(opening_tab(&tabs[..2], Some(shikimori)), Some(mal), "a kept tab that is gone is not kept");
        assert_eq!(opening_tab(&tabs[..1], None), Some(anilist), "nothing is logged in: the first tab");
        assert_eq!(opening_tab(&[], Some(anilist)), None, "no tracker can log in: no tab");
    }

    #[test]
    fn every_switched_on_tracker_has_a_name_before_the_services_are_built() {
        for id in tracking::ENABLED {
            let tab = plan(None, id).expect("with no tracker to ask, each one counts as ready");
            assert!(!tab.name.is_empty(), "{id:?}");
            assert!(matches!(tab.body, Body::Connect(false)), "{id:?}");
            assert_eq!(page_name(id).parse::<i64>().ok().map(TrackerId), Some(id), "a tab's page names its tracker");
        }
    }

    #[test]
    fn a_tracker_that_cannot_log_in_yet_has_no_section() {
        assert_eq!(section(true, true), Section::Track);
        assert_eq!(section(true, false), Section::Track, "a stored login still shows its rows");
        assert_eq!(section(false, true), Section::Connect);
        assert_eq!(section(false, false), Section::Hidden);
    }

    #[test]
    fn shikimori_without_its_client_is_not_ready_and_gets_no_date_rows() {
        use kuro_track::shikimori::api::{Endpoints, OAuthClient};
        use kuro_track::shikimori::ShikimoriTracker;

        let secrets = || Arc::new(kuro_prefs::memory::MemorySecrets::default()) as Arc<dyn kuro_platform::secrets::SecretStore>;
        let redirect = kuro_track::shikimori::registry::SHIKIMORI_REDIRECT;
        let off = ShikimoriTracker::new(Endpoints::production(), None, redirect, secrets()).unwrap();
        assert!(off.setup_message().is_some(), "no client in config.toml: the section is hidden");
        let client = OAuthClient { client_id: "id".into(), client_secret: "secret".into() };
        let on = ShikimoriTracker::new(Endpoints::production(), Some(client), redirect, secrets()).unwrap();
        assert!(on.setup_message().is_none());

        let row = kuro_data::track::Track::new(Kind::Manga, TrackerId::SHIKIMORI.0);
        let manga = shown(&on, &Bound::Manga(MangaTrack::from_db(&row))).expect("Shikimori tracks manga");
        assert!(!manga.dates, "Shikimori keeps no start or finish date");
        assert!(manga.deletable && manga.private.is_none());
        let row = kuro_data::track::Track::new(Kind::Anime, TrackerId::SHIKIMORI.0);
        assert!(!shown(&on, &Bound::Anime(AnimeTrack::from_db(&row))).expect("and anime").dates);
    }

    #[test]
    fn logged_out_the_page_is_a_tab_bar_over_a_plain_connect_button() {
        crate::testing::with_display(|| {
            let built = sheet().expect("the tracking page");
            let widgets = crate::screens::alone::descendants(&built.content);
            assert_eq!(widgets.iter().filter(|w| w.is::<gtk::StackSwitcher>()).count(), 1, "one tab bar");
            let buttons: Vec<&Button> = widgets.iter().filter_map(|w| w.downcast_ref::<Button>()).collect();
            assert!(buttons.iter().any(|b| b.label().as_deref() == Some("Connect")), "a logged-out tab offers Connect");
            assert!(buttons.iter().all(|b| !b.has_css_class("suggested-action")), "and no button of the page shouts");
        });
    }
}
