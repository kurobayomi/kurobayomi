//! Item header of the episode and season lists (T-39a-066) and the missing-count rule that feeds it. Upstream:
//! `presentation/entries/components/ItemHeader.kt`, `MissingItemCountListItem.kt`. The missing
//! count itself is `kuro_domain::items::missing::missing_items_count`, which implements the upstream algorithm.

use kuro_domain::items::missing::missing_items_count;
use kuro_i18n::{MsgKey, tr_plural};
use kuro_platform::kind::Kind;

/// What the header shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemHeader {
    /// `anime_num_episodes` or `anime_num_seasons` plural text.
    pub count: String,
    /// `missing_items` plural text, or `None` when there is nothing missing.
    pub missing: Option<String>,
    /// The header is disabled while any row is selected.
    pub enabled: bool,
}

/// Builds the header for the processed (filtered and sorted) list. `count` is that list's length (OD-1: Kotlin parity).
pub fn item_header(kind: Kind, seasons: bool, count: usize, missing: i32, any_selected: bool) -> ItemHeader {
    let count_key = if seasons {
        MsgKey::anime_num_seasons
    } else {
        crate::entry_core::wording::count(kind)
    };
    ItemHeader {
        count: tr_plural(count_key, count as i64, &[]),
        missing: (missing > 0).then(|| tr_plural(MsgKey::missing_items, i64::from(missing), &[])),
        enabled: !any_selected,
    }
}

/// `missingItemsCount = maxOf(missingEpisodesCount, missingSeasonsCount)` (behaviour 8).
pub fn header_missing_count(episode_numbers: &[f64], season_numbers: &[f64]) -> i32 {
    missing_items_count(episode_numbers).max(missing_items_count(season_numbers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_uses_the_singular_and_plural_episode_text() {
        assert_eq!(item_header(Kind::Anime, false, 1, 0, false).count, "1 episode");
        assert_eq!(item_header(Kind::Anime, false, 3, 0, false).count, "3 episodes");
    }

    #[test]
    fn season_list_uses_the_season_plural() {
        let header = item_header(Kind::Anime, true, 2, 0, false);
        assert_eq!(header.count, tr_plural(MsgKey::anime_num_seasons, 2, &[]));
    }

    #[test]
    fn missing_text_hidden_at_zero_and_plural_otherwise() {
        assert_eq!(item_header(Kind::Anime, false, 4, 0, false).missing, None);
        assert_eq!(
            item_header(Kind::Anime, false, 4, 2, false).missing,
            Some(tr_plural(MsgKey::missing_items, 2, &[]))
        );
    }

    #[test]
    fn header_is_disabled_while_selecting() {
        assert!(item_header(Kind::Anime, false, 4, 0, false).enabled);
        assert!(!item_header(Kind::Anime, false, 4, 0, true).enabled);
    }

    #[test]
    fn missing_count_takes_the_larger_of_episodes_and_seasons() {
        // Episodes 1, 2, 4, 4.5, 7 and an unknown (-1): missing 1 and 2 (3 in total).
        let episodes = [1.0, 2.0, 4.0, 4.5, 7.0, -1.0];
        assert_eq!(header_missing_count(&episodes, &[]), 3);
        // Seasons 1 and 4: missing 2. The larger count wins.
        assert_eq!(header_missing_count(&episodes, &[1.0, 4.0]), 3);
        assert_eq!(header_missing_count(&[1.0, 2.0], &[1.0, 5.0]), 3);
        assert_eq!(header_missing_count(&[], &[]), 0);
    }
}
