//! The registered migrations (T-11a-002).
//!
//! Three steps are registered, all `Version::Always`, in the order of `Migrations.kt`: they re-register the periodic
//! jobs of the automatic backup and of the anime and manga library updates on every start, so a restored profile or
//! a changed preference takes effect at the next launch.
//!
//! The other 41 steps of `Migrations.kt` are not registered. Each one rewrites a key of the Android shared
//! preferences file, deletes an Android directory, cancels an Android job or logs a tracker out, and a Kurobayomi
//! profile starts without any of that. Where the conversion still matters for an imported Aniyomi backup it lives in
//! `kuro_prefs::legacy` (NORM); the rest is dropped (DROP). A key of a removed desktop feature (`kuro_prefs::registry::DROPPED_KEYS`)
//! is ignored by the import, so its steps are DROP too. The disposition of every row, in run order:
//!
//! ```text
//!  #  class                                version  disposition  reason
//!  4  TrustExtensionRepositoryMigration         7   NORM  repository URL lists move to the repository importer
//!  5  InternalChapterCacheDirMigration         15   DROP  Android cache directory; the cache is created fresh
//!  6  CoverToExternalFileMigration             19   DROP  Android cover files; covers are fetched again
//!  7  DeleteExternalChapterCacheDirMigration   26   DROP  Android external cache directory
//!  8  ResetSortPreferenceRemovedMigration      44   NORM  library sort value that no longer exists
//!  9  MigrateToTriStateMigration               52   NORM  library filters become tri-state values
//! 10  ForceMALLogOutMigration                  54   DROP  login state lives in the secret store only
//! 11  DOHMigration                             57   NORM  the DNS-over-HTTPS flag becomes a provider number
//! 12  ResetRotationMigration                   59   DROP  reader orientation removed; the key is ignored on import
//! 13  MigrateRotationViewerValuesMigration     60   DROP  reader orientation removed; the key is ignored on import
//! 14  RemoveOneTwoHourUpdateMigration          61   NORM  update interval of 1 or 2 hours becomes 3
//! 15  SetupBackgroundTasksMigration            64   DROP  Android job tags; the scheduler owns the jobs
//! 16  MigrateSortingModeMigration              64   NORM  library sort numbers become sort names
//! 17  AddAllLangMigration                      70   NORM  the language set gains the all entry
//! 18  RemoveQuickUpdateMigration               71   NORM  quick update intervals become 12 hours
//! 19  CombineUpdateRestrictionMigration        72   NORM  the ongoing-only flag joins the restriction set
//! 20  MigrateSecureScreenMigration             75   DROP  secure screen and the installer choice removed; the keys are ignored on import
//! 21  RemoveReaderTapMigration                 77   NORM  reader tap flag becomes a navigation mode
//! 22  RenameEnumMigration                      81   NORM  enum names renamed
//! 23  MergeSortTypeDirectionMigration          82   NORM  sort type and direction merged into one string
//! 24  EnableAutoBackupMigration                84   NORM  a backup interval of 0 becomes 12
//! 25  MoveChapterPreferencesMigration          85   NORM  chapter flags moved to library preferences
//! 26  SplitPreferencesMigration                86   NORM  preference file split by purpose
//! 27  PlayerPreferenceMigration                92   NORM  player settings renamed
//! 28  MovePlayerPreferencesMigration           93   NORM  player settings moved (tracking queue DROP)
//! 29  UseWorkManagerMigration                  96   DROP  WorkManager tags; the scheduler owns the jobs
//! 30  RemoveBackgroundJobsMigration            97   DROP  WorkManager tags
//! 31  MigrateTriStateMigration                 99   NORM  tri-state filter keys renamed
//! 32  DontRunJobsMigration                    105   NORM  update restriction value removed
//! 33  RelativeTimestampMigration              106   NORM  relative time flag becomes a mode
//! 34  NoAppStateMigration                     113   DROP  app-state keys are not imported
//! 35  ExternalRepoMigration                   114   NORM  repository entries wrapped as URLs
//! 36  PrivatePreferenceMigration              116   DROP  private keys are not imported
//! 37  PermaTrustExtensionsMigration           117   DROP  trust is decided by the extension store
//! 38  NavigationOptionsMigration              120   NORM  bottom navigation style becomes a start screen
//! 39  LogOutMALMigration                      121   DROP  login state lives in the secret store only
//! 40  EnumsMigration                          123   NORM  player enums stored by name
//! 41  VideoPlayerPreferenceMigration          126   NORM  subtitle selection JSON split into three lists
//! 42  VideoOrientationMigration               127   DROP  player orientation removed; the key is ignored on import
//! 43  CategoryPreferencesCleanupMigration     129   PORT  `kuro_prefs::category::prune_dangling_category_ids`
//! 44  PrefLangMigration                       130   NORM  language codes validated
//! ```
//!
//! Rows 1 to 3 are the three registered steps below. Rows 4 to 44 are the 41 above.

use super::{Migration, Version, steps};

pub static MIGRATIONS: &[Migration] = &[
    Migration::of("SetupBackupCreateMigration", Version::Always, steps::backup_create),
    Migration::of("SetupAnimeLibraryUpdateMigration", Version::Always, steps::anime_library_update),
    Migration::of("SetupMangaLibraryUpdateMigration", Version::Always, steps::manga_library_update),
];
