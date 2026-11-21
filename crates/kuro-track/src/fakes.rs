//! Test doubles shared by the unit tests of this crate.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kuro_platform::secrets::{SecretError, SecretStore};
use kuro_prefs::store::PrefStore;
use tokio::sync::watch;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::base::BaseTracker;
use crate::connectivity::Connectivity;
use crate::ctx::{BindingLocks, TrackCtx};
use crate::error::TrackError;
use crate::events::EventBus;
use crate::manager::{REGISTRY_ORDER, TrackerManager};
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::prefs::TrackPrefs;
use crate::queue::RetryQueue;
use crate::queue::retry::RetryScheduler;
use crate::repo::{
    AnimeEpisodeRepo, AnimeHistoryRepo, AnimeTrackRepo, MangaChapterRepo, MangaHistoryRepo,
    MangaTrackRepo, RowsWatch, UnitRef,
};
use crate::tracker::{
    AnimeTracker, DeletableAnimeTracker, DeletableMangaTracker, EnhancedAnimeTracker,
    EnhancedMangaTracker, LogoRef, MangaTracker, Tracker,
};
use crate::trackers::enhanced::EntryRef;
use kuro_testkit::{TempDir, TestDb};

/// Calls in the order they happened, shared between a tracker and its repository.
pub type Log = Arc<Mutex<Vec<String>>>;

pub fn new_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

/// In-memory secret store. `fail_set_for` makes `set` fail for one account.
#[derive(Default)]
pub struct MemSecrets {
    map: Mutex<BTreeMap<(String, String), String>>,
    fail_set_for: Mutex<Option<String>>,
    unavailable: Mutex<bool>,
}

impl MemSecrets {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn fail_set_for(&self, account: &str) {
        *self.fail_set_for.lock().unwrap() = Some(account.to_owned());
    }

    /// Makes the injected `set` failure report `Unavailable` (no secret service) instead of `Failed`.
    pub fn fail_with_unavailable(&self) {
        *self.unavailable.lock().unwrap() = true;
    }

    pub fn raw(&self, account: &str) -> Option<String> {
        self.map
            .lock()
            .unwrap()
            .get(&("kurobayomi.track".to_owned(), account.to_owned()))
            .cloned()
    }

    /// Every stored value, for leak checks.
    pub fn all_values(&self) -> Vec<String> {
        self.map.lock().unwrap().values().cloned().collect()
    }
}

impl SecretStore for MemSecrets {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(&(service.to_owned(), account.to_owned()))
            .cloned())
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError> {
        if self.fail_set_for.lock().unwrap().as_deref() == Some(account) {
            return Err(if *self.unavailable.lock().unwrap() {
                SecretError::Unavailable("injected".into())
            } else {
                SecretError::Failed("injected".into())
            });
        }
        self.map
            .lock()
            .unwrap()
            .insert((service.to_owned(), account.to_owned()), secret.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        self.map
            .lock()
            .unwrap()
            .remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

pub fn prefs() -> Arc<TrackPrefs> {
    Arc::new(TrackPrefs::new(PrefStore::in_memory()))
}

pub const SCORES: [&str; 3] = ["a", "b", "c"];
pub const STATUS_PLANNED: i64 = 0;
pub const STATUS_WATCHING: i64 = 1;
pub const STATUS_COMPLETED: i64 = 2;
pub const STATUS_REWATCHING: i64 = 3;

/// Anime tracker double. `enhanced` and `deletable` decide what the `as_*` methods expose.
pub struct FakeAnime {
    pub base: BaseTracker,
    pub log: Log,
    pub enhanced: bool,
    pub deletable: bool,
    /// The remove dialog deletes the local row before the remote one.
    pub local_first: bool,
    pub accepted: &'static [&'static str],
    pub fail: Mutex<Option<String>>,
    /// When set, `refresh` (the first call of a progress update) waits on it first. Two trackers sharing a barrier of 2 finish only when they run together.
    pub gate: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    /// Overrides of the capability defaults, for the row gating tests.
    pub private_support: bool,
    pub reading_dates: bool,
    scores: Vec<String>,
}

impl FakeAnime {
    pub fn new(id: TrackerId, name: &'static str, secrets: Arc<MemSecrets>, log: Log) -> Self {
        Self {
            base: BaseTracker::new(id, name, secrets, EventBus::default()),
            log,
            enhanced: false,
            deletable: false,
            local_first: false,
            accepted: &[],
            fail: Mutex::new(None),
            gate: Mutex::new(None),
            private_support: false,
            reading_dates: false,
            scores: SCORES.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn check_fail(&self) -> Result<(), TrackError> {
        match self.fail.lock().unwrap().clone() {
            Some(m) => Err(TrackError::Message(m)),
            None => Ok(()),
        }
    }

    async fn wait_gate(&self) {
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.wait().await;
        }
    }
}

#[async_trait]
impl Tracker for FakeAnime {
    fn id(&self) -> TrackerId {
        self.base.id()
    }
    fn name(&self) -> &str {
        self.base.name()
    }
    fn supports_private_tracking(&self) -> bool {
        self.private_support
    }
    fn removes_local_first(&self) -> bool {
        self.local_first
    }
    fn supports_reading_dates(&self) -> bool {
        self.reading_dates
    }
    fn logo(&self) -> LogoRef {
        LogoRef::IconName("fake")
    }
    fn logo_color(&self) -> u32 {
        0xFF00_0000
    }
    fn completion_status(&self) -> i64 {
        STATUS_COMPLETED
    }
    fn score_list(&self) -> &[String] {
        &self.scores
    }
    async fn login(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.check_fail()?;
        self.base.save_credentials(username, password)
    }
    fn logout(&self) -> Result<(), TrackError> {
        self.base.logout()
    }
    fn is_logged_in(&self) -> bool {
        self.base.is_logged_in()
    }
    fn login_state(&self) -> watch::Receiver<bool> {
        self.base.login_state()
    }
    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.base.save_credentials(username, password)
    }
    fn username(&self) -> String {
        self.base.username()
    }
    fn password(&self) -> String {
        self.base.password()
    }
    fn as_anime(&self) -> Option<&dyn AnimeTracker> {
        Some(self)
    }
    fn as_enhanced_anime(&self) -> Option<&dyn EnhancedAnimeTracker> {
        self.enhanced.then_some(self as &dyn EnhancedAnimeTracker)
    }
    fn as_deletable_anime(&self) -> Option<&dyn DeletableAnimeTracker> {
        self.deletable.then_some(self as &dyn DeletableAnimeTracker)
    }
}

#[async_trait]
impl AnimeTracker for FakeAnime {
    fn status_list(&self) -> Vec<i64> {
        vec![
            STATUS_PLANNED,
            STATUS_WATCHING,
            STATUS_COMPLETED,
            STATUS_REWATCHING,
        ]
    }
    fn watching_status(&self) -> i64 {
        STATUS_WATCHING
    }
    fn rewatching_status(&self) -> i64 {
        STATUS_REWATCHING
    }
    fn status_label(&self, status: i64) -> Option<&'static str> {
        (status == STATUS_WATCHING).then_some("status_watching")
    }
    fn display_score(&self, t: &AnimeTrack) -> String {
        format!("{}", t.score)
    }
    async fn update(
        &self,
        t: &AnimeTrack,
        did_watch_episode: bool,
    ) -> Result<AnimeTrack, TrackError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("update:{did_watch_episode}"));
        self.check_fail()?;
        Ok(t.clone())
    }
    async fn bind(&self, t: &mut AnimeTrack, _has_seen: bool) -> Result<(), TrackError> {
        self.log.lock().unwrap().push("bind".into());
        self.check_fail()?;
        t.remote_id = t.remote_id.max(1);
        Ok(())
    }
    async fn search(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, TrackError> {
        self.check_fail()?;
        Ok(vec![AnimeTrackSearch::new(
            self.id(),
            1,
            query,
            "https://example.test/1",
        )])
    }
    async fn refresh(&self, t: &AnimeTrack) -> Result<AnimeTrack, TrackError> {
        self.wait_gate().await;
        self.check_fail()?;
        Ok(t.clone())
    }
}

#[async_trait]
impl EnhancedAnimeTracker for FakeAnime {
    fn accepted_sources(&self) -> &[&'static str] {
        self.accepted
    }
    /// Matches by `entry.url` when it is set (as the real trackers do), else a fixed URL.
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<AnimeTrackSearch>, TrackError> {
        let url = if entry.url.is_empty() {
            "https://example.test/9"
        } else {
            entry.url.as_str()
        };
        Ok((!entry.title.is_empty()).then(|| {
            let mut r = AnimeTrackSearch::new(self.id(), 9, &entry.title, url);
            r.anime_id = entry.id;
            r
        }))
    }
    fn is_track_from(&self, t: &AnimeTrack, _entry: &EntryRef, _source_id: Option<&str>) -> bool {
        t.tracker_id == self.id()
    }
    fn migrate_track(
        &self,
        t: &AnimeTrack,
        _new_source_id: &str,
        _new_entry: &EntryRef,
    ) -> Option<AnimeTrack> {
        Some(t.clone())
    }
}

#[async_trait]
impl DeletableAnimeTracker for FakeAnime {
    async fn delete(&self, _t: &AnimeTrack) -> Result<(), TrackError> {
        self.log.lock().unwrap().push("delete".into());
        self.check_fail()
    }
}

/// Manga tracker double. Same shape as [`FakeAnime`].
pub struct FakeManga {
    pub base: BaseTracker,
    pub log: Log,
    pub enhanced: bool,
    pub deletable: bool,
    pub accepted: &'static [&'static str],
    pub fail: Mutex<Option<String>>,
    /// See [`FakeAnime::gate`].
    pub gate: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    scores: Vec<String>,
}

impl FakeManga {
    pub fn new(id: TrackerId, name: &'static str, secrets: Arc<MemSecrets>, log: Log) -> Self {
        Self {
            base: BaseTracker::new(id, name, secrets, EventBus::default()),
            log,
            enhanced: false,
            deletable: false,
            accepted: &[],
            fail: Mutex::new(None),
            gate: Mutex::new(None),
            scores: SCORES.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn check_fail(&self) -> Result<(), TrackError> {
        match self.fail.lock().unwrap().clone() {
            Some(m) => Err(TrackError::Message(m)),
            None => Ok(()),
        }
    }

    async fn wait_gate(&self) {
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.wait().await;
        }
    }
}

#[async_trait]
impl Tracker for FakeManga {
    fn id(&self) -> TrackerId {
        self.base.id()
    }
    fn name(&self) -> &str {
        self.base.name()
    }
    fn logo(&self) -> LogoRef {
        LogoRef::IconName("fake")
    }
    fn logo_color(&self) -> u32 {
        0xFF00_0000
    }
    fn completion_status(&self) -> i64 {
        STATUS_COMPLETED
    }
    fn score_list(&self) -> &[String] {
        &self.scores
    }
    async fn login(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.check_fail()?;
        self.base.save_credentials(username, password)
    }
    fn logout(&self) -> Result<(), TrackError> {
        self.base.logout()
    }
    fn is_logged_in(&self) -> bool {
        self.base.is_logged_in()
    }
    fn login_state(&self) -> watch::Receiver<bool> {
        self.base.login_state()
    }
    fn save_credentials(&self, username: &str, password: &str) -> Result<(), TrackError> {
        self.base.save_credentials(username, password)
    }
    fn username(&self) -> String {
        self.base.username()
    }
    fn password(&self) -> String {
        self.base.password()
    }
    fn as_manga(&self) -> Option<&dyn MangaTracker> {
        Some(self)
    }
    fn as_enhanced_manga(&self) -> Option<&dyn EnhancedMangaTracker> {
        self.enhanced.then_some(self as &dyn EnhancedMangaTracker)
    }
    fn as_deletable_manga(&self) -> Option<&dyn DeletableMangaTracker> {
        self.deletable.then_some(self as &dyn DeletableMangaTracker)
    }
}

#[async_trait]
impl MangaTracker for FakeManga {
    fn status_list(&self) -> Vec<i64> {
        vec![
            STATUS_PLANNED,
            STATUS_WATCHING,
            STATUS_COMPLETED,
            STATUS_REWATCHING,
        ]
    }
    fn reading_status(&self) -> i64 {
        STATUS_WATCHING
    }
    fn rereading_status(&self) -> i64 {
        STATUS_REWATCHING
    }
    fn status_label(&self, status: i64) -> Option<&'static str> {
        (status == STATUS_WATCHING).then_some("status_reading")
    }
    fn display_score(&self, t: &MangaTrack) -> String {
        format!("{}", t.score)
    }
    async fn update(
        &self,
        t: &MangaTrack,
        did_read_chapter: bool,
    ) -> Result<MangaTrack, TrackError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("update:{did_read_chapter}"));
        self.check_fail()?;
        Ok(t.clone())
    }
    async fn bind(&self, t: &mut MangaTrack, _has_read: bool) -> Result<(), TrackError> {
        self.log.lock().unwrap().push("bind".into());
        self.check_fail()?;
        t.remote_id = t.remote_id.max(1);
        Ok(())
    }
    async fn search(&self, query: &str) -> Result<Vec<MangaTrackSearch>, TrackError> {
        self.check_fail()?;
        Ok(vec![MangaTrackSearch::new(
            self.id(),
            1,
            query,
            "https://example.test/1",
        )])
    }
    async fn refresh(&self, t: &MangaTrack) -> Result<MangaTrack, TrackError> {
        self.wait_gate().await;
        self.check_fail()?;
        Ok(t.clone())
    }
}

#[async_trait]
impl EnhancedMangaTracker for FakeManga {
    fn accepted_sources(&self) -> &[&'static str] {
        self.accepted
    }
    /// Matches by `entry.url` when it is set (as the real trackers do), else a fixed URL.
    async fn match_entry(&self, entry: &EntryRef) -> Result<Option<MangaTrackSearch>, TrackError> {
        let url = if entry.url.is_empty() {
            "https://example.test/9"
        } else {
            entry.url.as_str()
        };
        Ok((!entry.title.is_empty()).then(|| {
            let mut r = MangaTrackSearch::new(self.id(), 9, &entry.title, url);
            r.manga_id = entry.id;
            r
        }))
    }
    fn is_track_from(&self, t: &MangaTrack, _entry: &EntryRef, _source_id: Option<&str>) -> bool {
        t.tracker_id == self.id()
    }
    fn migrate_track(
        &self,
        t: &MangaTrack,
        _new_source_id: &str,
        _new_entry: &EntryRef,
    ) -> Option<MangaTrack> {
        Some(t.clone())
    }
}

#[async_trait]
impl DeletableMangaTracker for FakeManga {
    async fn delete(&self, _t: &MangaTrack) -> Result<(), TrackError> {
        self.log.lock().unwrap().push("delete".into());
        self.check_fail()
    }
}

/// Anime repository double. Upserts keep the row id of an existing `(anime_id, tracker)` pair.
#[derive(Default)]
pub struct MemAnimeRepo {
    pub rows: Mutex<Vec<AnimeTrack>>,
    pub log: Option<Log>,
    pub watchers: Mutex<Vec<(i64, watch::Sender<Result<Vec<AnimeTrack>, String>>)>>,
}

impl MemAnimeRepo {
    /// Sends the rows of every watched entry to its watchers, dropping the closed ones.
    fn publish(&self) {
        let rows = self.rows.lock().unwrap().clone();
        let mut watchers = self.watchers.lock().unwrap();
        watchers.retain(|(_, tx)| !tx.is_closed());
        for (entry, tx) in watchers.iter() {
            let list = rows
                .iter()
                .filter(|r| r.anime_id == *entry)
                .cloned()
                .collect();
            let _ = tx.send(Ok(list));
        }
    }
}

#[async_trait]
impl AnimeTrackRepo for MemAnimeRepo {
    async fn upsert_all(&self, ts: &[AnimeTrack]) -> Result<(), TrackError> {
        for t in ts {
            self.upsert(t).await?;
        }
        Ok(())
    }

    async fn get_by_id(&self, id: i64) -> Result<Option<AnimeTrack>, TrackError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == Some(id))
            .cloned())
    }

    async fn list_all(&self) -> Result<Vec<AnimeTrack>, TrackError> {
        Ok(self.rows.lock().unwrap().clone())
    }

    fn watch_for_entry(&self, anime_id: i64) -> RowsWatch<AnimeTrack> {
        let initial: Vec<AnimeTrack> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.anime_id == anime_id)
            .cloned()
            .collect();
        let (tx, rx) = watch::channel(Ok(initial));
        self.watchers.lock().unwrap().push((anime_id, tx));
        rx
    }

    async fn list_for_entry(&self, anime_id: i64) -> Result<Vec<AnimeTrack>, TrackError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.anime_id == anime_id)
            .cloned()
            .collect())
    }

    async fn upsert(&self, t: &AnimeTrack) -> Result<i64, TrackError> {
        if let Some(log) = &self.log {
            log.lock().unwrap().push("upsert".into());
        }
        let mut rows = self.rows.lock().unwrap();
        let pos = rows
            .iter()
            .position(|r| r.anime_id == t.anime_id && r.tracker_id == t.tracker_id);
        let id = match pos {
            Some(i) => rows[i].id.unwrap_or(i as i64 + 1),
            None => rows.len() as i64 + 1,
        };
        let stored = AnimeTrack {
            id: Some(id),
            ..t.clone()
        };
        match pos {
            Some(i) => rows[i] = stored,
            None => rows.push(stored),
        }
        drop(rows);
        self.publish();
        Ok(id)
    }

    async fn delete(&self, anime_id: i64, tracker_id: TrackerId) -> Result<(), TrackError> {
        self.rows
            .lock()
            .unwrap()
            .retain(|r| !(r.anime_id == anime_id && r.tracker_id == tracker_id));
        self.publish();
        Ok(())
    }
}

/// Manga repository double. Same rules as [`MemAnimeRepo`].
#[derive(Default)]
pub struct MemMangaRepo {
    pub rows: Mutex<Vec<MangaTrack>>,
    pub log: Option<Log>,
    pub watchers: Mutex<Vec<(i64, watch::Sender<Result<Vec<MangaTrack>, String>>)>>,
}

impl MemMangaRepo {
    /// Sends the rows of every watched entry to its watchers, dropping the closed ones.
    fn publish(&self) {
        let rows = self.rows.lock().unwrap().clone();
        let mut watchers = self.watchers.lock().unwrap();
        watchers.retain(|(_, tx)| !tx.is_closed());
        for (entry, tx) in watchers.iter() {
            let list = rows
                .iter()
                .filter(|r| r.manga_id == *entry)
                .cloned()
                .collect();
            let _ = tx.send(Ok(list));
        }
    }
}

#[async_trait]
impl MangaTrackRepo for MemMangaRepo {
    async fn upsert_all(&self, ts: &[MangaTrack]) -> Result<(), TrackError> {
        for t in ts {
            self.upsert(t).await?;
        }
        Ok(())
    }

    async fn get_by_id(&self, id: i64) -> Result<Option<MangaTrack>, TrackError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == Some(id))
            .cloned())
    }

    async fn list_all(&self) -> Result<Vec<MangaTrack>, TrackError> {
        Ok(self.rows.lock().unwrap().clone())
    }

    fn watch_for_entry(&self, manga_id: i64) -> RowsWatch<MangaTrack> {
        let initial: Vec<MangaTrack> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.manga_id == manga_id)
            .cloned()
            .collect();
        let (tx, rx) = watch::channel(Ok(initial));
        self.watchers.lock().unwrap().push((manga_id, tx));
        rx
    }

    async fn list_for_entry(&self, manga_id: i64) -> Result<Vec<MangaTrack>, TrackError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.manga_id == manga_id)
            .cloned()
            .collect())
    }

    async fn upsert(&self, t: &MangaTrack) -> Result<i64, TrackError> {
        if let Some(log) = &self.log {
            log.lock().unwrap().push("upsert".into());
        }
        let mut rows = self.rows.lock().unwrap();
        let pos = rows
            .iter()
            .position(|r| r.manga_id == t.manga_id && r.tracker_id == t.tracker_id);
        let id = match pos {
            Some(i) => rows[i].id.unwrap_or(i as i64 + 1),
            None => rows.len() as i64 + 1,
        };
        let stored = MangaTrack {
            id: Some(id),
            ..t.clone()
        };
        match pos {
            Some(i) => rows[i] = stored,
            None => rows.push(stored),
        }
        drop(rows);
        self.publish();
        Ok(id)
    }

    async fn delete(&self, manga_id: i64, tracker_id: TrackerId) -> Result<(), TrackError> {
        self.rows
            .lock()
            .unwrap()
            .retain(|r| !(r.manga_id == manga_id && r.tracker_id == tracker_id));
        self.publish();
        Ok(())
    }
}

/// Anime repository whose next `upsert` fails once, then delegates.
pub struct FlakyAnimeRepo {
    pub inner: Arc<MemAnimeRepo>,
    pub fail_next: AtomicBool,
    pub fail_delete: AtomicBool,
}

#[async_trait]
impl AnimeTrackRepo for FlakyAnimeRepo {
    async fn upsert(&self, t: &AnimeTrack) -> Result<i64, TrackError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(TrackError::Message("disk busy".into()));
        }
        self.inner.upsert(t).await
    }
    async fn upsert_all(&self, ts: &[AnimeTrack]) -> Result<(), TrackError> {
        self.inner.upsert_all(ts).await
    }
    async fn get_by_id(&self, id: i64) -> Result<Option<AnimeTrack>, TrackError> {
        self.inner.get_by_id(id).await
    }
    async fn list_all(&self) -> Result<Vec<AnimeTrack>, TrackError> {
        self.inner.list_all().await
    }
    fn watch_for_entry(&self, anime_id: i64) -> RowsWatch<AnimeTrack> {
        self.inner.watch_for_entry(anime_id)
    }
    async fn list_for_entry(&self, anime_id: i64) -> Result<Vec<AnimeTrack>, TrackError> {
        self.inner.list_for_entry(anime_id).await
    }
    async fn delete(&self, anime_id: i64, tracker_id: TrackerId) -> Result<(), TrackError> {
        if self.fail_delete.swap(false, Ordering::SeqCst) {
            return Err(TrackError::Message("disk busy".into()));
        }
        self.inner.delete(anime_id, tracker_id).await
    }
}

/// Episodes and chapters in memory: `(parent entry id, unit)`. One struct serves both kinds.
#[derive(Default)]
pub struct MemUnits {
    pub rows: Mutex<Vec<(i64, UnitRef)>>,
}

impl MemUnits {
    /// Adds a unit and returns its id.
    pub fn add(&self, parent: i64, number: f64, seen: bool) -> i64 {
        let mut rows = self.rows.lock().unwrap();
        let id = rows.len() as i64 + 1;
        rows.push((parent, UnitRef { id, number, seen }));
        id
    }

    pub fn seen_ids(&self) -> Vec<i64> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, u)| u.seen)
            .map(|(_, u)| u.id)
            .collect()
    }

    fn list_of(&self, parent: i64) -> Vec<UnitRef> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| *p == parent)
            .map(|(_, u)| *u)
            .collect()
    }

    fn mark(&self, ids: &[i64]) {
        for (_, u) in self.rows.lock().unwrap().iter_mut() {
            if ids.contains(&u.id) {
                u.seen = true;
            }
        }
    }
}

#[async_trait]
impl AnimeEpisodeRepo for MemUnits {
    async fn list(&self, anime_id: i64) -> Result<Vec<UnitRef>, TrackError> {
        Ok(self.list_of(anime_id))
    }
    async fn mark_seen(&self, ids: &[i64]) -> Result<(), TrackError> {
        self.mark(ids);
        Ok(())
    }
}

#[async_trait]
impl MangaChapterRepo for MemUnits {
    async fn list(&self, manga_id: i64) -> Result<Vec<UnitRef>, TrackError> {
        Ok(self.list_of(manga_id))
    }
    async fn mark_read(&self, ids: &[i64]) -> Result<(), TrackError> {
        self.mark(ids);
        Ok(())
    }
}

/// History times in memory. The entry id is not used, so one list serves a test.
#[derive(Default)]
pub struct MemHistory {
    pub times: Mutex<Vec<Option<i64>>>,
}

#[async_trait]
impl AnimeHistoryRepo for MemHistory {
    async fn seen_times(&self, _anime_id: i64) -> Result<Vec<Option<i64>>, TrackError> {
        Ok(self.times.lock().unwrap().clone())
    }
}

#[async_trait]
impl MangaHistoryRepo for MemHistory {
    async fn read_times(&self, _manga_id: i64) -> Result<Vec<Option<i64>>, TrackError> {
        Ok(self.times.lock().unwrap().clone())
    }
}

/// Network switch in memory.
pub struct MemNet(pub AtomicBool);

impl MemNet {
    pub fn set_online(&self, online: bool) {
        self.0.store(online, Ordering::SeqCst);
    }
}

impl Connectivity for MemNet {
    fn is_online(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The eleven trackers, with the kind each one implements. Jellyfin (anime) and Kavita (manga) are enhanced and
/// accept one source each.
const ANIME_KINDS: [TrackerId; 4] = [
    TrackerId::KITSU,
    TrackerId::SIMKL,
    TrackerId::JELLYFIN,
    TrackerId::ANILIST,
];

fn tracker_name(id: TrackerId) -> &'static str {
    match id.0 {
        1 => "MyAnimeList",
        2 => "AniList",
        3 => "Kitsu",
        4 => "Shikimori",
        5 => "Bangumi",
        6 => "Komga",
        7 => "MangaUpdates",
        8 => "Kavita",
        9 => "Suwayomi",
        101 => "Simkl",
        _ => "Jellyfin",
    }
}

/// A full tracking context over fakes, with a real retry queue on a temporary database.
pub struct Harness {
    /// Keeps the queue database alive.
    pub _dir: TempDir,
    pub ctx: Arc<TrackCtx>,
    pub log: Log,
    pub anime: HashMap<TrackerId, Arc<FakeAnime>>,
    pub manga: HashMap<TrackerId, Arc<FakeManga>>,
    pub anime_repo: Arc<MemAnimeRepo>,
    pub episodes: Arc<MemUnits>,
    pub chapters: Arc<MemUnits>,
    pub anime_history: Arc<MemHistory>,
    pub net: Arc<MemNet>,
}

impl Harness {
    /// Must run inside a tokio runtime (the registry starts watcher tasks).
    pub fn new() -> Self {
        Self::with_tracker(None)
    }

    /// Like [`Harness::new`], with `replace` standing in for the fake that has the same tracker id.
    pub fn with_tracker(replace: Option<Arc<dyn Tracker>>) -> Self {
        let dir = TempDir::new().unwrap();
        let secrets = MemSecrets::new();
        let log = new_log();
        let mut trackers: Vec<Arc<dyn Tracker>> = Vec::new();
        let mut anime = HashMap::new();
        let mut manga = HashMap::new();
        for id in REGISTRY_ORDER {
            let name = tracker_name(id);
            if ANIME_KINDS.contains(&id) {
                let mut f = FakeAnime::new(id, name, secrets.clone(), log.clone());
                if id == TrackerId::JELLYFIN {
                    f.enhanced = true;
                    f.accepted = &["src.jf"];
                }
                let f = Arc::new(f);
                anime.insert(id, Arc::clone(&f));
                trackers.push(f);
            } else {
                let mut f = FakeManga::new(id, name, secrets.clone(), log.clone());
                if id == TrackerId::KAVITA {
                    f.enhanced = true;
                    f.accepted = &["src.kv"];
                }
                let f = Arc::new(f);
                manga.insert(id, Arc::clone(&f));
                trackers.push(f);
            }
        }
        if let Some(r) = replace {
            trackers.retain(|t| t.id() != r.id());
            trackers.push(r);
        }
        let anime_repo = Arc::new(MemAnimeRepo {
            log: Some(log.clone()),
            ..Default::default()
        });
        let manga_repo = Arc::new(MemMangaRepo {
            log: Some(log.clone()),
            ..Default::default()
        });
        let episodes = Arc::new(MemUnits::default());
        let chapters = Arc::new(MemUnits::default());
        let anime_history = Arc::new(MemHistory::default());
        let net = Arc::new(MemNet(AtomicBool::new(true)));
        let queue = Arc::new(RetryQueue::new(TestDb::handle(&dir)).unwrap());
        let scheduler = Arc::new(RetryScheduler::new(Arc::clone(&queue), net.clone()));
        let ctx = Arc::new(TrackCtx {
            trackers: Arc::new(TrackerManager::new(trackers).unwrap()),
            anime_tracks: anime_repo.clone(),
            manga_tracks: manga_repo.clone(),
            anime_episodes: episodes.clone(),
            manga_chapters: chapters.clone(),
            anime_history: anime_history.clone(),
            manga_history: Arc::new(MemHistory::default()),
            queue,
            scheduler,
            net: net.clone(),
            prefs: prefs(),
            locks: BindingLocks::default(),
            refresh_limit: Default::default(),
            events: EventBus::default(),
        });
        Self {
            _dir: dir,
            ctx,
            log,
            anime,
            manga,
            anime_repo,
            episodes,
            chapters,
            anime_history,
            net,
        }
    }

    /// Logs the tracker in with stored credentials, without the login request.
    pub fn login(&self, id: TrackerId) {
        if let Some(a) = self.anime.get(&id) {
            a.save_credentials("user", "pass").unwrap();
        }
        if let Some(m) = self.manga.get(&id) {
            m.save_credentials("user", "pass").unwrap();
        }
    }

    pub fn anime_tracker(&self, id: TrackerId) -> Arc<FakeAnime> {
        Arc::clone(&self.anime[&id])
    }

    pub fn manga_tracker(&self, id: TrackerId) -> Arc<FakeManga> {
        Arc::clone(&self.manga[&id])
    }
}

/// An anime binding with the given progress and the fake's default fields.
pub fn anime_binding(entry_id: i64, tracker: TrackerId, last_seen: f64) -> AnimeTrack {
    AnimeTrack {
        id: None,
        anime_id: entry_id,
        tracker_id: tracker,
        remote_id: 77,
        library_id: None,
        title: "Show".into(),
        last_episode_seen: last_seen,
        total_episodes: 12,
        status: STATUS_WATCHING,
        score: 0.0,
        remote_url: "https://example.test/77".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

/// A manga binding with the given progress.
pub fn manga_binding(manga_id: i64, tracker: TrackerId, last_read: f64) -> MangaTrack {
    MangaTrack {
        id: None,
        manga_id,
        tracker_id: tracker,
        remote_id: 88,
        library_id: None,
        title: "Manga".into(),
        last_chapter_read: last_read,
        total_chapters: 20,
        status: STATUS_WATCHING,
        score: 0.0,
        remote_url: "https://example.test/88".into(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

/// A log writer that keeps everything in memory, for the logging tests.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

thread_local! {
    /// The buffer and level of the capture running on this thread, if any.
    static CAPTURE: std::cell::RefCell<Option<(Captured, tracing::Level)>> = const { std::cell::RefCell::new(None) };
}

fn capture_wants(meta: &tracing::Metadata<'_>) -> bool {
    CAPTURE
        .try_with(|c| {
            c.borrow()
                .as_ref()
                .is_some_and(|(_, level)| meta.level() <= level)
        })
        .unwrap_or(false)
}

/// Lets a log statement through only on a thread that is capturing, at that capture's level. It never lets tracing
/// cache an answer per log statement: the answer depends on the thread asking.
struct CaptureFilter;

impl<S> tracing_subscriber::layer::Filter<S> for CaptureFilter {
    fn enabled(
        &self,
        meta: &tracing::Metadata<'_>,
        _: &tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        capture_wants(meta)
    }

    fn callsite_enabled(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing_subscriber::filter::LevelFilter> {
        Some(tracing_subscriber::filter::LevelFilter::TRACE)
    }
}

/// Writes a formatted line into the buffer of the capture running on the writing thread.
struct CaptureSink;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureSink {
    type Writer = Box<dyn std::io::Write>;

    fn make_writer(&'a self) -> Self::Writer {
        match CAPTURE.try_with(|c| c.borrow().as_ref().map(|(buf, _)| buf.clone())) {
            Ok(Some(buf)) => Box::new(buf),
            _ => Box::new(std::io::sink()),
        }
    }
}

/// Ends the capture of [`capture_logs`] when dropped. It belongs to the thread that started the capture.
pub struct CaptureGuard {
    previous: Option<(Captured, tracing::Level)>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _ = CAPTURE.try_with(|c| *c.borrow_mut() = previous);
    }
}

/// Captures the log lines of this thread at `level` and above until the guard drops.
///
/// Tracing caches, per log statement and for the whole process, whether any subscriber wants it. A subscriber set only
/// for this thread does not survive that: while it is the only one alive, tracing asks only the thread that first
/// reaches a statement, so a parallel test on a thread with no subscriber caches "never" and the statement stays
/// silent here too. So one subscriber is installed for the whole test binary instead, and it decides per event on
/// whichever thread is capturing; every statement stays "sometimes", whichever thread reaches it first.
pub fn capture_logs(level: tracing::Level) -> (Captured, CaptureGuard) {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(CaptureSink)
            .with_ansi(false)
            .with_filter(CaptureFilter);
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
            .expect("the kuro-track tests install no other global log subscriber");
    });
    // A statement first reached on another thread while the subscriber above was being installed may have stored
    // "never" just after the install refreshed the cache; refresh it again before this capture starts.
    tracing::callsite::rebuild_interest_cache();

    let captured = Captured::default();
    let previous = CAPTURE.with(|c| c.borrow_mut().replace((captured.clone(), level)));
    (
        captured,
        CaptureGuard {
            previous,
            _not_send: std::marker::PhantomData,
        },
    )
}

#[cfg(test)]
mod capture_tests {
    use super::capture_logs;

    /// A log statement first reached on a thread that captures nothing is still captured on the thread that does, and
    /// the other thread's line is not.
    #[test]
    fn a_statement_first_reached_on_another_thread_is_still_captured_here() {
        fn probe() {
            tracing::warn!("capture probe");
        }
        let (captured, _guard) = capture_logs(tracing::Level::TRACE);
        std::thread::spawn(probe).join().unwrap();
        probe();
        let text = captured.text();
        assert_eq!(text.matches("capture probe").count(), 1, "{text}");
    }

    /// Lines below the capture's level are left out, and nothing is captured once the guard drops.
    #[test]
    fn the_level_and_the_guard_bound_the_capture() {
        let (captured, guard) = capture_logs(tracing::Level::WARN);
        tracing::info!("below the level");
        tracing::warn!("at the level");
        drop(guard);
        tracing::error!("after the guard");
        let text = captured.text();
        assert!(
            text.contains("WARN") && text.contains("at the level"),
            "{text}"
        );
        assert!(
            !text.contains("below the level") && !text.contains("after the guard"),
            "{text}"
        );
    }
}
