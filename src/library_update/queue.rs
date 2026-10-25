//! The entry model of a library pass and the queue it builds (behaviour 3: category selection, season
//! expansion, item restrictions, title order). Pure: the caller supplies the library rows and the preferences.

use kuro_i18n::{MsgKey, tr};
use std::collections::{HashMap, HashSet};

pub use kuro_db::codec::UpdateStrategy;
pub use kuro_db::codec::FetchType;
use kuro_platform::kind::Kind;

/// `SManga.COMPLETED` / `SAnime.COMPLETED`.
pub const COMPLETED_STATUS: i64 = 2;

/// One favourite entry as a pass sees it. A manga in two categories yields two rows (one per category).
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    pub id: i64,
    pub source: i64,
    pub url: String,
    pub title: String,
    pub kind: Kind,
    /// `0` for no category, `-1` for a season child.
    pub category: i64,
    pub update_strategy: UpdateStrategy,
    pub status: i64,
    pub favorite: bool,
    /// Chapters (manga) or episodes or seasons (anime), as the library view counts them.
    pub total: i64,
    /// Read chapters, or seen episodes or seasons.
    pub read: i64,
    pub next_update_ms: i64,
    pub fetch_type: FetchType,
    pub parent_id: Option<i64>,
    pub fetch_interval: i32,
    pub last_update_ms: i64,
}

impl LibraryEntry {
    /// Unread chapters, or unseen episodes and seasons.
    pub fn unread_or_unseen(&self) -> i64 {
        self.total - self.read
    }

    pub fn has_started(&self) -> bool {
        self.read > 0
    }
}

/// The item restriction tokens of the preferences (`library_update_manga_restriction`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueRestrictions {
    pub skip_completed: bool,
    pub skip_unread: bool,
    pub skip_not_started: bool,
    pub skip_outside_release: bool,
}

const DEFAULT_TOKENS: [&str; 4] = ["manga_fully_read", "manga_ongoing", "manga_started", "manga_outside_release_period"];

impl QueueRestrictions {
    pub fn from_tokens<S: AsRef<str>>(tokens: &[S]) -> Self {
        let has = |token: &str| tokens.iter().any(|t| t.as_ref() == token);
        Self {
            skip_completed: has("manga_ongoing"),
            skip_unread: has("manga_fully_read"),
            skip_not_started: has("manga_started"),
            skip_outside_release: has("manga_outside_release_period"),
        }
    }

    /// The preference when it was never written: the four default skip tokens.
    pub fn from_pref<S: AsRef<str>>(pref: Option<&[S]>) -> Self {
        match pref {
            Some(tokens) => Self::from_tokens(tokens),
            None => Self::from_tokens(&DEFAULT_TOKENS),
        }
    }
}

/// Why an entry was left out of the queue. The text is for the debug log only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NotAlwaysUpdate,
    Completed,
    NotCaughtUp,
    NotStarted,
    NotInReleasePeriod,
    /// The user turned the title's source off.
    SourceOff,
}

impl SkipReason {
    pub fn message(self) -> String {
        match self {
            Self::NotAlwaysUpdate => tr(MsgKey::skipped_reason_not_always_update),
            Self::Completed => tr(MsgKey::skipped_reason_completed),
            Self::NotCaughtUp => tr(MsgKey::skipped_reason_not_caught_up),
            Self::NotStarted => tr(MsgKey::skipped_reason_not_started),
            Self::NotInReleasePeriod => tr(MsgKey::skipped_reason_not_in_release_period),
            Self::SourceOff => tr(MsgKey::source_switch_off),
        }
    }
}

pub struct QueueInput<'a> {
    pub library: &'a [LibraryEntry],
    /// `Some(id)` for a manual run of one category; it ignores the include and exclude lists.
    pub explicit_category: Option<i64>,
    pub include_categories: &'a [i64],
    pub exclude_categories: &'a [i64],
    pub restrictions: QueueRestrictions,
    pub include_season_children: bool,
    /// Upper bound of the fetch window (`Window::upper_ms`); a later `next_update` is outside the release period.
    pub release_window_upper_ms: i64,
    /// Season children per `Seasons`-type parent id. A missing key means no children.
    pub season_children: &'a HashMap<i64, Vec<LibraryEntry>>,
    /// The sources the user turned off: their titles are not updated.
    pub off_sources: &'a HashSet<i64>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Queue {
    /// Sorted by title, byte-wise.
    pub entries: Vec<LibraryEntry>,
    pub skipped: Vec<(i64, SkipReason)>,
}

pub fn build_queue(input: &QueueInput) -> Queue {
    let selected = select_by_category(input);
    let expanded = expand_seasons(selected, input);
    let mut queue = Queue::default();
    for entry in expanded {
        match skip_reason(&entry, input) {
            Some(reason) => queue.skipped.push((entry.id, reason)),
            None => queue.entries.push(entry),
        }
    }
    queue.entries.sort_by(|a, b| a.title.cmp(&b.title));
    queue
}

fn select_by_category(input: &QueueInput) -> Vec<LibraryEntry> {
    if let Some(category) = input.explicit_category {
        return input.library.iter().filter(|e| e.category == category).cloned().collect();
    }
    let included: Vec<&LibraryEntry> = if input.include_categories.is_empty() {
        input.library.iter().collect()
    } else {
        input.library.iter().filter(|e| input.include_categories.contains(&e.category)).collect()
    };
    let excluded: HashSet<i64> = if input.exclude_categories.is_empty() {
        HashSet::new()
    } else {
        input.library.iter().filter(|e| input.exclude_categories.contains(&e.category)).map(|e| e.id).collect()
    };
    let mut seen = HashSet::new();
    included
        .into_iter()
        .filter(|e| !excluded.contains(&e.id))
        .filter(|e| seen.insert(e.id))
        .cloned()
        .collect()
}

fn expand_seasons(selected: Vec<LibraryEntry>, input: &QueueInput) -> Vec<LibraryEntry> {
    selected
        .into_iter()
        .flat_map(|entry| match entry.fetch_type {
            FetchType::Episodes => vec![entry],
            FetchType::Seasons if !input.include_season_children => Vec::new(),
            FetchType::Seasons => input
                .season_children
                .get(&entry.id)
                .into_iter()
                .flatten()
                .filter(|child| child.fetch_type == FetchType::Episodes && !child.favorite)
                .map(|child| LibraryEntry { category: -1, ..child.clone() })
                .collect(),
        })
        .collect()
}

fn skip_reason(entry: &LibraryEntry, input: &QueueInput) -> Option<SkipReason> {
    let r = input.restrictions;
    if input.off_sources.contains(&entry.source) {
        return Some(SkipReason::SourceOff);
    }
    if entry.update_strategy != UpdateStrategy::AlwaysUpdate {
        return Some(SkipReason::NotAlwaysUpdate);
    }
    // a local title is read from disk: no request to spare, and its status (a details.json may say "completed") must not
    // keep new files out
    if entry.source == kuro_local::LOCAL_SOURCE_ID {
        return None;
    }
    if r.skip_completed && entry.status == COMPLETED_STATUS {
        return Some(SkipReason::Completed);
    }
    if r.skip_unread && entry.unread_or_unseen() != 0 {
        return Some(SkipReason::NotCaughtUp);
    }
    if r.skip_not_started && entry.total > 0 && !entry.has_started() {
        return Some(SkipReason::NotStarted);
    }
    if r.skip_outside_release && entry.next_update_ms > input.release_window_upper_ms {
        return Some(SkipReason::NotInReleasePeriod);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, title: &str, category: i64) -> LibraryEntry {
        LibraryEntry {
            id,
            source: 1001,
            url: format!("/{id}"),
            title: title.into(),
            kind: Kind::Manga,
            category,
            update_strategy: UpdateStrategy::AlwaysUpdate,
            status: 1,
            favorite: true,
            total: 10,
            read: 3,
            next_update_ms: 0,
            fetch_type: FetchType::Episodes,
            parent_id: None,
            fetch_interval: 7,
            last_update_ms: 0,
        }
    }

    fn run(library: &[LibraryEntry], explicit: Option<i64>, include: &[i64], exclude: &[i64], r: QueueRestrictions) -> Queue {
        let none = HashMap::new();
        build_queue(&QueueInput {
            library,
            explicit_category: explicit,
            include_categories: include,
            exclude_categories: exclude,
            restrictions: r,
            include_season_children: false,
            release_window_upper_ms: i64::MAX,
            season_children: &none,
            off_sources: &HashSet::new(),
        })
    }

    fn ids(queue: &Queue) -> Vec<i64> {
        queue.entries.iter().map(|e| e.id).collect()
    }

    #[test]
    fn unread_or_unseen_is_total_minus_read_for_each_row() {
        let rows = [(10, 3), (0, 0), (5, 5), (7, 0)];
        for (total, read) in rows {
            let e = LibraryEntry { total, read, ..entry(1, "a", 0) };
            assert_eq!(e.unread_or_unseen(), total - read);
        }
    }

    #[test]
    fn fetch_type_maps_stored_zero_to_seasons_and_one_to_episodes() {
        assert_eq!(FetchType::from_db(0), FetchType::Seasons);
        assert_eq!(FetchType::from_db(1), FetchType::Episodes);
    }

    #[test]
    fn manual_category_run_returns_only_that_category_and_ignores_prefs() {
        let lib = [entry(1, "a", 0), entry(2, "b", 4), entry(3, "c", 4)];
        // Q-05: include/exclude are ignored when a category is given.
        let q = run(&lib, Some(4), &[9], &[4], QueueRestrictions::default());
        assert_eq!(ids(&q), vec![2, 3]);
        // Q-04: category 0 selects the uncategorised entries.
        let q0 = run(&lib, Some(0), &[], &[], QueueRestrictions::default());
        assert_eq!(ids(&q0), vec![1]);
    }

    #[test]
    fn a_title_whose_source_is_off_is_skipped() {
        let none = HashMap::new();
        let off = HashSet::from([1001]);
        let mut other = entry(2, "B", 0);
        other.source = 2002;
        let library = [entry(1, "A", 0), other];
        let q = build_queue(&QueueInput {
            library: &library,
            explicit_category: None,
            include_categories: &[],
            exclude_categories: &[],
            restrictions: QueueRestrictions::default(),
            include_season_children: false,
            release_window_upper_ms: i64::MAX,
            season_children: &none,
            off_sources: &off,
        });
        assert_eq!(q.skipped, vec![(1, SkipReason::SourceOff)]);
        assert_eq!(q.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn not_always_update_is_skipped_regardless_of_restrictions() {
        // Q-08
        let mut e = entry(1, "a", 0);
        e.update_strategy = UpdateStrategy::OnlyFetchOnce;
        let q = run(&[e], None, &[], &[], QueueRestrictions::default());
        assert_eq!(q.skipped, vec![(1, SkipReason::NotAlwaysUpdate)]);
    }

    #[test]
    fn completed_is_allowed_unless_ongoing_restriction_is_set() {
        // Q-09
        let mut e = entry(1, "a", 0);
        e.status = COMPLETED_STATUS;
        let no_restriction = run(std::slice::from_ref(&e), None, &[], &[], QueueRestrictions::default());
        assert_eq!(ids(&no_restriction), vec![1]);
        let restricted = run(std::slice::from_ref(&e), None, &[], &[], QueueRestrictions::from_tokens(&["manga_ongoing"]));
        assert_eq!(restricted.skipped, vec![(1, SkipReason::Completed)]);
    }

    #[test]
    fn manual_run_still_applies_restrictions() {
        // Q-12
        let mut e = entry(1, "a", 3);
        e.status = COMPLETED_STATUS;
        let q = run(&[e], Some(3), &[], &[], QueueRestrictions::from_tokens(&["manga_ongoing"]));
        assert!(q.entries.is_empty());
    }

    #[test]
    fn queue_is_sorted_by_title_bytewise() {
        // Q-17
        let lib = [entry(1, "b", 0), entry(2, "B", 0), entry(3, "a", 0), entry(4, "é", 0)];
        let q = run(&lib, None, &[], &[], QueueRestrictions::default());
        let titles: Vec<&str> = q.entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["B", "a", "b", "é"]);
    }

    #[test]
    fn season_expansion_keeps_favourite_free_episode_children_only() {
        // Q-13, Q-14, Q-15
        let mut parent = entry(10, "series", 0);
        parent.fetch_type = FetchType::Seasons;
        let mut child_ok = entry(11, "s1", -1);
        child_ok.fetch_type = FetchType::Episodes;
        child_ok.favorite = false;
        let mut child_fav = entry(12, "s2", -1);
        child_fav.fetch_type = FetchType::Episodes;
        child_fav.favorite = true;
        let mut child_nested = entry(13, "s3", -1);
        child_nested.fetch_type = FetchType::Seasons;
        child_nested.favorite = false;
        let mut seasons = HashMap::new();
        seasons.insert(10, vec![child_ok.clone(), child_fav, child_nested]);
        let lib = [parent.clone()];

        let with = build_queue(&QueueInput {
            library: &lib,
            explicit_category: None,
            include_categories: &[],
            exclude_categories: &[],
            restrictions: QueueRestrictions::default(),
            include_season_children: true,
            release_window_upper_ms: i64::MAX,
            season_children: &seasons,
            off_sources: &HashSet::new(),
        });
        assert_eq!(ids(&with), vec![11]);
        assert_eq!(with.entries[0].category, -1);

        // Q-13: include_season_children false drops the parent altogether.
        let without = build_queue(&QueueInput {
            library: &lib,
            explicit_category: None,
            include_categories: &[],
            exclude_categories: &[],
            restrictions: QueueRestrictions::default(),
            include_season_children: false,
            release_window_upper_ms: i64::MAX,
            season_children: &seasons,
            off_sources: &HashSet::new(),
        });
        assert!(without.entries.is_empty());
        // Q-16: a parent with no entry in the map contributes nothing.
        let empty_map = HashMap::new();
        let missing = build_queue(&QueueInput {
            library: &lib,
            explicit_category: None,
            include_categories: &[],
            exclude_categories: &[],
            restrictions: QueueRestrictions::default(),
            include_season_children: true,
            release_window_upper_ms: i64::MAX,
            season_children: &empty_map,
            off_sources: &HashSet::new(),
        });
        assert!(missing.entries.is_empty());
    }
}
