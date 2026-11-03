//! The Audio list of the player's ⋮ menu and the Subtitles list of the bar's button: the tracks of the loaded file, read
//! from mpv each time the menu opens. Both lists start with Off (audio Off comes from `load_track_lists`, subtitle Off is added here). The current
//! track carries the check mark; choosing one selects it.

use std::rc::Rc;

use kuro_i18n::{MsgKey, tr};
use kuro_player::mpv::MpvHandle;
use kuro_player::tracks::{TRACK_NONE, TrackProps, VideoTrack, load_track_lists, track_id_from_value, track_label};

use super::Shared;
use super::sheets::Choice;

/// mpv's `track-list/*` properties as the track lists read them.
pub struct MpvTracks<'a>(pub &'a MpvHandle);

impl TrackProps for MpvTracks<'_> {
    fn count(&self) -> Option<i64> {
        Some(self.0.get_i64("track-list/count").unwrap_or(0))
    }
    fn kind(&self, index: i64) -> Option<String> {
        self.0.get_string(&format!("track-list/{index}/type"))
    }
    fn lang(&self, index: i64) -> Option<String> {
        self.0.get_string(&format!("track-list/{index}/lang"))
    }
    fn title(&self, index: i64) -> Option<String> {
        self.0.get_string(&format!("track-list/{index}/title"))
    }
    fn id(&self, index: i64) -> Option<i64> {
        self.0.get_i64(&format!("track-list/{index}/id"))
    }
}

/// The Audio list, read from mpv now, the playing track marked.
pub fn audio(shared: &Shared) -> Vec<Choice> {
    let (tracks, selected) = read(shared, "aid");
    tracks.audio.into_iter().map(|track| choice(shared, "aid", track, selected)).collect()
}

/// The Subtitles list, read from mpv now: Off, then the tracks, the playing one marked.
pub fn subtitles(shared: &Shared) -> Vec<Choice> {
    let (tracks, selected) = read(shared, "sid");
    std::iter::once(VideoTrack { id: TRACK_NONE, name: tr(MsgKey::off), language: None })
        .chain(tracks.subtitles)
        .map(|track| choice(shared, "sid", track, selected))
        .collect()
}

/// Whether the loaded file has a subtitle track (the bar's Subtitles button shows only then).
pub fn has_subtitles(shared: &Shared) -> bool {
    let player = shared.borrow();
    let tracks = MpvTracks(player.playback.mpv());
    (0..tracks.count().unwrap_or(0)).any(|index| tracks.kind(index).as_deref() == Some("sub"))
}

/// Whether a subtitle track is shown now (the bar's Subtitles button is marked then).
pub fn subtitles_on(shared: &Shared) -> bool {
    track_id_from_value(shared.borrow().playback.mpv().get_string("sid").as_deref()) != TRACK_NONE
}

/// The audio and subtitle tracks playing now, by title, so a reload of the same episode (another quality or server)
/// selects the same ones again rather than the defaults by language.
pub struct Picked {
    audio: Option<String>,
    subtitle: Option<String>,
    subtitles_off: bool,
}

/// The tracks `mpv` plays now.
pub fn picked(mpv: &MpvHandle) -> Picked {
    let tracks = MpvTracks(mpv);
    let id_of = |property: &str| track_id_from_value(mpv.get_string(property).as_deref());
    let title_of = |kind: &str, id: i64| {
        (0..tracks.count().unwrap_or(0)).find(|&i| tracks.kind(i).as_deref() == Some(kind) && tracks.id(i) == Some(id)).and_then(|i| tracks.title(i))
    };
    let (aid, sid) = (id_of("aid"), id_of("sid"));
    Picked { audio: title_of("audio", aid), subtitle: title_of("sub", sid), subtitles_off: sid == TRACK_NONE }
}

impl Picked {
    /// Marks the side tracks of the reload (`audio-add` and `sub-add` commands: verb, link, flag, title): the one with
    /// the picked title is selected and the others of its kind are not; with subtitles off, none is. A kind whose pick
    /// is not among them keeps the defaults.
    pub fn apply(&self, commands: &mut [Vec<String>]) {
        for (verb, title, off) in [("audio-add", &self.audio, false), ("sub-add", &self.subtitle, self.subtitles_off)] {
            let found = title.as_ref().is_some_and(|t| commands.iter().any(|c| c[0] == verb && c.get(3) == Some(t)));
            if !found && !off {
                continue;
            }
            for command in commands.iter_mut().filter(|c| c[0] == verb && c.len() > 2) {
                let on = !off && title.as_ref().is_some_and(|t| command.get(3) == Some(t));
                command[2] = if on { "select" } else { "auto" }.to_owned();
            }
        }
    }
}

/// The track lists of the loaded file and the track that plays under `property` (`aid` or `sid`).
fn read(shared: &Shared, property: &str) -> (kuro_player::tracks::TrackLists, i64) {
    let player = shared.borrow();
    let mpv = player.playback.mpv();
    (load_track_lists(&MpvTracks(mpv)), track_id_from_value(mpv.get_string(property).as_deref()))
}

/// One track row. Choosing it selects the track in mpv under `property` (`aid` or `sid`).
fn choice(shared: &Shared, property: &'static str, track: VideoTrack, selected: i64) -> Choice {
    let title = track_label(track.id, &track.name, track.language.as_deref());
    // kept as one tag: a source's track often has only a name ("French [CC]"), and the next one may call it "Français"
    let language = kuro_player::track_select::language_tag(track.language.as_deref(), &track.name);
    let (shared, id) = (Rc::clone(shared), track.id);
    // said on the video at once: a subtitle only shows at its next line
    let heading = tr(if property == "aid" { MsgKey::pref_player_audio } else { MsgKey::pref_player_subtitle });
    let shown = format!("{heading}: {title}");
    Choice {
        title,
        subtitle: None,
        current: id == selected,
        enabled: true,
        pick: Box::new(move || {
            let player = shared.borrow();
            if let Err(e) = player.playback.mpv().set_track(property, id) {
                tracing::warn!(error = %e, property, "player: the track was not selected");
            }
            player.playback.show_text(&shown);
            drop(player);
            // the next episodes of this anime start with the same language
            super::remember_track(shared.borrow().episode_id, property, language.as_deref());
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(verb: &str, title: &str, flag: &str) -> Vec<String> {
        vec![verb.to_owned(), format!("https://cdn.test/{title}"), flag.to_owned(), title.to_owned()]
    }

    fn flags(commands: &[Vec<String>]) -> Vec<(&str, &str)> {
        commands.iter().map(|c| (c[3].as_str(), c[2].as_str())).collect()
    }

    #[test]
    fn a_reload_selects_the_audio_and_subtitle_picked_before_it() {
        let mut commands = vec![add("audio-add", "Japanese", "select"), add("audio-add", "English", "auto"), add("sub-add", "English", "select"), add("sub-add", "Français", "auto")];
        Picked { audio: Some("English".into()), subtitle: Some("Français".into()), subtitles_off: false }.apply(&mut commands);
        assert_eq!(flags(&commands), [("Japanese", "auto"), ("English", "select"), ("English", "auto"), ("Français", "select")]);
    }

    #[test]
    fn subtitles_off_stay_off_and_a_pick_the_stream_lacks_keeps_the_defaults() {
        let mut commands = vec![add("audio-add", "Japanese", "select"), add("sub-add", "English", "select")];
        Picked { audio: Some("Korean".into()), subtitle: None, subtitles_off: true }.apply(&mut commands);
        assert_eq!(flags(&commands), [("Japanese", "select"), ("English", "auto")]);
    }
}
