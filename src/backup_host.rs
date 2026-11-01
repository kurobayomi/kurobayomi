//! The GUI half of the backup host (T-22a-041, T-22a-046, T-21a-019, T-21a-043): it registers the [`BackupHost`] the
//! Create and Restore pages start their runs through, shows the restore banner and notifications, and owns the one
//! stop path of a running restore. The work itself is in `backup_run`.

use crate::ui::DialogOpen as _;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_backup::job::{JobEvent, start_manual_report};
use kuro_backup::notify;
use kuro_backup::options::BackupOptions;
use kuro_backup::restore::job::RestoreEnd;
use kuro_backup::restore::options::RestoreOptions;
use kuro_backup::restore::progress::ProgressEvent;
use kuro_backup::restore::screen::{CLOSE_CHOICES, CloseAction, CloseChoice, StopFlag, on_close_choice, on_close_requested};
use kuro_backup::schema::Backup;
use kuro_backup::text::{self, Text};
use kuro_i18n::{MsgKey, tr};
use kuro_platform::notifier::Notifier;
use kuro_prefs::source::SourcePrefs;
use kuro_platform::paths::Paths as _;

use crate::backup_run::{LegacyImport, Library, run_restore};
use crate::gio_notifier::GioNotifier;
use crate::pref_model::backup_pages::{self, BackupHost};
use crate::pref_model::net::BlockingHttp;
use crate::ui::{DialogButton, Role};

/// The action both Cancel buttons call: the banner and the notification (T-22a-046).
const CANCEL_ACTION: &str = "cancel-restore";

/// The translation of a backup text: the catalogue's when it has the key, the English default when it does not.
pub fn shown(t: Text) -> String {
    MsgKey::ALL.iter().find(|k| k.id() == t.key).map_or_else(|| t.english.to_owned(), |k| tr(*k))
}

/// A restore that is running. The stop flag is the one the job reads at each check point.
struct Session {
    stop: StopFlag,
    /// Kept while the window is closed so the process lives until the job ends.
    hold: Option<gio::ApplicationHoldGuard>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The window banner and its text, set by [`attach`].
    static BANNER: RefCell<Option<(gtk::Revealer, gtk::Label)>> = const { RefCell::new(None) };
}

/// Shows the banner with `text`, or hides it.
fn set_banner(text: Option<&str>) {
    BANNER.with(|b| {
        if let Some((revealer, label)) = b.borrow().as_ref() {
            if let Some(text) = text {
                label.set_text(text);
            }
            revealer.set_reveal_child(text.is_some());
        }
    });
}

/// True while a restore runs.
pub fn restore_running() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}

/// The one stop path of a restore: the close dialog, the notification Cancel and the banner Cancel all end here.
pub fn request_stop() {
    SESSION.with(|s| {
        if let Some(session) = s.borrow().as_ref() {
            session.stop.request();
        }
    });
}

enum Msg {
    Progress(ProgressEvent),
    Done(Result<RestoreEnd, String>),
}

/// Registers the host, the Cancel action and the close handler of `win`. Returns the banner that goes at the top of the
/// window, hidden until a restore starts. Call once from the GTK main thread.
pub fn attach(app: &gtk::Application, win: &gtk::ApplicationWindow) -> gtk::Widget {
    let label = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let cancel = gtk::Button::builder().label(tr(MsgKey::action_cancel)).css_classes(["flat"]).build();
    cancel.set_action_name(Some(&format!("app.{CANCEL_ACTION}")));
    let bar = gtk::Box::builder().spacing(12).css_classes(["kb-banner"]).build();
    bar.append(&crate::ui::loading(crate::ui::LoadSize::Inline));
    bar.append(&label);
    bar.append(&cancel);
    let banner = gtk::Revealer::builder().child(&bar).transition_type(gtk::RevealerTransitionType::SlideDown).build();

    let action = gio::SimpleAction::new(CANCEL_ACTION, None);
    action.connect_activate(|_, _| request_stop());
    app.add_action(&action);

    let notifier = GioNotifier::new(app);
    BANNER.with(|b| *b.borrow_mut() = Some((banner.clone(), label)));
    let ctx = Rc::new(Ctx { notifier });
    backup_pages::set_host(BackupHost {
        start_backup: Box::new({
            let ctx = ctx.clone();
            move |path, options| ctx.start_backup(path, options)
        }),
        start_restore: Box::new({
            let ctx = ctx.clone();
            move |file, backup, options| ctx.start_restore(file, backup, options)
        }),
    });

    win.connect_close_request(|win| {
        if on_close_requested(restore_running()) == CloseAction::Close {
            return glib::Propagation::Proceed;
        }
        ask_before_closing(win);
        glib::Propagation::Stop
    });
    banner.upcast()
}

struct Ctx {
    notifier: GioNotifier,
}

/// The library and preferences a run reads and writes.
fn library() -> Library {
    let app = crate::data::app();
    let paths = kuro_platform::system_paths::SystemPaths::system().expect("the app directories resolve");
    Library { prefs: app.base.store().clone(), db: app.db.clone(), extensions_dir: paths.extensions_dir() }
}

impl Ctx {
    fn start_backup(&self, path: PathBuf, options: BackupOptions) {
        let library = library();
        let (tx, rx) = async_channel::unbounded();
        let notifier: Arc<dyn Notifier> = Arc::new(self.notifier);
        let target = kuro_backup::file::Target::Manual(path);
        let started = start_manual_report(backup_pages::job_slot(), notifier, tx, move || library.create(&options, &target));
        match started {
            Ok(()) => {
                // the result notice is the job's; the page only needs the event loop drained
                glib::spawn_future_local(async move { while rx.recv().await.is_ok_and(|e| !matches!(e, JobEvent::Finished(_))) {} });
            }
            Err(e) => crate::toast::notify(e.user_text()),
        }
    }

    fn start_restore(&self, file: PathBuf, backup: Backup, options: RestoreOptions) {
        if restore_running() {
            crate::toast::notify(shown(text::RESTORE_IN_PROGRESS));
            return;
        }
        tracing::info!(file = %file.display(), "restore started");
        let stop = StopFlag::default();
        SESSION.with(|s| {
            *s.borrow_mut() = Some(Session { stop: stop.clone(), hold: None });
        });
        set_banner(Some(&shown(text::RESTORING_BACKUP)));
        let library = library();
        let (tx, rx) = async_channel::unbounded();
        let worker_tx = tx.clone();
        let spawned = std::thread::Builder::new().name("kuro-restore".into()).spawn(move || {
            let result = restore_on_worker(&library, backup, options, &stop, &worker_tx);
            let _ = worker_tx.send_blocking(Msg::Done(result));
        });
        if let Err(e) = spawned {
            tracing::error!("cannot start the restore thread: {e}");
            finish(&self.notifier, &Err(e.to_string()));
            return;
        }
        let notifier = self.notifier;
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Msg::Progress(event) => {
                        // progress stays in the banner: a desktop notice per title piles up (the freedesktop backend
                        // cannot replace a notice whose first send is still in flight); only the result notifies
                        set_banner(Some(&format!("{} ({}/{})", event.title, event.done, event.total)));
                    }
                    Msg::Done(result) => {
                        finish(&notifier, &result);
                        break;
                    }
                }
            }
        });
    }
}

fn restore_on_worker(
    library: &Library,
    backup: Backup,
    options: RestoreOptions,
    stop: &StopFlag,
    tx: &async_channel::Sender<Msg>,
) -> Result<RestoreEnd, String> {
    let paths = kuro_platform::system_paths::SystemPaths::system().map_err(|e| e.to_string())?;
    let http = BlockingHttp::shared();
    let legacy = LegacyImport { links: SourcePrefs::new(library.prefs.clone()).manga_extension_repos, repo_http: &*http };
    run_restore(library, &paths.data_dir(), &paths.log_dir(), backup, options, stop, &mut |e| drop(tx.send_blocking(Msg::Progress(e))), Some(legacy))
}

/// The end of a restore: the progress notice goes, the result notice shows, the banner hides and the process hold is
/// released.
fn finish(notifier: &GioNotifier, result: &Result<RestoreEnd, String>) {
    notifier.withdraw(notify::ID_RESTORE_PROGRESS);
    match result {
        Ok(end) => notify::restore_end(end).show(notifier),
        Err(reason) => notify::restore_failed(reason).show(notifier),
    }
    set_banner(None);
    // dropping the session releases the process hold of a closed window
    drop(SESSION.with(|s| s.borrow_mut().take()));
}

/// The close dialog of a window that is closing while a restore runs: Keep running or Cancel restore and close. Both
/// close the window; Cancel also asks the job to stop (T-22a-046).
fn ask_before_closing(win: &gtk::ApplicationWindow) {
    debug_assert_eq!(CLOSE_CHOICES.len(), 2);
    let owner = win.clone();
    let answer = Rc::new(move |choice: CloseChoice| {
        let stop = SESSION.with(|s| s.borrow().as_ref().map(|session| session.stop.clone())).unwrap_or_default();
        if on_close_choice(choice, &stop) == CloseAction::Close {
            // the process lives until the job has ended, whichever button was pressed
            SESSION.with(|s| {
                if let Some(session) = s.borrow_mut().as_mut() {
                    session.hold = owner.application().map(|app| app.hold());
                }
            });
            owner.destroy();
        }
    });
    let (cancel_and_close, keep) = (Rc::clone(&answer), Rc::clone(&answer));
    // Keep running is the cancel button: Escape and the close button pick it too
    let buttons = vec![
        DialogButton::cancel(shown(text::KEEP_RUNNING)),
        DialogButton::new(shown(text::CANCEL_AND_CLOSE), Role::Normal, move || cancel_and_close(CloseChoice::CancelAndClose)),
    ];
    let title = shown(text::RESTORING_BACKUP);
    crate::ui::alert(&title, "", buttons, move || keep(CloseChoice::KeepRunning)).open_over(win);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_looked_up_in_the_catalogue_and_keeps_its_english_default_without_it() {
        assert_eq!(shown(text::RESTORING_BACKUP), "Restoring backup");
        assert_eq!(shown(text::KEEP_RUNNING), "Keep running", "a Kurobayomi-only key is in the catalogue too");
        assert_eq!(shown(text::LABEL_LIBRARY), "Library");
        assert_eq!(shown(Text { key: "no_such_key", english: "Fallback" }), "Fallback");
    }

    #[test]
    fn the_stop_request_reaches_only_a_running_session() {
        request_stop();
        assert!(!restore_running());
        let stop = StopFlag::default();
        SESSION.with(|s| *s.borrow_mut() = Some(Session { stop: stop.clone(), hold: None }));
        assert!(restore_running());
        request_stop();
        assert!(stop.requested());
        SESSION.with(|s| *s.borrow_mut() = None);
        assert!(!restore_running());
    }
}
