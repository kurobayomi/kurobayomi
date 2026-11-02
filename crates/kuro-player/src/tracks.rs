//! Track selection values for `aid`, `sid` and `secondary-sid` (area 45, T-45a-015). mpv writes
//! `no` for a disabled track and `auto` while it chooses one; the app uses `-1` for disabled.

use std::path::Path;

use crate::types::Track;

/// Track id meaning "disabled" (`no` in mpv).
pub const TRACK_NONE: i64 = -1;

/// Reads a track property. `no`, a missing value and a non-integer all give `TRACK_NONE`.
pub fn track_id_from_value(value: Option<&str>) -> i64 {
    value
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(TRACK_NONE)
}

/// Reads a track change event. `auto` is ignored (`None`), so no selection update happens.
pub fn track_change(value: &str) -> Option<i64> {
    match value {
        "auto" => None,
        other => Some(track_id_from_value(Some(other))),
    }
}

/// What to write for a track id: `no` for `TRACK_NONE`, the integer otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackValue {
    No,
    Id(i64),
}

impl TrackValue {
    pub fn for_id(id: i64) -> Self {
        if id == TRACK_NONE {
            Self::No
        } else {
            Self::Id(id)
        }
    }
}

/// `VideoTrack(id, name, language)` in the player's track lists (T-45b-016). The "Off" entry has
/// no language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoTrack {
    pub id: i64,
    pub name: String,
    pub language: Option<String>,
}

/// Read access to mpv's `track-list/*` properties (D§7.1). `count` is `None` when the mpv handle
/// is gone (the Kotlin NPE, E-42); a missing count is `Some(0)`.
pub trait TrackProps {
    fn count(&self) -> Option<i64>;
    fn kind(&self, index: i64) -> Option<String>;
    fn lang(&self, index: i64) -> Option<String>;
    fn title(&self, index: i64) -> Option<String>;
    fn id(&self, index: i64) -> Option<i64>;
}

/// The audio and subtitle lists of the loaded file. The audio list starts with "Off".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackLists {
    pub subtitles: Vec<VideoTrack>,
    pub audio: Vec<VideoTrack>,
}

/// `loadTracks()` (behaviour 5.1). A track with an unknown type, or without an id, is skipped.
/// A gone mpv handle clears both lists.
pub fn load_track_lists(props: &impl TrackProps) -> TrackLists {
    let Some(count) = props.count() else {
        return TrackLists {
            subtitles: Vec::new(),
            audio: Vec::new(),
        };
    };
    let mut lists = TrackLists {
        subtitles: Vec::new(),
        audio: vec![VideoTrack {
            id: TRACK_NONE,
            name: kuro_i18n::tr(kuro_i18n::MsgKey::off),
            language: None,
        }],
    };
    for index in 0..count {
        let Some(kind) = props.kind(index) else {
            continue;
        };
        let Some(id) = props.id(index) else {
            continue;
        };
        let track = VideoTrack {
            id,
            name: props.title(index).unwrap_or_default(),
            language: Some(props.lang(index).unwrap_or_default()),
        };
        match kind.as_str() {
            "sub" => lists.subtitles.push(track),
            "audio" => lists.audio.push(track),
            _ => {}
        }
    }
    lists
}

/// The default tracks chosen after load (`onFinishLoadingTracks`, behaviour 5.2). `None` means
/// that property is not written. `secondary_sid` is only written together with `sid`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultTracks {
    pub sid: Option<i64>,
    pub secondary_sid: Option<i64>,
    pub aid: Option<i64>,
}

/// Subtitle choice for `id` (`selectSub`, behaviour 5.6). `current` is `(sid, secondary-sid)` and the
/// result is the new pair. Picking the primary promotes the secondary, picking the secondary clears
/// it, a new id becomes the secondary while a primary exists and the primary otherwise.
pub fn select_sub(current: (i64, i64), id: i64) -> (i64, i64) {
    let (primary, secondary) = current;
    if id == primary {
        (secondary, TRACK_NONE)
    } else if id == secondary {
        (primary, TRACK_NONE)
    } else if primary != TRACK_NONE {
        (primary, id)
    } else {
        (id, TRACK_NONE)
    }
}

/// Footer of the subtitle sheet, shown under the list (T-46b-012).
pub const SECONDARY_FOOTER: &str = "Secondary subtitles will have no ASS/SSA Styling.";

/// The row label of a track (`getTrackTitle`, behaviour 1.4). Off (`-1`) is its name verbatim.
/// Otherwise: `#id: name` when there is no language, `#id: name (lang)` with both, `#id: lang` with
/// only a language, and `#id: ` when both are blank. Blank means empty or whitespace.
pub fn track_label(id: i64, name: &str, language: Option<&str>) -> String {
    if id == TRACK_NONE {
        return name.to_owned();
    }
    use kuro_i18n::{Arg, MsgKey, trf};
    let has_name = !name.trim().is_empty();
    let lang = language.filter(|l| !l.trim().is_empty());
    let number = ("arg1", Arg::Int(id));
    match (lang, has_name) {
        (Some(lang), true) => trf(MsgKey::player_sheets_track_title_w_lang, &[number, ("arg2", Arg::Str(name)), ("arg3", Arg::Str(lang))]),
        (Some(lang), false) => trf(MsgKey::player_sheets_track_lang_wo_title, &[number, ("arg2", Arg::Str(lang))]),
        (None, _) => trf(MsgKey::player_sheets_track_title_wo_lang, &[number, ("arg2", Arg::Str(if has_name { name } else { "" }))]),
    }
}

/// The slot badge of a checked row in the two-slot subtitle list: `#1` for the primary, `#2` for
/// the secondary, `None` for an unselected track (T-46b-012).
pub fn slot_label(current: (i64, i64), id: i64) -> Option<&'static str> {
    let (primary, secondary) = current;
    if id != TRACK_NONE && id == primary {
        Some("#1")
    } else if id != TRACK_NONE && id == secondary {
        Some("#2")
    } else {
        None
    }
}

/// Picks the default tracks. `prefer` is the TrackSelect seam: given a list and whether it is the
/// subtitle list, it returns the index of the preferred track. Subtitles fall back to the first
/// track. Audio falls back to index 1, the first real track after "Off".
pub fn default_tracks(
    lists: &TrackLists,
    prefer: impl Fn(&[VideoTrack], bool) -> Option<usize>,
) -> DefaultTracks {
    let mut out = DefaultTracks::default();
    let subtitle_index =
        prefer(&lists.subtitles, true).or((!lists.subtitles.is_empty()).then_some(0));
    if let Some(track) = subtitle_index.and_then(|i| lists.subtitles.get(i)) {
        out.sid = Some(track.id);
        out.secondary_sid = Some(TRACK_NONE);
    }
    let audio_index = prefer(&lists.audio, false).or((lists.audio.len() > 1).then_some(1));
    out.aid = audio_index
        .and_then(|i| lists.audio.get(i))
        .map(|track| track.id);
    out
}

/// The commands that add a video's side tracks (T-45a-016), in Kotlin order: every audio track,
/// then every subtitle track. `finish_now` is set when both lists are empty: nothing is loaded, so
/// the caller finishes track loading at once (`onFinishLoadingTracks`, T-45a-084).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalTracks {
    pub commands: Vec<Vec<String>>,
    pub finish_now: bool,
}

pub fn external_track_commands(audio: &[Track], subtitles: &[Track]) -> ExternalTracks {
    let add = |command: &str, track: &Track| {
        vec![
            command.to_owned(),
            track.url.clone(),
            "auto".to_owned(),
            track.lang.clone(),
        ]
    };
    let commands = audio
        .iter()
        .map(|track| add("audio-add", track))
        .chain(subtitles.iter().map(|track| add("sub-add", track)))
        .collect();
    ExternalTracks {
        commands,
        finish_now: audio.is_empty() && subtitles.is_empty(),
    }
}

/// The kind of track a user adds from a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddKind {
    Audio,
    Subtitle,
}

/// The command that adds a track from a file (D section 7.3, behaviour 5.4): `audio-add` or
/// `sub-add`, the path, `cached`, and the title when there is one.
pub fn add_track_args(kind: AddKind, path: &str, title: Option<&str>) -> Vec<String> {
    let command = match kind {
        AddKind::Audio => "audio-add",
        AddKind::Subtitle => "sub-add",
    };
    let mut args = vec![command.to_owned(), path.to_owned(), "cached".to_owned()];
    args.extend(title.map(str::to_owned));
    args
}

/// The command for a file the user picked (T-45b-072): the file name is the title. `None` when the
/// path is not valid text, so no command is sent.
pub fn add_file_args(kind: AddKind, path: &Path) -> Option<Vec<String>> {
    let text = path.to_str()?;
    let title = path.file_name().and_then(|name| name.to_str());
    Some(add_track_args(kind, text, title))
}

/// The command for the result of the file dialog. A cancelled pick (`None`) sends nothing
/// (T-46a-027). The dialog has no filter, so any file is accepted.
pub fn picked_file_args(kind: AddKind, picked: Option<&Path>) -> Option<Vec<String>> {
    add_file_args(kind, picked?)
}

#[cfg(test)]
mod label_tests {
    use super::*;

    #[test]
    fn a_cancelled_pick_sends_nothing_and_a_pick_adds_the_file() {
        assert_eq!(picked_file_args(AddKind::Subtitle, None), None);
        assert_eq!(
            picked_file_args(AddKind::Audio, Some(Path::new("/a/b.mka"))),
            Some(vec![
                "audio-add".into(),
                "/a/b.mka".into(),
                "cached".into(),
                "b.mka".into()
            ])
        );
    }

    #[test]
    fn off_is_the_name_verbatim() {
        assert_eq!(track_label(-1, "Off", None), "Off");
    }

    #[test]
    fn the_label_rule_table() {
        assert_eq!(track_label(1, "Japanese", None), "#1: Japanese");
        assert_eq!(
            track_label(2, "Commentary", Some("eng")),
            "#2: Commentary (eng)"
        );
        assert_eq!(track_label(3, "", Some("eng")), "#3: eng");
        assert_eq!(track_label(4, "  ", Some(" ")), "#4: ");
        assert_eq!(track_label(5, "", None), "#5: ");
    }

    #[test]
    fn a_blank_language_counts_as_no_language() {
        assert_eq!(track_label(6, "Dub", Some("  ")), "#6: Dub");
    }

    #[test]
    fn slots_mark_the_primary_then_the_secondary() {
        assert_eq!(slot_label((3, 7), 3), Some("#1"));
        assert_eq!(slot_label((3, 7), 7), Some("#2"));
        assert_eq!(slot_label((3, 7), 9), None);
        assert_eq!(slot_label((-1, -1), -1), None);
    }

    #[test]
    fn the_subtitle_footer_text_is_exact() {
        assert_eq!(
            SECONDARY_FOOTER,
            "Secondary subtitles will have no ASS/SSA Styling."
        );
    }
}

#[cfg(test)]
mod add_track_tests {
    use super::*;

    #[test]
    fn add_commands_use_the_cached_form_with_an_optional_title() {
        assert_eq!(
            add_track_args(AddKind::Audio, "/m/a.aac", Some("a.aac")),
            vec!["audio-add", "/m/a.aac", "cached", "a.aac"]
        );
        assert_eq!(
            add_track_args(AddKind::Subtitle, "/m/s.ass", None),
            vec!["sub-add", "/m/s.ass", "cached"]
        );
    }

    #[test]
    fn a_picked_file_takes_its_name_as_the_title() {
        assert_eq!(
            add_file_args(AddKind::Subtitle, Path::new("/home/u/subs/en.srt")),
            Some(vec![
                "sub-add".to_owned(),
                "/home/u/subs/en.srt".to_owned(),
                "cached".to_owned(),
                "en.srt".to_owned(),
            ])
        );
    }
}

#[cfg(test)]
mod external_track_tests {
    use super::*;

    fn track(url: &str, lang: &str) -> Track {
        Track {
            url: url.to_owned(),
            lang: lang.to_owned(),
        }
    }

    #[test]
    fn audio_then_subtitles_in_order() {
        let out = external_track_commands(
            &[track("https://a/1.aac", "ja")],
            &[
                track("https://s/1.vtt", "en"),
                track("https://s/2.vtt", "fr"),
            ],
        );
        assert_eq!(
            out.commands,
            vec![
                vec!["audio-add", "https://a/1.aac", "auto", "ja"],
                vec!["sub-add", "https://s/1.vtt", "auto", "en"],
                vec!["sub-add", "https://s/2.vtt", "auto", "fr"],
            ]
        );
        assert!(!out.finish_now);
    }

    #[test]
    fn empty_lists_finish_loading_at_once() {
        let out = external_track_commands(&[], &[]);
        assert!(out.commands.is_empty());
        assert!(out.finish_now);
    }

    #[test]
    fn one_empty_list_still_sends_the_other() {
        let out = external_track_commands(&[], &[track("https://s/1.vtt", "en")]);
        assert_eq!(out.commands.len(), 1);
        assert!(!out.finish_now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_maps_no_and_garbage_to_minus_one() {
        assert_eq!(track_id_from_value(Some("no")), -1);
        assert_eq!(track_id_from_value(Some("abc")), -1);
        assert_eq!(track_id_from_value(None), -1);
        assert_eq!(track_id_from_value(Some("3")), 3);
    }

    #[test]
    fn writing_minus_one_is_no_and_other_ids_are_ints() {
        assert_eq!(TrackValue::for_id(-1), TrackValue::No);
        assert_eq!(TrackValue::for_id(2), TrackValue::Id(2));
    }

    #[test]
    fn auto_change_is_ignored() {
        assert_eq!(track_change("auto"), None);
        assert_eq!(track_change("no"), Some(-1));
        assert_eq!(track_change("4"), Some(4));
    }

    /// (type, id, lang, title) of one track. A `None` type means the entry is missing.
    type Row = (
        Option<&'static str>,
        Option<i64>,
        Option<&'static str>,
        Option<&'static str>,
    );

    struct FakeTracks {
        count: Option<i64>,
        rows: Vec<Row>,
    }

    impl TrackProps for FakeTracks {
        fn count(&self) -> Option<i64> {
            self.count
        }
        fn kind(&self, index: i64) -> Option<String> {
            self.rows[index as usize].0.map(str::to_owned)
        }
        fn lang(&self, index: i64) -> Option<String> {
            self.rows[index as usize].2.map(str::to_owned)
        }
        fn title(&self, index: i64) -> Option<String> {
            self.rows[index as usize].3.map(str::to_owned)
        }
        fn id(&self, index: i64) -> Option<i64> {
            self.rows[index as usize].1
        }
    }

    fn sample() -> FakeTracks {
        FakeTracks {
            count: Some(4),
            rows: vec![
                (Some("video"), Some(1), None, None),
                (Some("audio"), Some(2), Some("jpn"), Some("Main")),
                (Some("sub"), Some(3), Some("eng"), None),
                (Some("sub"), Some(4), None, Some("Signs")),
            ],
        }
    }

    #[test]
    fn audio_list_starts_with_off_and_subtitles_do_not() {
        let lists = load_track_lists(&sample());
        assert_eq!(
            lists.audio[0],
            VideoTrack {
                id: -1,
                name: "Off".into(),
                language: None
            }
        );
        assert_eq!(lists.audio.len(), 2);
        assert_eq!(lists.audio[1].id, 2);
        assert_eq!(lists.audio[1].language.as_deref(), Some("jpn"));
        assert_eq!(lists.subtitles.len(), 2);
        assert!(lists.subtitles.iter().all(|t| t.id != -1));
    }

    #[test]
    fn missing_title_and_language_default_to_empty() {
        let lists = load_track_lists(&sample());
        assert_eq!(lists.subtitles[0].name, "");
        assert_eq!(lists.subtitles[0].language.as_deref(), Some("eng"));
        assert_eq!(lists.subtitles[1].language.as_deref(), Some(""));
    }

    #[test]
    fn unknown_type_and_missing_id_are_skipped() {
        let fake = FakeTracks {
            count: Some(3),
            rows: vec![
                (Some("button"), Some(9), None, None),
                (Some("sub"), None, None, None),
                (None, Some(5), None, None),
            ],
        };
        let lists = load_track_lists(&fake);
        assert!(lists.subtitles.is_empty());
        assert_eq!(lists.audio.len(), 1);
    }

    #[test]
    fn gone_handle_clears_both_lists() {
        let fake = FakeTracks {
            count: None,
            rows: vec![],
        };
        let lists = load_track_lists(&fake);
        assert!(lists.subtitles.is_empty());
        assert!(lists.audio.is_empty());
    }

    #[test]
    fn default_subtitle_is_first_and_audio_is_index_one() {
        let lists = load_track_lists(&sample());
        let out = default_tracks(&lists, |_, _| None);
        assert_eq!(out.sid, Some(3));
        assert_eq!(out.secondary_sid, Some(-1));
        assert_eq!(out.aid, Some(2));
    }

    #[test]
    fn preferred_track_wins_over_fallback() {
        let lists = load_track_lists(&sample());
        let out = default_tracks(&lists, |tracks, subtitle| {
            if subtitle {
                tracks.iter().position(|t| t.id == 4)
            } else {
                None
            }
        });
        assert_eq!(out.sid, Some(4));
        assert_eq!(out.aid, Some(2));
    }

    #[test]
    fn no_subtitles_and_no_real_audio_write_nothing() {
        let fake = FakeTracks {
            count: Some(0),
            rows: vec![],
        };
        let out = default_tracks(&load_track_lists(&fake), |_, _| None);
        assert_eq!(out, DefaultTracks::default());
    }

    #[test]
    fn picking_the_primary_promotes_the_secondary() {
        assert_eq!(select_sub((2, 5), 2), (5, TRACK_NONE));
    }

    #[test]
    fn picking_the_secondary_clears_it() {
        assert_eq!(select_sub((2, 5), 5), (2, TRACK_NONE));
    }

    #[test]
    fn a_new_id_is_the_secondary_when_a_primary_exists() {
        assert_eq!(select_sub((2, TRACK_NONE), 7), (2, 7));
    }

    #[test]
    fn a_new_id_is_the_primary_when_none_is_set() {
        assert_eq!(select_sub((TRACK_NONE, TRACK_NONE), 3), (3, TRACK_NONE));
        assert_eq!(select_sub((TRACK_NONE, 4), 3), (3, TRACK_NONE));
    }
}
