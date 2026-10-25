//! Storage reads and preference reads for the library screen. `snapshot` runs on the DB worker; `request` runs on
//! the GTK thread when the user changes a setting. The pipeline itself is in `query`.

use std::collections::BTreeSet;
use std::sync::Arc;

use kuro_data::entry::EntryRepo;
use kuro_data::track::TrackRepo;
use kuro_db::handle::Db;
use kuro_domain::category::repo::CategoryRepo;
use kuro_domain::error::DomainError;
use kuro_platform::kind::Kind;
use kuro_domain::library::filter::LibraryFilters;
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::page::BadgeSettings;
use kuro_domain::track::repo::TrackPort;
use kuro_prefs::library::LibraryPrefs;
use kuro_prefs::tristate::TriState;

use crate::misc::diet_defaults as diet;

use super::query::{Details, Request, Snapshot};

/// Reads the categories, the library rows, the search fields and the tracks of one kind.
pub fn snapshot(db: &Arc<Db>, kind: Kind) -> Result<Snapshot, DomainError> {
    let categories = CategoryRepo::new(db.clone(), kind).visible()?;
    let mut snap = Snapshot { categories, ..Snapshot::default() };
    for l in EntryRepo::new(db.clone()).library(kind)? {
        snap.rows.push(LibraryRow::from_entry(&l));
        snap.details.insert(
            l.entry.id,
            Details {
                author: l.entry.author.clone(),
                artist: l.entry.artist.clone(),
                description: l.entry.description.clone(),
                genres: l.entry.genre.clone().unwrap_or_default(),
            },
        );
    }
    for t in TrackPort::all(&TrackRepo::new(db.clone()), kind)? {
        snap.tracks.entry(t.entry_id).or_default().push((t.tracker_id, t.score));
    }
    Ok(snap)
}

/// The filter preferences of one kind, plus the global downloaded-only switch and the logged-in trackers.
pub fn filters(
    prefs: &LibraryPrefs,
    kind: Kind,
    downloaded_only: bool,
    logged_in: &BTreeSet<i64>,
) -> LibraryFilters {
    // The menu's Watching (old unread key) and Completed (old completed key) are the user's own progress (user,
    // 2026-10-25): both need a started entry; Watching has items left, Completed has none. Both on is every started
    // entry. The series-status, Bookmarked and interval filters are cut: they stay off.
    let downloaded = include_only(prefs.filter_downloaded(kind).get());
    let watching = include_only(prefs.filter_unread(kind).get()) == TriState::ENABLED_IS;
    let finished = include_only(prefs.filter_completed(kind).get()) == TriState::ENABLED_IS;
    let started = if watching || finished { TriState::ENABLED_IS } else { TriState::DISABLED };
    let unread = match (watching, finished) {
        (true, false) => TriState::ENABLED_IS,
        (false, true) => TriState::ENABLED_NOT,
        _ => TriState::DISABLED,
    };
    let (completed, bookmarked, interval_custom) = (TriState::DISABLED, TriState::DISABLED, TriState::DISABLED);
    let tracking = logged_in
        .iter()
        .map(|id| (*id, include_only(prefs.filter_tracked(kind, *id as i32).get())))
        .collect();
    LibraryFilters {
        downloaded,
        unread,
        started,
        bookmarked,
        completed,
        interval_custom,
        tracking,
        downloaded_only,
        // on: the "Customized update frequency" filter only applies with it (independent of what a library update skips)
        skip_outside_release_period: true,
    }
}

/// A library filter is include-or-off: an exclude value stored by an older version (or an Aniyomi backup) reads as
/// off, so the view menu's checks and the pipeline always agree.
pub fn include_only(stored: TriState) -> TriState {
    if stored == TriState::ENABLED_IS { TriState::ENABLED_IS } else { TriState::DISABLED }
}

/// Whether a preference change can alter the library pipeline or its layout for `kind` (D-3, D-6). Other keys,
/// such as the remembered tab, do not reload the library.
pub fn affects_library(kind: Kind, key: &str) -> bool {
    const SHARED: &[&str] = &[
        "pref_display_mode_library",
        "__APP_STATE_pref_downloaded_only",
    ];
    const ANIME: &[&str] = &[
        "pref_filter_animelib_downloaded_v2",
        "pref_filter_animelib_unread_v2",
        "pref_filter_animelib_completed_v2",
        "animelib_sorting_mode",
        "library_random_anime_sort_seed",
    ];
    const MANGA: &[&str] = &[
        "pref_filter_library_downloaded_v2",
        "pref_filter_library_unread_v2",
        "pref_filter_library_completed_v2",
        "library_sorting_mode",
        "library_random_manga_sort_seed",
    ];
    if SHARED.contains(&key) {
        return true;
    }
    match kind {
        Kind::Anime => ANIME.contains(&key) || key.starts_with("pref_filter_animelib_tracked_"),
        Kind::Manga => MANGA.contains(&key) || key.starts_with("pref_filter_library_tracked_"),
    }
}

/// The badges of both libraries: fixed, since their switches are cut.
pub fn badges() -> BadgeSettings {
    BadgeSettings {
        download: diet::LIBRARY_DOWNLOAD_BADGE,
        unread: diet::LIBRARY_UNREAD_BADGE,
        local: diet::LIBRARY_LOCAL_BADGE,
        language: diet::LIBRARY_LANGUAGE_BADGE,
    }
}

/// The complete settings of one pass.
pub fn request(
    prefs: &LibraryPrefs,
    kind: Kind,
    downloaded_only: bool,
    logged_in: &BTreeSet<i64>,
    query: Option<String>,
) -> Request {
    let random_seed = kuro_prefs::category::random_sort_seed(kind);
    Request {
        kind,
        filters: filters(prefs, kind, downloaded_only, logged_in),
        badges: badges(),
        query,
        random_seed,
        logged_in: logged_in.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_data::entry::Entry;

    fn test_db() -> (tempfile::TempDir, Arc<Db>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("open manga db");
        (dir, Arc::new(Db::new(&path, writer).expect("manga db handle")))
    }

    #[test]
    fn snapshot_lists_visible_categories_and_one_row_per_membership() {
        let (_dir, db) = test_db();
        let repo = EntryRepo::new(db.clone());
        let id = repo
            .insert(&Entry {
                favorite: true,
                source: 7,
                url: "u".into(),
                title: "Title".into(),
                author: Some("Someone".into()),
                ..Entry::create(Kind::Manga)
            })
            .expect("insert");
        let a = kuro_testkit::insert_category(&db, Kind::Manga, "a");
        let b = kuro_testkit::insert_category(&db, Kind::Manga, "b");
        repo.set_categories(id, &[a, b]).expect("link");
        let snap = snapshot(&db, Kind::Manga).expect("snapshot");
        assert_eq!(snap.rows.len(), 2);
        assert!(snap.rows.iter().all(|r| r.id == id && r.title == "Title"));
        assert_eq!(snap.details.get(&id).and_then(|d| d.author.clone()).as_deref(), Some("Someone"));
        assert!(snap.categories.iter().any(|c| c.id == a) && snap.categories.iter().any(|c| c.id == b));
        assert!(snap.tracks.is_empty());
    }

    #[test]
    fn only_library_keys_of_this_kind_reload_the_library() {
        assert!(affects_library(Kind::Manga, "library_sorting_mode"));
        assert!(!affects_library(Kind::Anime, "library_sorting_mode"));
        assert!(affects_library(Kind::Anime, "pref_filter_animelib_tracked_3_v2"));
        assert!(affects_library(Kind::Manga, "pref_display_mode_library"));
        assert!(!affects_library(Kind::Manga, "__APP_STATE_last_used_category"));
    }

    #[test]
    fn a_stored_exclude_filter_reads_as_off() {
        assert_eq!(include_only(TriState::ENABLED_NOT), TriState::DISABLED);
        assert_eq!(include_only(TriState::ENABLED_IS), TriState::ENABLED_IS);
        assert_eq!(include_only(TriState::DISABLED), TriState::DISABLED);
        let prefs = LibraryPrefs::new(kuro_prefs::store::PrefStore::in_memory());
        prefs.filter_unread.set(TriState::ENABLED_NOT).expect("write");
        prefs.filter_tracked_manga(2).set(TriState::ENABLED_NOT).expect("write");
        let f = filters(&prefs, Kind::Manga, false, &BTreeSet::from([2]));
        assert_eq!(f.unread, TriState::DISABLED);
        assert_eq!(f.tracking.get(&2), Some(&TriState::DISABLED));
        assert!(!f.has_active(), "an old exclude value is no active filter");
    }

    #[test]
    fn watching_and_completed_are_the_users_progress() {
        let prefs = LibraryPrefs::new(kuro_prefs::store::PrefStore::in_memory());
        // 10 episodes each, seen: none, four, all
        let rows: Vec<LibraryRow> = [0, 4, 10]
            .into_iter()
            .enumerate()
            .map(|(i, done)| LibraryRow { done, ..crate::library::fixtures::row(i as i64, 0) })
            .collect();
        let kept = |p: &LibraryPrefs| -> Vec<i64> {
            let f = filters(p, Kind::Anime, false, &BTreeSet::new());
            let facts = kuro_domain::library::filter::EntryFacts { downloads: 0, tracker_ids: &[] };
            rows.iter().filter(|r| f.keep(r, facts, &BTreeSet::new())).map(|r| r.id).collect()
        };
        assert_eq!(kept(&prefs), vec![0, 1, 2]);
        prefs.filter_unseen.set(TriState::ENABLED_IS).expect("write");
        assert_eq!(kept(&prefs), vec![1], "Watching: started with episodes left");
        prefs.filter_completed_anime.set(TriState::ENABLED_IS).expect("write");
        assert_eq!(kept(&prefs), vec![1, 2], "both: every started entry");
        prefs.filter_unseen.set(TriState::DISABLED).expect("write");
        assert_eq!(kept(&prefs), vec![2], "Completed: nothing left, whatever the series status");
    }

    #[test]
    fn request_reads_the_default_preferences_as_disabled_filters_and_no_search() {
        let store = kuro_prefs::store::PrefStore::in_memory();
        let prefs = LibraryPrefs::new(store);
        let req = request(&prefs, Kind::Manga, false, &BTreeSet::new(), None);
        assert_eq!(req.filters.unread, kuro_prefs::tristate::TriState::DISABLED);
        assert!(req.badges.unread && !req.badges.download);
        assert!(req.query.is_none());
        assert!(req.filters.skip_outside_release_period, "the default restrictions include the release period");
    }
}
