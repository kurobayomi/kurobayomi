//! The bodies of the three always-run migrations (T-11a-004 to T-11a-006): they register the periodic jobs of the
//! automatic backup and of the manga and anime library updates, from the stored preferences, on every start. The
//! settings pages and the preference restore call the same two functions, [`setup_backup`] and
//! [`setup_library_update`], after a value changes.
//!
//! The job names are the unique names the scheduler keys on. Calling a function twice replaces the job; an interval
//! of 0 cancels it. Changing one media type never touches the other's job.

use kuro_platform::kind::Kind;
use std::collections::BTreeSet;
use std::time::Duration;

use kuro_prefs::library::LibraryPrefs;
use kuro_prefs::store::PrefStore;

use super::{MigrationContext, MigrationFuture};
use crate::sched::{BACKOFF_BASE, Backoff, Constraints, FLEX, JobScheduler, PeriodicSpec};

/// Unique name of the automatic backup job.
pub const BACKUP_JOB: &str = "BackupCreator";
/// Unique name of the manga library update job.
pub const MANGA_UPDATE_JOB: &str = "LibraryUpdate";
/// Unique name of the anime library update job.
pub const ANIME_UPDATE_JOB: &str = "AnimeLibraryUpdate";

/// Restriction value: the update needs a connection (`DEVICE_ONLY_ON_WIFI`).
pub const RESTRICTION_WIFI: &str = "wifi";
/// Restriction value: the update needs an unmetered connection (`DEVICE_NETWORK_NOT_METERED`).
pub const RESTRICTION_UNMETERED: &str = "network_not_metered";
/// Restriction value: the update needs external power (`DEVICE_CHARGING`).
pub const RESTRICTION_CHARGING: &str = "ac";

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

/// The constraints of a library update job from the `library_update_restriction` values. `wifi` needs a connection,
/// `network_not_metered` needs an unmetered one, `ac` needs power, and the battery must never be low. Unknown values
/// (the retired `battery_not_low`) add nothing.
pub fn library_update_constraints(restrictions: &BTreeSet<String>) -> Constraints {
    Constraints {
        requires_connected: restrictions.contains(RESTRICTION_WIFI),
        requires_unmetered: restrictions.contains(RESTRICTION_UNMETERED),
        requires_ac: restrictions.contains(RESTRICTION_CHARGING),
        battery_not_low: true,
    }
}

/// `MangaLibraryUpdateJob.setupTask` and its anime twin: a periodic job every `pref_library_update_interval_key`
/// hours, or no job when it is 0. `override_hours` stands in for the stored interval when a caller runs before the
/// store holds the new value.
pub fn setup_library_update(
    media: Kind,
    store: &PrefStore,
    scheduler: &dyn JobScheduler,
    override_hours: Option<u32>,
) {
    let prefs = LibraryPrefs::new(store.clone());
    let interval = override_hours.unwrap_or_else(|| u32::try_from(prefs.auto_update_interval.get()).unwrap_or(0));
    if interval == 0 {
        scheduler.cancel_unique(media.job_name());
        return;
    }
    scheduler.enqueue_unique_periodic(PeriodicSpec {
        name: media.job_name(),
        interval: hours(interval),
        flex: FLEX,
        // the settings diet cut `library_update_restriction`: no device restriction, only the low-battery rule
        constraints: library_update_constraints(&BTreeSet::new()),
        backoff: Backoff::Linear(BACKOFF_BASE),
    });
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
        let Some((store, scheduler)) = services(ctx) else { return Ok(false) };
        setup_library_update(Kind::Manga, store, scheduler, None);
        Ok(true)
    })
}

/// `SetupAnimeLibraryUpdateMigration`.
pub fn anime_library_update(ctx: &MigrationContext) -> MigrationFuture<'_> {
    Box::pin(async move {
        let Some((store, scheduler)) = services(ctx) else { return Ok(false) };
        setup_library_update(Kind::Anime, store, scheduler, None);
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
    fn manga_interval_six_gives_one_job_with_only_the_battery_constraint() {
        let store = PrefStore::in_memory();
        set_hours(&store, "pref_library_update_interval_key", 6);
        let sched = InProcessScheduler::default();
        setup_library_update(Kind::Manga, &store, &sched, None);
        let jobs = sched.snapshot();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "LibraryUpdate");
        assert_eq!(jobs[0].spec.interval, Duration::from_secs(6 * 3600));
        assert!(!jobs[0].spec.constraints.requires_connected, "a wired desktop must not be blocked");
        assert!(!jobs[0].spec.constraints.requires_unmetered && !jobs[0].spec.constraints.requires_ac);
        assert!(jobs[0].spec.constraints.battery_not_low);
        assert_eq!(jobs[0].spec.backoff, Backoff::Linear(Duration::from_secs(600)));
    }

    #[test]
    fn no_restriction_leaves_only_the_battery_constraint() {
        assert_eq!(library_update_constraints(&BTreeSet::new()), Constraints { battery_not_low: true, ..Constraints::NONE });
    }

    #[test]
    fn a_library_interval_of_zero_cancels_only_that_job() {
        let store = PrefStore::in_memory();
        let rec = Recording::default();
        setup_library_update(Kind::Manga, &store, &rec, None);
        setup_library_update(Kind::Anime, &store, &rec, None);
        assert_eq!(rec.calls(), vec!["cancel LibraryUpdate", "cancel AnimeLibraryUpdate"], "the default interval is 0");
    }

    #[test]
    fn the_override_wins_over_the_stored_interval() {
        let store = PrefStore::in_memory();
        set_hours(&store, "pref_library_update_interval_key", 12);
        let rec = Recording::default();
        setup_library_update(Kind::Manga, &store, &rec, Some(48));
        assert_eq!(rec.only_spec().interval, Duration::from_secs(48 * 3600));
        let off = Recording::default();
        setup_library_update(Kind::Manga, &store, &off, Some(0));
        assert_eq!(off.calls(), vec!["cancel LibraryUpdate"], "an override of 0 turns the job off");
    }

    #[test]
    fn anime_uses_its_own_name_and_cancelling_it_leaves_manga_intact() {
        let store = PrefStore::in_memory();
        set_hours(&store, "pref_library_update_interval_key", 24);
        let sched = InProcessScheduler::default();
        setup_library_update(Kind::Manga, &store, &sched, None);
        setup_library_update(Kind::Anime, &store, &sched, None);
        let names: Vec<&str> = sched.snapshot().iter().map(|j| j.name).collect();
        assert_eq!(names, vec!["AnimeLibraryUpdate", "LibraryUpdate"]);

        setup_library_update(Kind::Anime, &store, &sched, Some(0));
        let names: Vec<&str> = sched.snapshot().iter().map(|j| j.name).collect();
        assert_eq!(names, vec!["LibraryUpdate"], "cancel anime, the manga job is still present");
    }

    fn run_step(step: for<'a> fn(&'a MigrationContext) -> MigrationFuture<'a>, ctx: &MigrationContext) -> bool {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(step(ctx)).unwrap()
    }

    #[test]
    fn the_steps_return_true_with_a_scheduler_and_false_without() {
        let store = PrefStore::in_memory();
        set_hours(&store, "pref_library_update_interval_key", 6);
        let sched = Arc::new(InProcessScheduler::default());
        let ctx = MigrationContext::new(false).with_prefs(store.clone()).with_scheduler(sched.clone());
        assert!(run_step(backup_create, &ctx));
        assert!(run_step(anime_library_update, &ctx));
        assert!(run_step(manga_library_update, &ctx));
        assert_eq!(sched.snapshot().len(), 3, "backup, anime and manga");

        let bare = MigrationContext::new(false).with_prefs(store);
        assert!(!run_step(backup_create, &bare), "no scheduler in the context");
        assert!(!run_step(manga_library_update, &MigrationContext::new(false)), "no store either");
    }

    #[test]
    fn running_the_steps_twice_leaves_one_job_per_name_with_the_same_spec() {
        let store = PrefStore::in_memory();
        set_hours(&store, "pref_library_update_interval_key", 6);
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
        assert_eq!(twice.len(), 3);
    }
}
