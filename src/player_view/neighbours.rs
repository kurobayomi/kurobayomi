//! The episode before or after another one in its entry, for the page's previous and next buttons and for autoplay. The
//! list is the one the reader walks (`reader::chapters::build_reader_list`): the entry's own sort, its unread, downloaded
//! and bookmarked filters while "skip filtered" is on, and downloaded-only mode, with the playing episode always in it.

use std::collections::HashSet;
use std::sync::Arc;

use kuro_data::entry::EntryRepo;
use kuro_data::item::{Item, ItemRepo};
use kuro_db::handle::Db;

use crate::reader::chapter_panel::Row;
use crate::reader::chapters::{ListOptions, build_reader_list};

/// Which neighbour to find.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

/// The row id of the episode before or after `episode_id` in its entry's playback list. `None` at either end, for a row
/// that does not exist, and when downloaded-only mode left the episode out of the list. `on_disk` holds the ids of the
/// downloaded episodes of the entry.
pub fn adjacent(db: &Arc<Db>, episode_id: i64, step: Step, downloaded_only: bool, on_disk: &HashSet<i64>) -> Result<Option<i64>, String> {
    let Some((list, at)) = playback_list(db, episode_id, downloaded_only, on_disk)? else { return Ok(None) };
    Ok(match step {
        Step::Previous => at.checked_sub(1).map(|i| list[i].id),
        Step::Next => list.get(at + 1).map(|e| e.id),
    })
}

/// The entry's playback list and the position of `episode_id` in it. `None` for a row that does not exist and when
/// downloaded-only mode left the episode out.
pub fn playback_list(db: &Arc<Db>, episode_id: i64, downloaded_only: bool, on_disk: &HashSet<i64>) -> Result<Option<(Vec<Item>, usize)>, String> {
    let repo = ItemRepo::new(Arc::clone(db));
    let Some(current) = repo.by_id(episode_id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let entry = EntryRepo::new(Arc::clone(db)).by_id(current.entry_id).map_err(|e| e.to_string())?;
    let episodes = repo.by_entry_id(current.entry_id, false).map_err(|e| e.to_string())?;
    let options = ListOptions {
        skip_read: crate::misc::diet_defaults::READER_SKIP_READ,
        skip_filtered: crate::misc::diet_defaults::READER_SKIP_FILTERED,
        skip_dupe: crate::misc::diet_defaults::READER_SKIP_DUPE,
        downloaded_only,
        item_flags: entry.item_flags,
    };
    let is_local = kuro_local::ids::is_local_source(entry.source);
    let Ok(list) = build_reader_list(&episodes, episode_id, &options, |item| is_local || on_disk.contains(&item.id)) else {
        return Ok(None);
    };
    Ok(list.iter().position(|e| e.id == episode_id).map(|at| (list, at)))
}

/// The playback list of `episode_id`'s entry as the episode list shows it, with the episode ids in the same order.
/// `None` before the app data exists or when the list cannot be read.
pub fn episode_rows(episode_id: i64) -> Option<(Vec<Row>, Vec<i64>)> {
    if !crate::data::is_installed() {
        return None;
    }
    let app = crate::data::app();
    let on_disk = downloaded_ids(&app.db, episode_id);
    let (list, at) = playback_list(&app.db, episode_id, app.base.downloaded_only.get(), &on_disk).ok()??;
    let rows = list
        .iter()
        .enumerate()
        .map(|(index, e)| Row {
            index,
            name: e.name.clone(),
            number: e.number,
            scanlator: e.scanlator.clone(),
            read: e.done,
            bookmarked: e.bookmark,
            downloaded: on_disk.contains(&e.id),
            current: index == at,
        })
        .collect();
    Some((rows, list.iter().map(|e| e.id).collect()))
}

/// [`adjacent`] over the app's database, its downloads and its downloaded-only mode.
pub fn neighbour(episode_id: i64, step: Step) -> Result<Option<i64>, String> {
    let app = crate::data::app();
    let on_disk = downloaded_ids(&app.db, episode_id);
    adjacent(&app.db, episode_id, step, app.base.downloaded_only.get(), &on_disk)
}

/// The ids of the downloaded episodes of the entry that episode `episode_id` belongs to.
pub fn downloaded_ids(db: &Arc<Db>, episode_id: i64) -> HashSet<i64> {
    let items = ItemRepo::new(Arc::clone(db));
    let Some(entry_id) = items.by_id(episode_id).ok().flatten().map(|e| e.entry_id) else { return Default::default() };
    let (Ok(entry), Ok(episodes)) = (EntryRepo::new(Arc::clone(db)).by_id(entry_id), items.by_entry_id(entry_id, false)) else { return Default::default() };
    let host = crate::updates::download_host::current();
    crate::entry_core::downloaded::download_states(&*host, kuro_platform::kind::Kind::Anime, &entry, &episodes)
        .into_iter()
        .filter(|(_, state)| *state == kuro_downloads::DownloadState::Downloaded)
        .map(|(id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use kuro_data::entry::{Entry, EntryRepo};
    use kuro_data::item::Item;
    use kuro_platform::kind::Kind;
    use kuro_testkit::{TempDir, TestDb};

    use super::*;

    #[test]
    fn the_neighbours_follow_the_episode_numbers_and_stop_at_the_ends() {
        let dir = TempDir::new().unwrap();
        let db = TestDb::handle(&dir);
        let anime_id = EntryRepo::new(Arc::clone(&db)).insert(&Entry { item_flags: kuro_db::flags::item::SORTING_NUMBER | kuro_db::flags::item::SORT_ASC, ..Entry::create(Kind::Anime) }).expect("anime row");
        let episodes: Vec<Item> = [3.0, 1.0, 2.0]
            .iter()
            .map(|&n| Item { entry_id: anime_id, url: format!("/{n}"), name: format!("E{n}"), number: n, ..Item::create(Kind::Anime) })
            .collect();
        let rows = ItemRepo::new(Arc::clone(&db)).add_all(&episodes).expect("rows");
        let id_of = |n: f64| rows.iter().find(|e| e.number == n).expect("row").id;

        assert_eq!(adjacent(&db, id_of(1.0), Step::Next, false, &HashSet::new()).unwrap(), Some(id_of(2.0)));
        assert_eq!(adjacent(&db, id_of(2.0), Step::Previous, false, &HashSet::new()).unwrap(), Some(id_of(1.0)));
        assert_eq!(adjacent(&db, id_of(3.0), Step::Next, false, &HashSet::new()).unwrap(), None, "the last episode has no next");
        assert_eq!(adjacent(&db, id_of(1.0), Step::Previous, false, &HashSet::new()).unwrap(), None, "the first episode has no previous");
        assert_eq!(adjacent(&db, 9_999, Step::Next, false, &HashSet::new()).unwrap(), None, "a missing episode has no neighbour");
    }

    #[test]
    fn the_neighbours_skip_the_episodes_the_entry_filters_hide_and_downloaded_only_keeps_the_ones_on_disk() {
        use kuro_db::flags::item as flag;
        let dir = TempDir::new().unwrap();
        let db = TestDb::handle(&dir);
        // number order, only the unseen episodes show
        let entry = Entry { item_flags: flag::SORTING_NUMBER | flag::SORT_ASC | flag::SHOW_UNDONE, ..Entry::create(Kind::Anime) };
        let anime_id = EntryRepo::new(Arc::clone(&db)).insert(&entry).expect("anime row");
        let episodes: Vec<Item> = [(1.0, false), (2.0, true), (3.0, false), (4.0, false)]
            .iter()
            .map(|&(n, done)| Item { entry_id: anime_id, url: format!("/{n}"), name: format!("E{n}"), number: n, done, ..Item::create(Kind::Anime) })
            .collect();
        let rows = ItemRepo::new(Arc::clone(&db)).add_all(&episodes).expect("rows");
        let id_of = |n: f64| rows.iter().find(|e| e.number == n).expect("row").id;

        assert_eq!(adjacent(&db, id_of(1.0), Step::Next, false, &HashSet::new()).unwrap(), Some(id_of(3.0)), "the seen episode 2 is skipped");
        assert_eq!(adjacent(&db, id_of(3.0), Step::Previous, false, &HashSet::new()).unwrap(), Some(id_of(1.0)));
        let disk: HashSet<i64> = [id_of(1.0), id_of(4.0)].into();
        assert_eq!(adjacent(&db, id_of(1.0), Step::Next, true, &disk).unwrap(), Some(id_of(4.0)), "only downloaded ones play offline");
    }
}
