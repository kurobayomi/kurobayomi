//! The bodies of the three always-run migrations (T-11a-004 to T-11a-006): they register the periodic job of the
//! automatic backup from the stored preferences, and cancel the manga and anime library update jobs, on every start.
//! The automatic library update interval is cut (a pass runs at every launch), so no library update job is registered.
//!
//! The job names are the unique names the scheduler keys on. Calling a function twice replaces the job; an interval
//! of 0 cancels it. Changing one media type never touches the other's job.

use kuro_platform::kind::Kind;
use std::time::Duration;

use kuro_prefs::store::PrefStore;

use super::{MigrationContext, MigrationFuture};
use crate::sched::{BACKOFF_BASE, Backoff, Constraints, FLEX, JobScheduler, PeriodicSpec};

/// Unique name of the automatic backup job.
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

fn hours(hours: u32) -> Duration {
    Duration::from_secs(u64::from(hours) * 3600)
}

/// `BackupCreateJob.setupTask`: a periodic job every `backup_interval` hours (default 12), or no job when it is 0.
pub fn setup_backup(store: &PrefStore, scheduler: &dyn JobScheduler) {
    let interval = u32::try_from(store.pref("backup_interval", 12i32).get()).unwrap_or(0);
    if interval == 0 {
        scheduler.cancel_unique(BACKUP_JOB);
        return;
    }
    scheduler.enqueue_unique_periodic(PeriodicSpec {
        name: BACKUP_JOB,
        interval: hours(interval),
        flex: FLEX,
        constraints: Constraints { battery_not_low: true, ..Constraints::NONE },
        backoff: Backoff::Exponential(BACKOFF_BASE),
    });
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
        let Some((store, scheduler)) = services(ctx) else { return Ok(false) };
        setup_backup(store, scheduler);
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
    use crate::sched::InProcessScheduler;

    /// Records every call, so a test can see the order and the specs.
    #[derive(Default)]
    struct Recording {
        calls: Mutex<Vec<String>>,
        specs: Mutex<Vec<PeriodicSpec>>,
    }

    impl JobScheduler for Recording {
        fn enqueue_unique_periodic(&self, spec: PeriodicSpec) {
            self.calls.lock().unwrap().push(format!("enqueue {}", spec.name));
            self.specs.lock().unwrap().push(spec);
        }

        fn cancel_unique(&self, name: &str) {
            self.calls.lock().unwrap().push(format!("cancel {name}"));
        }
    }

    impl Recording {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn only_spec(&self) -> PeriodicSpec {
            let specs = self.specs.lock().unwrap();
            assert_eq!(specs.len(), 1, "exactly one job was registered");
            specs[0].clone()
        }
    }

    fn set_hours(store: &PrefStore, key: &'static str, value: i32) {
        store.pref(key, 0i32).set(value).unwrap();
    }

    #[test]
    fn backup_interval_twelve_registers_one_job_named_backup_creator() {
        let store = PrefStore::in_memory();
        set_hours(&store, "backup_interval", 12);
        let sched = InProcessScheduler::default();
        setup_backup(&store, &sched);
        let jobs = sched.snapshot();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "BackupCreator");
        assert_eq!(jobs[0].spec.interval, Duration::from_secs(12 * 3600));
    }

    #[test]
    fn the_default_backup_spec_follows_the_android_job() {
        let store = PrefStore::in_memory();
        let rec = Recording::default();
        setup_backup(&store, &rec);
        let spec = rec.only_spec();
        assert_eq!(spec.interval, Duration::from_secs(43_200), "the default is 12 hours");
        assert_eq!(spec.flex, Duration::from_secs(600));
        assert_eq!(spec.constraints, Constraints { battery_not_low: true, ..Constraints::NONE });
        assert_eq!(spec.backoff, Backoff::Exponential(Duration::from_secs(600)));
    }

    #[test]
    fn backup_interval_zero_cancels_and_registers_nothing() {
        let store = PrefStore::in_memory();
        set_hours(&store, "backup_interval", 0);
        let rec = Recording::default();
        setup_backup(&store, &rec);
        assert_eq!(rec.calls(), vec!["cancel BackupCreator"]);
    }

    #[test]
    fn setting_up_twice_replaces_the_job_instead_of_duplicating_it() {
        let store = PrefStore::in_memory();
        let sched = InProcessScheduler::default();
        setup_backup(&store, &sched);
        set_hours(&store, "backup_interval", 6);
        setup_backup(&store, &sched);
        let jobs = sched.snapshot();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].spec.interval, Duration::from_secs(6 * 3600));
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
        assert_eq!(sched.snapshot().len(), 1, "the backup; no library update job");

        let bare = MigrationContext::new(false).with_prefs(store);
        assert!(!run_step(backup_create, &bare), "no scheduler in the context");
        assert!(!run_step(manga_library_update, &MigrationContext::new(false)), "no store either");
    }

    #[test]
    fn running_the_steps_twice_leaves_one_job_per_name_with_the_same_spec() {
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
        assert_eq!(twice.len(), 1);
    }
}
