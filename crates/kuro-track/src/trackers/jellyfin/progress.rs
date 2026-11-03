//! Pure progress rules of the Jellyfin tracker: the season continuous rule, the episodes to mark, and the `DatePlayed`
//! format (T-33a-015, T-33a-016).

use chrono::{DateTime, Utc};

use super::dto::JFItem;
use super::{COMPLETED, UNSEEN, WATCHING};
use crate::trackers::enhanced::EnhancedError;

/// Kotlin `equalsTo` tolerance (`JellyfinApi.kt:162`).
pub const EPISODE_EPSILON: f64 = 0.001;

/// Season progress from the episode walk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeasonProgress {
    pub total: i64,
    pub last_seen: f64,
    pub status: i64,
}

/// The continuous rule. Episodes are ordered by `IndexNumber` first (E-74). The total is the maximum index (E-75).
/// The first unwatched episode decides the status: none watched gives UNSEEN at 0.0, all watched gives COMPLETED at the
/// total, otherwise WATCHING at the index of the episode before the first unwatched one (E-70 to E-72).
pub fn season_progress(items: &[JFItem]) -> Result<SeasonProgress, EnhancedError> {
    if items.is_empty() {
        return Err(EnhancedError::Decode("no episodes".into()));
    }
    let total = items
        .iter()
        .filter_map(|e| e.index_number)
        .max()
        .ok_or_else(|| EnhancedError::Decode("no episode index numbers".into()))?;
    let mut ordered: Vec<&JFItem> = items.iter().collect();
    // Episodes without an index sort first, so one of them can sit before the first unwatched episode and fail (E-73).
    ordered.sort_by_key(|e| e.index_number.unwrap_or(i64::MIN));
    let progress = match ordered.iter().position(|e| !e.user_data.played) {
        None => SeasonProgress {
            total,
            last_seen: total as f64,
            status: COMPLETED,
        },
        Some(0) => SeasonProgress {
            total,
            last_seen: 0.0,
            status: UNSEEN,
        },
        Some(first) => {
            let before = ordered
                .get(first - 1)
                .and_then(|e| e.index_number)
                .ok_or_else(|| EnhancedError::Decode("episode without index number".into()))?;
            SeasonProgress {
                total,
                last_seen: before as f64,
                status: WATCHING,
            }
        }
    };
    Ok(progress)
}

/// Ids of the episodes to mark played for a target `last_seen`, in ascending index order.
///
/// Range mode marks every unplayed episode with an index at or below the target (D-11). Strict mode is the Kotlin rule:
/// the one episode whose index equals the target, played or not (E-77).
pub fn select_marks(items: &[JFItem], last_seen: f64, range: bool) -> Vec<String> {
    let target = last_seen + EPISODE_EPSILON;
    let mut picked: Vec<&JFItem> = items
        .iter()
        .filter(|e| match e.index_number {
            None => false,
            Some(n) if range => !e.user_data.played && (n as f64) <= target,
            Some(n) => ((n as f64) - last_seen).abs() < EPISODE_EPSILON,
        })
        .collect();
    picked.sort_by_key(|e| e.index_number);
    picked.into_iter().map(|e| e.id.clone()).collect()
}

/// `DatePlayed` in UTC: `yyyy-MM-ddTHH:mm:ss.sssZ` (D-10).
pub fn played_date(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::trackers::jellyfin::dto::JFUserData;

    fn ep(id: &str, index: Option<i64>, played: bool) -> JFItem {
        JFItem {
            name: format!("Episode {id}"),
            id: id.to_owned(),
            user_data: JFUserData { played },
            index_number: index,
        }
    }

    fn walk(flags: &[bool]) -> Vec<JFItem> {
        flags
            .iter()
            .enumerate()
            .map(|(i, p)| ep(&format!("e{}", i + 1), Some(i as i64 + 1), *p))
            .collect()
    }

    #[test]
    fn t_33a_015_e70_a_watched_prefix_is_watching_at_the_last_watched_index() {
        let p = season_progress(&walk(&[true, true, false, false])).unwrap();
        assert_eq!(p.last_seen, 2.0);
        assert_eq!(p.status, WATCHING);
        assert_eq!(p.total, 4);
    }

    #[test]
    fn t_33a_015_e71_nothing_watched_is_unseen() {
        let p = season_progress(&walk(&[false, false])).unwrap();
        assert_eq!((p.last_seen, p.status), (0.0, UNSEEN));
    }

    #[test]
    fn t_33a_015_e72_everything_watched_is_completed_at_the_total() {
        let p = season_progress(&walk(&[true, true])).unwrap();
        assert_eq!((p.total, p.last_seen, p.status), (2, 2.0, COMPLETED));
    }

    #[test]
    fn t_33a_015_e73_a_null_index_before_the_first_unwatched_is_a_decode_error() {
        let items = vec![ep("y", None, true), ep("z", Some(2), false)];
        assert!(matches!(
            season_progress(&items),
            Err(EnhancedError::Decode(_))
        ));
    }

    #[test]
    fn t_33a_015_e74_the_order_is_by_index_not_by_server_position() {
        // Server order [3,1,2] with indexes 1 and 2 watched and 3 not watched.
        let items = vec![
            ep("e3", Some(3), false),
            ep("e1", Some(1), true),
            ep("e2", Some(2), true),
        ];
        let p = season_progress(&items).unwrap();
        assert_eq!((p.last_seen, p.status), (2.0, WATCHING));
    }

    #[test]
    fn t_33a_015_e75_the_total_is_the_maximum_index() {
        let items = vec![
            ep("a", Some(1), true),
            ep("b", Some(5), true),
            ep("c", Some(2), true),
        ];
        assert_eq!(season_progress(&items).unwrap().total, 5);
    }

    #[test]
    fn t_33a_015_e90_an_empty_season_is_a_decode_error() {
        assert!(matches!(
            season_progress(&[]),
            Err(EnhancedError::Decode(_))
        ));
    }

    #[test]
    fn t_33a_016_e76_range_mode_marks_every_unplayed_episode_up_to_the_target() {
        let items = walk(&[false, false, false, false, false]);
        assert_eq!(select_marks(&items, 3.0, true), vec!["e1", "e2", "e3"]);
    }

    #[test]
    fn t_33a_016_e77_strict_mode_marks_only_the_equal_index() {
        let items = walk(&[false, false, false, false, false]);
        assert_eq!(select_marks(&items, 3.0, false), vec!["e3"]);
    }

    #[test]
    fn t_33a_016_e78_strict_mode_with_no_equal_index_marks_nothing() {
        let items = walk(&[false, false, false, false, false]);
        assert!(select_marks(&items, 3.5, false).is_empty());
    }

    #[test]
    fn t_33a_016_e79_range_mode_skips_episodes_already_played() {
        let items = walk(&[false, true, false, false, false]);
        assert_eq!(select_marks(&items, 3.0, true), vec!["e1", "e3"]);
    }

    #[test]
    fn t_33a_016_e80_the_date_is_utc_with_milliseconds() {
        // 14:03:21 in Asia/Riyadh (UTC+3) is 11:03:21 UTC.
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 11, 3, 21).unwrap()
            + chrono::Duration::milliseconds(123);
        assert_eq!(played_date(now), "2026-10-08T11:03:21.123Z");
    }
}
