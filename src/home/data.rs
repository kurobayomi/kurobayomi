//! Home's two rows as plain data (docs/design/home.md): what goes in Continue and in New for you, in which order, and the
//! line under each cover. No GTK here, so every rule is tested on its own; `home::screen` draws the result.

use std::collections::HashMap;

use kuro_domain::updates::model::UpdateRow;
use kuro_platform::kind::Kind;
use kuro_prefs::library::LibraryDisplayMode;

/// The most covers a row is built with. The row shows as many as fit the page (`cover_grid::columns`); the rest are
/// behind "See all".
pub const ROW_LIMIT: usize = 20;

/// The rows of both kinds in one list, newest first by `at`, each with its kind. Ties keep anime first, then the order
/// each kind's list gave. At most `limit`.
pub fn merge_by_time<T>(anime: Vec<T>, manga: Vec<T>, at: impl Fn(&T) -> i64, limit: usize) -> Vec<(Kind, T)> {
    let mut all: Vec<(Kind, T)> = anime.into_iter().map(|row| (Kind::Anime, row)).chain(manga.into_iter().map(|row| (Kind::Manga, row))).collect();
    // a stable sort: equal times keep the order above
    all.sort_by_key(|(_, row)| std::cmp::Reverse(at(row)));
    all.truncate(limit);
    all
}

/// One cover of New for you: a library title with units the user has not watched or read yet.
#[derive(Clone, Debug, PartialEq)]
pub struct NewCard {
    pub kind: Kind,
    pub entry_id: i64,
    pub title: String,
    /// How many new units the title has.
    pub count: usize,
    /// The oldest new unit: the one a click opens, so nothing is skipped.
    pub first_unit_id: i64,
    pub first_unit_name: String,
    /// When the newest of them arrived (epoch ms).
    pub latest: i64,
    pub source_id: i64,
    pub thumbnail_url: Option<String>,
    pub cover_last_modified: i64,
}

/// The new units of both kinds, one card per title, the title with the most recent arrival first. `rows` are the
/// Updates feed of both kinds (library titles only, as the feed's query is); units already watched or read are not new.
pub fn group_new(rows: &[UpdateRow], limit: usize) -> Vec<NewCard> {
    let mut unread: Vec<&UpdateRow> = rows.iter().filter(|row| !row.done).collect();
    // newest first; among units that arrived together the higher id first, as the feed orders them
    unread.sort_by_key(|row| (std::cmp::Reverse(row.date_fetch), std::cmp::Reverse(row.item_id)));
    let mut cards: Vec<NewCard> = Vec::new();
    let mut index: HashMap<(Kind, i64), usize> = HashMap::new();
    for row in unread {
        match index.get(&(row.kind, row.entry_id)) {
            Some(&at) => {
                // a later (older) unit of a title already listed: it becomes the one a click opens
                let card = &mut cards[at];
                card.count += 1;
                card.first_unit_id = row.item_id;
                card.first_unit_name = row.item_name.clone();
            }
            None => {
                index.insert((row.kind, row.entry_id), cards.len());
                cards.push(NewCard {
                    kind: row.kind,
                    entry_id: row.entry_id,
                    title: row.entry_title.clone(),
                    count: 1,
                    first_unit_id: row.item_id,
                    first_unit_name: row.item_name.clone(),
                    latest: row.date_fetch,
                    source_id: row.source_id,
                    thumbnail_url: row.thumbnail_url.clone(),
                    cover_last_modified: row.cover_last_modified,
                });
            }
        }
    }
    cards.truncate(limit);
    cards
}

/// Where the user is in the unit a Continue cover opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    /// The unit was never started (a resume moved on to the next one, or nothing is stored).
    Start,
    /// Anime: seconds watched of a known length.
    Watched { seconds: i64, total: i64 },
    /// Manga: the last page read, counted from 0 as the database stores it, and the chapter's page count when the
    /// reader stored it (`items.page_count`).
    Page { last_read: i64, pages: Option<i64> },
}

/// The spot in the unit a resume opens. `same_unit` is whether that unit is the one the history row names: only then is
/// the stored progress about it. `progress` is the unit's stored progress (seconds or last page), `total` its size when
/// known: an episode's length in seconds, a chapter's page count.
pub fn spot(kind: Kind, same_unit: bool, progress: i64, total: Option<i64>) -> Spot {
    if !same_unit || progress <= 0 {
        return Spot::Start;
    }
    match kind {
        Kind::Anime => match total {
            Some(total) if total > progress => Spot::Watched { seconds: progress, total },
            _ => Spot::Start,
        },
        Kind::Manga => Spot::Page { last_read: progress, pages: total.filter(|pages| *pages > 0) },
    }
}

/// The minutes left of an episode, rounded up, at least 1.
pub fn minutes_left(seconds: i64, total: i64) -> i64 {
    ((total - seconds).max(0) + 59).div_euclid(60).max(1)
}

/// How much of a unit is done, 0 to 1: the seconds watched of an episode, the pages read of a chapter (the page shown
/// counts, so page 9 of 20 is 9/20); `None` when the size is not known.
pub fn fraction(spot: Spot) -> Option<f64> {
    match spot {
        Spot::Watched { seconds, total } if total > 0 => Some((seconds as f64 / total as f64).clamp(0.0, 1.0)),
        Spot::Page { last_read, pages: Some(pages) } if pages > 0 => Some(((last_read + 1) as f64 / pages as f64).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// What Home shows: the welcome view, or its two rows, or the empty line that points to Browse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub welcome: bool,
    pub continue_row: bool,
    pub new_row: bool,
    pub empty: bool,
}

/// Decides what Home shows. Without a source repository it is the welcome view only, as the first run's one screen
/// (docs/design/app-flow.md). With one, each row shows when it has covers, and the empty line only once both queries
/// answered with nothing (so it never flashes while they load).
pub fn shown(repositories: bool, answered: bool, has_continue: bool, has_new: bool) -> Shown {
    if !repositories {
        return Shown { welcome: true, continue_row: false, new_row: false, empty: false };
    }
    Shown { welcome: false, continue_row: has_continue, new_row: has_new, empty: answered && !has_continue && !has_new }
}

/// How many titles a section shows in the Library's List mode; "See all" has the rest.
pub const LIST_ROWS: i32 = 5;

/// How a section lays out its titles in the Library's display `mode` (Home shares the Library's view options): as many
/// per line as `columns` and that many shown for a grid; one per line and [`LIST_ROWS`] shown for the list.
pub fn layout_of(mode: LibraryDisplayMode, columns: i32) -> (i32, i32) {
    match mode {
        LibraryDisplayMode::List => (1, LIST_ROWS),
        _ => (columns, columns),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(kind: Kind, entry_id: i64, item_id: i64, date_fetch: i64, done: bool) -> UpdateRow {
        UpdateRow {
            kind,
            entry_id,
            entry_title: format!("title {entry_id}"),
            item_id,
            item_name: format!("unit {item_id}"),
            scanlator: None,
            done,
            bookmark: false,
            fillermark: false,
            progress: 0,
            total_seconds: 0,
            source_id: 1,
            date_fetch,
            date_upload: 0,
            thumbnail_url: None,
            cover_last_modified: 0,
        }
    }

    #[test]
    fn both_kinds_merge_newest_first_and_stop_at_the_limit() {
        let merged = merge_by_time(vec![30, 10], vec![20, 5], |at| *at, 3);
        assert_eq!(merged, [(Kind::Anime, 30), (Kind::Manga, 20), (Kind::Anime, 10)]);
        let tie = merge_by_time(vec![7], vec![7], |at| *at, 10);
        assert_eq!(tie, [(Kind::Anime, 7), (Kind::Manga, 7)], "a tie keeps anime first");
    }

    #[test]
    fn new_units_group_by_title_and_a_click_opens_the_oldest() {
        let rows = [
            update(Kind::Anime, 1, 13, 300, false),
            update(Kind::Manga, 2, 58, 250, false),
            update(Kind::Anime, 1, 12, 200, false),
            update(Kind::Anime, 1, 11, 100, true),
            update(Kind::Manga, 3, 9, 50, true),
        ];
        let cards = group_new(&rows, 10);
        assert_eq!(cards.len(), 2, "a title with nothing new is left out");
        assert_eq!((cards[0].kind, cards[0].entry_id, cards[0].count), (Kind::Anime, 1, 2));
        assert_eq!((cards[0].first_unit_id, cards[0].first_unit_name.as_str(), cards[0].latest), (12, "unit 12", 300), "the oldest unwatched, not the newest");
        assert_eq!((cards[1].kind, cards[1].entry_id, cards[1].count), (Kind::Manga, 2, 1));
    }

    #[test]
    fn the_same_entry_id_in_two_kinds_is_two_titles() {
        let rows = [update(Kind::Anime, 1, 2, 20, false), update(Kind::Manga, 1, 3, 10, false)];
        assert_eq!(group_new(&rows, 10).len(), 2);
    }

    #[test]
    fn new_for_you_orders_by_arrival_whatever_order_the_kinds_came_in() {
        let rows = [update(Kind::Manga, 2, 5, 10, false), update(Kind::Anime, 1, 4, 99, false)];
        assert_eq!(group_new(&rows, 10)[0].entry_id, 1);
        assert_eq!(group_new(&rows, 1).len(), 1, "the limit holds");
    }

    #[test]
    fn the_spot_is_the_stored_progress_only_for_the_unit_that_opens() {
        assert_eq!(spot(Kind::Anime, true, 600, Some(1440)), Spot::Watched { seconds: 600, total: 1440 });
        assert_eq!(spot(Kind::Anime, false, 600, Some(1440)), Spot::Start, "a resume that moved on starts the next unit");
        assert_eq!(spot(Kind::Anime, true, 600, None), Spot::Start, "no length, no time left");
        assert_eq!(spot(Kind::Anime, true, 1440, Some(1440)), Spot::Start);
        assert_eq!(spot(Kind::Manga, true, 7, None), Spot::Page { last_read: 7, pages: None }, "a chapter not read since the count is stored");
        assert_eq!(spot(Kind::Manga, true, 7, Some(20)), Spot::Page { last_read: 7, pages: Some(20) });
        assert_eq!(spot(Kind::Manga, true, 7, Some(0)), Spot::Page { last_read: 7, pages: None }, "no pages is no count");
        assert_eq!(spot(Kind::Manga, true, 0, None), Spot::Start);
    }

    #[test]
    fn minutes_left_round_up_and_the_bar_follows_the_seconds() {
        assert_eq!(minutes_left(600, 1440), 14);
        assert_eq!(minutes_left(1430, 1440), 1, "a few seconds left is one minute");
        assert_eq!(fraction(Spot::Watched { seconds: 360, total: 1440 }), Some(0.25));
        assert_eq!(fraction(Spot::Page { last_read: 3, pages: None }), None, "no page count, no line");
        assert_eq!(fraction(Spot::Page { last_read: 8, pages: Some(20) }), Some(0.45), "page 9 of 20");
        assert_eq!(fraction(Spot::Page { last_read: 19, pages: Some(20) }), Some(1.0));
    }

    #[test]
    fn without_a_repository_home_is_the_welcome_view_only() {
        assert_eq!(shown(false, true, true, true), Shown { welcome: true, continue_row: false, new_row: false, empty: false }, "even with an old history");
        assert_eq!(shown(true, true, false, false), Shown { welcome: false, continue_row: false, new_row: false, empty: true });
        assert_eq!(shown(true, false, false, false), Shown { welcome: false, continue_row: false, new_row: false, empty: false }, "no empty line while loading");
        assert_eq!(shown(true, true, true, false), Shown { welcome: false, continue_row: true, new_row: false, empty: false });
    }

    #[test]
    fn a_grid_fills_one_line_and_the_list_shows_a_few_rows() {
        for mode in [LibraryDisplayMode::CompactGrid, LibraryDisplayMode::ComfortableGrid, LibraryDisplayMode::CoverOnlyGrid] {
            assert_eq!(layout_of(mode, 7), (7, 7), "{mode:?}: one line of as many as fit");
        }
        assert_eq!(layout_of(LibraryDisplayMode::List, 7), (1, LIST_ROWS), "the Library's rows, one per line");
    }
}
