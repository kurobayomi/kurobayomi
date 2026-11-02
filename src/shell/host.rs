//! The shell object (T-36a-001): one per process on the GTK thread. It owns the tokio runtime handle that `main`
//! creates once, the queue of launch requests that arrive before the window exists, the home tab selection that
//! survives layout changes, and the one download service.

use crate::ui::DialogOpen as _;
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use kuro_downloads::anime::lifecycle::CLOSE_PROMPT;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::notifier::Notifier;
use kuro_platform::paths::Paths;
use kuro_prefs::ui::StartScreen;

use super::argv::LaunchRequest;
use super::bootstrap::Bootstrap;
use super::lifecycle::Lifecycle;
use super::tabs::HomeTabs;
use crate::ui::{DialogButton, Role};

pub struct Shell {
    runtime: tokio::runtime::Handle,
    boot: RefCell<Bootstrap<LaunchRequest>>,
    home: RefCell<Option<Rc<RefCell<HomeTabs>>>>,
    /// The one download service of both kinds, once [`Shell::start_downloads`] has started it.
    download_service: RefCell<Option<crate::downloads::Service>>,
}

thread_local! {
    static SHELL: OnceCell<Rc<Shell>> = const { OnceCell::new() };
}

impl Shell {
    /// Creates the shell. Call once from `main` on the GTK thread, right after the runtime is built. A second call
    /// returns the first shell.
    pub fn install(runtime: tokio::runtime::Handle, paths: &dyn Paths) -> Rc<Shell> {
        SHELL.with(|cell| {
            cell.get_or_init(|| {
                let index_cache = paths.download_index_cache_dir();
                if let Err(e) = std::fs::create_dir_all(&index_cache) {
                    tracing::warn!("the download index snapshot folder was not created: {e}");
                }
                Rc::new(Shell { runtime, boot: RefCell::new(Bootstrap::new()), home: RefCell::new(None), download_service: RefCell::new(None) })
            })
            .clone()
        })
    }

    /// The window's first step (Kotlin `BaseActivity.onCreate`): runs `apply_theme`, then records it, so the first
    /// widget is built under the theme. The returned [`Lifecycle`] refuses `build_widget` if the theme did not run.
    pub fn build(apply_theme: impl FnOnce()) -> Lifecycle {
        let mut life = Lifecycle::new();
        apply_theme();
        life.apply_theme();
        life
    }

    /// The installed shell, or `None` before [`Shell::install`] (unit tests).
    pub fn get() -> Option<Rc<Shell>> {
        SHELL.with(|cell| cell.get().cloned())
    }

    /// The runtime every background task of the app runs on.
    pub fn runtime(&self) -> &tokio::runtime::Handle {
        &self.runtime
    }

    /// A launch request arrived. `Some` means the window exists and the caller applies it now; `None` means it is
    /// queued until [`Shell::window_built`].
    pub fn submit(&self, request: LaunchRequest) -> Option<LaunchRequest> {
        self.boot.borrow_mut().request(request)
    }

    /// The window is built: the queued requests, in arrival order.
    pub fn window_built(&self) -> Vec<LaunchRequest> {
        self.boot.borrow_mut().window_built()
    }

    /// The migrations finished: the library page may leave its loading state (T-53a-069). The window is built only
    /// after the data opens, so `main` calls this when it starts building it.
    pub fn migration_done(&self) {
        self.boot.borrow_mut().migration_done();
    }

    #[cfg(test)]
    /// What the library page shows now.
    pub fn library_state(&self) -> super::bootstrap::LibraryState {
        self.boot.borrow().library_state()
    }

    /// Starts the one download service on the data the app has opened (call once, after the migrations); its notices go
    /// to `notifier`. A failure is logged and the app runs without downloads. The first call wins.
    pub fn start_downloads(&self, notifier: Arc<dyn Notifier>) {
        if self.download_service.borrow().is_some() {
            return;
        }
        let _entered = self.runtime.enter();
        let paths = match kuro_platform::system_paths::SystemPaths::system() {
            Ok(paths) => paths,
            Err(e) => {
                tracing::error!("the download service did not start: {e}");
                return;
            }
        };
        let app = crate::data::app();
        match crate::downloads::start_service(&paths, Arc::clone(&app.db), app.base.store().clone(), notifier) {
            Ok(service) => {
                crate::updates::download_host::install(Arc::new(crate::updates::download_host::LiveBackend::new(
                    service.downloads.clone(),
                    Arc::clone(&app.db),
                    self.runtime.clone(),
                )));
                crate::downloads::wire_library_download(Arc::clone(&app.db));
                *self.download_service.borrow_mut() = Some(service);
            }
            Err(e) => tracing::error!("the download service did not start: {e}"),
        }
    }

    /// The one download service of both kinds, or `None` before [`Shell::start_downloads`] has started it.
    pub fn download_service(&self) -> Option<kuro_downloads::service::Downloads> {
        self.download_service.borrow().as_ref().map(|service| service.downloads.clone())
    }

    /// Asks before the main window closes while a lane downloads (D4): the dialog says "Downloads will pause. Quit?" and
    /// Quit closes the window. There is no background process, so the run stops and the saved queue goes on at the next
    /// start. Connect it once to the main window.
    pub fn guard_close(window: &gtk::ApplicationWindow) {
        window.connect_close_request(|win| {
            let running = Shell::get().and_then(|shell| shell.download_service()).is_some_and(|downloads| crate::downloads::any_lane_running(&downloads));
            if !running {
                return gtk::glib::Propagation::Proceed;
            }
            let quit = win.clone();
            let buttons = vec![
                DialogButton::cancel(tr(MsgKey::action_cancel)),
                DialogButton::new(&tr(MsgKey::ui_quit), Role::Destructive, move || quit.destroy()),
            ];
            crate::ui::alert(&tr(MsgKey::ui_downloads_running), CLOSE_PROMPT, buttons, || {}).open_over(win);
            gtk::glib::Propagation::Stop
        });
    }

    /// The home tab selection. It is built once, the first time this runs, so a wide or narrow switch keeps it.
    pub fn home_tabs(&self, start: StartScreen) -> Rc<RefCell<HomeTabs>> {
        self.home.borrow_mut().get_or_insert_with(|| Rc::new(RefCell::new(HomeTabs::new(start)))).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::ui::Tab;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(1).build().unwrap()
    }

    fn shell(rt: &tokio::runtime::Runtime) -> Shell {
        Shell { runtime: rt.handle().clone(), boot: RefCell::new(Bootstrap::new()), home: RefCell::new(None), download_service: RefCell::new(None) }
    }

    #[test]
    fn requests_before_the_window_wait_and_after_it_apply_at_once() {
        let rt = rt();
        let shell = shell(&rt);
        let search = LaunchRequest::OpenUri("a://b".into());
        assert_eq!(shell.submit(search.clone()), None);
        assert_eq!(shell.window_built(), vec![search]);
        let later = LaunchRequest::OpenUri("c://d".into());
        assert_eq!(shell.submit(later.clone()), Some(later));
    }

    #[test]
    fn the_library_loads_until_the_migrations_are_done() {
        let rt = rt();
        let shell = shell(&rt);
        assert_eq!(shell.library_state(), super::super::bootstrap::LibraryState::Loading);
        shell.migration_done();
        assert_eq!(shell.library_state(), super::super::bootstrap::LibraryState::Ready);
    }

    #[test]
    fn one_home_selection_serves_every_layout() {
        let rt = rt();
        let shell = shell(&rt);
        let first = shell.home_tabs(StartScreen::ANIME);
        first.borrow_mut().select(Tab::Browse);
        // a later layout asks again, even with other prefs: it gets the same selection
        let again = shell.home_tabs(StartScreen::MANGA);
        assert!(Rc::ptr_eq(&first, &again));
        assert_eq!(again.borrow().current(), Tab::Browse);
    }

    #[test]
    fn build_applies_the_theme_before_a_widget_may_be_built() {
        let ran = std::cell::Cell::new(false);
        let life = Shell::build(|| ran.set(true));
        assert!(ran.get());
        assert_eq!(life.build_widget(), Ok(()));
    }

    #[test]
    fn the_runtime_handle_runs_tasks() {
        let rt = rt();
        let shell = shell(&rt);
        assert_eq!(shell.runtime().block_on(async { 7 }), 7);
    }
}
