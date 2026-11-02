//! The library pipeline as a pure function (design R-4, behaviour 2.3 to 2.6): badges, grouping, filter, sort,
//! search and counts, all from kuro-domain's library model. Storage reads live in `load`; the worker runs
//! [`compute`] off the GTK thread.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use kuro_domain::category::model::Category;
use kuro_platform::kind::Kind;
use kuro_domain::library::filter::{EntryFacts, LibraryFilters};
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::order::{SortContext, sort_page};
use kuro_domain::library::page::{BadgeSettings, LibraryItem, LibraryPage, build_pages, distinct_entry_count};
use kuro_domain::library::search::{SearchFields, matches};

/// Entry fields the search grammar reads and a [`LibraryRow`] does not carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub author: Option<String>,
    pub artist: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
}

/// What the view needs to know about a source: its display name (search) and language (badge).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceInfo {
    pub name: String,
    pub language: String,
}

/// What the pipeline reads from storage. Built by `load::snapshot` on the worker.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Visible categories in display order (`sort, _id`).
    pub categories: Vec<Category>,
    /// One row per (entry, category link), in view order.
    pub rows: Vec<LibraryRow>,
    pub details: HashMap<i64, Details>,
    /// Every track of each entry as (tracker id, 10-point score). Logged-out trackers are dropped by the pipeline.
    pub tracks: BTreeMap<i64, Vec<(i64, f64)>>,
}

/// Lookups that live outside storage: the number of downloaded items of an entry, and source facts.
#[derive(Clone)]
pub struct Facts {
    pub downloads: Arc<dyn Fn(&LibraryRow) -> i64 + Send + Sync>,
    pub source: Arc<dyn Fn(i64) -> SourceInfo + Send + Sync>,
}

/// The settings of one pass, read from the preferences on the GTK thread.
#[derive(Clone, Debug)]
pub struct Request {
    pub kind: Kind,
    pub filters: LibraryFilters,
    pub badges: BadgeSettings,
    /// None when no search is active. `Some("")` matches every entry (E-03).
    pub query: Option<String>,
    pub random_seed: i32,
    pub logged_in: BTreeSet<i64>,
}

/// The pipeline output: the pages to show and the counts the toolbar and the empty states read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    pub pages: Vec<LibraryPage>,
    /// Any filter is on, the forced downloaded-only override included (T2-PIPE-07).
    pub has_active_filters: bool,
    /// Distinct entries across the pages after filter and search (E-11).
    pub distinct_count: usize,
    /// Every library row, filtered or not: the selection refreshes its snapshots from these (T-37a-059).
    pub library: Vec<LibraryRow>,
}

impl Built {
    /// `isLibraryEmpty`: nothing is left on any page.
    pub fn is_library_empty(&self) -> bool {
        self.distinct_count == 0
    }
}

/// Runs the pipeline. Order matches upstream: badges and grouping, then per category filter, sort and search.
pub fn compute(req: &Request, snap: &Snapshot, facts: &Facts) -> Built {
    let pages = build_pages(
        &snap.categories,
        &snap.rows,
        req.badges,
        |row| (facts.downloads)(row),
        |source| (facts.source)(source).language,
    );
    // Decision (T-37a-058, E-14): only logged-in trackers feed the mean, so an entry with only logged-out tracks has no
    // scores and sorts as -1.0 (`tracker_mean`, which also maps a NaN mean to -1.0).
    let scores: BTreeMap<i64, Vec<f64>> = snap
        .tracks
        .iter()
        .map(|(id, tracks)| {
            let logged = tracks.iter().filter(|(t, _)| req.logged_in.contains(t)).map(|(_, s)| *s).collect();
            (*id, logged)
        })
        .collect();
    let ctx = SortContext { tracker_scores: &scores, random_seed: req.random_seed };
    let no_details = Details::default();

    let pages: Vec<LibraryPage> = pages
        .into_iter()
        .map(|page| {
            let kept: Vec<LibraryItem> = page
                .items
                .into_iter()
                .filter(|item| {
                    let trackers: Vec<i64> = snap
                        .tracks
                        .get(&item.row.id)
                        .map(|t| t.iter().map(|(id, _)| *id).collect())
                        .unwrap_or_default();
                    let entry = EntryFacts { downloads: (facts.downloads)(&item.row), tracker_ids: &trackers };
                    req.filters.keep(&item.row, entry, &req.logged_in)
                })
                .collect();
            let sort = page.category.sort(req.kind);
            let mut items = sort_page(kept, sort, &ctx);
            if let Some(query) = &req.query {
                items.retain(|item| {
                    let details = snap.details.get(&item.row.id).unwrap_or(&no_details);
                    let source = (facts.source)(item.row.source);
                    let fields = SearchFields {
                        id: item.row.id,
                        title: &item.row.title,
                        author: details.author.as_deref(),
                        artist: details.artist.as_deref(),
                        description: details.description.as_deref(),
                        source_name: &source.name,
                        genres: &details.genres,
                    };
                    matches(query, &fields)
                });
            }
            LibraryPage { category: page.category, items }
        })
        .collect();

    let distinct_count = distinct_entry_count(&pages);
    Built {
        pages,
        has_active_filters: req.filters.has_active() || req.filters.downloaded_only,
        distinct_count,
        library: snap.rows.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::tristate::TriState;

    const ASC_ALPHA: i64 = 0x40;
    const DESC_ALPHA: i64 = 0;

    fn cat(id: i64, name: &str, flags: i64) -> Category {
        Category { id, name: name.into(), order: id, flags, hidden: false }
    }

    fn row(id: i64, title: &str, category: i64, total: i64, done: i64) -> LibraryRow {
        LibraryRow {
            id,
            source: 7,
            title: title.into(),
            category,
            total,
            done,
            bookmarks: 0,
            fillermarks: 0,
            latest_upload: 0,
            item_fetched_at: 0,
            last_activity: 0,
            last_update: 0,
            date_added: 0,
            status: 1,
            fetch_interval: 0,
            viewer_flags: 0,
            always_update: true,
            next_update: 0,
        }
    }

    fn facts() -> Facts {
        Facts {
            downloads: Arc::new(|_| 0),
            source: Arc::new(|s| SourceInfo { name: format!("Src{s}"), language: "en".into() }),
        }
    }

    fn request() -> Request {
        Request {
            kind: Kind::Manga,
            filters: LibraryFilters::default(),
            badges: BadgeSettings::default(),
            query: None,
            random_seed: 0,
            logged_in: BTreeSet::new(),
        }
    }

    fn snapshot(categories: Vec<Category>, rows: Vec<LibraryRow>) -> Snapshot {
        Snapshot { categories, rows, details: HashMap::new(), tracks: BTreeMap::new() }
    }

    fn ids(built: &Built) -> Vec<Vec<i64>> {
        built.pages.iter().map(|p| p.items.iter().map(|i| i.row.id).collect()).collect()
    }

    #[test]
    fn rows_group_by_category_and_sort_alphabetically_ignoring_case() {
        let snap = snapshot(
            vec![cat(5, "Hold", ASC_ALPHA)],
            vec![row(1, "bleach", 5, 3, 0), row(2, "Akira", 5, 3, 0), row(3, "chainsaw", 5, 3, 0)],
        );
        let built = compute(&request(), &snap, &facts());
        assert_eq!(ids(&built), vec![vec![2, 1, 3]]);
        assert_eq!(built.distinct_count, 3);
        assert!(!built.is_library_empty());
    }

    #[test]
    fn descending_flags_reverse_and_each_category_uses_its_own_sort() {
        let snap = snapshot(
            vec![cat(5, "A", ASC_ALPHA), cat(6, "B", DESC_ALPHA)],
            vec![row(1, "x", 5, 1, 0), row(2, "y", 5, 1, 0), row(3, "x", 6, 1, 0), row(4, "y", 6, 1, 0)],
        );
        let built = compute(&request(), &snap, &facts());
        assert_eq!(ids(&built), vec![vec![1, 2], vec![4, 3]]);
    }

    #[test]
    fn unseen_sort_puts_zero_counts_last_in_both_directions() {
        // flags 12 (UnreadCount), with and without the ascending bit
        let asc = snapshot(
            vec![cat(5, "A", 12 | 0x40)],
            vec![row(1, "a", 5, 1, 1), row(2, "b", 5, 3, 0), row(3, "c", 5, 2, 0)],
        );
        assert_eq!(ids(&compute(&request(), &asc, &facts())), vec![vec![3, 2, 1]]);
        let desc =
            snapshot(vec![cat(5, "A", 12)], vec![row(1, "a", 5, 1, 1), row(2, "b", 5, 3, 0), row(3, "c", 5, 2, 0)]);
        assert_eq!(ids(&compute(&request(), &desc, &facts())), vec![vec![2, 3, 1]]);
    }

    #[test]
    fn unread_filter_keeps_rows_with_pending_items_only() {
        let snap = snapshot(vec![cat(5, "A", ASC_ALPHA)], vec![row(1, "a", 5, 3, 3), row(2, "b", 5, 3, 1)]);
        let mut req = request();
        req.filters.unread = TriState::ENABLED_IS;
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![2]]);
        req.filters.unread = TriState::ENABLED_NOT;
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![1]]);
    }

    #[test]
    fn an_entry_in_two_categories_is_on_both_pages_and_counts_once() {
        let snap = snapshot(
            vec![cat(5, "A", ASC_ALPHA), cat(6, "B", ASC_ALPHA)],
            vec![row(1, "x", 5, 1, 0), row(1, "x", 6, 1, 0)],
        );
        let built = compute(&request(), &snap, &facts());
        assert_eq!(ids(&built), vec![vec![1], vec![1]]);
        assert_eq!(built.distinct_count, 1);
    }

    #[test]
    fn default_page_is_dropped_when_no_entry_is_uncategorized() {
        let snap = snapshot(vec![cat(0, "", ASC_ALPHA), cat(5, "A", ASC_ALPHA)], vec![row(1, "x", 5, 1, 0)]);
        assert_eq!(ids(&compute(&request(), &snap, &facts())), vec![vec![1]]);
    }

    #[test]
    fn search_matches_title_details_source_and_genre_and_the_id_form() {
        let mut snap = snapshot(
            vec![cat(5, "A", ASC_ALPHA)],
            vec![row(1, "Berserk", 5, 1, 0), row(2, "Dune", 5, 1, 0), row(3, "Other", 5, 1, 0)],
        );
        snap.details
            .insert(2, Details { author: Some("Herbert".into()), genres: vec!["Sci-Fi".into()], ..Details::default() });
        let mut req = request();
        req.query = Some("herbert".into());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![2]]);
        req.query = Some("sci-fi".into());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![2]]);
        req.query = Some("src7".into());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![1, 2, 3]]);
        req.query = Some("id:3".into());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![3]]);
        req.query = Some(String::new());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn negated_search_terms_exclude_matching_sources() {
        let snap = snapshot(vec![cat(5, "A", ASC_ALPHA)], vec![row(1, "a", 5, 1, 0), row(2, "b", 5, 1, 0)]);
        let mut req = request();
        req.query = Some("-src7".into());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![Vec::<i64>::new()]);
        req.query = Some("-src8".into());
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![1, 2]]);
    }

    #[test]
    fn downloaded_only_counts_as_an_active_filter() {
        let snap = snapshot(vec![cat(5, "A", ASC_ALPHA)], vec![row(1, "a", 5, 1, 0)]);
        let mut req = request();
        req.filters.downloaded_only = true;
        assert!(compute(&req, &snap, &facts()).has_active_filters);
        req.filters.downloaded_only = false;
        assert!(!compute(&req, &snap, &facts()).has_active_filters);
        req.filters.unread = TriState::ENABLED_IS;
        assert!(compute(&req, &snap, &facts()).has_active_filters);
    }

    #[test]
    fn tracker_filter_only_sees_logged_in_trackers() {
        let mut snap = snapshot(vec![cat(5, "A", ASC_ALPHA)], vec![row(1, "a", 5, 1, 0), row(2, "b", 5, 1, 0)]);
        snap.tracks.insert(1, vec![(10, 8.0)]);
        snap.tracks.insert(2, vec![(11, 9.0)]);
        let mut req = request();
        req.logged_in = BTreeSet::from([10]);
        req.filters.tracking.insert(10, TriState::ENABLED_IS);
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![1]]);
        req.filters.tracking.insert(10, TriState::ENABLED_NOT);
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![2]]);
    }

    #[test]
    fn tracker_mean_sort_puts_unscored_entries_first_when_ascending() {
        let mut snap = snapshot(
            vec![cat(5, "A", 0x40 | 32)],
            vec![row(1, "a", 5, 1, 0), row(2, "b", 5, 1, 0), row(3, "c", 5, 1, 0)],
        );
        snap.tracks.insert(1, vec![(10, 7.5)]);
        snap.tracks.insert(3, vec![(10, 9.0)]);
        snap.tracks.insert(2, vec![(11, 5.0)]);
        let mut req = request();
        req.logged_in = BTreeSet::from([10]);
        assert_eq!(ids(&compute(&req, &snap, &facts())), vec![vec![2, 1, 3]]);
    }

    #[test]
    fn no_visible_category_with_an_active_search_is_an_empty_library_not_a_panic() {
        // T-37a-047 (T2-PIPE-12): nothing visible, search on.
        let snap = snapshot(vec![], vec![row(1, "x", 5, 1, 0)]);
        let mut req = request();
        req.query = Some("x".into());
        let built = compute(&req, &snap, &facts());
        assert!(built.pages.is_empty());
        assert!(built.is_library_empty());
    }

    #[test]
    fn a_visible_empty_category_keeps_its_page_under_an_active_search() {
        // T-37a-047: an empty visible category is still a page, with no items.
        let snap = snapshot(vec![cat(5, "A", ASC_ALPHA)], vec![row(1, "x", 6, 1, 0)]);
        let mut req = request();
        req.query = Some("x".into());
        let built = compute(&req, &snap, &facts());
        assert_eq!(ids(&built), vec![Vec::<i64>::new()]);
        assert!(built.is_library_empty());
    }

    #[test]
    fn random_sort_is_stable_for_one_seed() {
        let rows: Vec<LibraryRow> = (1..=12).map(|i| row(i, &format!("t{i}"), 5, 1, 0)).collect();
        let snap = snapshot(vec![cat(5, "A", 60 | 0x40)], rows);
        let mut req = request();
        req.random_seed = 42;
        let first = ids(&compute(&req, &snap, &facts()));
        assert_eq!(first, ids(&compute(&req, &snap, &facts())));
        assert_eq!(first[0].len(), 12);
    }
}
