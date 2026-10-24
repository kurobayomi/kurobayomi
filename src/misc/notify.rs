//! The one desktop notifier of the app. Every notice goes out here as a [`Post`]: the update passes' notices built below
//! (the large-queue warning, the per-entry "new chapters / episodes" notice, the "N update(s) failed" notice, the summary
//! and the progress), the app update's, the reader's saved and shared pictures, and every notice the library crates send
//! through the platform `Notifier` (downloads, backup and restore, incognito), which `gio_notifier` turns into a `Post`.
//!
//! Any thread may send; [`attach`] shows the posts on the main thread, where the one delivery applies the same rules to
//! every notice:
//! - a notice of a channel the user muted (`muted_notification_channels`, channels by `ids::channel`) is not shown;
//! - the content rules (`rules::prepare`);
//! - one notice per id: a show or withdraw within [`MIN_GAP`] of the last send of that id waits, and only the newest
//!   waits. The freedesktop backend replaces or closes a notice by the number the server answered with, and while that
//!   answer is on its way a second send pops up as a new notice (a restore once showed one per title).
//!
//! Decisions:
//! - A notification has no timeout of its own, so a [`Post`] with one is withdrawn by a timer on the main context.
//! - Every tap and button is an `AppAction` (see `router`) sent to the one `app.notice` action. The notice is withdrawn
//!   before the work starts, so a slow download or a failed mark never leaves a stale notice behind.
//! - The tap of an entry notice opens the entry's page by its id; the reader or player at the newest new item is not
//!   wired because the shell has no route that takes an item url.
//! - The "Download" button queues the named items with autostart on through the downloads backend
//!   (`library::actions::download_notified`); "Mark as read" also removes the downloads of the changed items when
//!   "remove after marked as read" is on. Both run off the GTK thread.
//! - The failure notice taps through to the error log (`update_errors.txt`), revealed in the file manager. It has no
//!   body and no tap while no log was written, so it never promises a tap that does nothing.
//! - A notice passes `rules::prepare` (3 buttons at most, escaped text, 200 character body) before the service is called.
//! - The queue pause and clear actions have no manager to call yet: the downloads backend only starts a queue.
//! - Opening a chapter pushes the reader by ids; an episode opens its anime's page, as the deep link page does.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_platform::kind::Kind;

use crate::library_update::notice::{self, EntryAction};
use crate::library_update::pass::EntryUpdate;
use crate::library_update::text;
use crate::misc::ids;
use crate::misc::router::{self, ActionHost, AppAction, QueueOp};
use crate::misc::rules;
use crate::updates::download_host;

/// What a notification button or tap does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An application action by its detailed name (`app.cancel-restore`), with an optional string parameter.
    Named { action: String, target: Option<String> },
    /// The entry's page; withdraws `notice` first when it is set.
    Entry { kind: Kind, id: i64, notice: Option<String> },
    Url(String),
    MarkRead { kind: Kind, entry_id: i64, urls: Vec<String>, notice_id: String },
    Download { kind: Kind, entry_id: i64, urls: Vec<String>, notice_id: String },
    /// The error log file.
    OpenLog(String),
    /// Any application action, for a button or tap whose target is not an entry (the app update notices).
    Action(AppAction),
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "manga",
        Kind::Anime => "anime",
    }
}

impl Target {
    /// The routed application action this target runs; `None` for a [`Target::Named`] action.
    pub fn app_action(&self) -> Option<AppAction> {
        Some(match self {
            Self::Named { .. } => return None,
            Self::Entry { kind: Kind::Manga, id, notice } => AppAction::OpenManga { manga_id: *id, notice: notice.clone() },
            Self::Entry { kind: Kind::Anime, id, notice } => AppAction::OpenAnime { anime_id: *id, notice: notice.clone() },
            Self::Url(url) => AppAction::OpenUrl { url: url.clone() },
            Self::MarkRead { kind: Kind::Manga, entry_id, urls, notice_id } => {
                AppAction::MarkChaptersRead { manga_id: *entry_id, urls: urls.clone(), notice: Some(notice_id.clone()) }
            }
            Self::MarkRead { kind: Kind::Anime, entry_id, urls, notice_id } => {
                AppAction::MarkEpisodesSeen { anime_id: *entry_id, urls: urls.clone(), notice: Some(notice_id.clone()) }
            }
            Self::Download { kind: Kind::Manga, entry_id, urls, notice_id } => {
                AppAction::DownloadChapters { manga_id: *entry_id, urls: urls.clone(), notice: Some(notice_id.clone()) }
            }
            Self::Download { kind: Kind::Anime, entry_id, urls, notice_id } => {
                AppAction::DownloadEpisodes { anime_id: *entry_id, urls: urls.clone(), notice: Some(notice_id.clone()) }
            }
            Self::OpenLog(path) => AppAction::RevealFile { path: path.into() },
            Self::Action(action) => action.clone(),
        })
    }

    /// The detailed application action name and its parameter: `app.notice` with the routed action as JSON, or the named
    /// action as it is.
    fn action(&self) -> Option<(String, Option<glib::Variant>)> {
        if let Self::Named { action, target } = self {
            return Some((action.clone(), target.as_ref().map(|t| t.to_variant())));
        }
        let json = self.app_action()?.to_target().ok()?;
        Some((format!("app.{}", router::NOTICE_ACTION_NAME), Some(json.to_variant())))
    }
}

/// One notification to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Post {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub low_priority: bool,
    /// Withdrawn after this long.
    pub timeout: Option<Duration>,
    pub tap: Option<Target>,
    pub buttons: Vec<(String, Target)>,
    /// A picture for the notice (PNG bytes), the saved page's thumbnail.
    pub icon: Option<Vec<u8>>,
}

/// The per-entry notice of an update, or `None` when the content is hidden or nothing is new (T-26a-034).
pub fn entry_post(kind: Kind, update: &EntryUpdate, hide_content: bool) -> Option<Post> {
    let entry = notice::entry_notice(kind, update, hide_content)?;
    let id = ids::new_items(kind, update.entry_id);
    let buttons = entry
        .actions
        .iter()
        .filter_map(|action| {
            let target = match action {
                EntryAction::MarkRead => Target::MarkRead {
                    kind,
                    entry_id: update.entry_id,
                    urls: update.items.iter().map(|i| i.url.clone()).collect(),
                    notice_id: id.clone(),
                },
                EntryAction::ViewItems => Target::Entry { kind, id: update.entry_id, notice: Some(id.clone()) },
                EntryAction::Download => Target::Download {
                    kind,
                    entry_id: update.entry_id,
                    urls: update.items.iter().map(|i| i.url.clone()).collect(),
                    notice_id: id.clone(),
                },
            };
            Some((action.label(kind).to_owned(), target))
        })
        .collect();
    let tap = Some(Target::Entry { kind, id: update.entry_id, notice: Some(id.clone()) });
    Some(Post {
        id,
        title: entry.title,
        body: Some(entry.text),
        low_priority: false,
        timeout: None,
        tap,
        buttons,
        icon: None,
    })
}

/// The "N update(s) failed" notice: low urgency, shown only when something failed (T-26a-035). With a `log` file its
/// tap opens the file and its body says so (T-26a-096); without one it has neither.
pub fn error_post(kind: Kind, failed: usize, log: Option<&Path>) -> Option<Post> {
    let error = notice::error_notice(failed, log.is_some())?;
    Some(Post {
        id: ids::library_error(kind),
        title: error.title,
        body: error.opens_log.then(|| error.text.to_owned()),
        low_priority: true,
        timeout: None,
        tap: log.map(|path| Target::OpenLog(path.to_string_lossy().into_owned())),
        buttons: Vec::new(),
        icon: None,
    })
}

/// The progress notice of an update pass: low priority, and one Cancel button that stops that pass and withdraws it
/// (T-35a-021, T-26a-054).
pub fn progress_post(kind: Kind, title: &str, body: Option<&str>) -> Post {
    let id = ids::library_progress(kind);
    let cancel = match kind {
        Kind::Manga => AppAction::CancelLibraryUpdate { notice: Some(id.clone()) },
        Kind::Anime => AppAction::CancelAnimeLibraryUpdate { notice: Some(id.clone()) },
    };
    Post {
        id,
        title: title.to_owned(),
        body: body.map(str::to_owned),
        low_priority: true,
        timeout: None,
        tap: None,
        buttons: vec![("Cancel".to_owned(), Target::Action(cancel))],
        icon: None,
    }
}

/// The large-queue warning: stays 30 s and its tap opens the FAQ page. Hidden content does not suppress it (T-26a-031).
pub fn warning_post(kind: Kind) -> Post {
    Post {
        id: ids::library_size_warning(kind),
        title: text::queue_warning_title().to_owned(),
        body: Some(text::queue_warning_text().to_owned()),
        low_priority: false,
        timeout: Some(Duration::from_millis(text::QUEUE_WARNING_TIMEOUT_MS)),
        tap: Some(Target::Url(text::QUEUE_WARNING_URL.to_owned())),
        buttons: Vec::new(),
        icon: None,
    }
}

/// The summary notice of one update pass: the new items of its entries, as the summary text gives them (T-26a-032).
/// The titles follow the summary, so hidden content adds no list.
pub fn summary_post(kind: Kind, summary: &text::SummaryText) -> Post {
    let body = match &summary.big_text {
        Some(titles) => format!("{}\n{titles}", summary.text),
        None => summary.text.clone(),
    };
    Post {
        id: ids::library_summary(kind),
        title: summary.title.to_owned(),
        body: Some(body),
        low_priority: false,
        timeout: None,
        tap: None,
        buttons: Vec::new(),
        icon: None,
    }
}

/// One step for the main thread, applied in the order it was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Show(Post),
    Withdraw(String),
}

static SENDER: OnceLock<async_channel::Sender<Op>> = OnceLock::new();

/// Sends one step to the main thread. A show goes through the content rules first. `false` when [`attach`] has not run
/// or the notice breaks a rule, so the caller can fall back to a toast.
pub fn send(op: Op) -> bool {
    let op = match op {
        Op::Show(post) => match rules::prepare(post) {
            Ok(post) => Op::Show(post),
            Err(error) => {
                tracing::warn!(%error, "a notice was not sent");
                return false;
            }
        },
        withdraw => withdraw,
    };
    SENDER.get().is_some_and(|tx| tx.try_send(op).is_ok())
}

/// Sends `post` to the main thread after the content rules (see [`send`]).
pub fn post(post: Post) -> bool {
    send(Op::Show(post))
}

/// Removes the notice with this id. A missing id is not an error.
pub fn withdraw(id: &str) {
    send(Op::Withdraw(id.to_owned()));
}

/// The shortest time between two sends of one id (see the module notes).
pub const MIN_GAP: Duration = Duration::from_secs(1);

/// What happens to one id now: sent at once, or held until the gap since its last send has passed.
#[derive(Debug, PartialEq, Eq)]
enum Pace {
    Now,
    /// Wait this long; a step is already waiting when `scheduled` is false, and the new one takes its place.
    Hold { wait: Duration, scheduled: bool },
}

/// The last send and the step waiting for each id.
#[derive(Default)]
struct Pacer {
    ids: std::collections::HashMap<String, (std::time::Instant, Option<Op>)>,
}

impl Pacer {
    fn pace(&mut self, op: Op, now: std::time::Instant) -> (Pace, Option<Op>) {
        let id = op_id(&op).to_owned();
        match self.ids.get_mut(&id) {
            Some((last, waiting)) if now.duration_since(*last) < MIN_GAP => {
                let scheduled = waiting.is_none();
                *waiting = Some(op);
                (Pace::Hold { wait: MIN_GAP - now.duration_since(*last), scheduled }, None)
            }
            _ => {
                self.ids.insert(id, (now, None));
                (Pace::Now, Some(op))
            }
        }
    }

    /// The step that waited for `id`, now due.
    fn due(&mut self, id: &str, now: std::time::Instant) -> Option<Op> {
        let (last, waiting) = self.ids.get_mut(id)?;
        let op = waiting.take()?;
        *last = now;
        Some(op)
    }
}

fn op_id(op: &Op) -> &str {
    match op {
        Op::Show(post) => &post.id,
        Op::Withdraw(id) => id,
    }
}

thread_local! {
    static PACER: std::cell::RefCell<Pacer> = std::cell::RefCell::new(Pacer::default());
}

/// Applies one step on the main thread, paced per id.
fn deliver(app: &gio::Application, op: Op) {
    let id = op_id(&op).to_owned();
    let (pace, now) = PACER.with(|p| p.borrow_mut().pace(op, std::time::Instant::now()));
    match pace {
        Pace::Now => apply(app, now.expect("sent now")),
        Pace::Hold { wait, scheduled: true } => {
            let app = app.clone();
            glib::timeout_add_local_once(wait, move || {
                if let Some(op) = PACER.with(|p| p.borrow_mut().due(&id, std::time::Instant::now())) {
                    apply(&app, op);
                }
            });
        }
        Pace::Hold { scheduled: false, .. } => {}
    }
}

/// True when the user muted the channel of this notice.
fn muted(id: &str) -> bool {
    crate::data::is_installed() && crate::data::app().base.muted_notification_channels.get().contains(ids::channel(id))
}

fn apply(app: &gio::Application, op: Op) {
    match op {
        Op::Show(post) if muted(&post.id) => {}
        Op::Show(post) => show(app, &post),
        Op::Withdraw(id) => app.withdraw_notification(&id),
    }
}

fn show(app: &gio::Application, post: &Post) {
    let notification = gio::Notification::new(&post.title);
    notification.set_body(post.body.as_deref());
    if post.low_priority {
        notification.set_priority(gio::NotificationPriority::Low);
    }
    if let Some((action, target)) = post.tap.as_ref().and_then(Target::action) {
        notification.set_default_action_and_target_value(&action, target.as_ref());
    }
    for (label, target) in &post.buttons {
        if let Some((action, value)) = target.action() {
            notification.add_button_with_target_value(label, &action, value.as_ref());
        }
    }
    if let Some(bytes) = &post.icon {
        notification.set_icon(&gio::BytesIcon::new(&glib::Bytes::from(bytes.as_slice())));
    }
    app.send_notification(Some(&post.id), &notification);
    if let Some(timeout) = post.timeout {
        let (app, id) = (app.clone(), post.id.clone());
        glib::timeout_add_local_once(timeout, move || deliver(&app, Op::Withdraw(id)));
    }
}

/// Shows the folder of `path` in the file manager.
fn reveal(path: &Path) {
    if let Err(error) = kuro_platform::opener::Opener::reveal(&kuro_platform::opener::SystemOpener, path) {
        tracing::warn!(%error, "the update error log could not be shown");
    }
}

/// Both rows of an open-item action exist, and the item belongs to the entry.
fn item_rows_exist(kind: Kind, entry_id: i64, item_id: i64) -> bool {
    let db = crate::data::app().db.clone();
    EntryRepo::new(db.clone()).by_id(entry_id).is_ok_and(|e| e.kind == kind)
        && ItemRepo::new(db).by_id(item_id).ok().flatten().is_some_and(|i| i.entry_id == entry_id)
}

/// Raises the window and opens the entry page by its id.
fn show_entry(kind: Kind, id: i64) {
    crate::ui::window().present();
    crate::ui::over_root(|| match kind {
        Kind::Manga => crate::entry_core::details::route::open_kind(Kind::Manga, id),
        Kind::Anime => crate::entry_core::details::route::open(id),
    });
}

/// The GTK and runtime half of the router.
struct GtkHost {
    app: gio::Application,
}

impl ActionHost for GtkHost {
    fn withdraw(&self, notice: &str) {
        deliver(&self.app, Op::Withdraw(notice.to_owned()));
    }

    fn open_entry(&self, kind: Kind, id: i64) {
        show_entry(kind, id);
    }

    fn open_downloads(&self, kind: Kind) {
        let name = match kind {
            Kind::Manga => "open-downloads",
            Kind::Anime => "open-anime-downloads",
        };
        self.app.activate_action(name, None);
    }

    fn open_item(&self, kind: Kind, entry_id: i64, item_id: i64) {
        glib::spawn_future_local(async move {
            let found = download_host::blocking(move || item_rows_exist(kind, entry_id, item_id)).await.unwrap_or(false);
            router::open_or_toast(
                found,
                || match kind {
                    Kind::Manga => {
                        crate::ui::window().present();
                        crate::ui::push(&crate::reader::launch::open_chapter(entry_id, item_id));
                    }
                    Kind::Anime => show_entry(kind, entry_id),
                },
                |text| crate::toast::notify(text),
            );
        });
    }

    fn mark_read(&self, _kind: Kind, entry_id: i64, urls: Vec<String>) {
        let data = crate::data::app();
        let db = data.db.clone();
        let remove = crate::library::actions::remove_after_marked_read();
        let backend = download_host::current();
        glib::spawn_future_local(async move {
            if let Err(error) = crate::library::actions::mark_notified_read(&db, backend, entry_id, &urls, remove).await {
                tracing::error!(%error, entry_id, "marking the notified items read failed");
            }
        });
    }

    fn download(&self, kind: Kind, entry_id: i64, urls: Vec<String>) {
        let data = crate::data::app();
        let db = data.db.clone();
        let backend = download_host::current();
        glib::spawn_future_local(async move {
            let queued = download_host::blocking(move || {
                crate::library::actions::download_notified(&db, &*backend, kind, entry_id, &urls)
            })
            .await;
            if queued.is_none() {
                tracing::error!(entry_id, "queueing the notified items did not finish");
            }
        });
    }

    fn queue(&self, kind: Kind, op: QueueOp) {
        match op {
            QueueOp::Resume => download_host::current().start_queue(kind),
            QueueOp::Pause | QueueOp::Clear => self.unavailable(&format!("{op:?} {} downloads", kind_name(kind))),
        }
    }

    fn cancel_update(&self, kind: Kind) {
        if !crate::misc::runner::stop(kind) {
            tracing::debug!(kind = kind_name(kind), "no update pass was running to cancel");
        }
    }

    fn cancel_restore(&self) {
        crate::backup_host::request_stop();
    }

    fn open_url(&self, url: &str) {
        crate::ui::open_link(url, None);
    }

    fn reveal(&self, path: &Path) {
        reveal(path);
    }

    fn unavailable(&self, what: &str) {
        tracing::warn!(action = what, "a notice action has no handler in this build");
    }

    fn reveal_roots(&self) -> Vec<std::path::PathBuf> {
        use kuro_platform::paths::Paths as _;
        kuro_platform::system_paths::SystemPaths::system()
            .map(|p| {
                let mut roots = vec![p.data_dir(), p.user_downloads_dir(), p.default_storage_dir(), p.cache_dir()];
                roots.extend(crate::misc::reveal::announced_folders());
                roots
            })
            .unwrap_or_default()
    }

}

/// Registers `app.notice` and starts showing posted notices. Call from the GTK main thread, with the application running;
/// a second call does nothing. Without an application (a test run) nothing is attached and [`post`] returns `false`.
pub fn attach() {
    let Some(app) = gio::Application::default() else { return };
    let (tx, rx) = async_channel::unbounded();
    if SENDER.set(tx).is_err() {
        return;
    }
    let action = gio::SimpleAction::new(router::NOTICE_ACTION_NAME, Some(glib::VariantTy::STRING));
    let host = GtkHost { app: app.clone() };
    action.connect_activate(move |_, param| {
        let text = param.and_then(|p| p.get::<String>()).unwrap_or_default();
        match AppAction::from_target(&text) {
            Ok(action) => router::route(&host, action),
            Err(error) => tracing::warn!(%error, "a notice action was rejected"),
        }
    });
    app.add_action(&action);
    glib::spawn_future_local(async move {
        while let Ok(op) = rx.recv().await {
            deliver(&app, op);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_update::source::SourceItem;

    #[test]
    fn the_summary_notice_lists_its_titles_only_when_content_shows() {
        // T-26a-032: one desktop notice per pass, with the titles unless the content is hidden
        let titles = vec!["A".to_owned(), "B".to_owned()];
        let shown = summary_post(Kind::Manga, &text::summary(Kind::Manga, &titles, false));
        assert_eq!((shown.title.as_str(), shown.body.as_deref()), ("New chapters found", Some("For 2 entries\nA\nB")));
        let hidden = summary_post(Kind::Manga, &text::summary(Kind::Manga, &titles, true));
        assert_eq!(hidden.body.as_deref(), Some("For 2 entries"));
        assert_eq!(hidden.id, ids::library_summary(Kind::Manga));
    }

    fn update(items: &[(f64, i64)]) -> EntryUpdate {
        EntryUpdate {
            entry_id: 12,
            title: "Berserk".into(),
            items: items
                .iter()
                .map(|(number, order)| SourceItem {
                    url: format!("/c{number}"),
                    number: *number,
                    source_order: *order,
                    ..SourceItem::default()
                })
                .collect(),
        }
    }

    #[test]
    fn the_warning_lasts_30_seconds_and_its_tap_opens_the_faq() {
        // T-26a-031
        let post = warning_post(Kind::Manga);
        assert_eq!(post.timeout, Some(Duration::from_secs(30)));
        assert_eq!(post.tap, Some(Target::Url(text::QUEUE_WARNING_URL.to_owned())));
        assert_eq!(post.body.as_deref(), Some(text::queue_warning_text()).as_deref());
        assert_ne!(post.id, warning_post(Kind::Anime).id);
    }

    #[test]
    fn the_entry_notice_offers_mark_read_and_view_and_taps_into_the_entry() {
        // T-26a-034
        let post = entry_post(Kind::Manga, &update(&[(1.0, 1), (2.0, 0)]), false).expect("shown");
        let labels: Vec<&str> = post.buttons.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Mark as read", "View chapters", "Download"]);
        assert_eq!(post.tap, Some(Target::Entry { kind: Kind::Manga, id: 12, notice: Some(post.id.clone()) }));
        let Target::MarkRead { urls, notice_id, .. } = &post.buttons[0].1 else { panic!("mark read first") };
        assert_eq!(urls, &["/c1".to_owned(), "/c2".to_owned()]);
        assert_eq!(notice_id, &post.id);
        let Target::Download { urls, .. } = &post.buttons[2].1 else { panic!("download last") };
        assert_eq!(urls, &["/c1".to_owned(), "/c2".to_owned()], "the button names the new items");
        let anime = entry_post(Kind::Anime, &update(&[(1.0, 0)]), false).expect("shown");
        assert_eq!(anime.buttons[0].0, "Mark as watched");
        assert_eq!(anime.buttons[1].0, "View episodes");
    }

    #[test]
    fn a_long_list_of_new_items_drops_the_download_button() {
        // T-26a-094: the limits are the notice module's constants (15 chapters, 10 episodes)
        use crate::library_update::notice::{ANIME_DOWNLOAD_ACTION_LIMIT, MANGA_DOWNLOAD_ACTION_LIMIT};
        let many: Vec<(f64, i64)> = (0..=MANGA_DOWNLOAD_ACTION_LIMIT).map(|n| (n as f64, n as i64)).collect();
        let post = entry_post(Kind::Manga, &update(&many), false).expect("shown");
        assert_eq!(post.buttons.len(), 2, "one over the chapter limit: no Download");
        let post = entry_post(Kind::Manga, &update(&many[..MANGA_DOWNLOAD_ACTION_LIMIT]), false).expect("shown");
        assert_eq!(post.buttons.len(), 3, "at the chapter limit: Download");
        let post = entry_post(Kind::Anime, &update(&many[..=ANIME_DOWNLOAD_ACTION_LIMIT]), false).expect("shown");
        assert_eq!(post.buttons.len(), 2, "one over the episode limit: no Download");
    }

    #[test]
    fn hidden_content_suppresses_the_entry_notice_but_not_the_warning() {
        assert!(entry_post(Kind::Manga, &update(&[(1.0, 0)]), true).is_none());
        assert!(entry_post(Kind::Manga, &update(&[]), false).is_none());
        assert_eq!(warning_post(Kind::Manga).title, text::queue_warning_title());
    }

    #[test]
    fn the_failure_notice_is_low_and_only_for_failures() {
        // T-26a-035
        assert!(error_post(Kind::Manga, 0, None).is_none());
        let post = error_post(Kind::Manga, 3, None).expect("shown");
        assert_eq!(post.title, "3 update(s) failed");
        assert!(post.low_priority);
        assert_eq!(post.body, None, "no log file, so no tap line");
        assert_eq!(post.tap, None);
    }

    #[test]
    fn the_failure_notice_opens_the_error_log_when_one_was_written() {
        // T-26a-096
        let log = Path::new("/cache/update_errors.txt");
        let post = error_post(Kind::Anime, 2, Some(log)).expect("shown");
        assert_eq!(post.body.as_deref(), Some("Tap to see details"));
        assert_eq!(post.tap, Some(Target::OpenLog("/cache/update_errors.txt".to_owned())));
        let (action, value) = post.tap.expect("tap").action().expect("encodes");
        assert_eq!(action, "app.notice");
        let action = AppAction::from_target(&value.expect("a parameter").get::<String>().expect("a string")).expect("decodes");
        assert_eq!(action, AppAction::RevealFile { path: "/cache/update_errors.txt".into() });
    }

    #[test]
    fn a_target_names_its_application_action() {
        let entry = Target::Entry { kind: Kind::Anime, id: 5, notice: Some("n".into()) };
        assert_eq!(entry.app_action().unwrap(), AppAction::OpenAnime { anime_id: 5, notice: Some("n".into()) });
        assert_eq!(Target::Url("https://x".into()).app_action().unwrap(), AppAction::OpenUrl { url: "https://x".into() });
        let urls = vec!["/a".to_owned()];
        let download = Target::Download { kind: Kind::Manga, entry_id: 2, urls: urls.clone(), notice_id: "n".into() };
        assert_eq!(
            download.app_action().unwrap(),
            AppAction::DownloadChapters { manga_id: 2, urls: urls.clone(), notice: Some("n".into()) }
        );
        let mark = Target::MarkRead { kind: Kind::Anime, entry_id: 2, urls: urls.clone(), notice_id: "n".into() };
        assert_eq!(mark.app_action().unwrap(), AppAction::MarkEpisodesSeen { anime_id: 2, urls, notice: Some("n".into()) });
        let (name, value) = mark.action().expect("encodes");
        assert_eq!(name, "app.notice", "every button goes through the one action");
        assert!(value.and_then(|v| v.get::<String>()).is_some_and(|json| json.contains("mark_episodes_seen")));
        let named = Target::Named { action: "app.cancel-restore".into(), target: None };
        assert_eq!(named.app_action(), None);
        assert_eq!(named.action(), Some(("app.cancel-restore".to_owned(), None)), "a named action goes out as it is");
    }

    #[test]
    fn a_second_send_of_one_id_within_the_gap_waits_and_only_the_newest_waits() {
        let mut pacer = Pacer::default();
        let start = std::time::Instant::now();
        let show = |text: &str| Op::Show(Post { id: "n".into(), title: text.into(), ..Post::default() });
        assert_eq!(pacer.pace(show("1"), start).0, Pace::Now);
        let later = start + Duration::from_millis(300);
        assert_eq!(pacer.pace(show("2"), later).0, Pace::Hold { wait: Duration::from_millis(700), scheduled: true });
        assert_eq!(pacer.pace(show("3"), later).0, Pace::Hold { wait: Duration::from_millis(700), scheduled: false }, "one timer");
        assert_eq!(pacer.pace(Op::Show(Post { id: "other".into(), ..Post::default() }), later).0, Pace::Now, "ids are apart");
        let due = start + MIN_GAP;
        assert_eq!(pacer.due("n", due), Some(show("3")), "the newest step goes out");
        assert_eq!(pacer.due("n", due), None);
        assert_eq!(pacer.pace(Op::Withdraw("n".into()), due + Duration::from_millis(10)).0, Pace::Hold { wait: Duration::from_millis(990), scheduled: true }, "a withdraw right after a show waits too");
        assert_eq!(pacer.pace(show("4"), due + MIN_GAP * 2).0, Pace::Now);
    }

    #[test]
    fn the_entry_notice_slot_comes_from_the_registry_and_every_post_passes_the_rules() {
        let post = entry_post(Kind::Manga, &update(&[(1.0, 1)]), false).expect("shown");
        assert_eq!(post.id, ids::new_items(Kind::Manga, 12));
        assert_eq!(warning_post(Kind::Anime).id, ids::library_size_warning(Kind::Anime));
        assert_eq!(error_post(Kind::Manga, 1, None).expect("shown").id, ids::library_error(Kind::Manga));
        for post in [post, warning_post(Kind::Manga)] {
            assert!(rules::prepare(post).is_ok());
        }
    }

    #[test]
    fn the_progress_notice_has_its_own_slot_per_kind_and_a_cancel_that_stops_that_kind() {
        // T-26a-054 / T-35a-021: the cancel names the pass of the notice's own kind
        let manga = progress_post(Kind::Manga, "Updating library", Some("Checking 1/3"));
        assert_eq!(manga.id, ids::library_progress(Kind::Manga));
        assert_eq!(manga.body.as_deref(), Some("Checking 1/3"));
        assert!(manga.low_priority);
        assert_eq!(
            manga.buttons,
            vec![("Cancel".to_owned(), Target::Action(AppAction::CancelLibraryUpdate { notice: Some(manga.id.clone()) }))]
        );
        let anime = progress_post(Kind::Anime, "Updating library", None);
        assert_ne!(anime.id, manga.id);
        assert_eq!(anime.buttons[0].1, Target::Action(AppAction::CancelAnimeLibraryUpdate { notice: Some(anime.id.clone()) }));
        assert!(rules::prepare(anime).is_ok());
    }

    #[test]
    fn nothing_is_sent_before_the_notifier_is_attached() {
        assert!(!post(warning_post(Kind::Manga)));
    }
}
