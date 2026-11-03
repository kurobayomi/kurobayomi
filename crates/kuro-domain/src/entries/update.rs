//! Entry metadata writes of both kinds (`UpdateManga.kt`, `UpdateAnime.kt`, `UpdateAnimeFromRemote.awaitUpdateFromSource`;
//! T-10a-007, T-17a-027 to T-17a-032). Every write is a coalesce update, so a `None` field keeps the stored column.
//!
//! The cover and background rule is ONE function, [`image_action`] (decision
//! 11b): the cached file is always deleted before a new timestamp is set, also when the stored URL is empty. The cache
//! files are a port ([`CacheFiles`]): the rule decides and the port deletes, so the decision is testable without files.
//! A source's answer becomes a [`RemoteEntry`] through [`remote_from_manga`] or [`remote_from_anime`] and the rest is
//! shared.

use chrono::{DateTime, TimeZone};
use kuro_data::DataResult;
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_data::item::ItemRepo;
use kuro_db::codecs::UpdateStrategy;
use kuro_db::codec::Memo;
use kuro_source::anime::SAnime;
use kuro_source::model::SManga;
use serde_json::{Map, Value};

use crate::app::source::LOCAL_SOURCE_ID;
use crate::entries::fetch_interval::{get_window, to_entry_update};
use crate::entries::source_convert::to_db_strategy;

/// What to do with a cached image and its timestamp (`CacheAction`, area 17 section 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheAction {
    /// Keep the image and the timestamp.
    Keep,
    /// Keep the image, write the timestamp.
    Set(i64),
    /// Delete the cached image, keep the timestamp.
    DeleteCacheThenKeep,
    /// Delete the cached image, write the timestamp.
    DeleteCacheThenSet(i64),
}

impl CacheAction {
    /// The value for `*_last_modified`: `None` keeps the stored value.
    pub fn last_modified(self) -> Option<i64> {
        match self {
            Self::Keep | Self::DeleteCacheThenKeep => None,
            Self::Set(now) | Self::DeleteCacheThenSet(now) => Some(now),
        }
    }

    /// True when the cached file must be deleted before the write.
    pub fn deletes(self) -> bool {
        matches!(self, Self::DeleteCacheThenKeep | Self::DeleteCacheThenSet(_))
    }
}

/// The 5-row rule shared by covers and backgrounds of both kinds (`UpdateAnimeFromRemote.kt:142-191`, `UpdateManga`).
/// `stored_url` is the local thumbnail or background URL; `has_custom` is asked whenever row 4 is reached, including for
/// a null stored URL (E-R7). A local source writes the timestamp and has no cache to delete (EC-17); every other new
/// image deletes the cache first (decision 11b), unless the user has a custom image, which stays.
pub fn image_action(
    stored_url: Option<&str>,
    is_local: bool,
    remote_url: Option<&str>,
    manual: bool,
    has_custom: impl FnOnce() -> bool,
    now_ms: i64,
) -> CacheAction {
    let Some(remote) = remote_url.filter(|url| !url.is_empty()) else {
        return CacheAction::Keep;
    };
    if !manual && stored_url == Some(remote) {
        return CacheAction::Keep;
    }
    if is_local {
        return CacheAction::Set(now_ms);
    }
    if has_custom() {
        return CacheAction::DeleteCacheThenKeep;
    }
    CacheAction::DeleteCacheThenSet(now_ms)
}

/// The cover and background files of an entry (`MangaCoverCache`, `AnimeCoverCache`, `AnimeBackgroundCache`). Area 34
/// owns the files. A kind without a background answers `false` and does nothing.
pub trait CacheFiles: Send + Sync {
    fn has_custom_cover(&self, e: &Entry) -> bool;
    fn has_custom_background(&self, e: &Entry) -> bool;
    fn delete_cover(&self, e: &Entry);
    fn delete_background(&self, e: &Entry);
}

/// What a source reports for an entry of either kind, in domain types (`SManga` or `SAnime` after the adapters below).
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEntry {
    pub title: Option<String>,
    pub author: Option<String>,
    pub artist: Option<String>,
    pub description: Option<String>,
    /// `None` when the source sent no genre text. `Some(None)` when it sent text with no genre in it (a blank string),
    /// which clears the stored genres on the metadata path (EC-MR-02) and keeps them on a refresh (EC-MR-03).
    pub genre: Option<Option<Vec<String>>>,
    pub thumbnail_url: Option<String>,
    pub background_url: Option<String>,
    pub status: i64,
    pub update_strategy: UpdateStrategy,
    pub initialized: bool,
    /// Anime only.
    pub memo: Option<Map<String, Value>>,
}

/// The manga a source reports.
pub fn remote_from_manga(m: &SManga) -> RemoteEntry {
    RemoteEntry {
        title: Some(m.title.clone()),
        author: m.author.clone(),
        artist: m.artist.clone(),
        description: m.description.clone(),
        genre: m.genre.as_deref().map(|text| kuro_source::model::manga::parse_genres(Some(text))),
        thumbnail_url: m.thumbnail_url.clone(),
        background_url: None,
        status: i64::from(m.status),
        update_strategy: to_db_strategy(m.update_strategy),
        initialized: m.initialized,
        memo: None,
    }
}

/// The anime a source reports.
pub fn remote_from_anime(a: &SAnime) -> RemoteEntry {
    RemoteEntry {
        title: Some(a.title.clone()),
        author: a.author.clone(),
        artist: a.artist.clone(),
        description: a.description.clone(),
        genre: a.genre.as_ref().map(|_| a.genres()),
        thumbnail_url: a.thumbnail_url.clone(),
        background_url: a.background_url.clone(),
        status: i64::from(a.status.to_i32()),
        update_strategy: to_db_strategy(a.update_strategy),
        initialized: a.initialized,
        memo: Some(a.memo.clone()),
    }
}

/// The title rule (E-T1 to E-T3, EC-18): the remote title, only for a non-favourite and only when it is not empty.
pub fn title_update(local: &Entry, remote_title: Option<&str>) -> Option<String> {
    remote_title.filter(|title| !title.is_empty()).filter(|_| !local.favorite).map(str::to_owned)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}

/// The metadata write of a refresh (`awaitUpdateFromSource`) for decided cache actions. A `None` field keeps the stored
/// value. An empty thumbnail or background URL is never written (E-U8). `initialized` is always set (E-U10). Genres are
/// written only when the remote genre text holds some (EC-19).
pub fn metadata_update(local: &Entry, remote: &RemoteEntry, title: Option<String>, cover: CacheAction, background: CacheAction) -> EntryUpdate {
    EntryUpdate {
        title,
        cover_last_modified: cover.last_modified(),
        background_last_modified: background.last_modified(),
        author: remote.author.clone(),
        artist: remote.artist.clone(),
        description: remote.description.clone(),
        genre: remote.genre.clone().flatten(),
        thumbnail_url: non_empty(remote.thumbnail_url.as_deref()),
        background_url: non_empty(remote.background_url.as_deref()),
        status: Some(remote.status),
        update_strategy: Some(remote.update_strategy),
        initialized: Some(true),
        memo: remote.memo.clone().map(Memo),
        ..EntryUpdate::new(local.id)
    }
}

/// `awaitUpdateFromSource` up to the write: decides the cache actions, deletes the files they name, and returns the
/// update to write.
pub fn prepare_update(local: &Entry, remote: &RemoteEntry, manual: bool, files: &dyn CacheFiles, now_ms: i64) -> EntryUpdate {
    let is_local = local.source == LOCAL_SOURCE_ID;
    let title = title_update(local, remote.title.as_deref());
    let cover = image_action(local.thumbnail_url.as_deref(), is_local, remote.thumbnail_url.as_deref(), manual, || files.has_custom_cover(local), now_ms);
    let background = image_action(
        local.background_url.as_deref(),
        is_local,
        remote.background_url.as_deref(),
        manual,
        || files.has_custom_background(local),
        now_ms,
    );
    if cover.deletes() {
        files.delete_cover(local);
    }
    if background.deletes() {
        files.delete_background(local);
    }
    metadata_update(local, remote, title, cover, background)
}

/// The entry write of a library refresh and of the title page (`UpdateManga`, `UpdateAnime`). Reads and writes go through
/// the repositories; the interval needs the items, so the item repository is held too.
#[derive(Clone)]
pub struct UpdateEntry {
    repo: EntryRepo,
    items: ItemRepo,
}

impl UpdateEntry {
    pub fn new(repo: EntryRepo, items: ItemRepo) -> Self {
        Self { repo, items }
    }

    /// `await(update)`: one coalesce write.
    pub fn update(&self, update: &EntryUpdate) -> DataResult<()> {
        self.repo.update(update)
    }

    /// `awaitAll(updates)`: every write in one transaction.
    pub fn update_all(&self, updates: &[EntryUpdate]) -> DataResult<()> {
        self.repo.update_all(updates)
    }

    /// `awaitUpdateFromSource(local, remote, manualFetch, files)`: the cover and the background follow [`image_action`],
    /// each with its own cache; the cache files are deleted before the write, as the rule says.
    pub fn update_from_source<Tz: TimeZone>(
        &self,
        local: &Entry,
        remote: &RemoteEntry,
        manual: bool,
        files: &dyn CacheFiles,
        now: &DateTime<Tz>,
    ) -> DataResult<()> {
        self.repo.update(&prepare_update(local, remote, manual, files, now.timestamp_millis()))
    }

    /// `awaitUpdateFetchInterval(entry, now, window)`: writes `next_update` and `fetch_interval`. `None` is the default
    /// window of `getWindow(now)`.
    pub fn update_fetch_interval<Tz: TimeZone>(&self, e: &Entry, now: &DateTime<Tz>, window: Option<(i64, i64)>) -> DataResult<()> {
        let window = window.unwrap_or_else(|| get_window(now));
        self.repo.update(&to_entry_update(&self.items, e, now, window, now.timestamp_millis()))
    }

    /// `awaitUpdateLastUpdate(id)`: `last_update = now`.
    pub fn update_last_update<Tz: TimeZone>(&self, id: i64, now: &DateTime<Tz>) -> DataResult<()> {
        self.repo.update(&EntryUpdate { last_update: Some(now.timestamp_millis()), ..EntryUpdate::new(id) })
    }

    /// `awaitUpdateCoverLastModified(id)`: `cover_last_modified = now`.
    pub fn update_cover_last_modified<Tz: TimeZone>(&self, id: i64, now: &DateTime<Tz>) -> DataResult<()> {
        self.repo.update(&EntryUpdate { cover_last_modified: Some(now.timestamp_millis()), ..EntryUpdate::new(id) })
    }

    /// `awaitUpdateBackgroundLastModified(id)` (anime).
    pub fn update_background_last_modified<Tz: TimeZone>(&self, id: i64, now: &DateTime<Tz>) -> DataResult<()> {
        self.repo.update(&EntryUpdate { background_last_modified: Some(now.timestamp_millis()), ..EntryUpdate::new(id) })
    }

    /// `awaitUpdateFavorite(id, favorite)`: `date_added` is now when favourited and 0 when not (EC-25).
    pub fn update_favorite<Tz: TimeZone>(&self, id: i64, favorite: bool, now: &DateTime<Tz>) -> DataResult<()> {
        let date_added = if favorite { now.timestamp_millis() } else { 0 };
        self.repo.update(&EntryUpdate { favorite: Some(favorite), date_added: Some(date_added), ..EntryUpdate::new(id) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Local;
    use kuro_platform::kind::Kind;
    use kuro_source::model::UpdateStrategy as SourceUpdateStrategy;

    use super::*;
    use crate::testdb::test_db;

    const NOW: i64 = 1_700_000_000_123;

    /// Records the deletions and answers the custom-file questions with fixed values.
    #[derive(Default)]
    struct Files {
        custom_cover: bool,
        custom_background: bool,
        deleted_covers: Mutex<usize>,
        deleted_backgrounds: Mutex<usize>,
    }

    impl CacheFiles for Files {
        fn has_custom_cover(&self, _e: &Entry) -> bool {
            self.custom_cover
        }
        fn has_custom_background(&self, _e: &Entry) -> bool {
            self.custom_background
        }
        fn delete_cover(&self, _e: &Entry) {
            *self.deleted_covers.lock().unwrap() += 1;
        }
        fn delete_background(&self, _e: &Entry) {
            *self.deleted_backgrounds.lock().unwrap() += 1;
        }
    }

    fn at(ms: i64) -> DateTime<Local> {
        Local.timestamp_millis_opt(ms).single().expect("valid instant")
    }

    fn entry(kind: Kind, source: i64, thumb: Option<&str>, favorite: bool) -> Entry {
        Entry { id: 7, source, favorite, thumbnail_url: thumb.map(str::to_owned), ..Entry::create(kind) }
    }

    fn remote_with(thumb: Option<&str>, title: &str) -> RemoteEntry {
        remote_from_manga(&SManga { thumbnail_url: thumb.map(str::to_owned), title: title.into(), ..SManga::new("/m", "M") })
    }

    /// The five rows, for a manga entry, an anime entry and a local entry (the local source skips the delete).
    #[test]
    fn image_action_is_one_rule() {
        for kind in [Kind::Manga, Kind::Anime] {
            let local = |thumb| entry(kind, 3, thumb, false);
            let act = |local: &Entry, remote: Option<&str>, manual, custom| image_action(local.thumbnail_url.as_deref(), local.source == LOCAL_SOURCE_ID, remote, manual, || custom, NOW);
            // Row 1: no remote thumbnail keeps everything.
            assert_eq!(act(&local(Some("t")), None, false, true), CacheAction::Keep, "{kind}");
            assert_eq!(act(&local(Some("t")), Some(""), true, true), CacheAction::Keep);
            // Row 2: the same URL keeps everything unless the fetch is manual (E-R3).
            assert_eq!(act(&local(Some("t")), Some("t"), false, false), CacheAction::Keep);
            assert_eq!(act(&local(Some("t")), Some("t"), true, false), CacheAction::DeleteCacheThenSet(NOW));
            // Row 3: a local entry writes the timestamp and deletes nothing (EC-17), custom or not.
            let local_entry = entry(kind, LOCAL_SOURCE_ID, None, false);
            assert_eq!(act(&local_entry, Some("n"), false, true), CacheAction::Set(NOW));
            // Row 4: a custom image stays, the cache goes, no timestamp.
            assert_eq!(act(&local(Some("old")), Some("new"), false, true), CacheAction::DeleteCacheThenKeep);
            assert_eq!(act(&local(None), Some("new"), false, true), CacheAction::DeleteCacheThenKeep);
            // Row 5: a new image deletes the cache and bumps the timestamp, also from an empty stored URL (decision 11b).
            assert_eq!(act(&local(Some("old")), Some("new"), false, false), CacheAction::DeleteCacheThenSet(NOW));
            assert_eq!(act(&local(None), Some("new"), false, false), CacheAction::DeleteCacheThenSet(NOW), "E-R7 as decided in 11b");
        }
    }

    #[test]
    fn timestamps_are_milliseconds_since_the_epoch() {
        assert_eq!(CacheAction::Set(NOW).last_modified(), Some(NOW));
        assert_eq!(CacheAction::Keep.last_modified(), None);
        assert_eq!(CacheAction::DeleteCacheThenKeep.last_modified(), None);
        assert!(CacheAction::DeleteCacheThenSet(1).deletes() && !CacheAction::Set(1).deletes());
    }

    #[test]
    fn prepare_update_deletes_the_files_the_actions_name_before_returning() {
        let files = Files::default();
        let local = Entry { background_url: Some("bg-a".into()), ..entry(Kind::Anime, 7, Some("a"), false) };
        let remote = RemoteEntry { background_url: Some("bg-b".into()), ..remote_with(Some("b"), "T") };
        let update = prepare_update(&local, &remote, false, &files, NOW);
        assert_eq!((*files.deleted_covers.lock().unwrap(), *files.deleted_backgrounds.lock().unwrap()), (1, 1));
        assert_eq!((update.cover_last_modified, update.background_last_modified), (Some(NOW), Some(NOW)));
        let custom = Files { custom_cover: true, ..Files::default() };
        let kept = prepare_update(&local, &remote, false, &custom, NOW);
        assert_eq!(kept.cover_last_modified, None, "a custom cover keeps its timestamp");
        assert_eq!(*custom.deleted_covers.lock().unwrap(), 1, "and the remote cache still goes");
    }

    #[test]
    fn a_manual_fetch_rewrites_the_timestamp_even_when_the_url_is_unchanged() {
        let files = Files::default();
        let local = entry(Kind::Anime, 7, Some("thumb-a"), false);
        let update = prepare_update(&local, &remote_with(Some("thumb-a"), "T"), true, &files, NOW);
        assert_eq!(update.cover_last_modified, Some(NOW), "E-R3");
        assert_eq!(*files.deleted_covers.lock().unwrap(), 1);
    }

    #[test]
    fn t_titles_follow_the_favourite_flag() {
        let stored = entry(Kind::Manga, 7, None, true);
        let plain = entry(Kind::Manga, 7, None, false);
        assert_eq!(title_update(&stored, Some("X")), None, "E-T1");
        assert_eq!(title_update(&plain, Some("X")), Some("X".into()), "E-T2");
        assert_eq!(title_update(&plain, Some("")), None, "E-T3");
        assert_eq!(title_update(&plain, None), None, "E-T4: not initialised");
    }

    #[test]
    fn the_metadata_write_keeps_absent_fields_and_drops_empty_urls() {
        let local = entry(Kind::Anime, 7, Some("a"), false);
        let mut source = SAnime { url: "/x".into(), title: "T".into(), ..SAnime::default() };
        source.genre = Some(", ,A, A,B".into());
        source.thumbnail_url = Some(String::new());
        source.author = Some(String::new());
        let update = metadata_update(&local, &remote_from_anime(&source), None, CacheAction::Keep, CacheAction::Keep);
        // Kotlin splits on ", " only (SAnime.getGenres), so the leading "," stays in the first part (DECISIONS, T-17a-027).
        assert_eq!(update.genre, Some(vec![",A".to_owned(), "A,B".to_owned()]), "E-U1");
        assert_eq!(update.thumbnail_url, None, "E-U8");
        assert_eq!(update.author, Some(String::new()), "E-U5: an empty author is written");
        assert_eq!(update.initialized, Some(true), "E-U10");
        assert_eq!(update.status, Some(0), "E-U4: the status code is written");
        assert_eq!(update.id, 7);
    }

    #[test]
    fn a_null_author_is_kept_and_the_memo_and_the_strategy_are_written_for_anime() {
        let local = entry(Kind::Anime, 7, None, false);
        let mut source = SAnime { url: "/x".into(), title: "T".into(), ..SAnime::default() };
        assert_eq!(metadata_update(&local, &remote_from_anime(&source), None, CacheAction::Keep, CacheAction::Keep).author, None, "E-U6");
        source.memo = serde_json::Map::new();
        source.update_strategy = SourceUpdateStrategy::OnlyFetchOnce;
        let update = metadata_update(&local, &remote_from_anime(&source), None, CacheAction::Keep, CacheAction::Keep);
        assert_eq!(update.memo, Some(Memo(serde_json::Map::new())), "E-U7: an empty memo is written");
        assert_eq!(update.update_strategy, Some(to_db_strategy(SourceUpdateStrategy::OnlyFetchOnce)), "E-U9");
        let manga = metadata_update(&entry(Kind::Manga, 7, None, false), &remote_with(None, "M"), None, CacheAction::Keep, CacheAction::Keep);
        assert_eq!((manga.memo, manga.background_url), (None, None), "a manga has no memo and no background");
    }

    #[test]
    fn a_null_or_blank_genre_keeps_the_stored_genres() {
        let local = entry(Kind::Anime, 7, None, false);
        let mut source = SAnime { url: "/x".into(), title: "T".into(), ..SAnime::default() };
        assert_eq!(metadata_update(&local, &remote_from_anime(&source), None, CacheAction::Keep, CacheAction::Keep).genre, None, "E-U2");
        source.genre = Some("   ".into());
        assert_eq!(metadata_update(&local, &remote_from_anime(&source), None, CacheAction::Keep, CacheAction::Keep).genre, None, "E-U3");
    }

    fn repos(db: std::sync::Arc<kuro_db::Db>) -> (EntryRepo, UpdateEntry) {
        let repo = EntryRepo::new(db.clone());
        (repo.clone(), UpdateEntry::new(repo, ItemRepo::new(db)))
    }

    #[test]
    fn a_favourite_keeps_its_title_and_a_browse_row_takes_the_remote_one() {
        for kind in [Kind::Manga, Kind::Anime] {
            let (_dir, db) = test_db();
            let (repo, update) = repos(db);
            let files = Files::default();
            let id = repo.insert(&Entry { title: "Kept".into(), favorite: true, url: "/fav".into(), ..Entry::create(kind) }).unwrap();
            update.update_from_source(&repo.by_id(id).unwrap(), &remote_with(None, "Remote"), false, &files, &at(9)).unwrap();
            assert_eq!(repo.by_id(id).unwrap().title, "Kept", "{kind} EC-18");
            let browse = repo.insert(&Entry { title: "Old".into(), url: "/browse".into(), ..Entry::create(kind) }).unwrap();
            update.update_from_source(&repo.by_id(browse).unwrap(), &remote_with(None, "Remote"), false, &files, &at(9)).unwrap();
            assert_eq!(repo.by_id(browse).unwrap().title, "Remote");
        }
    }

    #[test]
    fn a_blank_genre_text_leaves_the_stored_genres_alone_and_the_strategy_is_written() {
        let (_dir, db) = test_db();
        let (repo, update) = repos(db);
        let id = repo.insert(&Entry { genre: Some(vec!["Action".into()]), ..Entry::create(Kind::Manga) }).unwrap();
        let local = repo.by_id(id).unwrap();
        let blank = remote_from_manga(&SManga { genre: Some(String::new()), ..SManga::new("/m", "M") });
        update.update_from_source(&local, &blank, false, &Files::default(), &at(9)).unwrap();
        assert_eq!(repo.by_id(id).unwrap().genre, Some(vec!["Action".to_owned()]), "EC-19: a blank genre keeps it");
        let strategy = remote_from_manga(&SManga { update_strategy: SourceUpdateStrategy::OnlyFetchOnce, ..SManga::new("/m", "M") });
        update.update_from_source(&local, &strategy, false, &Files::default(), &at(9)).unwrap();
        assert_eq!(repo.by_id(id).unwrap().update_strategy, UpdateStrategy::OnlyFetchOnce);
    }

    #[test]
    fn favorite_sets_and_clears_date_added_for_both_kinds() {
        for kind in [Kind::Manga, Kind::Anime] {
            let (_dir, db) = test_db();
            let (repo, update) = repos(db);
            let id = repo.insert(&Entry::create(kind)).unwrap();
            update.update_favorite(id, true, &at(1_700)).unwrap();
            assert_eq!(repo.by_id(id).unwrap().date_added, 1_700, "{kind} EC-25");
            update.update_favorite(id, false, &at(1_800)).unwrap();
            assert_eq!(repo.by_id(id).unwrap().date_added, 0);
            update.update_last_update(id, &at(2_000)).unwrap();
            assert_eq!(repo.by_id(id).unwrap().last_update, 2_000);
            update.update_cover_last_modified(id, &at(3_000)).unwrap();
            assert_eq!(repo.by_id(id).unwrap().cover_last_modified, 3_000);
        }
    }

    #[test]
    fn the_background_timestamp_is_written_for_anime() {
        let (_dir, db) = test_db();
        let (repo, update) = repos(db);
        let id = repo.insert(&Entry::create(Kind::Anime)).unwrap();
        update.update_background_last_modified(id, &at(4_000)).unwrap();
        assert_eq!(repo.by_id(id).unwrap().background_last_modified, Some(4_000));
    }

    #[test]
    fn a_fetch_interval_update_writes_next_update_and_interval_for_both_kinds() {
        for kind in [Kind::Manga, Kind::Anime] {
            let (_dir, db) = test_db();
            let (repo, update) = repos(db);
            let id = repo.insert(&Entry::create(kind)).unwrap();
            update.update_fetch_interval(&repo.by_id(id).unwrap(), &at(1_700_000_000_000), None).unwrap();
            let after = repo.by_id(id).unwrap();
            assert_eq!(after.fetch_interval, 7, "{kind}: no items, the seven day default");
            assert!(after.next_update > 0);
        }
    }

    #[test]
    fn anime_update_from_source_writes_the_background_rule() {
        let (_dir, db) = test_db();
        let (repo, update) = repos(db);
        let id = repo.insert(&Entry { title: "Kept".into(), favorite: true, ..Entry::create(Kind::Anime) }).unwrap();
        let files = Files::default();
        let remote = remote_from_anime(&SAnime { title: "Remote".into(), background_url: Some("bg".into()), ..SAnime::default() });
        update.update_from_source(&repo.by_id(id).unwrap(), &remote, false, &files, &at(9)).unwrap();
        assert_eq!(*files.deleted_covers.lock().unwrap(), 0, "no remote cover, nothing deleted");
        let after = repo.by_id(id).unwrap();
        assert_eq!(after.title, "Kept", "a favourite keeps its title");
        assert_eq!(after.background_url.as_deref(), Some("bg"));
        assert_eq!(after.background_last_modified, Some(9), "a new background bumps its timestamp");
    }
}
