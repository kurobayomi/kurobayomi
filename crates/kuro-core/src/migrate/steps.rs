//! The bodies of the three always-run migrations (T-11a-004 to T-11a-006): they cancel the automatic backup job and
//! the manga and anime library update jobs on every start. The automatic library update interval is cut (a pass runs
//! at every launch), and automatic backups are cut (a backup is made by hand, 2026-11-21), so no job is registered;
//! the steps only cancel a job an earlier run registered.
//!
//! The job names are the unique names the scheduler keys on. Cancelling one job never touches another.

use kuro_platform::kind::Kind;

use kuro_prefs::store::PrefStore;

use super::{MigrationContext, MigrationFuture};
use crate::sched::JobScheduler;

/// Unique name of the automatic backup job an earlier run registered.
pub const BACKUP_JOB: &str = "BackupCreator";
/// Unique name of the manga library update job.
pub const MANGA_UPDATE_JOB: &str = "LibraryUpdate";
/// Unique name of the anime library update job.
pub const ANIME_UPDATE_JOB: &str = "AnimeLibraryUpdate";

pub trait MediaExt {
    fn job_name(self) -> &'static str;
}

impl MediaExt for Kind {
    fn job_name(self) -> &'static str {
        match self {
            Self::Manga => MANGA_UPDATE_JOB,
            Self::Anime => ANIME_UPDATE_JOB,
        }
    }
}

/// `BackupCreateJob.setupTask`. Automatic backups are cut, so this only cancels a job an earlier run registered.
pub fn setup_backup(scheduler: &dyn JobScheduler) {
    scheduler.cancel_unique(BACKUP_JOB);
}

/// `MangaLibraryUpdateJob.setupTask` and its anime twin. The automatic update interval is cut, so this only cancels
/// a job an earlier run registered.
pub fn setup_library_update(media: Kind, scheduler: &dyn JobScheduler) {
    scheduler.cancel_unique(media.job_name());
}

/// Hands a step the store and the scheduler, or answers `Ok(false)` when a test left one out.
fn services(ctx: &MigrationContext) -> Option<(&PrefStore, &dyn JobScheduler)> {
    let found = ctx.prefs().zip(ctx.scheduler());
    if found.is_none() {
        tracing::debug!("a migration needs the preference store and the scheduler; skipped");
    }
    found
}

/// `SetupBackupCreateMigration`.
pub fn backup_create(ctx: &MigrationContext) -> MigrationFuture<'_> {
    Box::pin(async move {
        let Some((_, scheduler)) = services(ctx) else { return Ok(false) };
        setup_backup(scheduler);
        Ok(true)
    })
}

/// `SetupMangaLibraryUpdateMigration`.
pub fn manga_library_update(ctx: &MigrationContext) -> MigrationFuture<'_> {
    Box::pin(async move {
        let Some((_, scheduler)) = services(ctx) else { return Ok(false) };
        setup_library_update(Kind::Manga, scheduler);
        Ok(true)
    })
}

/// `SetupAnimeLibraryUpdateMigration`.
pub fn anime_library_update(ctx: &MigrationContext) -> MigrationFuture<'_> {
    Box::pin(async move {
        let Some((_, scheduler)) = services(ctx) else { return Ok(false) };
        setup_library_update(Kind::Anime, scheduler);
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::sched::{InProcessScheduler, PeriodicSpec};

    /// Records every call, so a test can see the order and the specs.
    #[derive(Default)]
    struct Recording {
        calls: Mutex<Vec<String>>,
    }

    impl JobScheduler for Recording {
        fn enqueue_unique_periodic(&self, spec: PeriodicSpec) {
            self.calls.lock().unwrap().push(format!("enqueue {}", spec.name));
        }

        fn cancel_unique(&self, name: &str) {
            self.calls.lock().unwrap().push(format!("cancel {name}"));
        }
    }

    impl Recording {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[test]
    fn the_backup_step_only_cancels_its_own_job() {
        let rec = Recording::default();
        setup_backup(&rec);
        assert_eq!(rec.calls(), vec!["cancel BackupCreator"]);
    }

    #[test]
    fn the_library_update_steps_only_cancel_their_own_job() {
        let rec = Recording::default();
        setup_library_update(Kind::Manga, &rec);
        setup_library_update(Kind::Anime, &rec);
        assert_eq!(rec.calls(), vec!["cancel LibraryUpdate", "cancel AnimeLibraryUpdate"]);
    }

    fn run_step(step: for<'a> fn(&'a MigrationContext) -> MigrationFuture<'a>, ctx: &MigrationContext) -> bool {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(step(ctx)).unwrap()
    }

    #[test]
    fn the_steps_return_true_with_a_scheduler_and_false_without() {
        let store = PrefStore::in_memory();
        let sched = Arc::new(InProcessScheduler::default());
        let ctx = MigrationContext::new(false).with_prefs(store.clone()).with_scheduler(sched.clone());
        assert!(run_step(backup_create, &ctx));
        assert!(run_step(anime_library_update, &ctx));
        assert!(run_step(manga_library_update, &ctx));
        assert!(sched.snapshot().is_empty(), "no automatic backup or library update job");

        let bare = MigrationContext::new(false).with_prefs(store);
        assert!(!run_step(backup_create, &bare), "no scheduler in the context");
        assert!(!run_step(manga_library_update, &MigrationContext::new(false)), "no store either");
    }

    #[test]
    fn running_the_steps_twice_registers_no_job() {
        let store = PrefStore::in_memory();
        let sched = Arc::new(InProcessScheduler::default());
        let ctx = MigrationContext::new(false).with_prefs(store).with_scheduler(sched.clone());
        for step in [backup_create, anime_library_update, manga_library_update] {
            run_step(step, &ctx);
        }
        let once: Vec<PeriodicSpec> = sched.snapshot().into_iter().map(|j| j.spec).collect();
        for step in [backup_create, anime_library_update, manga_library_update] {
            run_step(step, &ctx);
        }
        let twice: Vec<PeriodicSpec> = sched.snapshot().into_iter().map(|j| j.spec).collect();
        assert_eq!(once, twice);
        assert!(twice.is_empty());
    }
}
