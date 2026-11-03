//! MAL status codes and the wire mapping.
//! Rows keep the raw `i64`; conversion happens at the API boundary.

/// Local status codes, as stored in `manga_sync.status` and `anime_sync.status`.
#[repr(i64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MalStatus {
    Reading = 1,
    Watching = 11,
    Completed = 2,
    OnHold = 3,
    Dropped = 4,
    PlanToRead = 6,
    PlanToWatch = 16,
    Rereading = 7,
    Rewatching = 17,
}

/// A stored code that is not one of the nine MAL codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownStatus(pub i64);

impl MalStatus {
    pub fn code(self) -> i64 {
        self as i64
    }
}

impl TryFrom<i64> for MalStatus {
    type Error = UnknownStatus;

    fn try_from(v: i64) -> Result<Self, Self::Error> {
        Ok(match v {
            1 => Self::Reading,
            11 => Self::Watching,
            2 => Self::Completed,
            3 => Self::OnHold,
            4 => Self::Dropped,
            6 => Self::PlanToRead,
            16 => Self::PlanToWatch,
            7 => Self::Rereading,
            17 => Self::Rewatching,
            other => return Err(UnknownStatus(other)),
        })
    }
}

/// The status choices the manga UI offers, in order (`MyAnimeList.kt:65-71`).
pub const MANGA_LIST: [MalStatus; 6] = [
    MalStatus::Reading,
    MalStatus::Completed,
    MalStatus::OnHold,
    MalStatus::Dropped,
    MalStatus::PlanToRead,
    MalStatus::Rereading,
];

/// The status choices the anime UI offers, in order (`MyAnimeList.kt:65-71`).
pub const ANIME_LIST: [MalStatus; 6] = [
    MalStatus::Watching,
    MalStatus::Completed,
    MalStatus::OnHold,
    MalStatus::Dropped,
    MalStatus::PlanToWatch,
    MalStatus::Rewatching,
];

/// Local manga code to the `status` form value. `None` for codes outside the manga list (E-STAT-2).
pub fn manga_wire(code: i64) -> Option<&'static str> {
    Some(match MalStatus::try_from(code).ok()? {
        MalStatus::Reading | MalStatus::Rereading => "reading",
        MalStatus::Completed => "completed",
        MalStatus::OnHold => "on_hold",
        MalStatus::Dropped => "dropped",
        MalStatus::PlanToRead => "plan_to_read",
        _ => return None,
    })
}

/// Local anime code to the `status` form value (`MyAnimeListUtils.kt:16-27`). Code 1 maps to `watching`.
pub fn anime_wire(code: i64) -> Option<&'static str> {
    Some(match MalStatus::try_from(code).ok()? {
        MalStatus::Watching | MalStatus::Reading | MalStatus::Rewatching | MalStatus::Rereading => {
            "watching"
        }
        MalStatus::Completed => "completed",
        MalStatus::OnHold => "on_hold",
        MalStatus::Dropped => "dropped",
        MalStatus::PlanToWatch | MalStatus::PlanToRead => "plan_to_watch",
    })
}

/// Wire string to local code (`MyAnimeListUtils.kt:29-38`). Unknown and absent values map to 1 (D-17) and log
/// a warning. The `is_rereading` / `is_rewatching` flag is applied by the caller.
pub fn from_wire(s: Option<&str>) -> i64 {
    match s {
        Some("reading") => 1,
        Some("watching") => 11,
        Some("completed") => 2,
        Some("on_hold") => 3,
        Some("dropped") => 4,
        Some("plan_to_read") => 6,
        Some("plan_to_watch") => 16,
        other => {
            tracing::warn!(status = ?other, "MAL status is unknown, using Reading");
            1
        }
    }
}

/// Label of a manga code (`MyAnimeList.kt:73-91`). `None` for every other code.
pub fn manga_label(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "Reading",
        2 => "Completed",
        3 => "On hold",
        4 => "Dropped",
        6 => "Plan to read",
        7 => "Rereading",
        _ => return None,
    })
}

/// Label of an anime code. Code 1 has no anime label.
pub fn anime_label(code: i64) -> Option<&'static str> {
    Some(match code {
        11 => "Watching",
        2 => "Completed",
        3 => "On hold",
        4 => "Dropped",
        16 => "Plan to watch",
        17 => "Rewatching",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_discriminants_round_trip() {
        for code in [1, 11, 2, 3, 4, 6, 16, 7, 17] {
            assert_eq!(MalStatus::try_from(code).unwrap().code(), code);
        }
        assert_eq!(MalStatus::try_from(5), Err(UnknownStatus(5)));
    }

    #[test]
    fn status_manga_wire_roundtrip() {
        for code in [1, 2, 3, 4, 6] {
            assert_eq!(from_wire(manga_wire(code)), code);
        }
        assert_eq!(manga_wire(7), Some("reading"));
        assert_eq!(
            from_wire(manga_wire(7)),
            1,
            "rereading is carried by the flag, not the string"
        );
    }

    #[test]
    fn status_anime_wire_roundtrip() {
        for code in [11, 2, 3, 4, 16] {
            assert_eq!(from_wire(anime_wire(code)), code);
        }
    }

    #[test]
    fn status_anime_reading_is_watching() {
        assert_eq!(anime_wire(1), Some("watching"));
    }

    #[test]
    fn status_anime_plan_to_read_is_plan_to_watch() {
        assert_eq!(anime_wire(6), Some("plan_to_watch"));
    }

    #[test]
    fn status_unknown_wire_is_reading() {
        assert_eq!(from_wire(Some("considering")), 1);
        assert_eq!(from_wire(None), 1);
        assert_eq!(from_wire(Some("")), 1);
    }

    #[test]
    fn status_manga_unknown_code_has_no_wire() {
        assert_eq!(manga_wire(11), None);
        assert_eq!(manga_wire(16), None);
        assert_eq!(manga_wire(99), None);
    }

    #[test]
    fn status_labels() {
        assert_eq!(manga_label(1), Some("Reading"));
        assert_eq!(manga_label(7), Some("Rereading"));
        assert_eq!(manga_label(11), None);
        assert_eq!(anime_label(11), Some("Watching"));
        assert_eq!(anime_label(17), Some("Rewatching"));
        assert_eq!(anime_label(1), None);
        for s in ANIME_LIST {
            assert!(anime_label(s.code()).is_some());
        }
        for s in MANGA_LIST {
            assert!(manga_label(s.code()).is_some());
        }
    }
}
