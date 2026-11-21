//! Home's two rows as plain data (docs/design/home.md): what goes in Continue and in New for you, in which order, and the
//! line under each cover. No GTK here, so every rule is tested on its own; `home::screen` draws the result.

use std::collections::HashMap;

use kuro_domain::updates::model::UpdateRow;
use kuro_platform::kind::Kind;

/// The most covers a row is built with. The row shows as many as fit the window (`fitting`); the rest are behind "See all".
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
    /// Manga: the last page read, counted from 0 as the database stores it. The page count is not stored.
    Page { last_read: i64 },
}

/// The spot in the unit a resume opens. `same_unit` is whether that unit is the one the history row names: only then is
/// the stored progress about it. `progress` is the unit's stored progress (seconds or last page), `total_seconds` an
/// episode's length when known.
pub fn spot(kind: Kind, same_unit: bool, progress: i64, total_seconds: Option<i64>) -> Spot {
    if !same_unit || progress <= 0 {
        return Spot::Start;
    }
    match kind {
        Kind::Anime => match total_seconds {
            Some(total) if total > progress => Spot::Watched { seconds: progress, total },
            _ => Spot::Start,
        },
        Kind::Manga => Spot::Page { last_read: progress },
    }
}

/// The minutes left of an episode, rounded up, at least 1.
pub fn minutes_left(seconds: i64, total: i64) -> i64 {
    ((total - seconds).max(0) + 59).div_euclid(60).max(1)
}

/// How much of an episode is watched, 0 to 1; `None` for a spot with no known length.
pub fn fraction(spot: Spot) -> Option<f64> {
    match spot {
        Spot::Watched { seconds, total } if total > 0 => Some((seconds as f64 / total as f64).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// How many covers of `cover_width` fit a row `width` wide with `gap` between them; at least 1.
pub fn fitting(width: i32, cover_width: i32, gap: i32) -> usize {
    if cover_width <= 0 {
        return 1;
    }
    usize::try_from(((width + gap) / (cover_width + gap)).max(1)).unwrap_or(1)
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
        assert_eq!(spot(Kind::Manga, true, 7, None), Spot::Page { last_read: 7 });
        assert_eq!(spot(Kind::Manga, true, 0, None), Spot::Start);
    }

    #[test]
    fn minutes_left_round_up_and_the_bar_follows_the_seconds() {
        assert_eq!(minutes_left(600, 1440), 14);
        assert_eq!(minutes_left(1430, 1440), 1, "a few seconds left is one minute");
        assert_eq!(fraction(Spot::Watched { seconds: 360, total: 1440 }), Some(0.25));
        assert_eq!(fraction(Spot::Page { last_read: 3 }), None);
    }

    #[test]
    fn as_many_covers_as_fit_and_never_none() {
        assert_eq!(fitting(1000, 156, 12), 5);
        assert_eq!(fitting(100, 156, 12), 1);
        assert_eq!(fitting(1000, 0, 12), 1);
    }
}
