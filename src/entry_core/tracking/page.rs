//! The tracking page of an entry (the `tracking_sheet` and `tracker_search` screens, E-25). Tracking keeps the progress on
//! the switched-on trackers (`tracking::ENABLED`: AniList and MyAnimeList), so the page has one section per tracker: a
//! connect button while it is not logged in, a search for the title while the entry is not tracked there, and once it
//! is, its summary over native edit rows (status, progress, score, start and finish dates, Open on the site, Stop
//! tracking). Each edit is written to that tracker at once (`actions`), and a failure shows a toast. The entry is named by
//! [`open_sheet`] before the page is pushed, and the search page reads the same entry and the tracker its Search button
//! named. A bind, an edit or a stop tells the details page, so its Tracking button follows.

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
use kuro_track::messages::english;
use kuro_track::model::search::{AnimeTrackSearch, MangaTrackSearch};
use kuro_platform::kind::Kind;
use kuro_track::model::{AnimeTrack, MangaTrack, TrackerId};
use kuro_track::picker::{StatusRow, anime_status_rows, manga_status_rows, picker_day, picker_today, remove_offered};
use kuro_track::sheet::{progress_text, score_text_shown};
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
    /// The tracker the search page searches: the one whose Search button opened it.
    static SEARCH_TRACKER: Cell<TrackerId> = const { Cell::new(TrackerId::ANILIST) };
    /// The search page's "Track privately" switch: a result tapped while it is on is bound privately.
    static SEARCH_PRIVATE: Cell<bool> = const { Cell::new(false) };
}

fn search_tracker() -> TrackerId {
    SEARCH_TRACKER.with(Cell::get)
}

/// The texts of one tracker's section that name the tracker.
struct Texts {
    intro: MsgKey,
    connect: MsgKey,
    search: MsgKey,
    open: MsgKey,
}

fn texts(tracker: TrackerId) -> Texts {
    if tracker == TrackerId::MYANIMELIST {
        Texts {
            intro: MsgKey::ui_tracking_keeps_your_progress_on_myanimelist,
            connect: MsgKey::ui_connect_myanimelist,
            search: MsgKey::ui_search_myanimelist,
            open: MsgKey::ui_open_on_myanimelist,
        }
    } else {
        Texts {
            intro: MsgKey::ui_tracking_keeps_your_progress_on_anilist,
            connect: MsgKey::ui_connect_anilist,
            search: MsgKey::ui_search_anilist,
            open: MsgKey::ui_open_on_anilist,
        }
    }
}

/// Names the entry, then pushes the tracking page for it. `on_change` runs after each bind, edit or stop.
pub fn open_sheet(entry: Entry, on_change: impl Fn() + 'static) {
    ENTRY.with(|slot| *slot.borrow_mut() = Some(entry));
    ON_CHANGE.with(|slot| *slot.borrow_mut() = Some(Rc::new(on_change)));
    ui::open("tracking_sheet");
}

fn entry() -> Option<Entry> {
    ENTRY.with(|slot| slot.borrow().clone())
}

/// Tells the details page that the binding changed. The callback is taken out of the slot before it runs.
fn changed() {
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

/// The tracking page: per switched-on tracker, the entry's rows there, or the connect button while it is not logged in.
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
    field.connect_activate(move |entry_field| run_search(&typed, entry_field.text().to_string(), &shown_status, &shown_results, &typed_generation));
    run_search(&entry, entry.title.clone(), &status, &results, &generation);
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

fn clear(root: &GBox) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
}

fn say(label: &Label, text: &str) {
    label.set_text(text);
    label.set_visible(!text.is_empty());
}

/// One section per switched-on tracker: connect while it is logged out, the entry's rows there once it is in.
fn render(root: &GBox) {
    clear(root);
    let host = tracker_host::host();
    for id in tracking::ENABLED {
        let tracker = host.as_ref().and_then(|host| host.manager.get(id));
        match (tracker, host.as_ref()) {
            (Some(tracker), Some(host)) if tracker.is_logged_in() => track_section(root, &tracker, Arc::clone(&host.ctx)),
            (_, host) => connect_section(root, id, host.is_some_and(|host| host.is_waiting(id, now_ms()))),
        }
    }
}

/// A tracker that is not logged in: one plain sentence and its connect button.
fn connect_section(root: &GBox, tracker: TrackerId, waiting: bool) {
    let texts = texts(tracker);
    root.append(&Label::builder().label(&tr(texts.intro)).wrap(true).xalign(0.0).build());
    if waiting {
        root.append(&ui::dim(&tracking::waiting_text()));
    }
    let connect = Button::with_label(&tr(texts.connect));
    connect.add_css_class("suggested-action");
    connect.set_halign(Align::Start);
    let changed = redraw(root);
    connect.connect_clicked(move |_| tracking::connect(tracker, Rc::clone(&changed)));
    root.append(&connect);
}

/// The entry's rows on one tracker. The binding is read off the GTK thread, and the rows replace the loading line when
/// they come back.
fn track_section(root: &GBox, tracker: &Arc<dyn Tracker>, ctx: Arc<TrackCtx>) {
    let Some(entry) = entry() else { return };
    let slot = GBox::new(O::Vertical, 0);
    slot.append(&crate::widgets::status::loading(Some(&tr(MsgKey::loading))));
    root.append(&slot);
    let again = redraw(root);
    let (weak, tracker, id, kind) = (slot.downgrade(), Arc::clone(tracker), entry.id, entry.kind);
    let tracker_id = tracker.id();
    crate::source_bridge::call_on_main(
        async move {
            match kind {
                Kind::Anime => ctx.anime_binding(id, tracker_id).await.map(|found| found.map(Bound::Anime)),
                Kind::Manga => ctx.manga_binding(id, tracker_id).await.map(|found| found.map(Bound::Manga)),
            }
        },
        move |result| {
            let Some(slot) = weak.upgrade() else { return };
            clear(&slot);
            match result {
                Ok(bound) => slot.append(&tracker_list(&entry, tracker.as_ref(), bound, again)),
                Err(err) => {
                    let retry = Rc::clone(&again);
                    let page = crate::widgets::status::error(&english(&kuro_track::dialog::failure_message(&err)), None, Some(move || retry()));
                    page.add_css_class("compact");
                    slot.append(&page);
                }
            }
        },
    );
}

/// One tracker's rows: the summary and the edit rows when tracked, or a search button when not.
fn tracker_list(entry: &Entry, tracker: &dyn Tracker, bound: Option<Bound>, redraw: Rc<dyn Fn()>) -> ListBox {
    let list = ui::boxed_list();
    match bound {
        Some(bound) => {
            let text = summary(tracker, &bound);
            list.append(&ui::row(tracker.name(), Some(text.as_str()), None));
            if let Some(shown) = shown(tracker, &bound) {
                edit_rows(&list, entry, shown, bound, redraw);
            }
        }
        None => {
            let search = Button::with_label(&kuro_i18n::tr(kuro_i18n::MsgKey::action_search));
            search.set_valign(Align::Center);
            let id = tracker.id();
            search.connect_clicked(move |_| {
                SEARCH_TRACKER.with(|slot| slot.set(id));
                ui::open("tracker_search");
            });
            list.append(&ui::row(tracker.name(), Some(&tr(MsgKey::ui_not_tracked_yet)), Some(search.upcast::<gtk::Widget>())));
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
    list.append(&ui::row(&tr(MsgKey::score), None, Some(score.upcast())));

    list.append(&date_row(true, shown.start, shown.finish, Rc::clone(&write)));
    list.append(&date_row(false, shown.finish, shown.start, Rc::clone(&write)));

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
fn button_row(label: &str, css: &[&str], run: impl Fn() + 'static) -> gtk::ListBoxRow {
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
    crate::source_bridge::call_on_main(async move { job.await }, move |result| match result {
        Ok(Ok(_)) => {
            changed();
            redraw();
        }
        Ok(Err(err)) => crate::toast::notify(failure(err)),
        Err(err) => crate::toast::notify(err.to_string()),
    });
}

/// The status, progress and score of a tracked entry, on one line. The score is left out while it is 0.
fn summary(tracker: &dyn Tracker, bound: &Bound) -> String {
    let (status, progress, score) = match bound {
        Bound::Anime(track) => {
            let Some(api) = tracker.as_anime() else { return String::new() };
            let score = score_text_shown(track.score).then(|| api.display_score(track));
            (api.status_label(track.status), progress_text(track.last_episode_seen, track.total_episodes), score)
        }
        Bound::Manga(track) => {
            let Some(api) = tracker.as_manga() else { return String::new() };
            let score = score_text_shown(track.score).then(|| api.display_score(track));
            (api.status_label(track.status), progress_text(track.last_chapter_read, track.total_chapters), score)
        }
    };
    [status.map(|key| status_text(Some(key))), Some(progress), score].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

/// Runs the search page's tracker search for `query` and shows its results. Only the latest search may show anything.
fn run_search(entry: &Entry, query: String, status: &Label, results: &GBox, generation: &Rc<Cell<u64>>) {
    let Some(host) = tracker_host::host() else {
        say(status, &tracking::not_ready());
        return;
    };
    let Some(tracker) = host.manager.get(search_tracker()) else { return };
    if query.trim().is_empty() {
        say(status, "");
        return;
    }
    let token = generation.get() + 1;
    generation.set(token);
    say(status, "");
    clear(results);
    results.append(&crate::widgets::status::loading(Some("Searching…")));
    let (kind, latest, shown_status, shown_results, entry, again) = (entry.kind, Rc::clone(generation), status.clone(), results.clone(), entry.clone(), (entry.clone(), query.clone(), Rc::clone(generation)));
    crate::source_bridge::call_on_main(
        async move { search_hits(tracker.as_ref(), kind, &query).await },
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
        list.append(&ui::row(hit.title(), None, None));
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
    crate::source_bridge::call_on_main(
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
    fn logged_out_the_page_explains_the_anilist_login_and_offers_connect() {
        crate::testing::with_display(|| {
            let built = sheet().expect("the tracking page");
            let widgets = crate::screens::alone::descendants(&built.content);
            let sentence = widgets.iter().any(|w| w.downcast_ref::<Label>().is_some_and(|l| l.text().as_str() == "Tracking keeps your progress on AniList."));
            let connect = widgets
                .iter()
                .filter_map(|w| w.downcast_ref::<Button>())
                .any(|b| b.label().as_deref() == Some("Connect AniList") && b.has_css_class("suggested-action"));
            assert!(sentence && connect, "a logged-out page explains the login and offers Connect AniList");
        });
    }
}
