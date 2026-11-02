//! The Create backup and Restore backup pages (T-47b-029 to T-47b-031). The rules live in `kuro-backup`
//! (`CreateScreen`, `RestoreScreen`); these pages draw them. A run needs the library and preference adapters that the
//! app registers once through [`set_host`]; until then a chosen file shows the failure text and nothing is written.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_backup::error::BackupError;
use kuro_backup::job::JobSlot;
use kuro_backup::options::BackupOptions;
use kuro_backup::restore::options::RestoreOptions;
use kuro_backup::restore::screen::{Check, RestoreScreen};
use kuro_backup::screen::{Card, CreateScreen, DialogOutcome, OptionId, ScreenAction};
use kuro_backup::schema::Backup;
use kuro_backup::text;
use kuro_backup::validate::check_file;
use kuro_i18n::{MsgKey, tr};

use crate::backup_run::NothingInstalled;
use crate::screens::Built;
use crate::theme::tokens::{SPACE_M, SPACE_S};

/// What starts a run. The app registers it with the adapters over its databases and preferences.
pub struct BackupHost {
    pub start_backup: Box<dyn Fn(PathBuf, BackupOptions)>,
    /// Gets the file, the backup the check decoded from it, and the chosen options.
    pub start_restore: Box<dyn Fn(PathBuf, Backup, RestoreOptions)>,
}

thread_local! {
    static HOST: RefCell<Option<Rc<BackupHost>>> = const { RefCell::new(None) };
    static PENDING_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Registers the host. Replaces an earlier one.
pub fn set_host(host: BackupHost) {
    HOST.with(|h| *h.borrow_mut() = Some(Rc::new(host)));
}

/// The slot that keeps two manual backups from running at once. The host starts its run on this slot.
pub fn job_slot() -> &'static JobSlot {
    static SLOT: OnceLock<JobSlot> = OnceLock::new();
    SLOT.get_or_init(JobSlot::new)
}

/// The file the next Restore page checks. Set it before opening the page; without one the page asks for a file.
pub fn set_restore_file(path: PathBuf) {
    PENDING_FILE.with(|p| *p.borrow_mut() = Some(path));
}

fn host() -> Option<Rc<BackupHost>> {
    HOST.with(|h| h.borrow().clone())
}

fn card_title(card: Card) -> String {
    tr(match card {
        Card::Library => MsgKey::label_library,
        Card::Settings => MsgKey::label_settings,
        Card::Extensions => MsgKey::label_extensions,
    })
}

fn option_label(id: OptionId) -> String {
    tr(match id {
        OptionId::LibraryEntries => MsgKey::entries,
        OptionId::Chapters => MsgKey::chapters_episodes,
        OptionId::Tracking => MsgKey::track,
        OptionId::History => MsgKey::history,
        OptionId::Categories => MsgKey::categories,
        OptionId::ReadEntries => MsgKey::non_library_settings,
        OptionId::AppSettings => MsgKey::app_settings,
        OptionId::ExtensionStores => MsgKey::extensionStores,
        OptionId::CustomButtons => MsgKey::custom_button_settings,
        OptionId::SourceSettings => MsgKey::source_settings,
        OptionId::PrivateSettings => MsgKey::private_settings,
        OptionId::Extensions => MsgKey::label_extensions,
    })
}

/// The restore options' labels, in the order of `RestoreOptions::to_bits`.
fn restore_label(index: usize) -> String {
    tr(match index {
        0 => MsgKey::label_library,
        1 => MsgKey::categories,
        2 => MsgKey::app_settings,
        3 => MsgKey::extensionStores,
        4 => MsgKey::custom_button_settings,
        5 => MsgKey::source_settings,
        _ => MsgKey::label_extensions,
    })
}

/// What Create says when the click is refused, or `None` when it goes on to the save dialog.
pub fn create_refusal(screen: &CreateScreen, running: bool) -> Option<String> {
    match screen.on_create(running, "") {
        Ok(_) => None,
        Err(BackupError::InProgress) => Some(tr(MsgKey::backup_in_progress)),
        Err(_) => Some(tr(MsgKey::empty_backup_error)),
    }
}

/// Pins `action` under `content`: a scrolled column above, a full-width button below.
fn with_action_bar(content: &gtk::Widget, action: &gtk::Button) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_vexpand(true);
    page.append(content);
    action.add_css_class("suggested-action");
    crate::ui::margins(action, 12);
    page.append(action);
    page.upcast()
}

/// A check row: the whole row toggles the box.
fn check_row(label: &str, active: bool) -> (adw::ActionRow, gtk::CheckButton) {
    let check = gtk::CheckButton::new();
    check.set_active(active);
    let row = crate::ui::row(label, None, Some(check.clone().upcast()));
    row.set_activatable(true);
    let toggle = check.clone();
    crate::widgets::row_activate::on_activate(&row, move |_| toggle.set_active(!toggle.is_active()));
    (row, check)
}

// ---------- create ----------

/// The Create backup page: three cards of options and a Create button pinned at the bottom.
pub fn create_page() -> Option<Built> {
    let screen = Rc::new(RefCell::new(CreateScreen::default()));
    let updating = Rc::new(Cell::new(false));
    let column = crate::ui::column();
    let mut rows: Vec<(OptionId, adw::ActionRow, gtk::CheckButton)> = Vec::new();
    for card in [Card::Library, Card::Settings, Card::Extensions] {
        column.append(&crate::ui::heading(&card_title(card)));
        let list = crate::ui::boxed_list();
        for (_, id) in OptionId::ROWS.iter().filter(|(c, _)| *c == card) {
            let (row, check) = check_row(&option_label(*id), id.get(&screen.borrow().options));
            list.append(&row);
            rows.push((*id, row, check));
        }
        column.append(&list);
        if card == Card::Extensions {
            // Android APK extensions are replaced by Kurobayomi extension files in a backup (T-21a-019)
            column.append(&crate::ui::dim(&crate::backup_host::shown(text::BACKUP_EXTENSIONS_INFO)));
        }
    }
    let warning = crate::ui::dim(kuro_backup::screen::CREDENTIALS_WARNING);
    column.append(&warning);
    let create = gtk::Button::with_label(&tr(MsgKey::action_create));
    let rows = Rc::new(rows);

    let refresh = {
        let (screen, rows, create, warning, updating) = (screen.clone(), rows.clone(), create.clone(), warning.clone(), updating.clone());
        Rc::new(move || {
            let screen = screen.borrow();
            updating.set(true);
            for (id, row, check) in rows.iter() {
                row.set_sensitive(id.enabled(&screen.options));
                check.set_active(id.get(&screen.options));
            }
            updating.set(false);
            create.set_sensitive(screen.create_enabled());
            warning.set_visible(screen.credentials_warning().is_some());
        })
    };
    refresh();
    for (id, _, check) in rows.iter() {
        let (id, screen, refresh, updating) = (*id, screen.clone(), refresh.clone(), updating.clone());
        check.connect_toggled(move |c| {
            if updating.get() {
                return;
            }
            id.set(&mut screen.borrow_mut().options, c.is_active());
            refresh();
        });
    }
    create.connect_clicked({
        let screen = screen.clone();
        move |_| on_create(&screen)
    });
    let content = with_action_bar(crate::ui::scrolled(&column).upcast_ref(), &create);
    Some(Built { content, end: vec![] })
}

fn on_create(screen: &Rc<RefCell<CreateScreen>>) {
    if let Some(text) = create_refusal(&screen.borrow(), job_slot().is_held()) {
        crate::toast::notify(text);
        return;
    }
    let name = kuro_backup::file::manual_suggested_name(chrono::Local::now());
    let dialog = gtk::FileDialog::builder().initial_name(name).build();
    let screen = screen.clone();
    dialog.save(Some(&crate::ui::window()), None::<&gio::Cancellable>, move |result| {
        let outcome = match result.ok().and_then(|file| file.path()) {
            Some(path) => DialogOutcome::Chosen(path),
            None => DialogOutcome::Cancelled,
        };
        let options = screen.borrow().options;
        match screen.borrow().on_dialog(outcome) {
            ScreenAction::KeepOpen => {}
            ScreenAction::StartJob { path } => {
                start_backup(path, options);
                crate::ui::pop();
            }
        }
    });
}

fn start_backup(path: PathBuf, options: BackupOptions) {
    match host() {
        // the options go to the job unchanged, chapters included when library entries is off (T-47b-030)
        Some(host) => (host.start_backup)(path, options),
        None => {
            tracing::warn!(path = %path.display(), "no backup host is registered");
            crate::toast::notify(tr(MsgKey::creating_backup_error));
        }
    }
}

// ---------- restore ----------

/// The Restore backup page. It checks the file on a worker thread: a spinner row shows meanwhile, the options stay
/// hidden and Restore stays off. Closing the page drops a result that arrives late and shows nothing (RACE-07).
pub fn restore_page() -> Option<Built> {
    let screen = Rc::new(RefCell::new(RestoreScreen::default()));
    let path: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(PENDING_FILE.with(|p| p.borrow_mut().take())));
    let alive = Rc::new(Cell::new(true));
    let updating = Rc::new(Cell::new(false));

    let column = crate::ui::column();
    let checking = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
    checking.append(&crate::ui::inline_spinner());
    checking.append(&gtk::Label::new(Some(&crate::backup_host::shown(text::CHECKING_BACKUP))));
    column.append(&checking);
    let choose = gtk::Button::with_label(&tr(MsgKey::file_select_backup));
    choose.set_halign(gtk::Align::Start);
    column.append(&choose);
    let card = gtk::Label::builder().wrap(true).xalign(0.0).selectable(true).build();
    // the backup summary sits in a one-row boxed list: the same rounded card as the option lists under it
    let frame = crate::ui::boxed_list();
    frame.append(&gtk::ListBoxRow::builder().child(&card).activatable(false).build());
    card.set_margin_start(SPACE_M);
    card.set_margin_end(SPACE_M);
    card.set_margin_top(SPACE_M);
    card.set_margin_bottom(SPACE_M);
    column.append(&frame);
    let options_list = crate::ui::boxed_list();
    let mut checks = Vec::new();
    let start = screen.borrow().options.to_bits();
    for (index, on) in start.iter().enumerate() {
        let (row, check) = check_row(&restore_label(index), *on);
        // custom buttons are cut: nothing to restore them into
        row.set_visible(index != 4);
        options_list.append(&row);
        checks.push(check);
    }
    column.append(&options_list);
    let restore = gtk::Button::with_label(&tr(MsgKey::action_restore));
    let checks = Rc::new(checks);

    let refresh = {
        let (screen, path, checking, choose, card, frame, options_list, restore) =
            (screen.clone(), path.clone(), checking.clone(), choose.clone(), card.clone(), frame.clone(), options_list.clone(), restore.clone());
        Rc::new(move || {
            let screen = screen.borrow();
            let has_file = path.borrow().is_some();
            checking.set_visible(has_file && matches!(screen.check, Check::Validating));
            choose.set_visible(!has_file);
            let text = screen.card_text_with(&crate::backup_host::shown);
            frame.set_visible(text.is_some());
            card.set_text(text.as_deref().unwrap_or(""));
            options_list.set_visible(screen.options_visible());
            restore.set_sensitive(screen.can_restore());
        })
    };
    for (index, check) in checks.iter().enumerate() {
        let (screen, refresh, updating) = (screen.clone(), refresh.clone(), updating.clone());
        check.connect_toggled(move |c| {
            if updating.get() {
                return;
            }
            screen.borrow_mut().options.set(index, c.is_active());
            refresh();
        });
    }

    let validate = {
        let (screen, path, alive, refresh) = (screen.clone(), path.clone(), alive.clone(), refresh.clone());
        Rc::new(move || {
            let Some(file) = path.borrow().clone() else { return };
            let options = screen.borrow().options;
            let mut fresh = RestoreScreen::default();
            fresh.options = options;
            *screen.borrow_mut() = fresh;
            refresh();
            let (screen, alive, refresh) = (screen.clone(), alive.clone(), refresh.clone());
            glib::spawn_future_local(async move {
                let checked = file.clone();
                let result = crate::background::blocking(move || check_file(&checked, &crate::library_update::live::LiveRegistry, &NothingInstalled)).await;
                // a page that was closed meanwhile drops the answer without a toast
                if !alive.get() {
                    return;
                }
                let result = result.unwrap_or_else(|_| Err(BackupError::Corrupt("the check stopped".to_owned())));
                screen.borrow_mut().apply_checked(&file, result);
                refresh();
            });
        })
    };
    choose.connect_clicked({
        let (path, validate) = (path.clone(), validate.clone());
        move |_| {
            let (path, validate) = (path.clone(), validate.clone());
            gtk::FileDialog::new().open(Some(&crate::ui::window()), None::<&gio::Cancellable>, move |result| {
                if let Some(file) = result.ok().and_then(|f| f.path()) {
                    *path.borrow_mut() = Some(file);
                    validate();
                }
            });
        }
    });
    restore.connect_clicked({
        let (screen, path) = (screen.clone(), path.clone());
        move |_| {
            let Some(file) = path.borrow().clone() else { return };
            let options = screen.borrow().options;
            let backup = screen.borrow_mut().take_backup();
            match (host(), backup) {
                (Some(host), Some(backup)) => (host.start_restore)(file, backup, options),
                _ => {
                    tracing::warn!("no backup host is registered, or the checked backup is gone");
                    crate::toast::notify(tr(MsgKey::restoring_backup_error));
                    return;
                }
            }
            crate::ui::pop();
        }
    });

    refresh();
    validate();
    let content = with_action_bar(crate::ui::scrolled(&column).upcast_ref(), &restore);
    // the page owns the flag: closing it ends the wait for the check
    let flag = alive.clone();
    content.connect_destroy(move |_| flag.set(false));
    Some(Built { content, end: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_backup::creator::SourceRegistry;
    use kuro_backup::validate::{TrackerRegistry, preflight_file};

    #[test]
    fn t_47b_029_the_three_cards_and_ten_rows_come_from_the_string_table() {
        assert_eq!(card_title(Card::Library), "Library");
        assert_eq!(card_title(Card::Settings), "Settings");
        assert_eq!(card_title(Card::Extensions), "Extensions");
        let labels: Vec<String> = OptionId::ROWS.iter().map(|(_, id)| option_label(*id)).collect();
        assert_eq!(labels.len(), 10);
        assert_eq!(labels[0], "Library entries");
        assert_eq!(labels[1], "Chapters and episodes");
        assert_eq!(labels[5], "All read/watched entries");
        assert_eq!(labels[8], "Include sensitive settings (e.g., tracker login tokens)");
        for (_, id) in OptionId::ROWS {
            assert_eq!(option_label(id), id.label(), "{id:?}: the table and kuro-backup agree");
        }
    }

    #[test]
    fn t_47b_029_children_follow_library_entries_and_private_follows_a_settings_parent() {
        let mut screen = CreateScreen::default();
        OptionId::LibraryEntries.set(&mut screen.options, false);
        for id in [OptionId::Chapters, OptionId::Tracking, OptionId::History, OptionId::ReadEntries] {
            assert!(!id.enabled(&screen.options), "{id:?}");
        }
        assert!(OptionId::Categories.enabled(&screen.options));
        OptionId::AppSettings.set(&mut screen.options, false);
        OptionId::SourceSettings.set(&mut screen.options, false);
        assert!(!OptionId::PrivateSettings.enabled(&screen.options));
    }

    #[test]
    fn t_47b_029_create_is_off_with_only_extensions_checked() {
        let mut screen = CreateScreen::default();
        for (_, id) in OptionId::ROWS {
            id.set(&mut screen.options, id == OptionId::Extensions);
        }
        assert!(!screen.create_enabled());
        assert_eq!(create_refusal(&screen, false).as_deref(), Some("No library entries to back up"));
    }

    #[test]
    fn t_47b_030_a_running_manual_backup_refuses_the_click_with_a_toast_text() {
        let screen = CreateScreen::default();
        assert_eq!(create_refusal(&screen, true).as_deref(), Some("Backup is already in progress"));
        assert_eq!(create_refusal(&screen, false), None);
        let guard = job_slot().try_acquire().expect("the slot is free");
        assert!(job_slot().is_held());
        drop(guard);
        assert!(!job_slot().is_held());
    }

    #[test]
    fn t_47b_030_the_save_dialog_starts_a_job_on_a_path_and_cancel_changes_nothing() {
        let screen = CreateScreen::default();
        assert_eq!(screen.on_dialog(DialogOutcome::Cancelled), ScreenAction::KeepOpen);
        let path = PathBuf::from("/tmp/kurobayomi_2026-10-09_10-00.tachibk");
        assert_eq!(screen.on_dialog(DialogOutcome::Chosen(path.clone())), ScreenAction::StartJob { path });
    }

    #[test]
    fn t_47b_031_restore_options_are_labelled_from_the_table_in_bit_order() {
        let labels: Vec<String> = (0..7).map(restore_label).collect();
        assert_eq!(labels, RestoreOptions::LABELS);
    }

    #[test]
    fn t_47b_031_a_file_that_is_not_a_backup_is_invalid_and_never_restorable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("not-a-backup.tachibk");
        std::fs::write(&file, b"this is not a backup").expect("write");
        let mut screen = RestoreScreen::default();
        assert!(!screen.can_restore(), "validating keeps Restore off");
        screen.apply_check(&file, preflight_file(&file, &crate::library_update::live::LiveRegistry, &NothingInstalled));
        assert!(matches!(screen.check, Check::Invalid { .. }));
        assert!(!screen.options_visible());
        assert!(!screen.can_restore());
        assert!(screen.card_text().expect("an error card").starts_with("Invalid backup file:"));
    }

    #[test]
    fn t_47b_031_nothing_installed_lists_every_source_as_missing() {
        assert!(!NothingInstalled.is_installed(1));
        assert_eq!(NothingInstalled.stub_label(7), "7");
        assert_eq!(TrackerRegistry::name(&NothingInstalled, 1), None);
        assert!(!NothingInstalled.logged_in(1));
    }
}
