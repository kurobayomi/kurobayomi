//! `SyncSeasonsWithSource` (T-10a-010): the season twin of [`crate::items::sync`]. A season is an anime entry whose
//! `parent_id` points at the parent. The order follows the Kotlin `await`: every source season goes
//! through `network_to_local` first (which inserts the unseen ones), then the stored children are diffed, then the
//! removed seasons are detached and the changed fields are written. Nothing is deleted.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Local};
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_data::item::ItemRepo;
use kuro_db::Db;
use kuro_platform::kind::Kind;
use kuro_source::anime::SAnime;

use crate::app::source::LOCAL_SOURCE_ID;
use crate::entries::fetch_interval::to_entry_update;
use crate::entries::network_to_local::network_to_local;
use crate::error::DomainError;
use crate::items::recognition::parse_season_number;

/// The season sync of one parent anime. Clones share the database handle.
#[derive(Clone)]
pub struct SyncSeasons {
    entries: EntryRepo,
    items: ItemRepo,
}

impl SyncSeasons {
    pub fn new(db: Arc<Db>) -> Self {
        Self { entries: EntryRepo::new(db.clone()), items: ItemRepo::new(db) }
    }

    /// Returns ALL source seasons as local rows (not only the new ones), each with `parent_id` set to the parent and
    /// `season_source_order` set to its position in the de-duplicated source list. `now_ms` is the clock instant.
    pub fn run(
        &self,
        raw: &[SAnime],
        parent: &Entry,
        manual: bool,
        window: (i64, i64),
        now_ms: i64,
    ) -> Result<Vec<Entry>, DomainError> {
        if raw.is_empty() && parent.source != LOCAL_SOURCE_ID {
            return Err(DomainError::NoSeasons);
        }

        // `distinctBy { url }`, keeping the first occurrence. A season that is the parent itself is skipped (EC-13).
        let mut seen_urls = BTreeSet::new();
        let seasons: Vec<&SAnime> = raw
            .iter()
            .filter(|season| seen_urls.insert(season.url.as_str()))
            .filter(|season| {
                let is_parent = season.url == parent.url;
                if is_parent {
                    tracing::warn!(url = %season.url, "season sync: the parent is listed as its own season, skipped");
                }
                !is_parent
            })
            .collect();

        // Read the stored children before any insert, so a season inserted below is "new" (not a stored child).
        let stored: Vec<Entry> = self.entries.seasons_by_parent(parent.id)?.into_iter().map(|season| season.entry).collect();

        let mut updates: Vec<EntryUpdate> = Vec::new();
        let mut out: Vec<Entry> = Vec::with_capacity(seasons.len());
        for (order, season) in seasons.iter().enumerate() {
            let order = order as i64;
            let season_number = parse_season_number(&parent.title, &season.title, Some(season.season_number));
            let wanted = Entry {
                url: season.url.clone(),
                title: season.title.clone(),
                source: parent.source,
                ..Entry::create(Kind::Anime)
            };
            let row = network_to_local(&self.entries, &wanted)?;
            let local = Entry { parent_id: Some(parent.id), season_source_order: Some(order), ..row };
            let changed = match stored.iter().find(|db| db.url == local.url) {
                Some(db) => {
                    db.title != local.title || db.season_number() != season_number || db.season_source_order() != order
                }
                None => true,
            };
            if changed {
                updates.push(season_update(&local, season_number, season.memo.clone()));
            }
            out.push(local);
        }

        let removed: Vec<i64> = stored
            .iter()
            .filter(|db| !seasons.iter().any(|season| season.url == db.url))
            .map(|db| db.id)
            .collect();

        if updates.is_empty() && removed.is_empty() {
            if manual || parent.fetch_interval == 0 || parent.next_update < window.0 {
                let now = DateTime::from_timestamp_millis(now_ms).map(|utc| utc.with_timezone(&Local)).unwrap_or_else(Local::now);
                let update = to_entry_update(&self.items, parent, &now, window, now_ms);
                self.entries.update(&update)?;
            }
            return Ok(out);
        }

        // EC-14: a removed season keeps its row and its library state; only its parent link is cleared.
        if !removed.is_empty() {
            self.entries.remove_parent_ids(&removed)?;
        }
        if !updates.is_empty() {
            self.entries.update_all(&updates)?;
        }
        self.entries.update(&EntryUpdate { last_update: Some(now_ms), ..EntryUpdate::new(parent.id) })?;
        Ok(out)
    }
}

/// The full field set of a season the sync writes (`toAnimeUpdate` of the in-memory season).
fn season_update(local: &Entry, season_number: f64, memo: serde_json::Map<String, serde_json::Value>) -> EntryUpdate {
    EntryUpdate {
        title: Some(local.title.clone()),
        parent_id: local.parent_id,
        season_number: Some(season_number),
        season_source_order: Some(local.season_source_order()),
        memo: Some(kuro_db::codec::Memo(memo)),
        ..EntryUpdate::new(local.id)
    }
}

#[cfg(test)]
mod sync_seasons {
    use super::*;
    use crate::testdb::test_db;

    const NOW: i64 = 1_700_000_000_000;
    const SOURCE: i64 = 7;

    fn setup() -> (tempfile::TempDir, SyncSeasons, EntryRepo, Entry) {
        let (dir, db) = test_db();
        let sync = SyncSeasons::new(db.clone());
        let repo = EntryRepo::new(db);
        let mut parent = Entry { url: "/show".into(), title: "Show".into(), source: SOURCE, ..Entry::create(Kind::Anime) };
        parent.id = repo.insert(&parent).expect("insert parent");
        (dir, sync, repo, parent)
    }

    fn stored_child(repo: &EntryRepo, parent: &Entry, url: &str, title: &str, favorite: bool) -> i64 {
        let child = Entry {
            url: url.into(),
            title: title.into(),
            source: SOURCE,
            parent_id: Some(parent.id),
            favorite,
            season_number: Some(-1.0),
            ..Entry::create(Kind::Anime)
        };
        repo.insert(&child).expect("insert child")
    }

    fn season(url: &str, title: &str) -> SAnime {
        SAnime { url: url.into(), title: title.into(), ..SAnime::default() }
    }

    #[test]
    fn new_updated_removed() {
        let (_dir, sync, repo, parent) = setup();
        stored_child(&repo, &parent, "/s1", "Old title", false);
        let gone = stored_child(&repo, &parent, "/gone", "Gone", true);

        let raw = [season("/s1", "Show Season 2"), season("/s2", "Show Season 3")];
        let out = sync.run(&raw, &parent, false, (0, 0), NOW).expect("sync");

        assert_eq!(out.iter().map(|a| a.url.as_str()).collect::<Vec<_>>(), ["/s1", "/s2"], "all source seasons, in order");
        assert_eq!(out.iter().map(|a| a.season_source_order()).collect::<Vec<_>>(), [0, 1]);
        assert!(out.iter().all(|a| a.parent_id == Some(parent.id)));

        let updated = repo.by_url_and_source(Kind::Anime, "/s1", SOURCE).unwrap().expect("updated season");
        assert_eq!(updated.title, "Show Season 2", "title is written");
        assert_eq!(updated.season_number(), 2.0, "season number is parsed from the title");
        assert_eq!(updated.season_source_order(), 0);

        let new = repo.by_url_and_source(Kind::Anime, "/s2", SOURCE).unwrap().expect("inserted season");
        assert_eq!(new.parent_id, Some(parent.id));
        assert_eq!(new.season_number(), 3.0);
        assert_eq!(new.season_source_order(), 1);

        let removed = repo.by_id(gone).expect("the removed row survives");
        assert_eq!(removed.parent_id, None, "EC-14: only the parent link is cleared");
        assert!(removed.favorite, "EC-14: the library state stays");
    }

    #[test]
    fn an_empty_list_from_a_non_local_source_is_no_seasons_and_writes_nothing() {
        let (_dir, sync, repo, parent) = setup();
        stored_child(&repo, &parent, "/s1", "Kept", false);
        let err = sync.run(&[], &parent, false, (0, 0), NOW).expect_err("no seasons");
        assert!(matches!(err, DomainError::NoSeasons));
        assert_eq!(repo.seasons_by_parent(parent.id).unwrap().len(), 1, "the DB is untouched");
    }

    #[test]
    fn an_empty_list_from_the_local_source_is_fine_and_detaches_every_child() {
        let (_dir, sync, repo, mut parent) = setup();
        parent.source = LOCAL_SOURCE_ID;
        let child = stored_child(&repo, &parent, "/s1", "Kept", false);
        let out = sync.run(&[], &parent, false, (0, 0), NOW).expect("local source may be empty");
        assert!(out.is_empty());
        assert_eq!(repo.by_id(child).unwrap().parent_id, None);
    }

    #[test]
    fn a_season_that_is_the_parent_itself_is_skipped() {
        let (_dir, sync, repo, parent) = setup();
        let raw = [season("/show", "Show"), season("/s1", "Show Season 2")];
        let out = sync.run(&raw, &parent, false, (0, 0), NOW).expect("sync");
        assert_eq!(out.iter().map(|a| a.url.as_str()).collect::<Vec<_>>(), ["/s1"], "EC-13");
        assert_eq!(repo.by_url_and_source(Kind::Anime, "/show", SOURCE).unwrap().unwrap().parent_id, None, "the parent is not its own child");
    }

    #[test]
    fn duplicate_urls_keep_the_first_occurrence() {
        let (_dir, sync, _repo, parent) = setup();
        let raw = [season("/s1", "First"), season("/s1", "Second")];
        let out = sync.run(&raw, &parent, false, (0, 0), NOW).expect("sync");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "First");
    }

    #[test]
    fn an_unchanged_list_leaves_the_parent_alone_while_its_interval_is_not_due() {
        let (_dir, sync, repo, mut parent) = setup();
        parent.fetch_interval = 3;
        parent.next_update = 4_102_444_800_000;
        repo.update(&EntryUpdate { fetch_interval: Some(3), next_update: Some(parent.next_update), ..EntryUpdate::new(parent.id) })
            .expect("set interval");
        let mut child = Entry { url: "/s1".into(), title: "Show Season 2".into(), source: SOURCE, parent_id: Some(parent.id), ..Entry::create(Kind::Anime) };
        child.season_number = Some(2.0);
        child.id = repo.insert(&child).expect("insert child");
        let raw = [season("/s1", "Show Season 2")];
        sync.run(&raw, &parent, false, (0, 0), NOW).expect("sync");
        let after = repo.by_id(parent.id).unwrap();
        assert_eq!(after.last_update, 0, "no change, so last_update is not written");
        assert_eq!(after.next_update, 4_102_444_800_000, "the interval is not recomputed while it is not due");
    }
}
