//! Category write and read tests against a migrated database.

use kuro_platform::kind::Kind;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use kuro_prefs::category::random_sort_seed;
use kuro_prefs::library::{LibraryPrefs, SortSetting};
use kuro_prefs::store::PrefStore;

use super::model::CategoryUpdate;
use super::service::{CategoryService, set_display_mode};
use crate::SYSTEM_CATEGORY_ID;
use crate::error::DomainError;
use crate::library::display_mode::LibraryDisplayMode;
use kuro_platform::sort::{Direction, LibrarySort, SortType};

fn service(kind: Kind) -> (tempfile::TempDir, CategoryService, Arc<LibraryPrefs>) {
    let (dir, db) = crate::testdb::test_db();
    let prefs = Arc::new(LibraryPrefs::new(PrefStore::in_memory()));
    (dir, CategoryService::new(db, kind, prefs.clone()), prefs)
}

fn names(svc: &CategoryService) -> Vec<(i64, String, i64)> {
    svc.repo().all().unwrap().into_iter().map(|c| (c.id, c.name, c.order)).collect()
}

fn user_ids(svc: &CategoryService) -> Vec<i64> {
    svc.repo().all().unwrap().into_iter().filter(|c| !c.is_system()).map(|c| c.id).collect()
}

#[tokio::test]
async fn create_numbers_orders_and_takes_the_global_sort_as_flags() {
    let (_d, svc, _p) = service(Kind::Manga);
    let a = svc.create("X").await.unwrap();
    let b = svc.create("Y").await.unwrap();
    let c = svc.create("Z").await.unwrap();
    assert_eq!((a.order, b.order, c.order), (0, 1, 2));
    assert_eq!(a.flags, 0x40, "default global sort is Alphabetical Ascending");
    assert_eq!(svc.repo().all().unwrap()[0].flags, 64, "fresh system row takes 64 (Q28)");
}

#[tokio::test]
async fn create_with_a_changed_global_sort_copies_it() {
    let (_d, svc, prefs) = service(Kind::Manga);
    let _ = prefs.manga_sorting_mode.set(SortSetting("LAST_READ,DESCENDING".into()));
    assert_eq!(svc.create("A").await.unwrap().flags, 0x04);
}

#[tokio::test]
async fn names_are_trimmed_and_checked_for_emptiness_and_duplicates() {
    let (_d, svc, _p) = service(Kind::Anime);
    assert!(matches!(svc.create("").await, Err(DomainError::InvalidName)));
    assert!(matches!(svc.create("   ").await, Err(DomainError::InvalidName)));
    let trimmed = svc.create("  Hold  ").await.unwrap();
    assert_eq!(trimmed.name, "Hold");
    assert!(matches!(svc.create("Hold").await, Err(DomainError::DuplicateName)));
    svc.create("a").await.unwrap();
    svc.create("A").await.unwrap();
    let hidden = svc.create("Gone").await.unwrap();
    svc.toggle_hidden(hidden.id).await.unwrap();
    assert!(matches!(svc.create("Gone").await, Err(DomainError::DuplicateName)), "hidden names count");
}

#[tokio::test]
async fn category_service_refuses_duplicates_for_both_kinds() {
    let (_d, db) = crate::testdb::test_db();
    let prefs = Arc::new(LibraryPrefs::new(PrefStore::in_memory()));
    for kind in [Kind::Manga, Kind::Anime] {
        let svc = CategoryService::new(db.clone(), kind, prefs.clone());
        assert!(matches!(svc.create("   ").await, Err(DomainError::InvalidName)), "{kind}");
        let first = svc.create("  Reading ").await.unwrap();
        assert_eq!(first.name, "Reading");
        assert!(matches!(svc.create("Reading").await, Err(DomainError::DuplicateName)), "{kind}: the second create is refused");
        let other = svc.create("Other").await.unwrap();
        assert!(matches!(svc.rename(other.id, "Reading").await, Err(DomainError::DuplicateName)), "{kind}: renaming onto a name");
        assert!(matches!(svc.update_partial(CategoryUpdate { id: other.id, name: Some("Reading".into()), ..Default::default() }).await, Err(DomainError::DuplicateName)));
    }
    // the kinds own their names: the same name is free in the other kind
    let other_kind = CategoryService::new(db, Kind::Manga, prefs);
    assert_eq!(other_kind.repo().all().unwrap().iter().filter(|c| c.name == "Reading").count(), 1);
}

#[tokio::test]
async fn rename_to_own_name_is_a_no_op_and_to_another_name_is_a_duplicate() {
    let (_d, svc, _p) = service(Kind::Manga);
    let a = svc.create("A").await.unwrap();
    let b = svc.create("B").await.unwrap();
    svc.rename(a.id, "A").await.unwrap();
    assert!(matches!(svc.rename(a.id, "B").await, Err(DomainError::DuplicateName)));
    svc.rename(b.id, "Bee").await.unwrap();
    assert_eq!(svc.repo().get(b.id).unwrap().unwrap().name, "Bee");
    assert!(matches!(svc.rename(SYSTEM_CATEGORY_ID, "x").await, Err(DomainError::SystemCategory)));
}

#[tokio::test]
async fn toggle_hidden_flips_and_hidden_rows_leave_the_visible_lists() {
    let (_d, svc, _p) = service(Kind::Manga);
    let a = svc.create("A").await.unwrap();
    svc.toggle_hidden(a.id).await.unwrap();
    assert!(svc.repo().get(a.id).unwrap().unwrap().hidden);
    assert!(!svc.repo().visible().unwrap().iter().any(|c| c.id == a.id));
    svc.toggle_hidden(a.id).await.unwrap();
    assert!(!svc.repo().get(a.id).unwrap().unwrap().hidden);
}

#[tokio::test]
async fn update_partial_with_nothing_changes_nothing_and_name_only_changes_the_name() {
    let (_d, svc, _p) = service(Kind::Manga);
    let a = svc.create("A").await.unwrap();
    svc.update_partial(CategoryUpdate { id: a.id, ..Default::default() }).await.unwrap();
    assert_eq!(svc.repo().get(a.id).unwrap().unwrap(), a);
    svc.update_partial(CategoryUpdate { id: a.id, name: Some("Renamed".into()), ..Default::default() }).await.unwrap();
    let after = svc.repo().get(a.id).unwrap().unwrap();
    assert_eq!((after.name.as_str(), after.order, after.flags, after.hidden), ("Renamed", a.order, a.flags, false));
}

#[tokio::test]
async fn move_before_renumbers_and_never_writes_the_system_row() {
    let (_d, svc, _p) = service(Kind::Manga);
    let a = svc.create("A").await.unwrap();
    let b = svc.create("B").await.unwrap();
    let c = svc.create("C").await.unwrap();
    svc.move_before(c.id, Some(a.id)).await.unwrap();
    assert_eq!(user_ids(&svc), vec![c.id, a.id, b.id]);
    let orders: Vec<i64> = svc.repo().all().unwrap().iter().filter(|c| !c.is_system()).map(|c| c.order).collect();
    assert_eq!(orders, vec![0, 1, 2]);
    assert_eq!(svc.repo().get(0).unwrap().unwrap().order, -1, "system sort untouched");
    svc.move_before(c.id, None).await.unwrap();
    assert_eq!(user_ids(&svc), vec![a.id, b.id, c.id]);
    assert!(matches!(svc.move_before(999, None).await, Err(DomainError::NotFound)));
    assert!(matches!(svc.move_before(SYSTEM_CATEGORY_ID, None).await, Err(DomainError::NotFound)));
    assert!(matches!(svc.move_before(a.id, Some(SYSTEM_CATEGORY_ID)).await, Err(DomainError::NotFound)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fifty_concurrent_moves_leave_a_permutation_of_the_user_orders() {
    let (_d, svc, _p) = service(Kind::Anime);
    let svc = Arc::new(svc);
    let mut ids = Vec::new();
    for n in 0..6 {
        ids.push(svc.create(&format!("C{n}")).await.unwrap().id);
    }
    let mut tasks = Vec::new();
    for i in 0..50usize {
        let svc = svc.clone();
        let moved = ids[i % ids.len()];
        let before = if i % 3 == 0 { None } else { Some(ids[(i * 7) % ids.len()]) };
        tasks.push(tokio::spawn(async move {
            let _ = svc.move_before(moved, before).await;
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let mut orders: Vec<i64> = svc.repo().all().unwrap().iter().filter(|c| !c.is_system()).map(|c| c.order).collect();
    orders.sort_unstable();
    assert_eq!(orders, (0..6).collect::<Vec<i64>>());
}

#[tokio::test]
async fn delete_renumbers_every_category_and_cleans_the_preference_sets() {
    let (_d, svc, prefs) = service(Kind::Manga);
    let a = svc.create("A").await.unwrap();
    let b = svc.create("B").await.unwrap();
    let c = svc.create("C").await.unwrap();
    let ids = |v: &[i64]| v.iter().map(|i| i.to_string()).collect::<BTreeSet<String>>();
    let set_of = |key: &'static str| prefs.store().pref::<BTreeSet<String>>(key, BTreeSet::new());
    let update = set_of("library_update_categories");
    let update_exclude = set_of("library_update_categories_exclude");
    let _ = update.set(ids(&[a.id, c.id]));
    let _ = update_exclude.set(ids(&[a.id, b.id]));
    let _ = prefs.default_manga_category.set(a.id as i32);
    let download_new = prefs.store().pref::<BTreeSet<String>>("download_new_categories", BTreeSet::new());
    let _ = download_new.set(ids(&[a.id]));
    svc.delete(a.id).await.unwrap();
    assert_eq!(user_ids(&svc), vec![b.id, c.id]);
    assert_eq!(names(&svc).iter().map(|(_, _, o)| *o).collect::<Vec<_>>(), vec![0, 1, 2], "system row takes index 0");
    assert_eq!(update.get(), ids(&[c.id]));
    assert_eq!(update_exclude.get(), ids(&[b.id]));
    assert_eq!(download_new.get(), BTreeSet::new());
    assert_eq!(prefs.default_manga_category.get(), -1, "the default named the deleted category");
    assert!(matches!(svc.delete(SYSTEM_CATEGORY_ID).await, Err(DomainError::SystemCategory)));
}

#[tokio::test]
async fn deleting_a_category_drops_its_links_and_the_entry_falls_back_to_default() {
    let (_d, db) = crate::testdb::test_db();
    let prefs = Arc::new(LibraryPrefs::new(PrefStore::in_memory()));
    let svc = CategoryService::new(db.clone(), Kind::Manga, prefs);
    let cat = svc.create("A").await.unwrap();
    let manga = kuro_data::entry::EntryRepo::new(db);
    let entry = manga
        .insert(&kuro_data::entry::Entry { favorite: true, source: 1, url: "u".into(), title: "T".into(), ..kuro_data::entry::Entry::create(Kind::Manga) })
        .unwrap();
    svc.set_entry_categories(entry, &[cat.id]).await.unwrap();
    assert_eq!(svc.repo().by_entry(entry, false).unwrap().len(), 1);
    svc.delete(cat.id).await.unwrap();
    assert!(svc.repo().by_entry(entry, false).unwrap().is_empty());
}

#[tokio::test]
async fn set_entry_categories_drops_default_unknown_and_duplicate_ids() {
    let (_d, db) = crate::testdb::test_db();
    let prefs = Arc::new(LibraryPrefs::new(PrefStore::in_memory()));
    let svc = CategoryService::new(db.clone(), Kind::Manga, prefs);
    let two = svc.create("Two").await.unwrap();
    let _one = svc.create("One").await.unwrap();
    let entry = kuro_data::entry::EntryRepo::new(db)
        .insert(&kuro_data::entry::Entry { favorite: true, source: 1, url: "u".into(), title: "T".into(), ..kuro_data::entry::Entry::create(Kind::Manga) })
        .unwrap();
    svc.set_entry_categories(entry, &[two.id, two.id, SYSTEM_CATEGORY_ID, 99]).await.unwrap();
    let got = svc.repo().by_entry(entry, false).unwrap();
    assert_eq!(got.iter().map(|c| c.id).collect::<Vec<_>>(), vec![two.id]);
    svc.set_entry_categories(entry, &[]).await.unwrap();
    assert!(svc.repo().by_entry(entry, false).unwrap().is_empty(), "empty means Default");
}

#[tokio::test]
async fn global_sort_writes_every_category_and_random_draws_one_seed_per_call() {
    let (_d, svc, prefs) = service(Kind::Anime);
    svc.create("A").await.unwrap();
    svc.create("B").await.unwrap();
    let mut draws = 0;
    let random = LibrarySort { ty: SortType::Random, dir: Direction::Ascending };
    svc.set_sort_mode(None, random, || {
        draws += 1;
        4242
    })
    .await
    .unwrap();
    svc.set_sort_mode(None, random, || {
        draws += 1;
        4243
    })
    .await
    .unwrap();
    assert_eq!(draws, 2, "one draw per call, also when Random was already selected");
    assert_eq!(random_sort_seed(Kind::Anime), 4243);
    assert_eq!(prefs.anime_sorting_mode.get().0, "RANDOM,ASCENDING");
    let flags: Vec<i64> = svc.repo().all().unwrap().iter().map(|c| c.flags).collect();
    assert!(flags.iter().all(|f| *f == random.flag()));
}

#[tokio::test]
async fn reset_writes_the_global_sort_to_every_category() {
    let (_d, manga, prefs) = service(Kind::Manga);
    let (_d2, anime, prefs2) = service(Kind::Anime);
    manga.create("M").await.unwrap();
    anime.create("A").await.unwrap();
    let _ = prefs.manga_sorting_mode.set(SortSetting("DATE_ADDED,DESCENDING".into()));
    let _ = prefs2.anime_sorting_mode.set(SortSetting("LAST_SEEN,ASCENDING".into()));
    manga.reset_category_flags().await.unwrap();
    anime.reset_category_flags().await.unwrap();
    assert!(manga.repo().all().unwrap().iter().all(|c| c.flags == LibrarySort { ty: SortType::DateAdded, dir: Direction::Descending }.flag()));
    assert!(anime.repo().all().unwrap().iter().all(|c| c.flags == LibrarySort { ty: SortType::LastActivity, dir: Direction::Ascending }.flag()));
}

#[test]
fn initial_flags_follow_the_global_sort_and_display_mode_is_one_key() {
    let (_d, svc, prefs) = service(Kind::Manga);
    assert_eq!(svc.initial_flags(), 0x40);
    let _ = prefs.manga_sorting_mode.set(SortSetting("UNREAD_COUNT,DESCENDING".into()));
    assert_eq!(svc.initial_flags(), 0x0C);
    set_display_mode(&prefs, LibraryDisplayMode::List);
    assert_eq!(prefs.display_mode.get(), LibraryDisplayMode::List);
}

#[tokio::test]
async fn by_entry_is_ordered_and_visible_filter_drops_hidden() {
    let (_d, db) = crate::testdb::test_db();
    let prefs = Arc::new(LibraryPrefs::new(PrefStore::in_memory()));
    let svc = CategoryService::new(db.clone(), Kind::Manga, prefs);
    let a = svc.create("A").await.unwrap();
    let b = svc.create("B").await.unwrap();
    let entry = kuro_data::entry::EntryRepo::new(db)
        .insert(&kuro_data::entry::Entry { favorite: true, source: 1, url: "u".into(), title: "T".into(), ..kuro_data::entry::Entry::create(Kind::Manga) })
        .unwrap();
    svc.set_entry_categories(entry, &[b.id, a.id]).await.unwrap();
    svc.toggle_hidden(b.id).await.unwrap();
    assert_eq!(svc.repo().by_entry(entry, false).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![a.id, b.id]);
    assert_eq!(svc.repo().by_entry(entry, true).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![a.id]);
}

#[tokio::test]
async fn watch_all_publishes_a_new_snapshot_after_an_insert() {
    let (_d, svc, _p) = service(Kind::Manga);
    let mut sub = svc.repo().watch_all();
    let first = tokio::time::timeout(Duration::from_secs(2), sub.changed()).await.unwrap().unwrap().unwrap();
    assert_eq!(first.len(), 1, "the seeded Default row");
    svc.create("New").await.unwrap();
    let next = tokio::time::timeout(Duration::from_millis(1000), async {
        loop {
            let snap = sub.changed().await.unwrap().unwrap();
            if snap.len() == 2 {
                return snap;
            }
        }
    })
    .await
    .expect("snapshot within the contract window");
    assert!(next.iter().any(|c| c.name == "New"));
}

#[tokio::test]
async fn lifecycle_table_of_area_08_reproduces_the_sort_values() {
    let (_d, svc, _p) = service(Kind::Manga);
    let reading = svc.create("Reading").await.unwrap();
    let plan = svc.create("Plan").await.unwrap();
    let done = svc.create("Done").await.unwrap();
    assert_eq!(names(&svc), vec![(0, String::new(), -1), (reading.id, "Reading".into(), 0), (plan.id, "Plan".into(), 1), (done.id, "Done".into(), 2)]);
    svc.delete(plan.id).await.unwrap();
    assert_eq!(names(&svc), vec![(0, String::new(), 0), (reading.id, "Reading".into(), 1), (done.id, "Done".into(), 2)]);
    let hold = svc.create("Hold").await.unwrap();
    assert_eq!(hold.order, 3);
    svc.move_before(hold.id, Some(reading.id)).await.unwrap();
    assert_eq!(
        names(&svc),
        vec![(0, String::new(), 0), (hold.id, "Hold".into(), 0), (reading.id, "Reading".into(), 1), (done.id, "Done".into(), 2)],
        "step 6: the system row stays 0 and ties with Hold, Default first by id"
    );
}

#[tokio::test]
async fn delete_waits_while_a_reorder_holds_the_lock() {
    let (_d, svc, _p) = service(Kind::Manga);
    let svc = Arc::new(svc);
    let a = svc.create("A").await.unwrap();
    let held = svc.reorder_lock.clone().lock_owned().await;
    let worker = svc.clone();
    let id = a.id;
    let task = tokio::spawn(async move { worker.delete(id).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(svc.repo().get(a.id).unwrap().is_some(), "the delete ran while the reorder lock was held");
    drop(held);
    task.await.unwrap().unwrap();
    assert!(svc.repo().get(a.id).unwrap().is_none());
}

#[tokio::test]
async fn anime_lists_break_sort_ties_by_id_so_default_comes_first() {
    let (_d, svc, _p) = service(Kind::Anime);
    let a = svc.create("A").await.unwrap();
    let b = svc.create("B").await.unwrap();
    svc.delete(a.id).await.unwrap();
    svc.move_before(b.id, None).await.unwrap();
    assert_eq!(svc.repo().all().unwrap().iter().map(|c| (c.id, c.order)).collect::<Vec<_>>(), vec![(0, 0), (b.id, 0)]);
    assert_eq!(svc.repo().all().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![0, b.id]);
    assert_eq!(svc.repo().visible().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![0, b.id]);
}
