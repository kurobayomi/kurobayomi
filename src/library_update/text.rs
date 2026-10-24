//! Notification texts of a library pass (behaviour 8.3, 8.5, 8.6, 8.7). Pure string builders; the notifier
//! only posts them.

use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_platform::kind::Kind;

pub const PROGRESS_TITLE_CHOP: usize = 40;
pub const SUMMARY_TITLE_CHOP: usize = 45;
/// Chapter or episode numbers listed in a description before "and N more".
pub const MAX_LISTED_NUMBERS: usize = 5;

/// A source with more queued entries than this in one pass gets the large-queue warning (unless it is unmetered).
pub const QUEUE_WARNING_THRESHOLD: usize = 60;
/// How long the large-queue warning stays on screen.
pub const QUEUE_WARNING_TIMEOUT_MS: u64 = 30_000;
pub fn queue_warning_title() -> String {
    tr(MsgKey::label_warning)
}
pub fn queue_warning_text() -> String {
    tr(MsgKey::notification_size_warning)
}
/// The help page the warning opens.
pub const QUEUE_WARNING_URL: &str = kuro_platform::links::URL_LARGE_UPDATES;

/// Whether the large-queue warning shows: `per_source` holds (queued entries, unmetered) for each source of the pass.
/// Only a source that is not unmetered and has more than [`QUEUE_WARNING_THRESHOLD`] entries counts.
pub fn queue_needs_warning(per_source: &[(usize, bool)]) -> bool {
    per_source.iter().any(|(count, unmetered)| !unmetered && *count > QUEUE_WARNING_THRESHOLD)
}

/// `chop` of the Kotlin helper: text longer than `count` characters is cut to `count - 1` characters plus "…".
pub fn chop(text: &str, count: usize) -> String {
    if text.chars().count() > count {
        let mut out: String = text.chars().take(count.saturating_sub(1)).collect();
        out.push('…');
        out
    } else {
        text.to_owned()
    }
}

/// Floor percent of `current / total`, `0` when `total` is zero.
pub fn percent(current: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (current.min(total) * 100 / total) as u32
}

/// "Updating library… (N%)".
pub fn progress_title(current: u64, total: u64) -> String {
    trf(MsgKey::notification_updating_progress, &[("arg1", Arg::Str(&format!("{}%", percent(current, total))))])
}

/// The in-progress titles, one per line, each chopped to 40 characters.
pub fn progress_body(titles: &[String]) -> String {
    titles.iter().map(|t| chop(t, PROGRESS_TITLE_CHOP)).collect::<Vec<_>>().join("\n")
}

/// Title and body of the summary notification of one kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryText {
    pub title: String,
    pub text: String,
    /// `None` when content is hidden (or when a single entry shows its title in `text`).
    pub big_text: Option<String>,
}

fn plural(count: usize, one: &str, other: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { other })
}

/// The summary of the entries that have new items. `titles` has one entry per updated entry.
pub fn summary(kind: Kind, titles: &[String], hide_content: bool) -> SummaryText {
    let (title, entry_word) = match kind {
        Kind::Manga => (tr(MsgKey::notification_new_chapters), ("entry", "entries")),
        Kind::Anime => (tr(MsgKey::notification_new_episodes), ("title", "titles")),
    };
    if titles.len() == 1 && !hide_content {
        return SummaryText { title, text: chop(&titles[0], SUMMARY_TITLE_CHOP), big_text: None };
    }
    let text = format!("For {}", plural(titles.len(), entry_word.0, entry_word.1));
    let big_text = (!hide_content).then(|| {
        titles.iter().map(|t| chop(t, SUMMARY_TITLE_CHOP)).collect::<Vec<_>>().join("\n")
    });
    SummaryText { title, text, big_text }
}

/// A chapter or episode number as shown: "2.5", "3" (no trailing zeros), at most three decimals.
pub fn format_number(number: f64) -> String {
    let text = format!("{number:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Description of the new items of one entry (`getNewChaptersDescription`). `numbers` has one value per new
/// item; a negative or NaN value is an unrecognised number.
pub fn new_items_description(kind: Kind, numbers: &[f64]) -> String {
    let (generic_one, generic_many, single, single_more, multi, multi_more) = match kind {
        Kind::Manga => ("new chapter", "new chapters", "Chapter", "Chapter", "Chapters", "Chapters"),
        Kind::Anime => ("new episode", "new episodes", "Episode", "Episode", "Episodes", "Episodes"),
    };
    let total = numbers.len();

    let mut sorted: Vec<f64> = numbers.iter().copied().filter(|n| *n >= 0.0).collect();
    sorted.sort_by(f64::total_cmp);
    let mut displayable: Vec<String> = Vec::new();
    for number in sorted {
        let formatted = format_number(number);
        if !displayable.contains(&formatted) {
            displayable.push(formatted);
        }
    }

    match displayable.len() {
        0 => plural(total, generic_one, generic_many),
        1 => {
            // The "and N more" count is the item count, so unrecognised numbers count too (Kotlin behaviour).
            let remaining = total - 1;
            if remaining == 0 {
                format!("{single} {}", displayable[0])
            } else {
                format!("{single_more} {} and {remaining} more", displayable[0])
            }
        }
        n if n > MAX_LISTED_NUMBERS => {
            let remaining = n - MAX_LISTED_NUMBERS;
            let listed = displayable[..MAX_LISTED_NUMBERS].join(", ");
            format!("{multi_more} {listed} and {remaining} more")
        }
        _ => format!("{multi} {}", displayable.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Kind = Kind::Manga;
    const A: Kind = Kind::Anime;

    #[test]
    fn large_queue_warning_needs_more_than_sixty_entries_from_a_metered_source() {
        // Q-18: 60 is fine, 61 warns, an unmetered source is exempt, and any one source is enough.
        assert!(!queue_needs_warning(&[(60, false)]));
        assert!(queue_needs_warning(&[(61, false)]));
        assert!(!queue_needs_warning(&[(500, true)]));
        assert!(queue_needs_warning(&[(3, false), (1, true), (70, false)]));
        assert!(!queue_needs_warning(&[]));
    }

    #[test]
    fn progress_percent_rounds_down_and_guards_zero() {
        assert_eq!(percent(0, 0), 0); // N-10
        assert_eq!(percent(1, 3), 33); // N-11
        assert_eq!(percent(2, 3), 66); // N-12
        assert_eq!(progress_title(1, 3), "Updating library… (33%)");
    }

    #[test]
    fn chop_cuts_on_characters_and_adds_an_ellipsis() {
        let forty = "a".repeat(40);
        assert_eq!(chop(&forty, 40), forty); // exactly the limit is kept
        let forty_one = "a".repeat(41);
        assert_eq!(chop(&forty_one, 40), format!("{}…", "a".repeat(39))); // N-13
        let japanese: String = "漫".repeat(50);
        let cut = chop(&japanese, 45);
        assert_eq!(cut.chars().count(), 45); // N-15: counts characters, not bytes
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn progress_body_lists_chopped_titles_by_line() {
        let body = progress_body(&["Short".into(), "b".repeat(50)]);
        let lines: Vec<&str> = body.split('\n').collect();
        assert_eq!(lines[0], "Short");
        assert_eq!(lines[1].chars().count(), 40);
    }

    #[test]
    fn summary_single_entry_shows_its_title_when_content_is_visible() {
        let s = summary(M, &["One Piece".into()], false);
        assert_eq!(s.title, "New chapters found");
        assert_eq!(s.text, "One Piece");
        assert_eq!(s.big_text, None);
    }

    #[test]
    fn summary_counts_entries_and_lists_titles_when_visible() {
        let titles: Vec<String> = vec!["A".into(), "B".into()];
        let s = summary(M, &titles, false);
        assert_eq!(s.text, "For 2 entries");
        assert_eq!(s.big_text.as_deref(), Some("A\nB")); // P-15
        let anime = summary(A, &titles, false);
        assert_eq!(anime.title, "New episodes found");
        assert_eq!(anime.text, "For 2 titles");
    }

    #[test]
    fn hidden_content_shows_only_the_generic_count() {
        let s = summary(M, &["Secret".into()], true); // N-17
        assert_eq!(s.text, "For 1 entry");
        assert_eq!(s.big_text, None);
        let many = summary(A, &["x".into(), "y".into(), "z".into()], true);
        assert_eq!(many.text, "For 3 titles");
    }

    #[test]
    fn description_generic_when_no_number_is_recognised() {
        assert_eq!(new_items_description(M, &[-1.0]), "1 new chapter"); // N-01
        assert_eq!(new_items_description(M, &[-1.0; 5]), "5 new chapters"); // N-02
        assert_eq!(new_items_description(A, &[-1.0, -1.0]), "2 new episodes");
    }

    #[test]
    fn description_single_number_with_and_more() {
        assert_eq!(new_items_description(M, &[2.5]), "Chapter 2.5"); // N-03
        let mut ten = vec![2.5];
        ten.extend([-1.0; 9]);
        assert_eq!(new_items_description(M, &ten), "Chapter 2.5 and 9 more"); // N-04
        assert_eq!(new_items_description(A, &[3.0]), "Episode 3");
    }

    #[test]
    fn description_lists_up_to_five_numbers() {
        assert_eq!(new_items_description(M, &[1.0, 2.0, 3.0]), "Chapters 1, 2, 3"); // N-05
        let six: Vec<f64> = (1..=6).map(f64::from).collect();
        assert_eq!(new_items_description(M, &six), "Chapters 1, 2, 3, 4, 5 and 1 more"); // N-06
        let seven: Vec<f64> = (1..=7).map(f64::from).collect();
        assert_eq!(new_items_description(M, &seven), "Chapters 1, 2, 3, 4, 5 and 2 more"); // N-07
        assert_eq!(new_items_description(A, &seven), "Episodes 1, 2, 3, 4, 5 and 2 more");
    }

    #[test]
    fn description_collapses_duplicates_after_formatting() {
        assert_eq!(new_items_description(M, &[1.0, 1.0, 2.0]), "Chapters 1, 2"); // N-08
    }

    #[test]
    fn numbers_format_without_trailing_zeros() {
        assert_eq!(format_number(2.0), "2"); // N-09
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(10.0), "10");
    }
}
