//! Create Backup screen model (area 21, T-21a-011). Holds the UI state; the GTK widgets live in the binary crate.
//! The screen starts from `BackupOptions::default()` on every visit and does not persist its selection (B-03).

use std::path::PathBuf;

use crate::error::BackupError;
use crate::options::BackupOptions;

/// The card a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    Library,
    Settings,
    Extensions,
}

/// One toggle on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionId {
    LibraryEntries,
    Chapters,
    Tracking,
    History,
    Categories,
    ReadEntries,
    AppSettings,
    ExtensionStores,
    CustomButtons,
    SourceSettings,
    PrivateSettings,
    Extensions,
}

impl OptionId {
    /// Rows in display order, grouped by card.
    pub const ROWS: [(Card, OptionId); 10] = [
        (Card::Library, OptionId::LibraryEntries),
        (Card::Library, OptionId::Chapters),
        (Card::Library, OptionId::Tracking),
        (Card::Library, OptionId::History),
        (Card::Library, OptionId::Categories),
        (Card::Library, OptionId::ReadEntries),
        (Card::Settings, OptionId::AppSettings),
        (Card::Settings, OptionId::SourceSettings),
        (Card::Settings, OptionId::PrivateSettings),
        (Card::Extensions, OptionId::Extensions),
    ];

    /// English label (B-02).
    pub fn label(self) -> &'static str {
        match self {
            Self::LibraryEntries => "Library entries",
            Self::Chapters => "Chapters and episodes",
            Self::Tracking => "Tracking",
            Self::History => "History",
            Self::Categories => "Categories",
            Self::ReadEntries => "All read/watched entries",
            Self::AppSettings => "App settings",
            Self::ExtensionStores => "Extension stores",
            Self::CustomButtons => "Custom buttons",
            Self::SourceSettings => "Source settings",
            Self::PrivateSettings => "Include sensitive settings (e.g., tracker login tokens)",
            Self::Extensions => "Extensions",
        }
    }

    pub fn get(self, o: &BackupOptions) -> bool {
        match self {
            Self::LibraryEntries => o.library_entries,
            Self::Chapters => o.chapters,
            Self::Tracking => o.tracking,
            Self::History => o.history,
            Self::Categories => o.categories,
            Self::ReadEntries => o.read_entries,
            Self::AppSettings => o.app_settings,
            Self::ExtensionStores => o.extension_stores,
            Self::CustomButtons => o.custom_button,
            Self::SourceSettings => o.source_settings,
            Self::PrivateSettings => o.private_settings,
            Self::Extensions => o.extensions,
        }
    }

    /// Toggle with the parent rules: a setter changes only its own field, except the private option's reset (E-27).
    pub fn set(self, o: &mut BackupOptions, on: bool) {
        match self {
            Self::LibraryEntries => o.set_library_entries(on),
            Self::Chapters => o.chapters = on,
            Self::Tracking => o.tracking = on,
            Self::History => o.history = on,
            Self::Categories => o.categories = on,
            Self::ReadEntries => o.read_entries = on,
            Self::AppSettings => o.set_app_settings(on),
            Self::ExtensionStores => o.extension_stores = on,
            Self::CustomButtons => o.custom_button = on,
            Self::SourceSettings => o.set_source_settings(on),
            Self::PrivateSettings => o.private_settings = on && o.private_enabled(),
            Self::Extensions => o.extensions = on,
        }
    }

    /// Live enable rule (p1-ui-parity 2.1).
    pub fn enabled(self, o: &BackupOptions) -> bool {
        match self {
            Self::Chapters | Self::Tracking | Self::History | Self::ReadEntries => o.entry_children_enabled(),
            Self::PrivateSettings => o.private_enabled(),
            _ => true,
        }
    }
}

/// What the dialog returned.
pub enum DialogOutcome {
    Chosen(PathBuf),
    Cancelled,
}

/// What the screen does after the dialog.
#[derive(Debug, PartialEq, Eq)]
pub enum ScreenAction {
    /// Start the job with this path and close the screen.
    StartJob { path: PathBuf },
    /// Keep the screen open with the same selection.
    KeepOpen,
}

#[derive(Default)]
pub struct CreateScreen {
    pub options: BackupOptions,
}

/// Shown on the Create screen and in the manual backup text while the file would contain credentials (T-21a-044).
pub const CREDENTIALS_WARNING: &str = "This backup contains login tokens. Keep it private.";

impl CreateScreen {
    /// Create is enabled only when `can_create()` holds.
    pub fn create_enabled(&self) -> bool {
        self.options.can_create()
    }

    /// The credentials warning, present only when sensitive settings are written (E-27 rule applied).
    pub fn credentials_warning(&self) -> Option<&'static str> {
        self.options.effective_private().then_some(CREDENTIALS_WARNING)
    }

    /// Clicking Create. Returns `InProgress` (toast "Backup is already in progress") and opens no dialog when a
    /// manual job is running. Otherwise the caller opens the save dialog with `suggested` as the name.
    pub fn on_create(&self, manual_job_running: bool, suggested: &str) -> Result<String, BackupError> {
        if !self.create_enabled() {
            return Err(BackupError::EmptyBackup);
        }
        if manual_job_running {
            return Err(BackupError::InProgress);
        }
        Ok(suggested.to_owned())
    }

    /// Applies the dialog result: a chosen file starts the job and closes the screen; cancel keeps it open.
    pub fn on_dialog(&self, outcome: DialogOutcome) -> ScreenAction {
        match outcome {
            DialogOutcome::Chosen(path) => ScreenAction::StartJob { path },
            DialogOutcome::Cancelled => ScreenAction::KeepOpen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_cards_with_six_three_and_one_rows() {
        let count = |c: Card| OptionId::ROWS.iter().filter(|(card, _)| *card == c).count();
        assert_eq!(count(Card::Library), 6);
        assert_eq!(count(Card::Settings), 3);
        assert_eq!(count(Card::Extensions), 1);
    }

    #[test]
    fn labels_match_b02_texts() {
        assert_eq!(OptionId::PrivateSettings.label(), "Include sensitive settings (e.g., tracker login tokens)");
        assert_eq!(OptionId::ReadEntries.label(), "All read/watched entries");
        assert_eq!(OptionId::CustomButtons.label(), "Custom buttons");
    }

    #[test]
    fn disabled_rules_follow_parents() {
        let mut s = CreateScreen::default();
        OptionId::LibraryEntries.set(&mut s.options, false);
        assert!(!OptionId::Chapters.enabled(&s.options));
        assert!(!OptionId::ReadEntries.enabled(&s.options));
        assert!(OptionId::Categories.enabled(&s.options));
        OptionId::LibraryEntries.set(&mut s.options, true);
        assert!(OptionId::Chapters.enabled(&s.options));
        // Toggling the parent keeps the children's values.
        assert!(OptionId::Chapters.get(&s.options));
    }

    #[test]
    fn private_row_enabled_only_with_a_settings_parent() {
        let mut s = CreateScreen::default();
        OptionId::AppSettings.set(&mut s.options, false);
        assert!(OptionId::SourceSettings.get(&s.options));
        assert!(OptionId::PrivateSettings.enabled(&s.options));
        OptionId::SourceSettings.set(&mut s.options, false);
        assert!(!OptionId::PrivateSettings.enabled(&s.options));
        assert!(!OptionId::PrivateSettings.get(&s.options));
    }

    #[test]
    fn create_gated_and_in_progress_refused() {
        let mut s = CreateScreen::default();
        assert!(s.create_enabled());
        assert_eq!(s.on_create(false, "x.tachibk").unwrap(), "x.tachibk");
        assert!(matches!(s.on_create(true, "x.tachibk"), Err(BackupError::InProgress)));
        for id in [OptionId::LibraryEntries, OptionId::Categories, OptionId::AppSettings, OptionId::SourceSettings] {
            id.set(&mut s.options, false);
        }
        assert!(!s.create_enabled());
        assert!(s.on_create(false, "x").is_err());
    }

    #[test]
    fn dialog_cancel_keeps_screen_and_choice_starts_job() {
        let s = CreateScreen::default();
        assert_eq!(s.on_dialog(DialogOutcome::Cancelled), ScreenAction::KeepOpen);
        let p = PathBuf::from("/tmp/a.tachibk");
        assert_eq!(s.on_dialog(DialogOutcome::Chosen(p.clone())), ScreenAction::StartJob { path: p });
    }

    #[test]
    fn screen_starts_from_defaults_every_visit() {
        let mut first = CreateScreen::default();
        OptionId::History.set(&mut first.options, false);
        let second = CreateScreen::default();
        assert!(OptionId::History.get(&second.options));
    }

    #[test]
    fn credentials_warning_only_when_private_is_written() {
        let mut screen = CreateScreen::default();
        assert_eq!(screen.credentials_warning(), None, "private is off by default");
        screen.options.app_settings = false;
        screen.options.source_settings = false;
        screen.options.private_settings = true;
        assert_eq!(screen.credentials_warning(), None, "E-27: no settings parent, nothing is written");
        screen.options.app_settings = true;
        assert_eq!(screen.credentials_warning(), Some("This backup contains login tokens. Keep it private."));
    }
}
