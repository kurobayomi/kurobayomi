//! The idempotency harness (T-11a-003). Every registered migration and every legacy conversion rule runs twice on
//! the same fixture, and the two results must be the same, key by key. The Kotlin steps that were not idempotent
//! (the rows marked "no" in the catalogue) pass here because a Rust rule changes a key only when its legacy key is
//! present, and the three registered steps only cancel a job by name.

use std::collections::BTreeSet;
use std::sync::Arc;

use kuro_prefs::legacy::{self, UPDATE_INTERVAL};
use kuro_prefs::store::PrefStore;
use kuro_prefs::values::PrefValue;

use super::list::MIGRATIONS;
use super::{MigrationContext, MigrationError, Strategy, run};
use crate::sched::{InProcessScheduler, PeriodicSpec};

/// The store as sorted key and value pairs, so two stores compare equal when they hold the same keys and values.
fn snapshot(store: &PrefStore) -> Vec<(String, PrefValue)> {
    let mut all = store.all();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all
}

/// Fails on the first key whose value differs between `first` and `second`.
fn assert_same(label: &str, first: &[(String, PrefValue)], second: &[(String, PrefValue)]) {
    let keys: BTreeSet<&String> = first.iter().chain(second).map(|(k, _)| k).collect();
    for key in keys {
        let find = |side: &[(String, PrefValue)]| side.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(find(first), find(second), "{label}: the second run changed {key}");
    }
}

fn set(items: &[&str]) -> PrefValue {
    PrefValue::Set(items.iter().map(|s| (*s).to_owned()).collect())
}

/// Stores that exercise the registered steps: defaults, every restriction, a stopped schedule, odd values.
fn step_fixtures() -> Vec<(&'static str, Vec<(&'static str, PrefValue)>)> {
    vec![
        ("empty profile", vec![]),
        (
            "everything on",
            vec![
                ("pref_library_update_interval_key", PrefValue::Int(24)),
                ("library_update_restriction", set(&["wifi", "network_not_metered", "ac"])),
            ],
        ),
        (
            "everything off",
            vec![("pref_library_update_interval_key", PrefValue::Int(0))],
        ),
        (
            "retired restriction value",
            vec![
                ("pref_library_update_interval_key", PrefValue::Int(12)),
                ("library_update_restriction", set(&["battery_not_low"])),
            ],
        ),
        ("negative interval", vec![("pref_library_update_interval_key", PrefValue::Int(-3))]),
    ]
}

fn run_all_steps(ctx: &MigrationContext) -> Result<bool, MigrationError> {
    let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
    runtime.block_on(run(&Strategy::Initial, ctx, MIGRATIONS, || {}))
}

fn specs(scheduler: &InProcessScheduler) -> Vec<PeriodicSpec> {
    scheduler.snapshot().into_iter().map(|job| job.spec).collect()
}

#[test]
fn every_registered_step_gives_the_same_result_when_it_runs_twice() {
    for (label, entries) in step_fixtures() {
        let store = PrefStore::in_memory();
        for (key, value) in &entries {
            store.set_raw(key, value.clone());
        }
        let scheduler = Arc::new(InProcessScheduler::default());
        let ctx = MigrationContext::new(false).with_prefs(store.clone()).with_scheduler(scheduler.clone());

        run_all_steps(&ctx).expect("first run");
        let (first_store, first_jobs) = (snapshot(&store), specs(&scheduler));
        run_all_steps(&ctx).expect("second run");
        assert_same(label, &first_store, &snapshot(&store));
        assert_eq!(first_jobs, specs(&scheduler), "{label}: the second run changed the registered jobs");
    }
}

#[test]
fn each_registered_step_alone_is_idempotent() {
    for migration in MIGRATIONS {
        let store = PrefStore::in_memory();
        store.set_raw("pref_library_update_interval_key", PrefValue::Int(6));
        let scheduler = Arc::new(InProcessScheduler::default());
        let ctx = MigrationContext::new(false).with_prefs(store.clone()).with_scheduler(scheduler.clone());
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");

        let first = runtime.block_on((migration.run)(&ctx)).expect("first run");
        let (first_store, first_jobs) = (snapshot(&store), specs(&scheduler));
        let second = runtime.block_on((migration.run)(&ctx)).expect("second run");
        assert_eq!(first, second, "{}: the result changed", migration.name);
        assert_same(migration.name, &first_store, &snapshot(&store));
        assert_eq!(first_jobs, specs(&scheduler), "{}: the registered jobs changed", migration.name);
    }
}

fn entry(key: &str, value: PrefValue) -> (String, PrefValue) {
    (key.to_owned(), value)
}

/// Entries that exercise every conversion rule of `kuro_prefs::legacy` once.
fn legacy_fixtures() -> Vec<(&'static str, Vec<(String, PrefValue)>)> {
    vec![
        ("a valid interval", vec![entry(UPDATE_INTERVAL, PrefValue::Int(24))]),
        ("a retired quick interval", vec![entry(UPDATE_INTERVAL, PrefValue::Int(6))]),
        (
            "ongoing only turned off",
            vec![
                entry(legacy::LEGACY_UPDATE_ONLY_NON_COMPLETED, PrefValue::Bool(false)),
                entry(legacy::UPDATE_RESTRICTIONS, set(&["manga_ongoing", "manga_fully_read"])),
            ],
        ),
        (
            "ongoing only left on",
            vec![
                entry(legacy::LEGACY_UPDATE_ONLY_NON_COMPLETED, PrefValue::Bool(true)),
                entry(legacy::UPDATE_RESTRICTIONS, set(&["manga_ongoing"])),
            ],
        ),
        (
            "repository sets",
            vec![entry("extension_repos", set(&["user/repo"])), entry("anime_extension_repos", set(&["https://a.test"]))],
        ),
        ("nothing legacy", vec![entry("theme_mode", PrefValue::Str("DARK".into()))]),
    ]
}

fn sorted(mut entries: Vec<(String, PrefValue)>) -> Vec<(String, PrefValue)> {
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

#[test]
fn every_legacy_rule_gives_the_same_result_when_it_runs_twice() {
    for (label, fixture) in legacy_fixtures() {
        let mut once = fixture.clone();
        legacy::convert(&mut once);
        let mut twice = once.clone();
        legacy::convert(&mut twice);
        assert_same(label, &sorted(once), &sorted(twice));
    }
}

/// The interval rule maps 1 and 2 hours to 3, and 3 hours is itself retired, so a second pass moves it on to 12. That
/// is the end state of the Kotlin chain (step 61 then step 71), so a second pass reaches the fixed point and every
/// later pass is stable. This test pins that: the rule is idempotent from its second application on.
#[test]
fn the_interval_rule_reaches_its_fixed_point_on_the_second_pass() {
    for hours in [1, 2] {
        let mut entries = vec![entry(UPDATE_INTERVAL, PrefValue::Int(hours))];
        legacy::convert(&mut entries);
        assert_eq!(entries, vec![entry(UPDATE_INTERVAL, PrefValue::Int(3))]);
        legacy::convert(&mut entries);
        let fixed_point = entries.clone();
        assert_eq!(fixed_point, vec![entry(UPDATE_INTERVAL, PrefValue::Int(12))], "the Kotlin chain ends at 12");
        legacy::convert(&mut entries);
        assert_eq!(entries, fixed_point, "a third pass changes nothing");
    }
}
