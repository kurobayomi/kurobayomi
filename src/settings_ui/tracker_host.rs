//! The tracker services behind the Tracking screen (tasks T-47a-055 to T-47a-059): the eleven trackers in one
//! `TrackerManager` over the app's secret store, the pending browser logins, and the arrival of a login redirect.
//!
//! The secret store is opened off the GTK thread (its first open probes the keyring and can take seconds), then the
//! trackers are built on the GTK thread inside the app's runtime, which the manager's watcher tasks need.
//!
//! Decisions:
//! - Shikimori and Bangumi read their OAuth client from `config.toml`; without one their rows say they are not
//!   configured. MyAnimeList uses the app's built-in client id unless the config file or `KUROBAYOMI_MAL_CLIENT_ID`
//!   names another; its login comes back to a loopback listener (`http://localhost:8765/myanimelist-auth`).
//! - The enhanced trackers see no installed-source directory yet (they are built over an empty one), so they cannot
//!   start a sync; this screen only needs their names and accepted source identities.
//! - The shell hands `kurobayomi://` login links to [`on_redirect`]. A login is pending in this process
//!   only: a link that arrives in a process that did not start the login has no attempt to finish and is dropped.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use kuro_platform::paths::Paths;
use kuro_platform::secrets::{SecretStore, SecretsMode};
use kuro_track::bangumi::BangumiTracker;
use kuro_track::connectivity::Connectivity;
use kuro_track::ctx::{BindingLocks, RefreshLimit, TrackCtx};
use kuro_track::events::EventBus;
use kuro_track::kitsu::KitsuTracker;
use kuro_track::login_rows::LOGIN_ROWS;
use kuro_track::mal::error::MalError;
use kuro_track::mal::login::LoginDialog;
use kuro_track::mal::redirect::DEFAULT_PORT;
use kuro_track::mal::store::{SecretTokenStore, TokenStore};
use kuro_track::mal::tracker::MalTracker;
use kuro_track::manager::TrackerManager;
use kuro_track::mangaupdates::MangaUpdatesTracker;
use kuro_track::repo::{SharedHistory, SharedItems, SharedTracks};
use kuro_track::messages::{english, MsgKey};
use kuro_track::model::TrackerId;
use kuro_track::oauth::{handle_redirect, Handled, PendingLogins, PENDING_TTL_MS};
use kuro_track::prefs::TrackPrefs;
use kuro_track::queue::retry::RetryScheduler;
use kuro_track::queue::RetryQueue;
use kuro_track::shikimori::config::TrackClientConfig;
use kuro_track::shikimori::registry::{BANGUMI_REDIRECT, SHIKIMORI_REDIRECT};
use kuro_track::shikimori::ShikimoriTracker;
use kuro_track::simkl::SimklTracker;
use kuro_track::tracker::Tracker;
use kuro_track::trackers::enhanced::FlagStore;
use kuro_track::trackers::jellyfin::JellyfinTracker;
use kuro_track::trackers::kavita::KavitaTracker;
use kuro_track::trackers::komga::KomgaTracker;
use kuro_track::trackers::secret_migration::UNENCRYPTED_WARNING;
use kuro_track::trackers::sources::{SourceDirectory, SourcePrefs};
use kuro_track::trackers::suwayomi::SuwayomiTracker;

/// The desktop's network state (T-35a-057). GIO reports link availability, not internet reachability.
struct GioNetwork;

#[async_trait::async_trait]
impl Connectivity for GioNetwork {
    fn is_online(&self) -> bool {
        use gtk::gio::prelude::NetworkMonitorExt as _;
        gtk::gio::NetworkMonitor::default().is_network_available()
    }
}

/// The services of the Tracking screen. AniList and MyAnimeList are switched on in this build: the other trackers are
/// built (the registry holds all eleven) but no screen shows them and no session reads them.
pub struct Host {
    pub manager: Arc<TrackerManager>,
    /// The MyAnimeList service as itself: its loopback login is not part of the `Tracker` trait.
    pub mal: Arc<MalTracker>,
    pub pending: RefCell<PendingLogins>,
    /// Browser logins whose redirect has not arrived, with the time (ms) each one began.
    waiting: RefCell<HashMap<TrackerId, i64>>,
    /// True when no keyring is reachable and the secrets live in the fallback file.
    pub fallback: bool,
    /// The tracking flows (progress, bind, edit, remove, retry queue) over the registry and the app databases.
    pub ctx: Arc<TrackCtx>,
}

impl Host {
    /// Marks `tracker` as waiting for its browser redirect since `now_ms`.
    pub fn begin_waiting(&self, tracker: TrackerId, now_ms: i64) {
        self.waiting.borrow_mut().insert(tracker, now_ms);
    }

    /// Whether `tracker` waits for a redirect at `now_ms`: it began less than the attempt's lifetime ago.
    pub fn is_waiting(&self, tracker: TrackerId, now_ms: i64) -> bool {
        self.waiting.borrow().get(&tracker).is_some_and(|started| now_ms - started <= PENDING_TTL_MS)
    }

    pub fn stop_waiting(&self, tracker: TrackerId) {
        self.waiting.borrow_mut().remove(&tracker);
    }
}

enum State {
    Idle,
    Loading,
    Ready(Rc<Host>),
    Failed,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::Idle) };
    static REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static WAITING: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// Runs `then` once the services are built: at once when they already are. Dropped when building fails.
pub fn when_ready(then: impl FnOnce() + 'static) {
    if host().is_some() {
        return then();
    }
    WAITING.with(|waiting| waiting.borrow_mut().push(Box::new(then)));
    ensure();
}

/// The services, once they are built.
pub fn host() -> Option<Rc<Host>> {
    STATE.with(|state| match &*state.borrow() {
        State::Ready(host) => Some(Rc::clone(host)),
        _ => None,
    })
}

/// The tracking flows, once the services are built. `None` before that, which is what a user with no tracker gets.
pub fn ctx() -> Option<Arc<TrackCtx>> {
    host().map(|host| Arc::clone(&host.ctx))
}

/// The page that redraws itself when a login finishes. The last page to register wins.
pub fn set_refresh(redraw: Rc<dyn Fn()>) {
    REFRESH.with(|slot| *slot.borrow_mut() = Some(redraw));
}

/// Redraws the page that registered itself with [`set_refresh`], if one is still alive.
pub fn refresh() {
    if let Some(redraw) = REFRESH.with(|slot| slot.borrow().clone()) {
        redraw();
    }
}

/// The text of the warning row: shown while the secrets are in the fallback file instead of the keyring.
pub fn keyring_warning(mode: SecretsMode) -> Option<&'static str> {
    (mode == SecretsMode::EncryptedFile).then_some(UNENCRYPTED_WARNING)
}

/// Starts building the services once. The page that registered itself with [`set_refresh`] redraws when they are there
/// (or when building failed).
pub fn ensure() {
    let start = STATE.with(|state| {
        let mut state = state.borrow_mut();
        matches!(*state, State::Idle).then(|| *state = State::Loading).is_some()
    });
    if !start {
        return;
    }
    gtk::glib::spawn_future_local(async move {
        let opened = gtk::gio::spawn_blocking(open_store).await;
        let built = match opened {
            Ok(Ok((secrets, config_dir))) => build(secrets, &config_dir),
            Ok(Err(err)) => Err(err),
            Err(_) => Err("the secret store task panicked".to_owned()),
        };
        STATE.with(|state| {
            *state.borrow_mut() = match built {
                Ok(host) => State::Ready(Rc::new(host)),
                Err(err) => {
                    tracing::warn!(%err, "the tracker services did not start");
                    State::Failed
                }
            }
        });
        let waiting = WAITING.with(|waiting| std::mem::take(&mut *waiting.borrow_mut()));
        if host().is_some() {
            waiting.into_iter().for_each(|then| then());
        }
        refresh();
    });
}

fn open_store() -> Result<(Arc<kuro_platform::secrets::FallbackSecrets>, std::path::PathBuf), String> {
    let paths = kuro_platform::system_paths::SystemPaths::system().map_err(|err| err.to_string())?;
    Ok((Arc::new(kuro_platform::secrets::open(&paths)), paths.config_dir()))
}

/// No installed-source directory: the enhanced trackers are built over an empty one.
struct NoSources;

impl SourceDirectory for NoSources {
    fn source_prefs(&self, _source_id: i64) -> Option<Arc<dyn SourcePrefs>> {
        None
    }
}

fn build(secrets: Arc<kuro_platform::secrets::FallbackSecrets>, config_dir: &std::path::Path) -> Result<Host, String> {
    let handle = crate::shell::host::Shell::get().map(|shell| shell.runtime().clone()).ok_or("no runtime")?;
    let _entered = handle.enter();
    let fallback = secrets.mode() == SecretsMode::EncryptedFile;
    let store: Arc<dyn SecretStore> = secrets;
    let events = EventBus::default();
    let app = crate::data::app();
    let prefs = app.base.store().clone();
    let flags: Arc<dyn FlagStore> = Arc::new(prefs.clone());
    let dir: Arc<dyn SourceDirectory> = Arc::new(NoSources);
    let clients = TrackClientConfig::load(config_dir);
    let text = |err: &dyn std::fmt::Display| err.to_string();

    let tokens: Arc<dyn TokenStore> = Arc::new(SecretTokenStore::new(Arc::clone(&store)));
    let mal = Arc::new(MalTracker::from_config(config_dir, tokens).map_err(|e| text(&e))?);
    let anilist = Arc::new(kuro_track::tracker::anilist::AniList::with_http(Arc::clone(&store), &prefs, events.clone()).map_err(|e| text(&e))?);
    let kitsu = KitsuTracker::new(kuro_track::kitsu::api::Endpoints::production(), Arc::clone(&store)).map_err(|e| text(&e))?;
    let shikimori = Arc::new(ShikimoriTracker::production(clients.shikimori.clone(), SHIKIMORI_REDIRECT, Arc::clone(&store)).map_err(|e| text(&e))?);
    let bangumi = Arc::new(BangumiTracker::production(clients.bangumi.clone(), BANGUMI_REDIRECT, Arc::clone(&store)).map_err(|e| text(&e))?);
    let mangaupdates = MangaUpdatesTracker::new(Arc::clone(&store), events.clone()).map_err(|e| text(&e))?;
    let simkl = Arc::new(SimklTracker::new(Arc::clone(&store), events.clone()).map_err(|e| text(&e))?);
    let komga = KomgaTracker::new(Arc::clone(&store), events.clone()).map_err(|e| text(&e))?;
    let kavita = KavitaTracker::new(Arc::clone(&dir), Arc::clone(&flags)).map_err(|e| text(&e))?;
    let suwayomi = SuwayomiTracker::new(Arc::clone(&dir), Arc::clone(&flags)).map_err(|e| text(&e))?;
    let jellyfin = JellyfinTracker::new(dir, flags).map_err(|e| text(&e))?;

    let all: Vec<Arc<dyn Tracker>> = vec![
        Arc::clone(&mal) as Arc<dyn Tracker>,
        anilist,
        Arc::new(kitsu),
        shikimori,
        bangumi,
        Arc::new(komga),
        Arc::new(mangaupdates),
        Arc::new(kavita),
        Arc::new(suwayomi),
        simkl,
        Arc::new(jellyfin),
    ];
    let manager = Arc::new(TrackerManager::new(all).map_err(|e| text(&e))?);

    // The queue lives in the anime database (its rows carry the kind, so both kinds share it).
    let queue = Arc::new(RetryQueue::new(Arc::clone(&app.db)).map_err(|e| text(&e))?);
    let net: Arc<dyn Connectivity> = Arc::new(GioNetwork);
    let scheduler = Arc::new(RetryScheduler::new(Arc::clone(&queue), Arc::clone(&net)));
    let ctx = Arc::new(TrackCtx {
        trackers: Arc::clone(&manager),
        anime_tracks: Arc::new(SharedTracks::new(Arc::clone(&app.db))),
        manga_tracks: Arc::new(SharedTracks::new(Arc::clone(&app.db))),
        anime_episodes: Arc::new(SharedItems::new(Arc::clone(&app.db))),
        manga_chapters: Arc::new(SharedItems::new(Arc::clone(&app.db))),
        anime_history: Arc::new(SharedHistory::new(Arc::clone(&app.db))),
        manga_history: Arc::new(SharedHistory::new(Arc::clone(&app.db))),
        queue,
        scheduler,
        net,
        prefs: Arc::new(TrackPrefs::new(prefs)),
        locks: BindingLocks::default(),
        refresh_limit: RefreshLimit::default(),
        events,
    });
    // The retry loop runs for the life of the app, and the binders of the add flow reach the same context.
    let _ = ctx.start_retry();
    crate::updates::track_binding::install(Arc::clone(&ctx));
    Ok(Host { manager, mal, pending: RefCell::default(), waiting: RefCell::default(), fallback, ctx })
}

/// The time `PendingLogins` measures attempts by.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// What finishing a login shows: the toast text.
fn finished(name: &str, result: Result<(), kuro_track::error::TrackError>) -> String {
    match result {
        Ok(()) => english(&MsgKey::TrackerLoggedIn(name.to_owned())),
        Err(err) => english(&kuro_track::dialog::failure_message(&err)),
    }
}

/// Starts the MyAnimeList login: binds the loopback listener, opens MAL's authorise page in the browser, and finishes
/// the login when MAL redirects back to it. The row shows its waiting state meanwhile; `changed` redraws it when the
/// login starts and when it ends, and the outcome is a toast.
pub fn start_mal_login(host: &Rc<Host>, changed: Rc<dyn Fn()>) {
    // a login that still waits holds the port and the verifier: a second start would break it
    if host.is_waiting(TrackerId::MYANIMELIST, now_ms()) {
        return crate::toast::notify(kuro_track::mal::messages::WAITING_FOR_LOGIN.to_owned());
    }
    let mal = Arc::clone(&host.mal);
    let host = Rc::clone(host);
    crate::source_bridge::call_on_main(
        {
            let mal = Arc::clone(&mal);
            async move { LoginDialog::open(&mal, DEFAULT_PORT).await }
        },
        move |opened| {
            let mut login = match opened {
                Ok(login) => login,
                Err(err) => return crate::toast::notify(finished(kuro_track::mal::tracker_impl::NAME, Err(err.into()))),
            };
            if !login.redirect_listening() {
                login.cancel(&mal);
                return crate::toast::notify(kuro_track::mal::messages::PORT_BUSY.to_owned());
            }
            host.begin_waiting(TrackerId::MYANIMELIST, now_ms());
            crate::ui::open_link(login.authorize_url(), None);
            changed();
            crate::source_bridge::call_on_main(
                async move { login.wait_redirect(&mal).await.unwrap_or(Err(MalError::LoginTimedOut)) },
                move |result| {
                    host.stop_waiting(TrackerId::MYANIMELIST);
                    crate::toast::notify(finished(kuro_track::mal::tracker_impl::NAME, result.map_err(Into::into)));
                    changed();
                },
            );
        },
    );
}

/// The trackers whose browser login [`on_redirect`] finishes: the link's credential (AniList's token, Shikimori's code)
/// is the password of `Tracker::login`.
const REDIRECT_LOGINS: [TrackerId; 2] = [TrackerId::ANILIST, TrackerId::SHIKIMORI];

/// A login link arrived (`kurobayomi://anilist-auth#access_token=...`, `kurobayomi://shikimori-auth?code=...`). Routes it
/// to the pending attempt and finishes that tracker's login on the runtime; the toast and the redraw follow on the main
/// context. False when the link is not for such a login in this run (the services are not built, or no attempt is
/// waiting), so the caller can treat the link as not its own. The tracker's login stores the token itself, through the
/// secret store.
pub fn on_redirect(uri: &str) -> bool {
    let Some(host) = host() else { return false };
    let handled = handle_redirect(&mut host.pending.borrow_mut(), &host.manager, uri, now_ms());
    match handled {
        Handled::Ignored => {}
        Handled::Toast(key) => {
            for row in LOGIN_ROWS {
                host.stop_waiting(row.tracker);
            }
            crate::toast::notify(english(&key));
            refresh();
        }
        Handled::Exchange(credential) if REDIRECT_LOGINS.contains(&credential.tracker) => {
            let id = credential.tracker;
            let Some(tracker) = host.manager.get(id) else { return false };
            let host_for_task = Rc::clone(&host);
            let value = credential.value;
            crate::source_bridge::call_on_main(
                {
                    let tracker = Arc::clone(&tracker);
                    async move { tracker.login("", &value).await }
                },
                move |result| {
                    host_for_task.stop_waiting(id);
                    crate::toast::notify(finished(tracker.name(), result));
                    refresh();
                },
            );
        }
        // The other trackers are switched off, so their redirects are not finished here.
        Handled::Exchange(_) | Handled::SimklRedirect(_) => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_row_shows_only_when_the_secrets_are_in_the_fallback_file() {
        assert_eq!(keyring_warning(SecretsMode::Primary), None);
        assert_eq!(keyring_warning(SecretsMode::EncryptedFile), Some(UNENCRYPTED_WARNING));
    }

    #[test]
    fn a_finished_login_toasts_the_tracker_or_the_failure() {
        assert_eq!(finished("Shikimori", Ok(())), "Shikimori logged in");
        let failed = finished("Shikimori", Err(kuro_track::error::TrackError::Message("bad code".into())));
        assert!(failed.contains("bad code"), "{failed}");
    }

    #[test]
    fn without_services_a_link_is_not_ours() {
        assert!(!on_redirect("aniyomi://anilist-auth#access_token=t"));
    }

    #[test]
    fn without_services_there_is_no_tracking_context() {
        assert!(ctx().is_none());
    }
}
