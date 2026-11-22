//! Home's suggestions (docs/design/home.md, discovery rows): what AniList answers for the user's recent titles and what
//! it calls trending, kept in one cache file (`Paths::discovery_cache_file`) and refreshed in the background, at most
//! once a day (or an hour after the seeds changed), and when Home's refresh button asks ([`refresh_now`]). Home reads only the cache, through [`follow`], so it never waits on
//! the network. A failed ask keeps the old answer and logs; the user is told nothing.
//!
//! The pure rules (when to ask, how the recommendations of several seeds merge, how the trending lists interleave, what
//! is left out) are the functions under "Rules", each tested.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gtk::glib;
use kuro_platform::kind::Kind;
use kuro_platform::paths::Paths;
use kuro_track::tracker::anilist::discovery::{Discovery, Suggestion};
use tokio::sync::watch;

/// How many recent titles seed the recommendations.
pub const SEEDS: usize = 5;
/// An answer older than this is asked again.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// After the seeds change, the earliest the next ask may come.
const HOUR_MS: i64 = 60 * 60 * 1000;
/// The first check after start, then one each hour.
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// What the cache file holds: when it was asked, for which seeds, and the answer.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cached {
    pub fetched_at: i64,
    pub seeds: Vec<(Kind, i64)>,
    pub answer: Discovery,
}

static LATEST: OnceLock<watch::Sender<Option<Arc<Cached>>>> = OnceLock::new();
/// The app's runtime, kept by [`start`] for the asks Home's refresh button makes later.
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

thread_local! {
    /// An ask is on its way; a second one waits for it instead of asking twice.
    static ASKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn latest() -> &'static watch::Sender<Option<Arc<Cached>>> {
    LATEST.get_or_init(|| watch::channel(read_cache().map(Arc::new)).0)
}

/// The cached answer now and at each refresh; `None` until the first answer.
pub fn follow() -> watch::Receiver<Option<Arc<Cached>>> {
    latest().subscribe()
}

fn cache_file() -> Option<std::path::PathBuf> {
    kuro_platform::system_paths::SystemPaths::system().ok().map(|paths| paths.discovery_cache_file())
}

fn read_cache() -> Option<Cached> {
    let raw = std::fs::read(cache_file()?).ok()?;
    serde_json::from_slice(&raw).inspect_err(|error| tracing::warn!(%error, "home suggestions: the cache file does not read; it is asked again")).ok()
}

fn write_cache(cached: &Cached) {
    let Some(file) = cache_file() else { return };
    let written = serde_json::to_vec(cached).map_err(|e| e.to_string()).and_then(|raw| {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        crate::reader::cache::write_atomic(&file, &raw).map_err(|e| e.to_string())
    });
    if let Err(error) = written {
        tracing::warn!(%error, "home suggestions: the cache could not be written");
    }
}

/// Starts the checks: the first a few seconds after start, then each hour. Call once on the GTK thread, inside the app's
/// runtime (as `community::start`).
pub fn start() {
    if !crate::data::is_installed() {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return tracing::warn!("home suggestions: no runtime on this thread; they are not refreshed");
    };
    // the cache is read now, so Home shows the last answer at once
    let _ = latest();
    let _ = RUNTIME.set(runtime.clone());
    glib::timeout_add_local_once(FIRST_CHECK, move || {
        check(runtime.clone(), false);
        glib::timeout_add_local(CHECK_EVERY, move || {
            check(runtime.clone(), false);
            glib::ControlFlow::Continue
        });
    });
}

/// Home's refresh button (user, 2026-11-22: refresh refreshes Home, suggestions included): asks AniList now, whatever
/// the age of the answer. Nothing happens before [`start`] or while an ask is on its way.
pub fn refresh_now() {
    if let Some(runtime) = RUNTIME.get() {
        check(runtime.clone(), true);
    }
}

/// Whether an ask is on its way (the refresh button turns until it is not).
pub fn is_asking() -> bool {
    ASKING.with(std::cell::Cell::get)
}

/// One check: the seeds from the history, and an ask when [`due`] or `forced`. One ask at a time.
fn check(runtime: tokio::runtime::Handle, forced: bool) {
    if is_asking() {
        return;
    }
    let seeds = seeds();
    let now = kuro_data::time::now_millis();
    let cached = latest().borrow().clone();
    if !forced && !due(now, cached.as_deref(), &seeds) {
        return;
    }
    ASKING.with(|asking| asking.set(true));
    let adult = kuro_prefs::source::SourcePrefs::new(crate::data::app().base.store().clone()).show_nsfw_source.get();
    let asked = seeds.clone();
    let task = runtime.spawn(async move {
        let transport = kuro_track::tracker::anilist::api::HttpTransport::new().map_err(|e| e.to_string())?;
        kuro_track::tracker::anilist::discovery::fetch(&transport, &asked, adult).await.map_err(|e| e.to_string())
    });
    glib::spawn_future_local(async move {
        let answer = task.await;
        ASKING.with(|asking| asking.set(false));
        match answer {
            Ok(Ok(answer)) => {
                let cached = Cached { fetched_at: now, seeds, answer };
                write_cache(&cached);
                latest().send_replace(Some(Arc::new(cached)));
            }
            Ok(Err(error)) => tracing::warn!(%error, "home suggestions: AniList did not answer; the last ones stay"),
            Err(error) => tracing::warn!(%error, "home suggestions: the ask stopped"),
        }
    });
}

/// The seeds: the most recent titles of the history that have a MyAnimeList id in the score store, both kinds in turn
/// ([`mixed_seeds`]).
fn seeds() -> Vec<(Kind, i64)> {
    let db = crate::data::app().db.clone();
    let history = kuro_domain::history::repo::HistoryRepo::new(db.clone());
    let scores = kuro_data::community_score::CommunityScoreRepo::new(db);
    let rows = |kind: Kind| history.list(kind, "").unwrap_or_default();
    let recent: Vec<(Kind, i64)> = super::data::merge_by_time(rows(Kind::Anime), rows(Kind::Manga), |row| row.at, usize::MAX)
        .into_iter()
        .filter_map(|(kind, row)| {
            let stored = scores.by_entry(row.entry_id).ok().flatten()?;
            let mal = stored.remote_id.filter(|id| *id > 0 && stored.provider == "myanimelist")?;
            Some((kind, mal))
        })
        .collect();
    mixed_seeds(recent, SEEDS)
}

// ---------- Rules ----------

/// Whether to ask AniList now: there is no answer yet, it is a day old, or the seeds changed and the last ask is an hour
/// old. No seeds and an answer already there: the trending lists alone are not worth a daily ask.
pub fn due(now: i64, cached: Option<&Cached>, seeds: &[(Kind, i64)]) -> bool {
    let Some(cached) = cached else { return true };
    let age = now - cached.fetched_at;
    age >= DAY_MS || (cached.seeds != seeds && age >= HOUR_MS)
}

/// A title, folded for comparing: lower case, letters and digits only.
pub fn fold(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// What the user already has: the folded titles of the library and the history, and the MyAnimeList ids of the seeds.
#[derive(Default)]
pub struct Known {
    pub titles: HashSet<String>,
    pub mal_ids: HashSet<i64>,
}

impl Known {
    /// Whether `title` is something the user already has, by MyAnimeList id or by title (romaji or English).
    pub fn has(&self, title: &Suggestion) -> bool {
        title.mal_id.is_some_and(|id| self.mal_ids.contains(&id))
            || self.titles.contains(&fold(&title.title))
            || title.english.as_deref().is_some_and(|english| self.titles.contains(&fold(english)))
    }
}

/// The seeds from `recent` (the history's titles with a MyAnimeList id, newest first): the newest of each kind in turn,
/// the kind of the newest title first, so a history of mostly manga still seeds anime when it has any. At most `limit`.
pub fn mixed_seeds(recent: Vec<(Kind, i64)>, limit: usize) -> Vec<(Kind, i64)> {
    let Some(first) = recent.first().map(|(kind, _)| *kind) else { return Vec::new() };
    let (same, other): (Vec<_>, Vec<_>) = recent.into_iter().partition(|(kind, _)| *kind == first);
    in_turn(same.into_iter(), other.into_iter(), limit)
}

/// `first` and `second` taken one each in turn, `first` leading, until `limit` or both are used up.
pub fn in_turn<T>(mut first: impl Iterator<Item = T>, mut second: impl Iterator<Item = T>, limit: usize) -> Vec<T> {
    let mut out = Vec::new();
    while out.len() < limit {
        let (a, b) = (first.next(), second.next());
        if a.is_none() && b.is_none() {
            break;
        }
        out.extend(a);
        if out.len() < limit {
            out.extend(b);
        }
    }
    out
}

/// Recommended for you: the recommendations of every seed in one list, anime and manga in turn so neither kind fills the
/// row (user, 2026-11-21: a manga seed's recommendations have hundreds of votes, an anime sequel's a handful). Within a
/// kind, a title recommended by several seeds or by more users first (the sum of the votes), then by score. Titles the
/// user has, adult ones unless `adult`, and the seeds themselves are left out. At most `limit`.
pub fn recommended(per_seed: &[Vec<Suggestion>], known: &Known, adult: bool, limit: usize) -> Vec<Suggestion> {
    let mut by_id: HashMap<i64, (Suggestion, i64)> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    for title in per_seed.iter().flatten() {
        if (title.adult && !adult) || known.has(title) {
            continue;
        }
        match by_id.get_mut(&title.anilist_id) {
            Some((_, votes)) => *votes += title.votes.max(1),
            None => {
                order.push(title.anilist_id);
                by_id.insert(title.anilist_id, (title.clone(), title.votes.max(1)));
            }
        }
    }
    let mut merged: Vec<(Suggestion, i64)> = order.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    merged.sort_by(|(a, av), (b, bv)| bv.cmp(av).then(b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0))));
    let (anime, manga): (Vec<_>, Vec<_>) = merged.into_iter().map(|(title, votes)| Suggestion { votes, ..title }).partition(|title| title.kind.is_anime());
    in_turn(anime.into_iter(), manga.into_iter(), limit)
}

/// The recommendations of each seed whose kind `shown` keeps (Show, Settings > Appearance).
pub fn of_kinds(per_seed: &[Vec<Suggestion>], shown: impl Fn(Kind) -> bool) -> Vec<Vec<Suggestion>> {
    per_seed.iter().map(|titles| titles.iter().filter(|title| shown(title.kind)).cloned().collect()).collect()
}

/// Trending now: AniList's trending anime and manga taken in turn, so neither kind fills the row. Titles the user has and
/// adult ones unless `adult` are left out. At most `limit`.
pub fn trending(anime: &[Suggestion], manga: &[Suggestion], known: &Known, adult: bool, limit: usize) -> Vec<Suggestion> {
    let keep = |title: &&Suggestion| !(title.adult && !adult) && !known.has(title);
    in_turn(anime.iter().filter(keep).cloned(), manga.iter().filter(keep).cloned(), limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(id: i64, kind: Kind, name: &str, votes: i64, score: Option<f64>) -> Suggestion {
        Suggestion { anilist_id: id, mal_id: Some(id + 1000), kind, title: name.into(), english: None, cover: None, score, adult: false, votes }
    }

    #[test]
    fn it_asks_without_an_answer_after_a_day_and_an_hour_after_the_seeds_change() {
        let seeds = vec![(Kind::Anime, 1)];
        let cached = Cached { fetched_at: 0, seeds: seeds.clone(), answer: Discovery::default() };
        assert!(due(0, None, &seeds));
        assert!(!due(DAY_MS - 1, Some(&cached), &seeds), "same seeds, under a day");
        assert!(due(DAY_MS, Some(&cached), &seeds));
        let other = vec![(Kind::Manga, 2)];
        assert!(!due(HOUR_MS - 1, Some(&cached), &other), "new seeds, but asked under an hour ago");
        assert!(due(HOUR_MS, Some(&cached), &other));
    }

    #[test]
    fn recommendations_of_several_seeds_merge_by_votes_and_leave_out_what_the_user_has() {
        let per_seed = vec![
            vec![title(1, Kind::Anime, "Perfect Blue", 400, Some(8.5)), title(2, Kind::Anime, "Erased", 280, Some(8.1))],
            vec![title(2, Kind::Anime, "Erased", 200, Some(8.1)), title(3, Kind::Manga, "Vagabond", 100, Some(8.7))],
        ];
        let none = Known::default();
        let merged = recommended(&per_seed, &none, false, 10);
        assert_eq!(merged.iter().map(|t| (t.anilist_id, t.votes)).collect::<Vec<_>>(), [(2, 480), (3, 100), (1, 400)], "two seeds outrank one; the kinds take turns");
        let known = Known { titles: [fold("Perfect blue!")].into(), mal_ids: [1003].into() };
        assert_eq!(recommended(&per_seed, &known, false, 10).iter().map(|t| t.anilist_id).collect::<Vec<_>>(), [2], "by title and by MyAnimeList id");
        assert_eq!(recommended(&per_seed, &none, false, 1).len(), 1);
    }

    #[test]
    fn a_manga_seed_with_many_votes_does_not_fill_the_row() {
        // live votes of 2026-11-21: One Piece's recommendations against an anime sequel's
        let per_seed = vec![
            vec![title(1, Kind::Manga, "HUNTER x HUNTER", 700, None), title(2, Kind::Manga, "Dragon Ball", 350, None), title(3, Kind::Manga, "NARUTO", 265, None)],
            vec![title(11, Kind::Anime, "WIND BREAKER", 9, None), title(12, Kind::Anime, "Kyou kara Ore wa!!", 2, None)],
        ];
        let row = recommended(&per_seed, &Known::default(), false, 4);
        assert_eq!(row.iter().map(|t| t.anilist_id).collect::<Vec<_>>(), [11, 1, 12, 2]);
    }

    #[test]
    fn a_hidden_kind_leaves_the_recommendations_of_every_seed() {
        let per_seed = vec![vec![title(1, Kind::Manga, "M", 9, None), title(2, Kind::Anime, "A", 3, None)], vec![title(3, Kind::Manga, "N", 1, None)]];
        let anime_only = of_kinds(&per_seed, |kind| kind == Kind::Anime);
        assert_eq!(anime_only.iter().map(|seed| seed.iter().map(|t| t.anilist_id).collect::<Vec<_>>()).collect::<Vec<_>>(), [vec![2], vec![]]);
        assert_eq!(of_kinds(&per_seed, |_| true), per_seed, "both kinds: all of them");
    }

    #[test]
    fn seeds_take_each_kind_in_turn_newest_first() {
        let recent = vec![(Kind::Manga, 13), (Kind::Manga, 114745), (Kind::Manga, 2), (Kind::Anime, 59088), (Kind::Anime, 1)];
        assert_eq!(mixed_seeds(recent.clone(), 3), [(Kind::Manga, 13), (Kind::Anime, 59088), (Kind::Manga, 114745)]);
        assert_eq!(mixed_seeds(recent, 10).len(), 5, "all of them when there is room");
        assert_eq!(mixed_seeds(vec![(Kind::Anime, 5)], 5), [(Kind::Anime, 5)]);
        assert!(mixed_seeds(Vec::new(), 5).is_empty());
    }

    #[test]
    fn adult_titles_show_only_with_the_setting() {
        let mut adult = title(9, Kind::Manga, "Adult", 50, None);
        adult.adult = true;
        let per_seed = vec![vec![adult.clone()]];
        assert!(recommended(&per_seed, &Known::default(), false, 10).is_empty());
        assert_eq!(recommended(&per_seed, &Known::default(), true, 10).len(), 1);
        assert!(trending(&[], &[adult], &Known::default(), false, 10).is_empty());
    }

    #[test]
    fn trending_takes_anime_and_manga_in_turn() {
        let anime = [title(1, Kind::Anime, "A1", 0, None), title(2, Kind::Anime, "A2", 0, None), title(3, Kind::Anime, "A3", 0, None)];
        let manga = [title(11, Kind::Manga, "M1", 0, None)];
        let row = trending(&anime, &manga, &Known::default(), false, 10);
        assert_eq!(row.iter().map(|t| t.anilist_id).collect::<Vec<_>>(), [1, 11, 2, 3]);
        assert_eq!(trending(&anime, &manga, &Known::default(), false, 3).len(), 3);
        let known = Known { titles: [fold("A1")].into(), mal_ids: HashSet::new() };
        let without = trending(&anime, &manga, &known, false, 10);
        assert_eq!(without.iter().map(|t| t.anilist_id).collect::<Vec<_>>(), [2, 11, 3], "a title the user has is left out; the turns go on");
    }

    #[test]
    fn titles_fold_to_letters_and_digits() {
        assert_eq!(fold("Kaguya-sama: Love Is War!"), "kaguyasamaloveiswar");
        assert_eq!(fold("[Oshi no Ko]"), "oshinoko");
    }
}
