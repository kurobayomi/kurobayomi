//! Detached remote writes of the track sheet (T-38b-031, T-38b-033; behaviour 8.2 to 8.6, E-TRK-1, E-TRK-7, E-TRK-15).
//! Each picker closes first and then runs its write detached, so the write survives the sheet closing. A write that
//! fails keeps the local row as it was and names the service in the banner (proposal: the upstream code is silent).

/// One remote edit of a bound row, from a picker or a toggle.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Status(i64),
    /// The chapter or episode number, already truncated.
    Progress(i64),
    /// The score label chosen in the list.
    Score(String),
    /// Epoch milliseconds of local midnight, or `0` to remove the date (E-DATE-3).
    StartDate(i64),
    FinishDate(i64),
    /// Hidden from the public list on the site (trackers with private support only).
    Private(bool),
}

/// The date a remove-date confirm stores: `0`, which clears the date (E-DATE-3, behaviour 8.6).
pub const REMOVE_DATE: i64 = 0;

/// The date an edit stores for a picked day: its local midnight in `zone` (see `dates::stored_ms`).
pub fn date_edit(is_start: bool, stored_ms: i64) -> Edit {
    if is_start { Edit::StartDate(stored_ms) } else { Edit::FinishDate(stored_ms) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remove_date_confirm_names_the_tracker_and_the_date() {
        let start = kuro_track::picker::DateRemoval { tracker: "AniList".into(), is_start: true };
        assert_eq!(start.text(), "This will remove your previously selected start date from AniList");
        let finish = kuro_track::picker::DateRemoval { tracker: "Kitsu".into(), is_start: false };
        assert!(finish.text().contains("finish date from Kitsu"));
    }

    #[test]
    fn remove_date_writes_zero_and_is_offered_only_for_a_set_date() {
        assert_eq!(REMOVE_DATE, 0);
        assert!(!kuro_track::picker::remove_offered(0), "an unset date has no remove row");
        assert!(kuro_track::picker::remove_offered(1_700_000_000_000));
        assert_eq!(date_edit(true, 0), Edit::StartDate(0));
        assert_eq!(date_edit(false, 5), Edit::FinishDate(5));
    }
}
