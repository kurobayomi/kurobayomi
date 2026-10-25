//! Tracking preferences over the shared `kuro-prefs` store (`TrackPreferences.kt`). Keys and defaults
//! are in `kuro-prefs::registry`. Credentials and tokens are not here; they live in the secret store.

use kuro_prefs::pref::Pref;
use kuro_prefs::store::PrefStore;

use crate::policy::AutoTrackState;
use kuro_prefs::pref_enum;

pref_enum! {
    /// Score scale of the AniList account (`anilist_score_type`). Stored by name; unknown text reads as
    /// `POINT_10`.
    pub enum AniListScoreType { POINT_100, POINT_10_DECIMAL, POINT_10, POINT_5, POINT_3 }
}

/// How a settings row is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Switch,
    /// Opens the list of [`AutoTrackState`] options.
    MarkReadList,
}

/// One row of the top group of Settings > Tracking. `title` is the display text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefRow {
    pub key: &'static str,
    pub title: &'static str,
    pub kind: RowKind,
}

/// The four top rows, in order (behaviour 12.2). The title of the airing row is trimmed for display.
pub const TOP_ROWS: [PrefRow; 4] = [
    PrefRow {
        key: "pref_auto_update_manga_sync_key",
        title: "Update progress after reading/watching",
        kind: RowKind::Switch,
    },
    PrefRow {
        key: "track_on_adding_to_library",
        title: "Open track menu on adding to library",
        kind: RowKind::Switch,
    },
    PrefRow {
        key: "show_next_episode_airing_time",
        title: "Show next episode's airing time",
        kind: RowKind::Switch,
    },
    PrefRow {
        key: "pref_auto_update_manga_on_mark_read",
        title: "Update progress when marked as read/watched",
        kind: RowKind::MarkReadList,
    },
];

/// The key of [`TrackPrefs::bangumi_episode_progress`].
pub const BANGUMI_EPISODE_PROGRESS_KEY: &str = "bangumi_episode_progress";

/// The key of [`TrackPrefs::auto_track`].
pub const AUTO_TRACK_KEY: &str = "auto_track_library";

/// Typed handles over the tracking keys.
pub struct TrackPrefs {
    /// `pref_auto_update_manga_sync_key`, shared by anime and manga.
    pub auto_update_track: Pref<bool>,
    pub track_on_adding_to_library: Pref<bool>,
    pub show_next_episode_airing_time: Pref<bool>,
    pub auto_update_on_mark_read: Pref<AutoTrackState>,
    pub anilist_score_type: Pref<AniListScoreType>,
    /// Bangumi anime progress through the episode endpoints (T-31a-044, D7). Default off. Not an Aniyomi key, so it
    /// is stored under `[extra]` and exported with the `kuro_` prefix.
    pub bangumi_episode_progress: Pref<bool>,
    /// Auto-track: a title that joins the library is bound on AniList when its match is certain (`auto_bind`). Default
    /// off. Not an Aniyomi key, so it is stored under `[extra]` and exported with the `kuro_` prefix.
    pub auto_track: Pref<bool>,
}

impl TrackPrefs {
    pub fn new(store: PrefStore) -> Self {
        Self {
            auto_update_track: Pref::new(store.clone(), "pref_auto_update_manga_sync_key", true),
            track_on_adding_to_library: Pref::new(
                store.clone(),
                "track_on_adding_to_library",
                true,
            ),
            show_next_episode_airing_time: Pref::new(
                store.clone(),
                "show_next_episode_airing_time",
                true,
            ),
            auto_update_on_mark_read: Pref::new(
                store.clone(),
                "pref_auto_update_manga_on_mark_read",
                AutoTrackState::ALWAYS,
            ),
            anilist_score_type: Pref::new(
                store.clone(),
                "anilist_score_type",
                AniListScoreType::POINT_10,
            ),
            bangumi_episode_progress: Pref::new(store.clone(), BANGUMI_EPISODE_PROGRESS_KEY, false),
            auto_track: Pref::new(store, AUTO_TRACK_KEY, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use kuro_prefs::store::PrefStore;
    use kuro_prefs::values::PrefValue;

    use super::*;

    #[test]
    fn defaults_match_the_source() {
        let p = TrackPrefs::new(PrefStore::in_memory());
        assert!(p.auto_update_track.get());
        assert!(p.track_on_adding_to_library.get());
        assert!(p.show_next_episode_airing_time.get());
        assert_eq!(p.auto_update_on_mark_read.get(), AutoTrackState::ALWAYS);
        assert_eq!(p.anilist_score_type.get(), AniListScoreType::POINT_10);
        assert!(
            !p.bangumi_episode_progress.get(),
            "T-31a-044: off by default"
        );
        assert!(!p.auto_track.get(), "auto-track is off until the user turns it on");
    }

    #[test]
    fn tr_27_043_top_rows_are_in_order_with_the_stored_keys() {
        let keys: Vec<&str> = TOP_ROWS.iter().map(|r| r.key).collect();
        assert_eq!(
            keys,
            vec![
                "pref_auto_update_manga_sync_key",
                "track_on_adding_to_library",
                "show_next_episode_airing_time",
                "pref_auto_update_manga_on_mark_read",
            ]
        );
        assert!(TOP_ROWS.iter().all(|r| r.title == r.title.trim()));
        assert_eq!(TOP_ROWS[3].kind, RowKind::MarkReadList);
    }

    #[test]
    fn unknown_stored_state_reads_as_always() {
        let store = PrefStore::in_memory();
        store.set_raw(
            "pref_auto_update_manga_on_mark_read",
            PrefValue::Str("BOGUS".to_owned()),
        );
        let p = TrackPrefs::new(store);
        assert_eq!(p.auto_update_on_mark_read.get(), AutoTrackState::ALWAYS);
    }

    #[test]
    fn tr_27_062_anilist_score_type_reads_stored_name_and_falls_back_on_bad_value() {
        let store = PrefStore::in_memory();
        store.set_raw("anilist_score_type", PrefValue::Str("POINT_5".to_owned()));
        assert_eq!(
            TrackPrefs::new(store.clone()).anilist_score_type.get(),
            AniListScoreType::POINT_5
        );
        store.set_raw(
            "anilist_score_type",
            PrefValue::Str("NOT_A_SCALE".to_owned()),
        );
        assert_eq!(
            TrackPrefs::new(store).anilist_score_type.get(),
            AniListScoreType::POINT_10
        );
    }
}
