//! `[player]` preferences (Aniyomi `PlayerPreferences`) and the
//! "mark as seen" rule.
//!
//! The settings diet kept the player keys here: the volume at start (and the last one used), the autoplay switch and the
//! default quality; [`AnimePlayerPrefs`] holds the track languages of one anime. The gesture, decoder, aspect,
//! speed, subtitle, advanced player and detach-window keys were cut: their hard-coded defaults are read where they
//! are used, and the keys are in `registry::DROPPED_KEYS`.

use crate::pref::Pref;
use crate::registry::define_prefs;
use crate::store::PrefStore;

/// Whether finishing playback at `last_second_seen` marks the episode as seen (`PlayerViewModel`).
///
/// True when the episode is unseen, `incognito` is off, the total length is known (greater than zero),
/// and the position has reached `progress` of the total.
pub fn should_mark_seen(last_second_seen: i64, total_seconds: i64, progress: f32, already_seen: bool, incognito: bool) -> bool {
    !already_seen
        && !incognito
        && total_seconds > 0
        && last_second_seen as f32 >= total_seconds as f32 * progress
}

define_prefs! {
    /// `[player]` section: the player keys the settings diet kept.
    pub struct PlayerPrefs in "player" {
        /// The volume a video starts at: a level from 0 to 100, or -1 for the last one used (`player_volume_last`).
        player_volume_value: f32 = "player_volume_value" => -1.0f32;
        /// The last volume set in the player, kept apart so a chosen start level is never overwritten.
        player_volume_last: f32 = "player_volume_last" => -1.0f32;
        autoplay_enabled: bool = "pref_auto_play_enabled" => false;
        /// The video height an episode starts at when its servers offer it (user, 2026-10-15: 1080p by default).
        default_quality: i32 = "pref_player_default_quality" => 1080;
    }
}

/// What the player keeps for one anime (user, 2026-11-02): the language of a track picked in it, the quality picked in it,
/// and whether its openings and endings are skipped by themselves, so its next episodes start the same way. A key never
/// written reads as the Settings value as it is now (`pref_audio_lang`, `pref_subtitle_lang`, the default quality); the
/// skips are off until turned on for the anime.
#[derive(Clone)]
pub struct AnimePlayerPrefs {
    store: PrefStore,
    anime_id: i64,
}

impl AnimePlayerPrefs {
    pub fn new(store: PrefStore, anime_id: i64) -> Self {
        Self { store, anime_id }
    }

    pub fn audio_lang(&self) -> Pref<String> {
        self.own("anime_audio_lang", "pref_audio_lang")
    }

    pub fn subtitle_lang(&self) -> Pref<String> {
        self.own("anime_subtitle_lang", "pref_subtitle_lang")
    }

    /// The video height picked for the anime; the Settings default quality until one is picked.
    pub fn quality(&self) -> Pref<i32> {
        let settings = PlayerPrefs::new(self.store.clone()).default_quality.get();
        self.store.pref(format!("anime_quality_{}", self.anime_id), settings)
    }

    /// Openings of this anime are skipped by themselves (after a short countdown that can be cancelled).
    pub fn auto_skip_opening(&self) -> Pref<bool> {
        self.store.pref(format!("anime_auto_skip_opening_{}", self.anime_id), false)
    }

    /// Endings of this anime are skipped by themselves (after a short countdown that can be cancelled).
    pub fn auto_skip_ending(&self) -> Pref<bool> {
        self.store.pref(format!("anime_auto_skip_ending_{}", self.anime_id), false)
    }

    /// The anime's key `<prefix>_<id>`, read fresh each time so an unset key follows the Settings language.
    fn own(&self, prefix: &str, settings_key: &'static str) -> Pref<String> {
        let settings = self.store.pref(settings_key, String::new()).get();
        self.store.pref(format!("{prefix}_{}", self.anime_id), settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::lookup;
    use crate::store::PrefStore;
    use crate::testutil::assert_section_registered;

    #[test]
    fn every_player_section_matches_its_registry_rows() {
        let store = PrefStore::in_memory();
        let player = PlayerPrefs::new(store.clone());
        assert_eq!(PlayerPrefs::FIELDS.len(), 4);
        assert_section_registered(PlayerPrefs::TABLE, PlayerPrefs::FIELDS, &player.defaults());
    }

    #[test]
    fn an_anime_follows_the_settings_languages_until_a_track_is_picked_for_it() {
        let store = PrefStore::in_memory();
        let (one, two) = (AnimePlayerPrefs::new(store.clone(), 1), AnimePlayerPrefs::new(store.clone(), 2));
        store.pref("pref_subtitle_lang", String::new()).set("en".to_owned()).unwrap();
        assert_eq!((one.subtitle_lang().get(), two.subtitle_lang().get()), ("en".to_owned(), "en".to_owned()));
        one.subtitle_lang().set("ja".to_owned()).unwrap();
        one.audio_lang().set("ja".to_owned()).unwrap();
        assert_eq!((one.subtitle_lang().get(), one.audio_lang().get()), ("ja".to_owned(), "ja".to_owned()), "the picks of anime 1");
        assert_eq!((two.subtitle_lang().get(), two.audio_lang().get()), ("en".to_owned(), String::new()), "anime 2 keeps the Settings");
        assert_eq!(one.quality().get(), 1080, "the Settings default until one is picked");
        one.quality().set(720).unwrap();
        one.auto_skip_opening().set(true).unwrap();
        assert_eq!((one.quality().get(), two.quality().get()), (720, 1080));
        assert!(one.auto_skip_opening().get() && !two.auto_skip_opening().get() && !one.auto_skip_ending().get());
        for key in [one.subtitle_lang().key().to_owned(), one.audio_lang().key().to_owned(), one.quality().key().to_owned(), one.auto_skip_opening().key().to_owned(), one.auto_skip_ending().key().to_owned()] {
            assert_eq!(lookup(&key).and_then(|(_, id)| id), Some("1"), "{key} is a registered family");
        }
    }

    #[test]
    fn plan_defaults_hold_on_an_empty_store() {
        let p = PlayerPrefs::new(PrefStore::in_memory());
        assert_eq!(p.player_volume_value.get(), -1.0);
        assert!(!p.autoplay_enabled.get());
        assert_eq!(p.default_quality.get(), 1080);
    }

    #[test]
    fn cut_player_keys_are_dropped_not_registered() {
        for key in [
            "pref_default_player_orientation_type_key",
            "pref_gesture_volume_brightness",
            "pref_swap_volume_and_brightness",
            "pref_remember_brightness",
            "player_brightness_value",
            "pref_mpv_conf",
            "pref_mpv_input",
            "mpv_scripts",
            "player_open_detached",
        ] {
            assert!(lookup(key).is_none(), "{key} is registered");
            assert!(crate::registry::is_dropped(key), "{key} is not in DROPPED_KEYS");
        }
    }

    #[test]
    fn mark_seen_needs_a_known_length_an_unseen_episode_and_no_incognito() {
        assert!(should_mark_seen(850, 1000, 0.85, false, false));
        assert!(!should_mark_seen(849, 1000, 0.85, false, false));
        assert!(!should_mark_seen(1000, 0, 0.85, false, false));
        assert!(!should_mark_seen(1000, 1000, 0.85, true, false));
        assert!(!should_mark_seen(1000, 1000, 0.85, false, true));
    }
}
