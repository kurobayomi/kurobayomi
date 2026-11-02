//! The migrate dialog of both kinds (`MigrateMangaDialog.kt`, `MigrateAnimeDialog.kt`; T-40a-015, T-40a-045, T-40a-052,
//! T-41a-035, T-41a-041): what to include, and the buttons Show manga or Show anime, Show seasons, Copy and Migrate.
//! `plan` says what the dialog offers for the old and the new title: a title can move only to one that lists its items
//! the same way (episodes to episodes, seasons to seasons; a manga always lists chapters). When the fetch types differ it
//! shows the matching error instead of the options and offers only the Show buttons, and Show seasons is offered unless the
//! new title is an episodes title (behavior 11.1).
//!
//! The options are, in display order (`migrate_flags::options`): the chapters or episodes, categories, custom cover and
//! custom background when the old title has them, and delete downloaded when it has downloads. The dim line under them
//! says that tracking data is always transferred. What Copy and Migrate do is the caller's: it supplies `run`, which
//! reports back when the migration ends. Until then the options sit dimmed under a spinner, the buttons are hidden, a
//! second request is refused and the window refuses to close; a failure keeps the dialog open with its reason.

use std::cell::OnceCell;
use std::rc::Rc;

use adw::prelude::*;
use kuro_i18n::{MsgKey, tr};
use kuro_source::anime::model::FetchType;

use super::dialogs::DialogSlot;
use super::migrate_engine::MigratePlan;
use super::migrate_flags::{self, RunGuard, options, preselected};
use kuro_platform::kind::Kind;
use crate::ui::{DialogButton, DialogOpen as _, Role};
use crate::theme::tokens::{SPACE_S};

/// A button of the dialog, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Show manga or Show anime: the new entry's page.
    ShowEntry,
    ShowSeasons,
    Copy,
    Migrate,
}

impl Action {
    fn label(self, kind: Kind) -> String {
        tr(match (self, kind) {
            (Action::ShowEntry, Kind::Manga) => MsgKey::action_show_manga,
            (Action::ShowEntry, Kind::Anime) => MsgKey::action_show_anime,
            (Action::ShowSeasons, _) => MsgKey::label_show_seasons,
            (Action::Copy, _) => MsgKey::copy,
            (Action::Migrate, _) => MsgKey::migrate,
        })
    }
}

/// What the old and the new title are, which decides what the dialog offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub kind: Kind,
    /// How the old and the new title list their items. A manga is always `Episodes` (a list of chapters).
    pub old: FetchType,
    pub new: FetchType,
    pub custom_cover: bool,
    pub custom_background: bool,
    pub downloads: bool,
}

/// The label of one option bit.
fn option_label(bit: u32, kind: Kind) -> String {
    tr(match bit {
        migrate_flags::ITEMS if kind == Kind::Manga => MsgKey::chapters,
        migrate_flags::ITEMS => MsgKey::episodes,
        migrate_flags::CATEGORIES => MsgKey::categories,
        migrate_flags::CUSTOM_COVER => MsgKey::custom_cover,
        migrate_flags::CUSTOM_BACKGROUND => MsgKey::custom_background,
        _ => MsgKey::delete_downloaded,
    })
}

/// What the dialog shows for an old and a new title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogPlan {
    /// The fetch-type mismatch message, shown in place of the options.
    pub message: Option<String>,
    /// The options with their bit and label.
    pub flags: Vec<(u32, String)>,
    /// The options that start ticked: the offered ones whose bit is set in the saved bitmap.
    pub selected: Vec<u32>,
    pub actions: Vec<Action>,
}

/// The dialog for `facts`: the options when the fetch types match, the error when they do not, the Show button always,
/// Show seasons unless the new title is an episodes title, and Copy and Migrate only when migrating is possible.
pub fn plan(facts: Facts, saved: i32) -> DialogPlan {
    let migrate = facts.old == facts.new;
    let message = (!migrate).then(|| tr(if facts.old == FetchType::Seasons { MsgKey::label_cant_migrate_season } else { MsgKey::label_cant_migrate_episode }));
    // the items option only for an old title that lists episodes (behavior 10)
    let bits = if migrate { options(facts.old == FetchType::Episodes, facts.custom_cover, facts.custom_background, facts.downloads) } else { Vec::new() };
    let selected = preselected(saved, &bits);
    let flags = bits.into_iter().map(|bit| (bit, option_label(bit, facts.kind))).collect();
    let mut actions = vec![Action::ShowEntry];
    if facts.new != FetchType::Episodes {
        actions.push(Action::ShowSeasons);
    }
    if migrate {
        actions.extend([Action::Copy, Action::Migrate]);
    }
    DialogPlan { message, flags, selected, actions }
}

/// How a started run reports back, on the GTK thread: `Err` carries the reason the dialog shows.
pub type Done = Box<dyn FnOnce(Result<(), String>)>;

/// What the buttons do.
pub struct MigrateHandlers {
    /// Show manga, Show anime and Show seasons: the new entry's page, where a seasons title lists its seasons.
    pub show_entry: Rc<dyn Fn()>,
    /// Starts the migration and calls `Done` when it ends. The dialog stays in its running state until then.
    pub run: Rc<dyn Fn(MigratePlan, Done)>,
    /// A migration succeeded and the dialog closed: the caller leaves the migrate page.
    pub finished: Rc<dyn Fn()>,
}

struct Dialog {
    /// Weak: the window owns the dialog (`keep_alive`), so a strong link would be a cycle.
    window: gtk::glib::WeakRef<adw::Dialog>,
    checks: Vec<(u32, gtk::CheckButton)>,
    body: gtk::Box,
    spinner: gtk::Spinner,
    error: gtk::Label,
    row: gtk::Box,
    guard: RunGuard,
    handlers: MigrateHandlers,
}

impl Dialog {
    /// The running state: the options sit under a 70 % cover (30 % opacity), a spinner is over them and the buttons are hidden.
    fn set_running(&self, running: bool) {
        self.body.set_opacity(if running { 0.3 } else { 1.0 });
        self.body.set_sensitive(!running);
        self.spinner.set_visible(running);
        self.row.set_visible(!running);
        // Escape and the close button do nothing while a run goes
        if let Some(window) = self.window.upgrade() {
            window.set_can_close(!running);
        }
    }

    /// Copy (`replace` false) and Migrate (`replace` true). A second request while one runs is refused: the guard is
    /// finished only when the run reports back, not when `run` returns.
    fn start(self: &Rc<Self>, replace: bool) {
        if !self.guard.begin() {
            return;
        }
        self.error.set_visible(false);
        self.set_running(true);
        let flags = self.checks.iter().filter(|(_, check)| check.is_active()).fold(0, |map, (bit, _)| map | bit) as i32;
        let this = Rc::clone(self);
        (self.handlers.run)(
            MigratePlan { replace, flags },
            Box::new(move |result| {
                this.guard.finish();
                this.set_running(false);
                match result {
                    Ok(()) => {
                        if let Some(window) = this.window.upgrade() {
                            window.close();
                        }
                        (this.handlers.finished)();
                    }
                    Err(reason) => {
                        // the dialog stays open, shows why and can be used again
                        this.error.set_label(&super::texts::migration_failed(&reason));
                        this.error.set_visible(true);
                    }
                }
            }),
        );
    }
}

/// The row the shared dialog puts its buttons in: the parent of the first button under `widget`.
fn button_row(widget: &gtk::Widget) -> Option<gtk::Box> {
    if widget.is::<gtk::Button>() {
        return widget.parent().and_downcast();
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        if let Some(found) = button_row(&next) {
            return Some(found);
        }
        child = next.next_sibling();
    }
    None
}

/// Shows the dialog over `anchor`'s window. Ctrl+Enter migrates and Ctrl+Shift+Enter copies when migrating is possible.
/// `saved` is the bitmap the options start from. While a run is going the window refuses to close, so Escape does nothing.
pub fn show(slot: &DialogSlot, anchor: &impl IsA<gtk::Widget>, facts: Facts, saved: i32, handlers: MigrateHandlers) -> adw::Dialog {
    let plan = plan(facts, saved);
    let body = gtk::Box::new(gtk::Orientation::Vertical, SPACE_S);
    let checks: Vec<(u32, gtk::CheckButton)> = plan
        .flags
        .iter()
        .map(|(flag, label)| {
            let check = gtk::CheckButton::with_label(label);
            check.set_active(plan.selected.contains(flag));
            body.append(&check);
            (*flag, check)
        })
        .collect();
    match &plan.message {
        Some(message) => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
            row.append(&gtk::Image::from_icon_name("dialog-error-symbolic"));
            row.append(&gtk::Label::builder().label(message).wrap(true).xalign(0.0).css_classes(["error"]).build());
            body.append(&row);
        }
        None => body.append(&gtk::Label::builder().label(super::texts::migrate_tracks_always()).xalign(0.0).css_classes(["dim-label"]).build()),
    }
    let error = gtk::Label::builder().visible(false).wrap(true).xalign(0.0).max_width_chars(50).css_classes(["error"]).build();
    body.append(&error);
    let spinner = crate::widgets::status::spinner();
    spinner.set_visible(false);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&body));
    overlay.add_overlay(&spinner);

    // the dialog is built after its buttons, which reach it through this cell
    let cell: Rc<OnceCell<Rc<Dialog>>> = Rc::default();
    let buttons = plan
        .actions
        .iter()
        .map(|action| {
            let (label, this) = (action.label(facts.kind), Rc::clone(&cell));
            match action {
                Action::ShowEntry | Action::ShowSeasons => DialogButton::new(label, Role::Normal, move || {
                    if let Some(dialog) = this.get() {
                        (dialog.handlers.show_entry)();
                    }
                }),
                Action::Copy | Action::Migrate => {
                    let replace = *action == Action::Migrate;
                    DialogButton::keep_open_unless(label, if replace { Role::Suggested } else { Role::Normal }, move || {
                        if let Some(dialog) = this.get() {
                            dialog.start(replace);
                        }
                        false
                    })
                }
            }
        })
        .collect();
    let window = crate::ui::dialog(&tr(MsgKey::migration_dialog_what_to_include), &overlay, 420, buttons);
    let row = button_row(&crate::ui::dialog_content(&window).expect("content")).expect("the button row");

    let dialog = Rc::new(Dialog { window: window.downgrade(), checks, body, spinner, error, row, guard: RunGuard::default(), handlers });
    let _ = cell.set(Rc::clone(&dialog));
    slot.adopt(&window);

    if plan.actions.contains(&Action::Migrate) {
        let (to_migrate, to_copy) = (Rc::clone(&dialog), Rc::clone(&dialog));
        super::keys::attach_migrate_keys(&window, move || to_migrate.start(true), move || to_copy.start(false));
    }
    // the dialog lives as long as its window
    super::glue::keep_alive(&window, &dialog);
    window.open_over(anchor);
    window
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    fn facts(kind: Kind, old: FetchType, new: FetchType) -> Facts {
        Facts { kind, old, new, custom_cover: false, custom_background: false, downloads: false }
    }

    #[test]
    fn a_mismatch_shows_the_error_for_the_old_kind_and_only_the_show_buttons() {
        let to_seasons = plan(facts(Kind::Anime, FetchType::Episodes, FetchType::Seasons), i32::MAX);
        assert_eq!(to_seasons.message, Some(tr(MsgKey::label_cant_migrate_episode)), "E41");
        assert_eq!(to_seasons.actions, [Action::ShowEntry, Action::ShowSeasons]);
        assert!(to_seasons.flags.is_empty(), "no options when migrating is off");
        let to_episodes = plan(facts(Kind::Anime, FetchType::Seasons, FetchType::Episodes), i32::MAX);
        assert_eq!(to_episodes.message, Some(tr(MsgKey::label_cant_migrate_season)), "E42");
        assert_eq!(to_episodes.actions, [Action::ShowEntry], "an episodes title has no Show seasons");
        let seasons = plan(facts(Kind::Anime, FetchType::Seasons, FetchType::Seasons), i32::MAX);
        assert_eq!((seasons.message, seasons.actions), (None, vec![Action::ShowEntry, Action::ShowSeasons, Action::Copy, Action::Migrate]));
        assert!(!seasons.flags.iter().any(|(bit, _)| *bit == migrate_flags::ITEMS), "the episodes option only for an episodes title");
    }

    #[test]
    fn the_options_follow_the_old_title_and_start_from_the_saved_bitmap() {
        let all = Facts { custom_cover: true, custom_background: true, downloads: true, ..facts(Kind::Anime, FetchType::Episodes, FetchType::Episodes) };
        let plan_all = plan(all, i32::MAX);
        assert_eq!(
            plan_all.flags.iter().map(|(flag, _)| *flag).collect::<Vec<_>>(),
            [migrate_flags::ITEMS, migrate_flags::CATEGORIES, migrate_flags::CUSTOM_COVER, migrate_flags::CUSTOM_BACKGROUND, migrate_flags::DELETE_DOWNLOADED],
            "T-41a-032: the five options in order"
        );
        assert_eq!(plan_all.selected.len(), 5, "the saved default selects all five");
        assert_eq!(plan(facts(Kind::Anime, FetchType::Episodes, FetchType::Episodes), i32::MAX).flags.len(), 2, "cover, background and delete only when they apply");
        let saved = plan(all, migrate_flags::CATEGORIES as i32 | migrate_flags::DELETE_DOWNLOADED as i32);
        assert_eq!(saved.selected, [migrate_flags::CATEGORIES, migrate_flags::DELETE_DOWNLOADED], "only the saved bits start ticked");
    }

    #[test]
    fn a_manga_offers_its_chapters_and_the_show_manga_button() {
        let manga = Facts { custom_cover: true, downloads: true, ..facts(Kind::Manga, FetchType::Episodes, FetchType::Episodes) };
        let shown = plan(manga, i32::MAX);
        assert_eq!(shown.flags.iter().map(|(bit, _)| *bit).collect::<Vec<_>>(), [migrate_flags::ITEMS, migrate_flags::CATEGORIES, migrate_flags::CUSTOM_COVER, migrate_flags::DELETE_DOWNLOADED]);
        assert_eq!(shown.flags[0].1, tr(MsgKey::chapters));
        assert_eq!(shown.actions, [Action::ShowEntry, Action::Copy, Action::Migrate]);
        assert_eq!(Action::ShowEntry.label(Kind::Manga), tr(MsgKey::action_show_manga));
        assert_eq!(Action::ShowEntry.label(Kind::Anime), tr(MsgKey::action_show_anime));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::browse::dialogs::buttons_of;

    type Pending = Rc<RefCell<Vec<Done>>>;

    fn handlers(log: &Rc<RefCell<Vec<String>>>, pending: &Pending) -> MigrateHandlers {
        let (show, run, finished) = (Rc::clone(log), Rc::clone(log), Rc::clone(log));
        let queue = Rc::clone(pending);
        MigrateHandlers {
            show_entry: Rc::new(move || show.borrow_mut().push("show".into())),
            run: Rc::new(move |plan, done| {
                run.borrow_mut().push(format!("run replace={} flags={}", plan.replace, plan.flags));
                queue.borrow_mut().push(done);
            }),
            finished: Rc::new(move || finished.borrow_mut().push("finished".into())),
        }
    }

    fn labels(window: &adw::Dialog) -> Vec<String> {
        buttons_of(window).iter().filter_map(|b| b.label()).map(|l| l.to_string()).collect()
    }

    fn all<T: IsA<gtk::Widget>>(window: &adw::Dialog) -> Vec<T> {
        fn walk<T: IsA<gtk::Widget>>(widget: &gtk::Widget, out: &mut Vec<T>) {
            if let Some(found) = widget.downcast_ref::<T>() {
                out.push(found.clone());
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                walk(&next, out);
                child = next.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(&crate::ui::dialog_content(window).expect("content"), &mut out);
        out
    }

    struct Rig {
        slot: DialogSlot,
        anchor: gtk::Label,
        log: Rc<RefCell<Vec<String>>>,
        pending: Pending,
    }

    fn rig() -> Rig {
        Rig { slot: DialogSlot::new(), anchor: gtk::Label::new(None), log: Rc::default(), pending: Rc::default() }
    }

    impl Rig {
        fn open(&self, facts: Facts, saved: i32) -> adw::Dialog {
            show(&self.slot, &self.anchor, facts, saved, handlers(&self.log, &self.pending))
        }

        fn plain(&self, kind: Kind) -> adw::Dialog {
            self.open(Facts { kind, old: FetchType::Episodes, new: FetchType::Episodes, custom_cover: false, custom_background: false, downloads: false }, i32::MAX)
        }

        /// The run the engine has not answered yet.
        fn answer(&self, result: Result<(), String>) {
            let done = self.pending.borrow_mut().pop().expect("a run is pending");
            done(result);
        }
    }

    fn the_dialog_follows_the_plan_and_the_buttons_run_their_handlers() {
        let rig = rig();
        let window = rig.open(Facts { kind: Kind::Anime, old: FetchType::Episodes, new: FetchType::Seasons, custom_cover: false, custom_background: false, downloads: false }, i32::MAX);
        assert_eq!(labels(&window), [Action::ShowEntry.label(Kind::Anime), Action::ShowSeasons.label(Kind::Anime)], "a mismatch offers the Show buttons");
        let text: Vec<String> = all::<gtk::Label>(&window).iter().map(|l| l.label().to_string()).collect();
        assert!(text.contains(&tr(MsgKey::label_cant_migrate_episode)), "the mismatch message is on the dialog: {text:?}");
        buttons_of(&window)[1].emit_clicked();
        assert_eq!(*rig.log.borrow(), ["show"], "Show seasons opens the new entry");
        assert!(rig.slot.current().is_none(), "and closes the dialog");

        for kind in [Kind::Manga, Kind::Anime] {
            rig.log.borrow_mut().clear();
            let window = rig.plain(kind);
            assert_eq!(labels(&window), [Action::ShowEntry.label(kind), Action::Copy.label(kind), Action::Migrate.label(kind)], "{kind:?}");
            let disclosure = all::<gtk::Label>(&window).into_iter().find(|l| l.label() == crate::browse::texts::migrate_tracks_always()).expect("disclosure");
            assert!(disclosure.has_css_class("dim-label"), "{kind:?}: the tracking line is dim");
            let checks = all::<gtk::CheckButton>(&window);
            assert_eq!(checks.len(), 2, "{kind:?}: the items and the categories");
            assert!(checks.iter().all(|c| c.is_active()), "{kind:?}: every offered option starts selected");
            checks[0].set_active(false);
            buttons_of(&window)[2].emit_clicked();
            assert_eq!(*rig.log.borrow(), ["run replace=true flags=2"], "{kind:?}: Migrate runs with the ticked options only");
            rig.answer(Ok(()));
            let window = rig.plain(kind);
            buttons_of(&window)[1].emit_clicked();
            assert_eq!(rig.log.borrow().last().map(String::as_str), Some("run replace=false flags=3"), "{kind:?}: Copy keeps the old entry");
            rig.answer(Ok(()));
        }
    }

    fn a_run_shows_the_spinner_over_a_dimmed_body_hides_the_buttons_and_refuses_a_second_start() {
        let rig = rig();
        let facts = Facts { kind: Kind::Anime, old: FetchType::Episodes, new: FetchType::Episodes, custom_cover: true, custom_background: true, downloads: true };
        let window = rig.open(facts, migrate_flags::ITEMS as i32);
        let checks = all::<gtk::CheckButton>(&window);
        assert_eq!(checks.iter().map(|c| c.is_active()).collect::<Vec<_>>(), [true, false, false, false, false], "only the saved bit starts ticked");
        let buttons = buttons_of(&window);
        let spinner = all::<gtk::Spinner>(&window).into_iter().next().expect("the spinner");
        assert!(!spinner.get_visible(), "no spinner before a run");
        buttons[1].emit_clicked();
        assert_eq!(*rig.log.borrow(), ["run replace=false flags=1"]);
        assert!(spinner.get_visible(), "T-41a-035: the spinner shows while the run goes");
        assert!(buttons.iter().all(|b| !b.is_visible()), "and the buttons are hidden");
        let body = checks[0].parent().expect("body");
        assert!((body.opacity() - 0.3).abs() < 0.01, "the options sit under a 70 % cover");
        buttons[2].emit_clicked();
        assert_eq!(rig.log.borrow().len(), 1, "a second request during the run is refused");
        assert_eq!(rig.pending.borrow().len(), 1);
        rig.answer(Ok(()));
        assert!(rig.slot.current().is_none(), "a success closes the dialog");
        assert_eq!(rig.log.borrow().last().map(String::as_str), Some("finished"));
    }

    fn escape_and_the_close_button_do_nothing_while_a_run_goes() {
        let rig = rig();
        let window = rig.plain(Kind::Manga);
        buttons_of(&window)[2].emit_clicked();
        assert!(!window.close(), "the close request is refused while it runs");
        assert!(rig.slot.current().is_some(), "the dialog is still the slot's while it runs");
        rig.answer(Ok(()));
        assert!(rig.slot.current().is_none());
    }

    fn a_failure_shows_the_reason_in_the_dialog_and_lets_it_run_again() {
        let rig = rig();
        let window = rig.plain(Kind::Manga);
        let buttons = buttons_of(&window);
        buttons[2].emit_clicked();
        rig.answer(Err("disk full".into()));
        let text: Vec<String> = all::<gtk::Label>(&window).iter().filter(|l| l.get_visible()).map(|l| l.label().to_string()).collect();
        assert!(text.contains(&"Migration failed: disk full".to_owned()), "{text:?}");
        assert!(buttons.iter().all(|b| b.get_visible() && b.parent().is_some_and(|row| row.get_visible()) && b.is_sensitive()), "the buttons come back");
        assert!(!rig.log.borrow().contains(&"finished".to_owned()), "the page is not left");
        assert!(rig.slot.current().is_some(), "the dialog stays open");
        buttons[1].emit_clicked();
        assert_eq!(rig.pending.borrow().len(), 1, "a new run starts");
        assert!(!all::<gtk::Label>(&window).iter().any(|l| l.get_visible() && l.label().starts_with("Migration failed")), "the old error is cleared");
        rig.answer(Ok(()));
    }

    fn the_keys_migrate_and_copy() {
        let rig = rig();
        let window = rig.plain(Kind::Manga);
        let controllers = window.observe_controllers();
        let keys: Vec<gtk::EventControllerKey> = (0..controllers.n_items()).filter_map(|i| controllers.item(i)).filter_map(|c| c.downcast().ok()).collect();
        let ctrl = gtk::gdk::ModifierType::CONTROL_MASK;
        assert!(keys.iter().any(|k| k.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Return, &0_u32, &ctrl])), "Ctrl+Enter is taken");
        assert_eq!(rig.log.borrow().last().map(String::as_str), Some("run replace=true flags=3"));
        rig.answer(Ok(()));
        let window = rig.plain(Kind::Manga);
        let controllers = window.observe_controllers();
        let keys: Vec<gtk::EventControllerKey> = (0..controllers.n_items()).filter_map(|i| controllers.item(i)).filter_map(|c| c.downcast().ok()).collect();
        let shift = gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK;
        assert!(keys.iter().any(|k| k.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Return, &0_u32, &shift])), "Ctrl+Shift+Enter is taken");
        assert_eq!(rig.log.borrow().last().map(String::as_str), Some("run replace=false flags=3"));
        rig.answer(Ok(()));
    }

    pub(crate) fn smoke() {
        the_dialog_follows_the_plan_and_the_buttons_run_their_handlers();
        a_run_shows_the_spinner_over_a_dimmed_body_hides_the_buttons_and_refuses_a_second_start();
        escape_and_the_close_button_do_nothing_while_a_run_goes();
        a_failure_shows_the_reason_in_the_dialog_and_lets_it_run_again();
        the_keys_migrate_and_copy();
    }
}
