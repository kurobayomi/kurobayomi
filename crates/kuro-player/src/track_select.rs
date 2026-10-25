//! Default audio and subtitle track choice (area 45 `TrackSelect.kt`, behaviour 5.3, T-45b-017).
//! Pure: the caller passes the track list and the preference strings and writes the returned id to
//! mpv. Preference lists are trimmed before empty tokens are dropped (E-21), and a language matches
//! on whole tags only (E-22).
//!
//! DECISION: display names come from `kuro_i18n::names` (the committed table; no `icu_displaynames`
//! 2.x release exists). A language matches when the track's `lang` tag equals the preference tag, or
//! when the track name contains the tag, or its English or native display name, as a whole word.
//! A tag without a table row has the tag itself as its name, so the tag fallback is kept. The
//! system locale is passed in by the caller as a plain tag.

/// One track as `track-list` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackCandidate {
    pub id: i64,
    pub name: String,
    pub lang: Option<String>,
}

/// Splits a comma list, trims each token and drops the empty ones.
pub fn tokens(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether the track is in the language. The name must contain the tag or a display name of it as
/// whole words, or the `lang` tag must equal the tag, ignoring case.
fn contains_lang(track: &TrackCandidate, tag: &str) -> bool {
    let name = track.name.to_lowercase();
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has_phrase = |phrase: &str| {
        let wanted: Vec<String> = phrase
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect();
        !wanted.is_empty()
            && words
                .windows(wanted.len())
                .any(|window| window.iter().zip(&wanted).all(|(a, b)| a == b))
    };
    let word_in_name = has_phrase(tag)
        || has_phrase(&kuro_i18n::names::english_name(tag))
        || has_phrase(&kuro_i18n::names::native_name(tag));
    let lang_matches = track
        .lang
        .as_deref()
        .is_some_and(|l| l.eq_ignore_ascii_case(tag));
    word_in_name || lang_matches
}

/// `TrackSelect.getPreferredTrackIndex`. `langs` is the preferred-language list in order. An empty
/// list uses `system_lang`. The first preferred language with a track wins. Blacklisted names are
/// dropped, a whitelisted name wins, and otherwise the first remaining track is used. Audio passes
/// empty `whitelist` and `blacklist`.
pub fn preferred_track(
    tracks: &[TrackCandidate],
    langs: &str,
    whitelist: &str,
    blacklist: &str,
    system_lang: &str,
) -> Option<i64> {
    let blacklist = tokens(blacklist);
    let whitelist = tokens(whitelist);
    let mut locales = tokens(langs);
    if locales.is_empty() {
        locales.push(system_lang.to_owned());
    }
    let chosen = locales
        .iter()
        .find(|tag| tracks.iter().any(|t| contains_lang(t, tag)))?;
    let filtered: Vec<&TrackCandidate> = tracks
        .iter()
        .filter(|t| {
            !blacklist
                .iter()
                .any(|b| t.name.to_lowercase().contains(&b.to_lowercase()))
        })
        .filter(|t| contains_lang(t, chosen))
        .collect();
    filtered
        .iter()
        .find(|t| {
            whitelist
                .iter()
                .any(|w| t.name.to_lowercase().contains(&w.to_lowercase()))
        })
        .or(filtered.first())
        .map(|t| t.id)
}

/// The preference strings `TrackSelect` reads. The subtitle list uses the whitelist and the
/// blacklist; the audio list never does (behaviour 5.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackPrefs {
    pub subtitle_langs: String,
    pub subtitle_whitelist: String,
    pub subtitle_blacklist: String,
    pub audio_langs: String,
    /// The system locale tag, used when a language list is empty.
    pub system_lang: String,
}

/// The `prefer` seam of `tracks::default_tracks`: the index in `tracks` of the preferred track.
/// Audio is chosen with an empty whitelist and blacklist (T21).
pub fn prefer_index(
    tracks: &[crate::tracks::VideoTrack],
    subtitle: bool,
    prefs: &TrackPrefs,
) -> Option<usize> {
    let candidates: Vec<TrackCandidate> = tracks
        .iter()
        .map(|t| TrackCandidate {
            id: t.id,
            name: t.name.clone(),
            lang: t.language.clone().filter(|l| !l.is_empty()),
        })
        .collect();
    let (langs, whitelist, blacklist) = if subtitle {
        (
            &prefs.subtitle_langs,
            &prefs.subtitle_whitelist,
            &prefs.subtitle_blacklist,
        )
    } else {
        (&prefs.audio_langs, &String::new(), &String::new())
    };
    let id = preferred_track(&candidates, langs, whitelist, blacklist, &prefs.system_lang)?;
    tracks.iter().position(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: i64, name: &str, lang: Option<&str>) -> TrackCandidate {
        TrackCandidate {
            id,
            name: name.to_owned(),
            lang: lang.map(str::to_owned),
        }
    }

    #[test]
    fn blank_tokens_are_trimmed_and_dropped() {
        assert_eq!(
            tokens("en, ,  fr ,"),
            vec!["en".to_owned(), "fr".to_owned()]
        );
        assert!(tokens(" , ").is_empty());
    }

    #[test]
    fn t14_first_preferred_language_with_a_track_wins() {
        let tracks = [
            track(1, "Japanese", Some("ja")),
            track(2, "English", Some("en")),
        ];
        assert_eq!(
            preferred_track(&tracks, "de, en, ja", "", "", "fr"),
            Some(2)
        );
    }

    #[test]
    fn t17_empty_preference_uses_the_system_language() {
        let tracks = [
            track(1, "Japanese", Some("ja")),
            track(2, "English", Some("en")),
        ];
        assert_eq!(preferred_track(&tracks, "", "", "", "ja"), Some(1));
    }

    #[test]
    fn t19_language_matches_whole_tags_only() {
        // "xen" must not match the tag "en" (E-22).
        let tracks = [track(1, "Xenon commentary", Some("xen"))];
        assert_eq!(preferred_track(&tracks, "en", "", "", "en"), None);
    }

    #[test]
    fn display_names_match_a_track_without_a_lang_tag() {
        let tracks = [track(1, "Deutsch", None), track(2, "English", None)];
        assert_eq!(preferred_track(&tracks, "en", "", "", "fr"), Some(2));
        assert_eq!(preferred_track(&tracks, "de", "", "", "fr"), Some(1));
        let pt = [track(1, "Portuguese (Brazil) dub", None)];
        assert_eq!(preferred_track(&pt, "pt-BR", "", "", "fr"), Some(1));
    }

    #[test]
    fn unknown_tag_falls_back_to_the_tag_itself() {
        let tracks = [track(1, "tlh commentary", None)];
        assert_eq!(preferred_track(&tracks, "tlh", "", "", "en"), Some(1));
    }

    #[test]
    fn t20_no_match_gives_none() {
        let tracks = [track(1, "Japanese", Some("ja"))];
        assert_eq!(preferred_track(&tracks, "de", "", "", "en"), None);
    }
}
