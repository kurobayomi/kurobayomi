//! The update runner of the app (T-26a-102, T-26a-104, T-26a-131): one `PassRunner` for both kinds over the app's
//! databases, the settings of a pass read from the preferences, the launch pass and the library's refresh button.
//!
//! Decisions:
//! - Passes run on the app's runtime, built by `kuro_core::runtime::build` (2 to 8 workers by core count); the 4
//!   workers of T-26a-131 and the "keep running in the background" check at shutdown belong to that builder and
//!   `main`, which this lane does not own.
//! - Entries find their sources through [`LiveRegistry`], the one registry over the recipe and local sources.
//!   An id it does not know ends the entry as "Source not installed" (the stub slot of `SourceRegistry`).
//! - The "only on Wi-Fi" restriction reads `kuro_platform::device::read()`. That reader cannot tell "wired" from
//!   "NetworkManager unreadable", so [`device_facts`] treats a machine with no sign of NetworkManager as unknown
//!   (SC-07), through `kuro_platform::device::network_manager_present`.

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kuro_db::handle::Db;
use kuro_domain::items::source_sync::EntrySyncLocks;
use kuro_data::source::StubSourceRepo;
use kuro_platform::kind::Kind;
use kuro_domain::download::prefs::DownloadPrefs;
use kuro_prefs::Prefs;
use kuro_prefs::store::PrefStore;
use kuro_downloads::queue::Place;
use kuro_downloads::service::Downloads;
use tokio::runtime::Handle;

use crate::downloads::directory::AppDirectory;
use crate::library_update::constraints::{self, DeviceFacts};
use crate::library_update::counters::PrefsHost;
use crate::library_update::download::DownloadPolicy;
use crate::library_update::entry::Deps;
use crate::library_update::error_log;
use crate::library_update::live::LiveRegistry;
use crate::library_update::metadata::{DbMetadataStore, MetadataDeps, MetadataProgress, MetadataRunner};
use crate::library_update::pass::{EntryUpdate, Failure, Host, PassRunner, PassSettings, PassStatus, Trigger};
use crate::library_update::queue::QueueRestrictions;
use crate::library_update::source::SourceRegistry;
use crate::library_update::renames::FolderRenames;
use crate::library_update::store::SqliteLibraryStore;
use crate::library_update::sync::DbItemSync;
use crate::library_update::text;
use crate::misc::diet_defaults as diet;
use crate::misc::notify;
use crate::updates::download_host;

/// The notifications and downloads of a pass: a desktop notice for the summary, and desktop notices (`notify`) for each
/// entry, for failures and for the large-queue warning, which fall back to a toast when no notifier is attached. New items go
/// to the one download service paused (fillermarked episodes skipped, D6), and one start at the end of the pass. Progress
/// and counters have no surface here: the counters are written by `PrefsHost`.
///
/// The host runs on the runtime's workers, where the app's thread-bound handles are out of reach, so it keeps the
/// database handle and the download service (with the runtime that runs it) it needs.
pub struct ToastHost {
    db: Arc<Db>,
    downloads: Option<(Downloads, Handle)>,
}

impl ToastHost {
    pub fn new(db: Arc<Db>, downloads: Option<(Downloads, Handle)>) -> Self {
        Self { db, downloads }
    }

    /// What the error log calls a source: `Name (LANG)` for a source the app holds, else the name its stub row keeps
    /// (a backup restore writes it for a source that is not installed), else the number.
    fn label(&self, kind: Kind, id: i64) -> String {
        if let Some(source) = LiveRegistry.get(kind, id) {
            return source.label();
        }
        let db = Arc::clone(&self.db);
        match StubSourceRepo::new(db, kind).get(id).ok().flatten() {
            Some(stub) if stub.name.trim().is_empty() => id.to_string(),
            Some(stub) if stub.lang.trim().is_empty() => stub.name,
            Some(stub) => kuro_source::display_name(&stub.name, &stub.lang),
            None => id.to_string(),
        }
    }
}

impl Host for ToastHost {
    fn set_last_update_ms(&self, _now_ms: i64) {}

    fn add_new_updates(&self, _kind: Kind, _count: usize) {}

    fn show_progress(&self, kind: Kind, title: &str, body: Option<&str>) {
        show_progress_once(kind, title, body);
    }

    fn cancel_progress(&self, kind: Kind) {
        withdraw_progress(kind);
    }

    fn show_summary(&self, kind: Kind, updates: &[EntryUpdate], hide_content: bool) {
        if updates.is_empty() {
            return;
        }
        // one notice per pass: one updated entry gets its own notice and its buttons, more get the summary
        if let [update] = updates
            && let Some(post) = notify::entry_post(kind, update, hide_content)
        {
            notify::post(post);
            return;
        }
        let titles: Vec<String> = updates.iter().map(|u| u.title.clone()).collect();
        let summary = text::summary(kind, &titles, hide_content);
        if !notify::post(notify::summary_post(kind, &summary)) {
            crate::toast::notify(format!("{}: {}", summary.title, summary.text));
        }
    }

    fn enqueue_downloads(&self, kind: Kind, entry_id: i64, items: &[crate::library_update::source::SourceItem]) {
        let urls: Vec<String> = items.iter().map(|item| item.url.clone()).collect();
        let ids = crate::library::actions::item_ids_by_url(&self.db, entry_id, &urls);
        if ids.is_empty() {
            return;
        }
        let Some((downloads, runtime)) = &self.downloads else { return };
        // paused, so the pass starts the queue once; this automatic path alone skips fillermarked episodes (D6)
        let request = AppDirectory::new(Arc::clone(&self.db)).enqueue_request(kind, entry_id, &ids, Place::Back, false, true);
        let Some(request) = request else { return };
        // a pass runs on the runtime's workers: block in place there, else block on the handle
        let queued = if Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| runtime.block_on(downloads.enqueue(request)))
        } else {
            runtime.block_on(downloads.enqueue(request))
        };
        match queued {
            Ok(done) if done.added.is_empty() => tracing::debug!(entry_id, items = ids.len(), "the downloads queued none of the new items"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, entry_id, "the new items were not queued"),
        }
    }

    fn start_downloads(&self, kind: Kind) {
        download_host::current().start_queue(kind);
    }

    fn write_error_log(&self, kind: Kind, failures: &[Failure]) -> Option<PathBuf> {
        let paths = kuro_platform::system_paths::SystemPaths::system().ok()?;
        match error_log::write(&error_log::path(&paths), kind, failures, &|id| self.label(kind, id)) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(%error, "the update error log was not written");
                None
            }
        }
    }

    fn show_error(&self, kind: Kind, failed: usize, log: Option<&Path>) {
        if let Some(post) = notify::error_post(kind, failed, log) {
            let toast = post.title.clone();
            if !notify::post(post) {
                crate::toast::notify(toast);
            }
        }
    }

    fn show_queue_warning(&self, kind: Kind) {
        if !notify::post(notify::warning_post(kind)) {
            crate::toast::notify(format!("{}: {}", text::queue_warning_title(), text::queue_warning_text()));
        }
    }
}

/// The facts a pass checks, from the platform's two readers. The connection comes from the conditions reader (it falls
/// back to the network monitor when NetworkManager is missing). Wi-Fi comes from the device reader, which reports
/// `false` both for "wired" and for "NetworkManager unreadable"; a machine that shows no sign of NetworkManager gets
/// `None`, so Wi-Fi-only never blocks it (SC-07). Any positive reading proves NetworkManager answered.
fn device_facts(
    conditions: kuro_platform::power_network::Conditions,
    device: kuro_platform::device::DeviceState,
    network_manager_seen: bool,
) -> DeviceFacts {
    let answered = device.on_wifi || device.network_not_metered || network_manager_seen;
    DeviceFacts {
        connected: Some(conditions.connected),
        wifi: answered.then_some(device.on_wifi),
        metered: Some(conditions.metered),
        on_ac: Some(conditions.on_ac),
    }
}

/// Whether the device restrictions the user chose hold right now. A blocked scheduled pass retries later.
///
/// Deviation (T-26a-012): the battery level is never read, so the battery-not-low constraint is not enforced on this
/// platform. Wi-Fi, metered network and power source are the constraints checked here.
async fn device_allows(_inputs: &Inputs) -> bool {
    // The device restrictions are cut from the settings: no restriction applies.
    let restrictions = BTreeSet::<String>::new();
    let conditions = kuro_platform::power_network::read_conditions().await;
    let device = kuro_platform::device::read().await;
    let seen = kuro_platform::device::network_manager_present();
    constraints::satisfied(&restrictions, device_facts(conditions, device, seen))
}

type Runner = PassRunner<LiveRegistry, SqliteLibraryStore, DbItemSync, PrefsHost<ToastHost>>;

/// The preferences a pass reads each time it starts.
pub struct Inputs {
    pub download: DownloadPrefs,
}

/// The download policy of a pass (T-26a-090, T-26a-091): the chapter or episode toggle. The unread-only switch and the
/// category lists are cut from the settings and hard-coded (off and empty).
pub fn download_policy(kind: Kind, download: &DownloadPrefs) -> DownloadPolicy {
    match kind {
        Kind::Manga => DownloadPolicy {
            enabled: download.download_new_chapters.get(),
            unread_only: diet::DOWNLOAD_NEW_UNREAD_ONLY,
            include_categories: Vec::new(),
            exclude_categories: Vec::new(),
        },
        Kind::Anime => DownloadPolicy {
            enabled: download.download_new_episodes.get(),
            unread_only: diet::DOWNLOAD_NEW_UNREAD_ONLY,
            include_categories: Vec::new(),
            exclude_categories: Vec::new(),
        },
    }
}

/// The settings of one pass of `kind`. A manual pass of one category passes `category`; the include and exclude lists
/// are then ignored by the queue.
pub fn pass_settings(kind: Kind, category: Option<i64>, inputs: &Inputs) -> PassSettings {
    // The update category lists, the Smart update switch and the season switch are cut: all categories, the default
    // item restrictions, no metadata refresh and no season refresh. Notifications never hide their content.
    let tokens: Vec<String> = diet::LIBRARY_UPDATE_ITEM_RESTRICTIONS.iter().map(|token| token.to_string()).collect();
    PassSettings {
        download: download_policy(kind, &inputs.download),
        category,
        include_categories: Vec::new(),
        exclude_categories: Vec::new(),
        restrictions: QueueRestrictions::from_pref(Some(&tokens)),
        auto_update_metadata: diet::LIBRARY_AUTO_UPDATE_METADATA,
        include_season_children: diet::LIBRARY_UPDATE_SEASON,
        hide_content: diet::NOTIFICATION_HIDE_CONTENT,
        ..PassSettings::new(kind)
    }
}

/// The folder renames of the item sync: the one download service renames the folders and keeps its index, and the source
/// names come from the one directory (a source it does not know keeps its folders).
fn folder_renames(downloads: Option<Downloads>, db: Arc<Db>) -> Arc<FolderRenames> {
    Arc::new(FolderRenames::new(downloads, AppDirectory::new(db)))
}

fn runner() -> Arc<Runner> {
    let app = crate::data::app();
    runner_over(app.db.clone(), app.base.store())
}

/// The runner over the database handle and a preference store, for the app and for the headless pass.
fn runner_over(db: Arc<Db>, store: &PrefStore) -> Arc<Runner> {
    // the shell and its service live on the GTK thread, so they are taken here (the headless pass has none)
    let shell = crate::shell::host::Shell::get();
    let downloads = shell.as_ref().and_then(|shell| shell.download_service());
    let runtime = shell.as_ref().map(|shell| shell.runtime().clone());
    let renames = folder_renames(downloads.clone(), db.clone());
    let sync = DbItemSync::new(
        db.clone(),
        Arc::new(EntrySyncLocks::default()),
        diet::LIBRARY_MARK_DUPLICATE_CHAPTER_READ,
        diet::LIBRARY_MARK_DUPLICATE_EPISODE_SEEN,
    )
    .with_renames(renames);
    let prefs = Prefs::with_store(store.clone(), Arc::new(kuro_prefs::memory::MemorySecrets::default()));
    let host = PrefsHost::new(ToastHost::new(db.clone(), downloads.zip(runtime)), &prefs);
    Arc::new(PassRunner::new(Deps {
        registry: LiveRegistry,
        store: SqliteLibraryStore::new(db),
        sync,
        host,
    }))
}

/// Hands the runner's two closures to the library's refresh button and to the Updates page's refresh button, so both
/// start the same runner and share one single-flight lock (T-26a-102, T-26a-103).
fn register_buttons(start: std::rc::Rc<dyn Fn(Kind, Option<i64>) -> bool>, running: std::rc::Rc<dyn Fn(Kind) -> bool>) {
    crate::library::mount::on_update(crate::library::mount::UpdateHost { start: start.clone(), running: running.clone() });
    crate::updates::library_trigger::install_shared(start, running);
}

type MetadataPasses = MetadataRunner<LiveRegistry, SqliteLibraryStore, DbMetadataStore>;

thread_local! {
    static STARTED: OnceCell<()> = const { OnceCell::new() };
    static METADATA: RefCell<Option<Arc<MetadataPasses>>> = const { RefCell::new(None) };
    static RUNNING: RefCell<Option<Arc<Runner>>> = const { RefCell::new(None) };
}

/// Cancels the running update pass of `kind` (the cancel button of a progress notice, T-35a-021). `false` when no pass
/// runs or the runner has not started. Call from the GTK main thread.
pub fn stop(kind: Kind) -> bool {
    let updates = RUNNING.with(|r| r.borrow().as_ref().and_then(|runner| runner.stop(kind))).is_some();
    let metadata = METADATA.with(|m| m.borrow().as_ref().is_some_and(|runner| runner.stop(kind)));
    updates || metadata
}

/// The progress notice of a metadata pass: the slot and Cancel button of an update pass of the same kind (T-26a-054).
struct MetadataNotices;

impl MetadataProgress for MetadataNotices {
    fn show(&self, kind: Kind, title: &str) {
        show_progress_once(kind, title, None);
    }

    fn done(&self, kind: Kind) {
        withdraw_progress(kind);
    }
}

/// Whether the progress notice of a kind (manga, anime) is up.
static PROGRESS_SHOWN: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

fn progress_shown(kind: Kind) -> &'static AtomicBool {
    &PROGRESS_SHOWN[usize::from(kind == Kind::Anime)]
}

/// The progress notice is posted once per pass, not on every finished entry: the desktop cannot replace a notice whose
/// first send is still in flight, so quick updates piled up as one notice each. It keeps its Cancel button.
fn show_progress_once(kind: Kind, title: &str, body: Option<&str>) {
    if !progress_shown(kind).swap(true, Ordering::Relaxed) {
        notify::post(notify::progress_post(kind, title, body));
    }
}

fn withdraw_progress(kind: Kind) {
    progress_shown(kind).store(false, Ordering::Relaxed);
    notify::send(notify::Op::Withdraw(crate::misc::ids::library_progress(kind)));
}

fn metadata_passes() -> Arc<MetadataPasses> {
    let app = crate::data::app();
    Arc::new(MetadataRunner::new(MetadataDeps {
        registry: LiveRegistry,
        store: SqliteLibraryStore::new(app.db.clone()),
        writer: DbMetadataStore::new(app.db.clone()),
        progress: Some(Arc::new(MetadataNotices)),
    }))
}

/// The manual "Refresh library covers" action (T-26a-050): starts the anime and the manga metadata pass, in that order
/// (T-26a-109). The passes have no schedule, and each is refused only by a metadata pass of its own kind (T-26a-051).
/// Returns, per kind, whether its pass started. Call from the GTK main thread inside the app's runtime, after [`start`];
/// before that nothing starts.
pub fn refresh_library_covers() -> [(Kind, bool); 2] {
    let runner = METADATA.with(|m| m.borrow().clone());
    let now_ms = chrono::Local::now().timestamp_millis();
    [Kind::Anime, Kind::Manga].map(|kind| (kind, runner.as_ref().is_some_and(|r| r.start_now(kind, now_ms))))
}

/// Builds the runner, starts the launch pass and hands the runner to the library's refresh button. Runs once; call from
/// the GTK main thread inside the app's runtime.
pub fn start() {
    if STARTED.with(|s| s.set(()).is_err()) || !crate::data::is_installed() {
        return;
    }
    let runner = runner();
    RUNNING.with(|r| *r.borrow_mut() = Some(runner.clone()));
    let app = crate::data::app();
    let inputs = Arc::new(Inputs {
        download: DownloadPrefs::new(app.base.store()),
    });
    METADATA.with(|m| *m.borrow_mut() = Some(metadata_passes()));

    let (start_runner, start_inputs) = (runner.clone(), inputs.clone());
    let running_runner = runner.clone();
    let local = LocalTitles { db: app.db.clone(), prefs: app.base.store().clone() };
    let start: std::rc::Rc<dyn Fn(Kind, Option<i64>) -> bool> = std::rc::Rc::new(move |kind, category| {
        start_runner.start_after(pass_settings(kind, category, &start_inputs), local.clone().before_pass(kind))
    });
    let running: std::rc::Rc<dyn Fn(Kind) -> bool> =
        std::rc::Rc::new(move |kind| running_runner.locks().holder(kind).is_some());
    register_update_action(start.clone());
    // every app launch checks the library for new releases (the pass waits for the sources like every other)
    start_all(&*start);
    register_buttons(start, running);
}

/// Resolves once the repository sources (anime and manga, one load installs both) have loaded. Every pass of the app
/// waits for it: a pass that ran before would find no source and fail each entry with "Source not installed".
async fn sources_loaded() {
    let mut ready = crate::recipe_sources::ready();
    let _ = ready.wait_for(|loaded| *loaded).await;
}

/// What the passes need to add the Local folder's new titles.
#[derive(Clone)]
struct LocalTitles {
    db: Arc<Db>,
    prefs: PrefStore,
}

impl LocalTitles {
    /// Waits for the sources, then puts the Local folder's new titles of `kind` in the library (off the GTK thread), so the
    /// pass that follows reads their chapters or episodes too: a launch, the refresh button and the schedule all sync it.
    async fn before_pass(self, kind: Kind) {
        sources_loaded().await;
        let _ = tokio::task::spawn_blocking(move || crate::local_flow::titles::discover_local(&self.db, &self.prefs, kind)).await;
    }
}

/// The application action a second process activates on the primary instance (T-26a-029).
pub const UPDATE_ACTION: &str = "library-update";

/// Starts a manual pass of every kind through `start`, which holds the single-flight lock of the runner: a kind that is
/// already running answers `false`, so a forwarded request never adds a second lock holder (test F-11).
pub fn start_all(start: &dyn Fn(Kind, Option<i64>) -> bool) -> [(Kind, bool); 2] {
    [Kind::Manga, Kind::Anime].map(|kind| (kind, start(kind, None)))
}

/// `app.library-update("manual")`: what `kurobayomi --library-update=manual` asks of the running app (T-26a-029).
fn register_update_action(start: std::rc::Rc<dyn Fn(Kind, Option<i64>) -> bool>) {
    use gtk::prelude::*;
    let Some(app) = gtk::gio::Application::default() else { return };
    let action = gtk::gio::SimpleAction::new(UPDATE_ACTION, Some(gtk::glib::VariantTy::STRING));
    action.connect_activate(move |_, param| {
        if param.and_then(|p| p.get::<String>()).as_deref() == Some("manual") {
            start_all(&*start);
        }
    });
    app.add_action(&action);
}

/// The exit status of a headless run: 0 unless a pass could not read the library. A kind that is already running, a
/// retry, a cancel and a blocked device all end with 0, because the next timer tick tries again (T-26a-020).
fn exit_code(statuses: &[PassStatus]) -> u8 {
    u8::from(statuses.iter().any(|status| matches!(status, PassStatus::Failed(_))))
}

/// One pass per kind over `runner`. A scheduled run checks the device restrictions first.
async fn headless_passes(runner: &Runner, inputs: &Inputs, trigger: Trigger) -> u8 {
    if trigger == Trigger::Auto && !device_allows(inputs).await {
        println!("kurobayomi: the device restrictions are not met, no update ran");
        return 0;
    }
    let mut statuses = Vec::new();
    for kind in [Kind::Manga, Kind::Anime] {
        let status = runner.run(pass_settings(kind, None, inputs), trigger).await.status;
        match &status {
            PassStatus::Skipped => println!("kurobayomi: an update of this kind is already running"),
            PassStatus::Failed(reason) => eprintln!("kurobayomi: the {kind:?} library update failed: {reason}"),
            _ => {}
        }
        statuses.push(status);
    }
    exit_code(&statuses)
}

/// `kurobayomi --library-update=auto` (and the fallback of `=manual` with no app running): one pass per kind over the
/// library without GTK, then the exit status. An app that is open owns the library, so this exits
/// 0 with "already running" (T-26a-020). A headless run has no notifier, so its notices are dropped.
pub fn run_headless(trigger: Trigger) -> u8 {
    let paths = match kuro_platform::system_paths::SystemPaths::system() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("kurobayomi: {error}");
            return 1;
        }
    };
    let library = match crate::backup_run::open_headless(&paths) {
        Ok(library) => library,
        Err(error) if error.to_lowercase().contains("lock") => {
            println!("kurobayomi: already running, the open app owns the library");
            return 0;
        }
        Err(error) => {
            eprintln!("kurobayomi: cannot open the library: {error}");
            return 1;
        }
    };
    let runtime = match kuro_core::runtime::build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("kurobayomi: no runtime: {error}");
            return 1;
        }
    };
    let store = library.prefs.clone();
    let inputs = Inputs {
        download: DownloadPrefs::new(&store),
    };
    let runner = runner_over(library.db, &store);
    runtime.block_on(headless_passes(&runner, &inputs, trigger))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::store::PrefStore;

    fn inputs() -> (PrefStore, Inputs) {
        let store = PrefStore::in_memory();
        (store.clone(), Inputs { download: DownloadPrefs::new(&store) })
    }

    #[test]
    fn a_pass_uses_the_hard_coded_category_and_restriction_settings() {
        let (_store, inputs) = inputs();
        let manga = pass_settings(Kind::Manga, None, &inputs);
        assert!(manga.include_categories.is_empty() && manga.exclude_categories.is_empty(), "all categories");
        assert!(!manga.auto_update_metadata && !manga.hide_content, "cut settings keep their hard-coded values");
    }

    #[test]
    fn a_manual_pass_names_its_category() {
        let (_store, inputs) = inputs();
        let settings = pass_settings(Kind::Manga, Some(5), &inputs);
        assert_eq!(settings.category, Some(5));
        assert_eq!(settings.kind, Kind::Manga);
    }

    #[test]
    fn stopping_before_the_runner_starts_cancels_nothing() {
        assert!(!stop(Kind::Manga));
    }

    #[test]
    fn the_download_policy_follows_the_new_item_toggles_only() {
        // T-26a-090, T-26a-091
        let (_store, inputs) = inputs();
        let off = pass_settings(Kind::Manga, None, &inputs).download;
        assert_eq!(off, DownloadPolicy::default(), "download_new is off by default");
        assert!(!pass_settings(Kind::Anime, None, &inputs).download.enabled);
        inputs.download.download_new_chapters.set(true).unwrap();
        let manga = pass_settings(Kind::Manga, None, &inputs).download;
        assert_eq!(manga, DownloadPolicy { enabled: true, unread_only: false, include_categories: vec![], exclude_categories: vec![] });
        assert!(!pass_settings(Kind::Anime, None, &inputs).download.enabled, "the chapter toggle does not turn anime on");
        inputs.download.download_new_episodes.set(true).unwrap();
        assert!(pass_settings(Kind::Anime, None, &inputs).download.enabled);
    }
    #[test]
    fn t_26a_103_the_updates_button_reaches_the_registered_runner_and_shares_its_single_flight() {
        use crate::updates::library_trigger::{self, Outcome};
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        assert_eq!(library_trigger::trigger(Kind::Anime), Outcome::Refused, "no runner, no start");
        let (busy, calls) = (Rc::new(Cell::new(false)), Rc::new(RefCell::new(Vec::new())));
        let (start_busy, start_calls, running_busy) = (busy.clone(), calls.clone(), busy.clone());
        register_buttons(
            Rc::new(move |kind, category| {
                start_calls.borrow_mut().push((kind, category));
                start_busy.set(true);
                true
            }),
            Rc::new(move |_| running_busy.get()),
        );
        assert_eq!(library_trigger::trigger(Kind::Anime), Outcome::Started, "the page starts a pass of the visible kind");
        assert_eq!(*calls.borrow(), vec![(Kind::Anime, None)], "a whole-library pass, no category");
        assert_eq!(library_trigger::trigger(Kind::Anime), Outcome::AlreadyRunning, "a pass is already running");
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(library_trigger::message(Outcome::Started), "Updating library");
        assert_eq!(library_trigger::message(Outcome::AlreadyRunning), "An update is already running");
    }

    #[test]
    fn the_metadata_passes_do_not_start_before_the_runner_exists() {
        // T-26a-050: the only way to start one is the manual call, and it needs the runner.
        assert!(refresh_library_covers().iter().all(|(_, started)| !started));
    }

    #[test]
    fn t_26a_109_the_cover_refresh_starts_the_anime_metadata_pass_before_the_manga_one() {
        let kinds: Vec<Kind> = refresh_library_covers().iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, vec![Kind::Anime, Kind::Manga]);
    }

    fn conditions(connected: bool) -> kuro_platform::power_network::Conditions {
        kuro_platform::power_network::Conditions { connected, metered: false, on_ac: true, battery_low: false }
    }

    fn wifi_only() -> BTreeSet<String> {
        [kuro_prefs::library::DEVICE_ONLY_ON_WIFI.to_owned()].into()
    }

    #[test]
    fn wifi_only_blocks_a_wired_machine_and_lets_wifi_through() {
        // SC-04
        let wired = kuro_platform::device::DeviceState { on_wifi: false, network_not_metered: true, charging: true };
        assert!(!constraints::satisfied(&wifi_only(), device_facts(conditions(true), wired, true)));
        let wifi = kuro_platform::device::DeviceState { on_wifi: true, ..wired };
        assert!(constraints::satisfied(&wifi_only(), device_facts(conditions(true), wifi, true)));
    }

    #[test]
    fn a_machine_without_network_manager_satisfies_wifi_only() {
        // SC-07
        let nothing = kuro_platform::device::DeviceState::default();
        let facts = device_facts(conditions(true), nothing, false);
        assert_eq!(facts.wifi, None);
        assert!(constraints::satisfied(&wifi_only(), facts));
        assert!(!constraints::satisfied(&wifi_only(), device_facts(conditions(true), nothing, true)), "seen but wired");
    }

    #[test]
    fn no_connection_blocks_whatever_the_restrictions() {
        let nothing = kuro_platform::device::DeviceState::default();
        assert!(!constraints::satisfied(&BTreeSet::new(), device_facts(conditions(false), nothing, false)));
    }

    #[tokio::test]
    async fn an_unknown_source_gives_the_stub_slot_and_a_missing_source_error() {
        use crate::library_update::error::UpdateError;
        use crate::library_update::source::SourceSlot;
        let slot = LiveRegistry.get_or_stub(Kind::Manga, 9);
        assert!(matches!(slot, SourceSlot::Missing));
        assert_eq!(slot.fetch(Kind::Manga, "/m", "M", false).await, Err(UpdateError::SourceNotInstalled));
    }

    #[test]
    fn a_forwarded_manual_request_starts_each_kind_once_and_a_second_one_adds_no_lock_holder() {
        // T-26a-029, test F-11: the closure stands for the runner's single-flight `start_now`
        let holders = RefCell::new(Vec::new());
        let start = |kind: Kind, category: Option<i64>| {
            assert_eq!(category, None, "a forwarded request updates the whole library");
            let mut holders = holders.borrow_mut();
            let free = !holders.contains(&kind);
            if free {
                holders.push(kind);
            }
            free
        };
        assert_eq!(start_all(&start), [(Kind::Manga, true), (Kind::Anime, true)]);
        assert_eq!(start_all(&start), [(Kind::Manga, false), (Kind::Anime, false)]);
        assert_eq!(holders.borrow().len(), 2);
    }

    #[test]
    fn a_headless_run_fails_only_when_the_library_could_not_be_read() {
        // T-26a-020: a busy kind, a cancel and a retry all exit 0
        assert_eq!(exit_code(&[PassStatus::Success, PassStatus::Success]), 0);
        assert_eq!(exit_code(&[PassStatus::Skipped, PassStatus::Retry, PassStatus::Cancelled]), 0);
        assert_eq!(exit_code(&[PassStatus::Success, PassStatus::Failed("no library".into())]), 1);
        assert_eq!(exit_code(&[]), 0);
    }
}
