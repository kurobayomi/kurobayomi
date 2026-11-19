//! The database side of the anime page: the entry, its episodes and seasons, the masked flag writes, the library defaults
//! and the live subscriptions. Every method is synchronous; the runner calls them on the blocking pool. Nothing here imports
//! GTK. The writes both pages make the same way are `entry_ops`. Upstream: `AnimeScreenModel.kt:204-283` (init),
//! `SetAnimeEpisodeFlags.kt`.

use std::sync::Arc;

use kuro_data::DataResult;
use kuro_data::anime_relation::AnimeRelationGroup;
use kuro_data::anime_relation::AnimeRelationRepo;
use kuro_data::category::CategoryRepo;
use kuro_data::community_score::{CommunityScore, CommunityScoreRepo};
use kuro_data::entry::{Entry, EntryRepo, SeasonEntry};
use kuro_data::item::{Item, ItemRepo, ItemUpdate};
use kuro_data::track::TrackRepo;
use kuro_db::handle::Db;
use kuro_db::subscribe::Subscription;
use kuro_domain::entries::defaults::{apply_item_defaults, apply_season_defaults};
use kuro_domain::entries::viewer_flags::set_skip_intro_length;
use kuro_platform::kind::Kind;
use kuro_prefs::store::PrefStore;

use super::actions::Mark;
use crate::entry_core::entry_ops;
use crate::entry_core::header::Scores;
use crate::entry_core::library_flow::LibraryRepos;
use crate::entry_core::refresh_rules::{self, Trigger};
use crate::entry_core::tracking::facts::Summary;
use crate::entry_core::tracking::session;

/// The scanlators of a manga's chapters: the ones the chapters have and the ones the reader excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scanlators {
    pub available: Vec<String>,
    pub excluded: std::collections::BTreeSet<String>,
}

/// Typed access to the tables of one details page, for an entry of one kind. Cheap to clone: it shares one `Db`.
#[derive(Clone)]
pub struct EntryStore {
    kind: Kind,
    entries: EntryRepo,
    items: ItemRepo,
    categories: CategoryRepo,
    tracks: TrackRepo,
    db: Arc<Db>,
}

impl EntryStore {
    pub fn new(db: Arc<Db>, kind: Kind) -> Self {
        Self {
            kind,
            entries: EntryRepo::new(Arc::clone(&db)),
            items: ItemRepo::new(Arc::clone(&db)),
            categories: CategoryRepo::new(Arc::clone(&db), kind),
            tracks: TrackRepo::new(Arc::clone(&db)),
            db,
        }
    }

    /// The kind of the entries of this page.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The database handle, for the pieces that work on it directly (the source refresh).
    pub fn db(&self) -> Arc<Db> {
        Arc::clone(&self.db)
    }

    /// The ids of the categories the entry is in, for the auto-download rule. Empty means uncategorized.
    pub fn category_ids(&self, entry_id: i64) -> DataResult<Vec<i64>> {
        Ok(self.categories.by_entry(entry_id)?.into_iter().map(|c| c.id).collect())
    }

    #[cfg(test)]
    /// How many trackers the entry is bound to, for the tracking button of the action row.
    pub fn track_count(&self, entry_id: i64) -> DataResult<usize> {
        Ok(self.tracks.by_entry(entry_id)?.len())
    }

    /// The tracking count and the logged-in flag of the entry, through the same filter as the sheet (T-TRK-12, D-10).
    pub fn track_summary(&self, entry_id: i64, source_id: i64) -> DataResult<Summary> {
        entry_ops::track_summary(&self.tracks, self.kind, entry_id, source_id)
    }

    /// The scores of the header's score line: the stored community score with the title it was matched as, and the
    /// user's own over the trackers logged in right now (`entry_ops::own_score`).
    pub fn scores(&self, entry_id: i64) -> DataResult<Scores> {
        let community = CommunityScoreRepo::new(self.db()).by_entry(entry_id)?.and_then(|row| row.score.map(|score| (score, row.matched_title)));
        let own = entry_ops::own_score(&self.tracks, entry_id, &session::logged_in_ids())?;
        Ok(Scores { community, own })
    }

    /// The stored community score of the entry, published again after each write to the score store.
    pub fn watch_community(&self, entry_id: i64) -> Subscription<Option<CommunityScore>> {
        CommunityScoreRepo::new(self.db()).watch_by_entry(entry_id)
    }

    /// Every category (the default row `0` included, the dialog drops it) and the ids the entry is linked to.
    pub fn categories(&self, entry_id: i64) -> DataResult<(Vec<(i64, String)>, Vec<i64>)> {
        entry_ops::categories(&self.categories, entry_id)
    }

    /// The repositories of the library button (`library_flow`).
    pub fn library(&self, prefs: &PrefStore) -> LibraryRepos {
        LibraryRepos::new(self.entries.clone(), self.categories.clone(), prefs.clone())
    }

    /// The new episodes a refresh queues (`refresh_rules::new_item_targets`).
    pub fn new_item_targets(&self, prefs: &PrefStore, id: i64, trigger: Trigger, new_items: &[Item]) -> DataResult<Vec<Item>> {
        refresh_rules::new_item_targets(&self.entries, &self.categories, &self.items, prefs, self.kind, id, trigger, new_items)
    }

    /// Replaces the category links of the entry in one transaction.
    pub fn set_categories(&self, entry_id: i64, category_ids: &[i64]) -> DataResult<()> {
        self.entries.set_categories(entry_id, category_ids)
    }

    /// The entry, or `None` when the row does not exist.
    pub fn entry(&self, id: i64) -> DataResult<Option<Entry>> {
        entry_ops::entry_or_none(&self.entries, id)
    }

    /// Every episode of the entry (the scanlator exclusions are a manga rule).
    pub fn episodes(&self, entry_id: i64) -> DataResult<Vec<Item>> {
        self.items.by_entry_id(entry_id, self.kind.is_manga())
    }

    pub fn scanlators(&self, entry_id: i64) -> DataResult<Scanlators> {
        Ok(Scanlators {
            available: self.entries.available_scanlators(entry_id)?.into_iter().collect(),
            excluded: self.entries.excluded_scanlators(entry_id)?,
        })
    }

    /// The excluded set becomes exactly `wanted`, in one transaction.
    pub fn set_excluded_scanlators(&self, entry_id: i64, wanted: &std::collections::BTreeSet<String>) -> DataResult<()> {
        self.entries.set_excluded_scanlators(entry_id, wanted)
    }

    pub fn watch_available(&self, entry_id: i64) -> Subscription<std::collections::BTreeSet<String>> {
        self.entries.subscribe_available_scanlators(entry_id)
    }

    pub fn watch_excluded(&self, entry_id: i64) -> Subscription<std::collections::BTreeSet<String>> {
        self.entries.subscribe_excluded_scanlators(entry_id)
    }

    pub fn seasons(&self, parent_id: i64) -> DataResult<Vec<SeasonEntry>> {
        self.entries.seasons_by_parent(parent_id)
    }

    /// One masked UPDATE of `entries.item_flags`: `(old & !mask) | (flag & mask)`.
    pub fn set_flag_bits(&self, entry_id: i64, flag: i64, mask: i64) -> DataResult<bool> {
        self.entries.set_item_flags_masked(entry_id, flag, mask)
    }

    /// The skip-intro length of the entry: the length bits and the disable bit, in one masked write.
    pub fn set_skip_intro_length(&self, entry_id: i64, length: i32) -> DataResult<bool> {
        set_skip_intro_length(&self.entries, entry_id, i64::from(length))
    }

    /// One masked UPDATE of `entries.season_flags`: the write of a pick in the season list options.
    pub fn set_season_flag_bits(&self, entry_id: i64, flag: i64, mask: i64) -> DataResult<bool> {
        self.entries.set_season_flags_masked(entry_id, flag, mask)
    }

    /// "Use as default for all titles" on an episode list: the episode default word goes to every
    /// favorite, and nothing else does (`setCurrentSettingsAsDefault` calls `setAnimeDefaultEpisodeFlags` only).
    /// Returns how many entries changed.
    pub fn apply_episode_defaults_to_library(&self, prefs: &PrefStore) -> DataResult<usize> {
        let mut changed = 0;
        for entry in self.entries.favorites(self.kind)? {
            if apply_item_defaults(&self.entries, prefs, &entry)? {
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// The same for a season list: the season default word goes to every favorite, the episode flags stay.
    pub fn apply_season_defaults_to_library(&self, prefs: &PrefStore) -> DataResult<usize> {
        let mut changed = 0;
        for entry in self.entries.favorites(self.kind)? {
            if apply_season_defaults(&self.entries, prefs, &entry)? {
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// The related groups of the entry, published again after a change to the relations or the anime they link.
    pub fn watch_related(&self, entry_id: i64) -> Subscription<Vec<AnimeRelationGroup>> {
        AnimeRelationRepo::new(self.db()).subscribe_related(entry_id)
    }

    /// Sets the bookmark or fillermark of exactly these episodes, in one transaction. The caller passes the ids that
    /// differ (`actions::bulk_mark_targets`), so no row is written that does not change.
    pub fn set_marks(&self, ids: &[i64], mark: Mark, on: bool) -> DataResult<()> {
        let updates: Vec<ItemUpdate> = ids
            .iter()
            .map(|id| match mark {
                Mark::Bookmark => ItemUpdate { bookmark: Some(on), ..ItemUpdate::new(*id) },
                Mark::Fillermark => ItemUpdate { fillermark: Some(on), ..ItemUpdate::new(*id) },
            })
            .collect();
        if updates.is_empty() { Ok(()) } else { self.items.update_all(&updates) }
    }

    /// `setFetchInterval` for an anime: stores `-wheel` (0 is automatic) and recomputes the next update (T-39a-026).
    pub fn set_fetch_interval(&self, entry_id: i64, wheel: i32) -> DataResult<()> {
        entry_ops::set_fetch_interval(&self.entries, &self.items, entry_id, wheel)
    }

    /// Marks these episodes seen or unseen in one transaction. Unseen also resets the position (`SetSeenStatus`).
    pub fn set_seen(&self, ids: &[i64], seen: bool) -> DataResult<()> {
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            rows.extend(self.items.by_id(*id)?);
        }
        entry_ops::mark_done(&self.items, &rows, seen).map(drop)
    }

    pub fn watch_anime(&self, entry_id: i64) -> Subscription<Option<Entry>> {
        self.entries.subscribe_by_id(entry_id)
    }

    pub fn watch_episodes(&self, entry_id: i64) -> Subscription<Vec<Item>> {
        self.items.subscribe_by_entry_id(entry_id, self.kind.is_manga())
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use kuro_db::codec::FetchType;

    use super::*;

    impl EntryStore {
        pub fn set_fetch_type_for_test(&self, id: i64, fetch_type: FetchType) {
            let update = kuro_data::entry::EntryUpdate { fetch_type: Some(fetch_type), ..kuro_data::entry::EntryUpdate::new(id) };
            self.entries.update(&update).expect("update");
        }
    }

    /// A migrated temp anime database with one entry (`favorite`, `initialized`) and `episodes` episodes, the first
    /// `seen` of them seen. Keep the `TempDir` alive.
    pub fn with_episodes(favorite: bool, initialized: bool, episodes: usize, seen: usize) -> (tempfile::TempDir, EntryStore, i64) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("anime schema");
        let db = Arc::new(Db::new(&path, writer).expect("anime handle"));
        let id = EntryRepo::new(Arc::clone(&db))
            .insert(&Entry {
                source: 7,
                url: "/a".into(),
                title: "Frieren".into(),
                favorite,
                initialized,
                ..Entry::create(Kind::Anime)
            })
            .expect("insert anime");
        let list: Vec<Item> = (1..=episodes)
            .map(|n| Item {
                entry_id: id,
                url: format!("/e{n}"),
                name: format!("Episode {n}"),
                number: n as f64,
                // a source lists the newest episode first, so the oldest has the highest order
                source_order: (episodes - n) as i64,
                done: n <= seen,
                ..Item::create(Kind::Anime)
            })
            .collect();
        ItemRepo::new(Arc::clone(&db)).add_all(&list).expect("insert episodes");
        (dir, EntryStore::new(db, Kind::Anime), id)
    }
}

#[cfg(test)]
mod tests {
    use kuro_db::flags::item as ep;

    use super::super::actions::{Mark, bulk_mark_targets};
    use super::fixture::with_episodes;
    use super::*;
    use crate::entry_core::favorite::{AddRoute, route_add};
    use crate::entry_core::library_flow::LibraryStore;

    #[test]
    fn favorite_writes_the_flag_and_the_date_and_a_missing_row_is_false() {
        let (_dir, store, id) = with_episodes(false, true, 0, 0);
        let library = store.library(&PrefStore::in_memory());
        assert!(library.add(id, &[], 5_000).unwrap());
        let row = store.entry(id).unwrap().unwrap();
        assert!(row.favorite && row.date_added == 5_000);
        assert!(library.remove(id, 6_000).unwrap());
        let row = store.entry(id).unwrap().unwrap();
        assert!(!row.favorite && row.date_added == 0);
        assert!(!library.add(4242, &[], 1).unwrap() && !library.remove(4242, 1).unwrap());
    }

    #[test]
    fn adding_to_the_library_writes_the_default_flags_and_opening_does_not() {
        let (_dir, store, id) = with_episodes(false, true, 1, 0);
        let prefs = PrefStore::in_memory();
        kuro_domain::entries::defaults::set_item_settings_default(&prefs, Kind::Anime, ep::SHOW_UNDONE | ep::SORT_ASC | ep::SORTING_NUMBER);
        assert_eq!(store.entry(id).unwrap().unwrap().item_flags, 0, "the stored word is untouched until the add");
        assert!(store.library(&prefs).add(id, &[], 9).unwrap());
        let row = store.entry(id).unwrap().unwrap();
        assert!(row.favorite);
        assert_eq!(row.item_flags & ep::UNDONE_MASK, ep::SHOW_UNDONE, "the library default reached the row");
        assert!(!store.library(&prefs).add(4242, &[], 9).unwrap());
    }

    #[test]
    fn the_add_route_takes_the_default_category_and_asks_when_it_is_unset_or_deleted() {
        // T-39b-026: default category exists, default is Uncategorized, default unset or deleted
        let (_dir, store, id) = with_episodes(false, true, 0, 0);
        let cat = kuro_testkit::insert_category(&store.db(), Kind::Anime, "Airing");
        let prefs = PrefStore::in_memory();
        let route = |prefs: &PrefStore| {
            let default = i64::from(kuro_prefs::library::LibraryPrefs::new(prefs.clone()).default_anime_category.get());
            route_add(default, &store.library(prefs).user_categories().unwrap())
        };
        kuro_prefs::library::LibraryPrefs::new(prefs.clone()).default_anime_category.set(cat as i32).unwrap();
        assert_eq!(route(&prefs), AddRoute::AddDefaultCategory(Some(cat)));
        kuro_prefs::library::LibraryPrefs::new(prefs.clone()).default_anime_category.set(0).unwrap();
        assert_eq!(route(&prefs), AddRoute::AddDefaultCategory(None));
        kuro_prefs::library::LibraryPrefs::new(prefs.clone()).default_anime_category.set(-1).unwrap();
        assert_eq!(route(&prefs), AddRoute::ChooseCategory);
        kuro_prefs::library::LibraryPrefs::new(prefs.clone()).default_anime_category.set(9_999).unwrap();
        assert_eq!(route(&prefs), AddRoute::ChooseCategory, "a deleted default asks");
        // the answer writes the links and the favorite together
        assert!(store.library(&prefs).add(id, &[cat], 9).unwrap());
        assert_eq!(store.categories(id).unwrap().1, vec![cat]);
        assert!(store.entry(id).unwrap().unwrap().favorite);
    }

    #[test]
    fn bulk_marks_write_only_the_episodes_that_differ_and_each_flag_alone() {
        let (_dir, store, id) = with_episodes(true, true, 3, 0);
        let all = store.episodes(id).unwrap();
        store.set_marks(&[all[0].id], Mark::Bookmark, true).unwrap();
        let fresh = store.episodes(id).unwrap();
        let targets = bulk_mark_targets(&fresh, Mark::Bookmark, true);
        assert_eq!(targets.len(), 2, "the bookmarked one is left out");
        let before: Vec<i64> = fresh.iter().map(|e| e.version).collect();
        store.set_marks(&targets, Mark::Bookmark, true).unwrap();
        let after = store.episodes(id).unwrap();
        assert!(after.iter().all(|e| e.bookmark && !e.fillermark), "only the bookmark flag changed");
        assert_eq!(after.iter().find(|e| e.id == all[0].id).unwrap().version, before[fresh.iter().position(|e| e.id == all[0].id).unwrap()], "the row that did not differ was not written");
        store.set_marks(&[all[1].id], Mark::Fillermark, true).unwrap();
        assert!(store.episodes(id).unwrap().iter().find(|e| e.id == all[1].id).unwrap().fillermark);
        store.set_marks(&[], Mark::Bookmark, false).unwrap();
    }

    #[test]
    fn seen_and_unseen_writes_reset_the_position_only_for_unseen() {
        let (_dir, store, id) = with_episodes(true, true, 2, 0);
        let ids: Vec<i64> = store.episodes(id).unwrap().iter().map(|e| e.id).collect();
        store.set_seen(&ids, true).unwrap();
        assert!(store.episodes(id).unwrap().iter().all(|e| e.done));
        store.set_seen(&ids[..1], false).unwrap();
        let rows = store.episodes(id).unwrap();
        assert_eq!(rows.iter().filter(|e| e.done).count(), 1);
        assert_eq!(rows.iter().find(|e| e.id == ids[0]).unwrap().progress, 0);
    }

    #[test]
    fn categories_list_every_row_with_the_links_of_the_entry_and_set_replaces_them() {
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        let a = kuro_testkit::insert_category(&store.db(), Kind::Anime, "Airing");
        let b = kuro_testkit::insert_category(&store.db(), Kind::Anime, "Later");
        store.set_categories(id, &[a]).unwrap();
        let (all, linked) = store.categories(id).unwrap();
        assert!(all.iter().any(|(i, n)| *i == a && n == "Airing") && all.iter().any(|(i, _)| *i == b));
        assert_eq!(linked, vec![a]);
        store.set_categories(id, &[b, b]).unwrap();
        assert_eq!(store.categories(id).unwrap().1, vec![b], "a repeated id is linked once");
        store.set_categories(id, &[]).unwrap();
        assert!(store.categories(id).unwrap().1.is_empty());
    }

    #[test]
    fn the_track_count_is_the_number_of_bound_trackers() {
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        assert_eq!(store.track_count(id).unwrap(), 0);
        let repo = TrackRepo::new(store.db());
        repo.insert(&kuro_data::track::Track { entry_id: id, ..kuro_data::track::Track::new(Kind::Anime, 2) }).unwrap();
        assert_eq!(store.track_count(id).unwrap(), 1);
    }

    #[test]
    fn reads_the_entry_and_its_episodes_and_a_missing_row_is_none() {
        let (_dir, store, id) = with_episodes(true, true, 3, 1);
        assert_eq!(store.entry(id).unwrap().unwrap().title, "Frieren");
        assert_eq!(store.episodes(id).unwrap().len(), 3);
        assert!(store.entry(9999).unwrap().is_none());
    }

    #[test]
    fn a_flag_write_changes_only_its_group() {
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        store.set_flag_bits(id, ep::SHOW_UNDONE, ep::UNDONE_MASK).unwrap();
        assert_eq!(store.entry(id).unwrap().unwrap().item_flags & ep::UNDONE_MASK, ep::SHOW_UNDONE);
        store.set_flag_bits(id, ep::SORTING_NUMBER, ep::SORTING_MASK).unwrap();
        store.set_flag_bits(id, ep::SORT_ASC, ep::SORT_DIR_MASK).unwrap();
        let word = store.entry(id).unwrap().unwrap().item_flags;
        assert_eq!((word & ep::SORTING_MASK, word & ep::SORT_DIR_MASK), (ep::SORTING_NUMBER, ep::SORT_ASC));
        assert_eq!(word & ep::UNDONE_MASK, ep::SHOW_UNDONE, "the filter group survives a sort");
    }

    #[test]
    fn the_episode_defaults_reach_every_favorite_and_leave_their_season_flags_alone() {
        use kuro_db::flags::season as sf;
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        store.set_season_flag_bits(id, sf::SHOW_UNSEEN, sf::UNSEEN_MASK).unwrap();
        let prefs = PrefStore::in_memory();
        kuro_domain::entries::defaults::set_item_settings_default(&prefs, Kind::Anime, ep::SHOW_DONE | ep::SORT_ASC | ep::SORTING_NUMBER);
        assert_eq!(store.apply_episode_defaults_to_library(&prefs).unwrap(), 1);
        let row = store.entry(id).unwrap().unwrap();
        assert_eq!(row.item_flags & ep::UNDONE_MASK, ep::SHOW_DONE, "the episode default was applied");
        assert_eq!(row.season_flags() & sf::UNSEEN_MASK, sf::SHOW_UNSEEN, "the season flags are not the user's request");
    }

    #[test]
    fn the_season_defaults_reach_every_favorite_and_leave_their_episode_flags_alone() {
        use kuro_db::flags::season as sf;
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        store.set_flag_bits(id, ep::SHOW_UNDONE, ep::UNDONE_MASK).unwrap();
        let prefs = PrefStore::in_memory();
        kuro_domain::entries::defaults::set_season_settings_default(&prefs, sf::SHOW_COMPLETED);
        assert_eq!(store.apply_season_defaults_to_library(&prefs).unwrap(), 1);
        let row = store.entry(id).unwrap().unwrap();
        assert_eq!(row.season_flags() & sf::COMPLETED_MASK, sf::SHOW_COMPLETED);
        assert_eq!(row.item_flags & ep::UNDONE_MASK, ep::SHOW_UNDONE);
    }

    #[test]
    fn season_writes_are_masked() {
        use kuro_db::flags::season as sf;
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        store.set_season_flag_bits(id, sf::SHOW_STARTED, sf::STARTED_MASK).unwrap();
        store.set_season_flag_bits(id, sf::SHOW_BOOKMARKED, sf::BOOKMARKED_MASK).unwrap();
        let word = store.entry(id).unwrap().unwrap().season_flags();
        assert_eq!((word & sf::STARTED_MASK, word & sf::BOOKMARKED_MASK), (sf::SHOW_STARTED, sf::SHOW_BOOKMARKED));
        store.set_season_flag_bits(id, sf::SORT_UPLOAD, sf::SORT_MASK).unwrap();
        let word = store.entry(id).unwrap().unwrap().season_flags();
        assert_eq!((word & sf::SORT_MASK, word & sf::STARTED_MASK), (sf::SORT_UPLOAD, sf::SHOW_STARTED), "a sort leaves the filters alone");
    }

    #[test]
    fn a_library_entry_with_the_same_title_is_a_duplicate_and_the_entry_itself_is_not() {
        let (_dir, store, id) = with_episodes(false, true, 0, 0);
        assert!(store.library(&PrefStore::in_memory()).duplicates(id).unwrap().is_empty());
        let twin = EntryRepo::new(store.db())
            .insert(&Entry { source: 8, url: "/b".into(), title: "FRIEREN".into(), favorite: true, ..Entry::create(Kind::Anime) })
            .unwrap();
        let found = store.library(&PrefStore::in_memory()).duplicates(id).unwrap();
        assert_eq!(found.iter().map(|a| a.id).collect::<Vec<_>>(), vec![twin]);
        assert!(store.library(&PrefStore::in_memory()).duplicates(twin).unwrap().is_empty(), "the other entry is not in the library");
    }
}
