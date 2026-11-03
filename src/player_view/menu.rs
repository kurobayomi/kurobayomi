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
    Choice {
        title,
        subtitle: None,
        current: id == selected,
        enabled: true,
        pick: Box::new(move || {
            if let Err(e) = shared.borrow().playback.mpv().set_track(property, id) {
                tracing::warn!(error = %e, property, "player: the track was not selected");
            }
            // the next episodes of this anime start with the same language
            super::remember_track(shared.borrow().episode_id, property, language.as_deref());
        }),
    }
}
