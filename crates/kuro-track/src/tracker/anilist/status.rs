//! Local statuses (stored integers) and the AniList `MediaListStatus` strings.
//! Manga and anime keep separate enums because `COMPLETED = 2` is shared and the other integers differ.

use super::error::AniListError;

/// AniList `MediaListStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiStatus {
    Current,
    Planning,
    Completed,
    Dropped,
    Paused,
    Repeating,
}

impl ApiStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiStatus::Current => "CURRENT",
            ApiStatus::Planning => "PLANNING",
            ApiStatus::Completed => "COMPLETED",
            ApiStatus::Dropped => "DROPPED",
            ApiStatus::Paused => "PAUSED",
            ApiStatus::Repeating => "REPEATING",
        }
    }

    /// Parses the API string. Any other text is [`AniListError::UnknownStatus`].
    pub fn from_api_str(s: &str) -> Result<Self, AniListError> {
        match s {
            "CURRENT" => Ok(ApiStatus::Current),
            "PLANNING" => Ok(ApiStatus::Planning),
            "COMPLETED" => Ok(ApiStatus::Completed),
            "DROPPED" => Ok(ApiStatus::Dropped),
            "PAUSED" => Ok(ApiStatus::Paused),
            "REPEATING" => Ok(ApiStatus::Repeating),
            other => Err(AniListError::UnknownStatus(other.to_owned())),
        }
    }
}

/// Manga statuses. Stored integers: 1, 2, 3, 4, 5, 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MangaStatus {
    Reading,
    Completed,
    OnHold,
    Dropped,
    PlanToRead,
    Rereading,
}

impl MangaStatus {
    /// Picker order shown to the user.
    pub const ORDER: [MangaStatus; 6] = [
        MangaStatus::Reading,
        MangaStatus::PlanToRead,
        MangaStatus::Completed,
        MangaStatus::Rereading,
        MangaStatus::OnHold,
        MangaStatus::Dropped,
    ];

    /// `None` for any integer that is not a manga status (for example 11, an anime status).
    pub fn from_db(v: i64) -> Option<Self> {
        match v {
            1 => Some(MangaStatus::Reading),
            2 => Some(MangaStatus::Completed),
            3 => Some(MangaStatus::OnHold),
            4 => Some(MangaStatus::Dropped),
            5 => Some(MangaStatus::PlanToRead),
            6 => Some(MangaStatus::Rereading),
            _ => None,
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            MangaStatus::Reading => 1,
            MangaStatus::Completed => 2,
            MangaStatus::OnHold => 3,
            MangaStatus::Dropped => 4,
            MangaStatus::PlanToRead => 5,
            MangaStatus::Rereading => 6,
        }
    }

    pub fn to_api(self) -> ApiStatus {
        match self {
            MangaStatus::Reading => ApiStatus::Current,
            MangaStatus::Completed => ApiStatus::Completed,
            MangaStatus::OnHold => ApiStatus::Paused,
            MangaStatus::Dropped => ApiStatus::Dropped,
            MangaStatus::PlanToRead => ApiStatus::Planning,
            MangaStatus::Rereading => ApiStatus::Repeating,
        }
    }

    pub fn from_api(a: ApiStatus) -> Self {
        match a {
            ApiStatus::Current => MangaStatus::Reading,
            ApiStatus::Completed => MangaStatus::Completed,
            ApiStatus::Paused => MangaStatus::OnHold,
            ApiStatus::Dropped => MangaStatus::Dropped,
            ApiStatus::Planning => MangaStatus::PlanToRead,
            ApiStatus::Repeating => MangaStatus::Rereading,
        }
    }

    /// i18n message key of the label.
    pub fn label_key(self) -> &'static str {
        match self {
            MangaStatus::Reading => "reading",
            MangaStatus::Completed => "completed",
            MangaStatus::OnHold => "paused",
            MangaStatus::Dropped => "dropped",
            MangaStatus::PlanToRead => "plan_to_read",
            MangaStatus::Rereading => "repeating",
        }
    }

    /// English text of the label, equal to the base strings of the i18n catalogue.
    pub fn label_en(self) -> &'static str {
        match self {
            MangaStatus::Reading => "Reading",
            MangaStatus::Completed => "Completed",
            MangaStatus::OnHold => "Paused",
            MangaStatus::Dropped => "Dropped",
            MangaStatus::PlanToRead => "Plan to read",
            MangaStatus::Rereading => "Rereading",
        }
    }
}

/// Anime statuses. Stored integers: 2, 3, 4, 11, 15, 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimeStatus {
    Watching,
    Completed,
    OnHold,
    Dropped,
    PlanToWatch,
    Rewatching,
}

impl AnimeStatus {
    /// Picker order shown to the user.
    pub const ORDER: [AnimeStatus; 6] = [
        AnimeStatus::Watching,
        AnimeStatus::PlanToWatch,
        AnimeStatus::Completed,
        AnimeStatus::Rewatching,
        AnimeStatus::OnHold,
        AnimeStatus::Dropped,
    ];

    /// `None` for any integer that is not an anime status (for example 1, a manga status).
    pub fn from_db(v: i64) -> Option<Self> {
        match v {
            2 => Some(AnimeStatus::Completed),
            3 => Some(AnimeStatus::OnHold),
            4 => Some(AnimeStatus::Dropped),
            11 => Some(AnimeStatus::Watching),
            15 => Some(AnimeStatus::PlanToWatch),
            16 => Some(AnimeStatus::Rewatching),
            _ => None,
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            AnimeStatus::Watching => 11,
            AnimeStatus::Completed => 2,
            AnimeStatus::OnHold => 3,
            AnimeStatus::Dropped => 4,
            AnimeStatus::PlanToWatch => 15,
            AnimeStatus::Rewatching => 16,
        }
    }

    pub fn to_api(self) -> ApiStatus {
        match self {
            AnimeStatus::Watching => ApiStatus::Current,
            AnimeStatus::Completed => ApiStatus::Completed,
            AnimeStatus::OnHold => ApiStatus::Paused,
            AnimeStatus::Dropped => ApiStatus::Dropped,
            AnimeStatus::PlanToWatch => ApiStatus::Planning,
            AnimeStatus::Rewatching => ApiStatus::Repeating,
        }
    }

    pub fn from_api(a: ApiStatus) -> Self {
        match a {
            ApiStatus::Current => AnimeStatus::Watching,
            ApiStatus::Completed => AnimeStatus::Completed,
            ApiStatus::Paused => AnimeStatus::OnHold,
            ApiStatus::Dropped => AnimeStatus::Dropped,
            ApiStatus::Planning => AnimeStatus::PlanToWatch,
            ApiStatus::Repeating => AnimeStatus::Rewatching,
        }
    }

    /// i18n message key of the label.
    pub fn label_key(self) -> &'static str {
        match self {
            AnimeStatus::Watching => "watching",
            AnimeStatus::Completed => "completed",
            AnimeStatus::OnHold => "paused",
            AnimeStatus::Dropped => "dropped",
            AnimeStatus::PlanToWatch => "plan_to_watch",
            AnimeStatus::Rewatching => "repeating_anime",
        }
    }

    /// English text of the label, equal to the base strings of the i18n catalogue.
    pub fn label_en(self) -> &'static str {
        match self {
            AnimeStatus::Watching => "Watching",
            AnimeStatus::Completed => "Completed",
            AnimeStatus::OnHold => "Paused",
            AnimeStatus::Dropped => "Dropped",
            AnimeStatus::PlanToWatch => "Plan to watch",
            AnimeStatus::Rewatching => "Rewatching",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn t_28a_003_stored_integers_round_trip_and_foreign_values_are_none() {
        for s in MangaStatus::ORDER {
            assert_eq!(MangaStatus::from_db(s.to_db()), Some(s));
        }
        for s in AnimeStatus::ORDER {
            assert_eq!(AnimeStatus::from_db(s.to_db()), Some(s));
        }
        assert_eq!(MangaStatus::from_db(11), None, "E-63");
        assert_eq!(AnimeStatus::from_db(1), None, "E-63");
        assert_eq!(MangaStatus::from_db(7), None);
        assert_eq!(AnimeStatus::from_db(5), None);
        let stored: Vec<i64> = [
            MangaStatus::Reading,
            MangaStatus::Completed,
            MangaStatus::OnHold,
            MangaStatus::Dropped,
            MangaStatus::PlanToRead,
            MangaStatus::Rereading,
        ]
        .iter()
        .map(|s| s.to_db())
        .collect();
        assert_eq!(stored, vec![1, 2, 3, 4, 5, 6]);
        let anime: Vec<i64> = [
            AnimeStatus::Completed,
            AnimeStatus::OnHold,
            AnimeStatus::Dropped,
            AnimeStatus::Watching,
            AnimeStatus::PlanToWatch,
            AnimeStatus::Rewatching,
        ]
        .iter()
        .map(|s| s.to_db())
        .collect();
        assert_eq!(anime, vec![2, 3, 4, 11, 15, 16]);
    }

    #[test]
    fn t_28a_004_option_lists_have_six_entries_and_exact_labels() {
        assert_eq!(MangaStatus::ORDER.len(), 6);
        assert_eq!(AnimeStatus::ORDER.len(), 6);
        let manga: Vec<&str> = MangaStatus::ORDER.iter().map(|s| s.label_en()).collect();
        assert_eq!(
            manga,
            [
                "Reading",
                "Plan to read",
                "Completed",
                "Rereading",
                "Paused",
                "Dropped"
            ]
        );
        let anime: Vec<&str> = AnimeStatus::ORDER.iter().map(|s| s.label_en()).collect();
        assert_eq!(
            anime,
            [
                "Watching",
                "Plan to watch",
                "Completed",
                "Rewatching",
                "Paused",
                "Dropped"
            ]
        );
        assert_eq!(MangaStatus::Rereading.label_key(), "repeating");
        assert_eq!(AnimeStatus::Rewatching.label_key(), "repeating_anime");
    }

    #[test]
    fn t_28a_007_all_twelve_pairs_map_both_ways() -> TestResult {
        let manga = [
            (MangaStatus::Reading, "CURRENT"),
            (MangaStatus::Completed, "COMPLETED"),
            (MangaStatus::OnHold, "PAUSED"),
            (MangaStatus::Dropped, "DROPPED"),
            (MangaStatus::PlanToRead, "PLANNING"),
            (MangaStatus::Rereading, "REPEATING"),
        ];
        for (local, api) in manga {
            assert_eq!(local.to_api().as_str(), api);
            assert_eq!(MangaStatus::from_api(ApiStatus::from_api_str(api)?), local);
        }
        let anime = [
            (AnimeStatus::Watching, "CURRENT"),
            (AnimeStatus::Completed, "COMPLETED"),
            (AnimeStatus::OnHold, "PAUSED"),
            (AnimeStatus::Dropped, "DROPPED"),
            (AnimeStatus::PlanToWatch, "PLANNING"),
            (AnimeStatus::Rewatching, "REPEATING"),
        ];
        for (local, api) in anime {
            assert_eq!(local.to_api().as_str(), api);
            assert_eq!(AnimeStatus::from_api(ApiStatus::from_api_str(api)?), local);
        }
        Ok(())
    }

    #[test]
    fn t_28a_007_unknown_api_string_is_an_error_not_a_panic() {
        assert!(matches!(
            ApiStatus::from_api_str("FOO"),
            Err(AniListError::UnknownStatus(s)) if s == "FOO"
        ));
    }
}
