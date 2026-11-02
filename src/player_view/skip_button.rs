//! The skip button: a pill over the video, above the control bar, that jumps past the playing opening or ending
//! ("Skip Opening", "Skip Ending"). The segment comes from mpv's chapters named Opening, OP, Ending or ED. When the
//! episode's anime has a MAL id, AniSkip's intervals are merged in, fetched once per file in the background. The rules
//! are `kuro_player::skip` and `kuro_player::chapters`; this file reads mpv and the database and shows the button.
//! Main thread only, except the AniSkip request, which runs on the app's async runtime and comes back over a channel.
//! Every failure of that request is silent.

use std::rc::Rc;

use gtk::prelude::*;
use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;
use kuro_data::track::{Track, TrackRepo};
use kuro_player::aniskip::{self, TrackerRef};
use kuro_player::chapters::{self, Segment};
use kuro_player::mpv::MpvHandle;
use kuro_player::skip::{Label, SkipConfig, SkipIntro, Tap};
use kuro_platform::system_paths::SystemPaths;
use kuro_prefs::network::NetworkPrefs;
use kuro_track::model::TrackerId;

use super::{Player, Shared};

/// The skip state of the playing file. It lives in the player and every FILE_LOADED starts it again.
pub struct State {
    machine: SkipIntro,
    /// The file's chapters: mpv's, with the kind their titles name, and AniSkip's intervals once they arrive.
    chapters: Vec<Segment>,
    /// The chapters are still to be read: the file is loaded but its duration is not known yet.
    pending: bool,
    /// Counts the file loads, so an AniSkip answer for an earlier file is dropped.
    load: u64,
    /// The label the button shows now; `None` hides it.
    label: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            machine: SkipIntro::new(SkipConfig::default()),
            chapters: Vec::new(),
            pending: true,
            load: 0,
            label: None,
        }
    }
}

impl State {
    /// FILE_LOADED: the next file starts with its own chapters, skip state and AniSkip lookup.
    pub fn file_loaded(&mut self) {
        *self = Self {
            load: self.load + 1,
            ..Self::default()
        };
    }
}

/// The button and the revealer that fades it in and out. The button never takes keyboard focus.
#[derive(Clone)]
pub struct Ui {
    revealer: gtk::Revealer,
    button: gtk::Button,
}

impl Ui {
    /// The button at the bottom right of the video, above the control bar.
    pub fn new(shared: &Shared) -> Self {
        let button = gtk::Button::new();
        button.add_css_class("suggested-action");
        button.add_css_class("pill");
        button.set_focusable(false);
        button.set_focus_on_click(false);
        {
            let shared = Rc::clone(shared);
            button.connect_clicked(move |_| skip(&shared));
        }
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::Crossfade);
        revealer.set_child(Some(&button));
        revealer.set_halign(gtk::Align::End);
        revealer.set_valign(gtk::Align::End);
        revealer.set_margin_end(crate::widgets::osd::MARGIN);
        Self { revealer, button }
    }

    /// Keeps the button just above `bar` (the control bar, on the same overlay): its bottom margin is the bar's height
    /// and the bar's margins, measured once the bar is on screen.
    pub fn place_above(&self, bar: &gtk::Box) {
        let revealer = self.revealer.downgrade();
        bar.connect_map(move |bar| {
            let (_, height, _, _) = bar.measure(gtk::Orientation::Vertical, -1);
            if let Some(revealer) = revealer.upgrade() {
                revealer.set_margin_bottom(height + 2 * crate::widgets::osd::MARGIN);
            }
        });
    }

    /// The widget to add over the video.
    pub fn widget(&self) -> &gtk::Revealer {
        &self.revealer
    }

    /// Shows `label` on the button, or hides the button for `None`.
    fn show(&self, label: Option<&str>) {
        match label {
            Some(text) => {
                if self.button.label().as_deref() != Some(text) {
                    self.button.set_label(text);
                }
                self.revealer.set_reveal_child(true);
            }
            None => self.revealer.set_reveal_child(false),
        }
    }
}

/// One tick of the player (every 250 ms): reads the file's chapters once its duration is known and starts its AniSkip
/// lookup, then moves the button to the segment the position is in.
pub fn tick(shared: &Shared, ui: &Ui) {
    let (label, request) = {
        let mut player = shared.borrow_mut();
        let mut request = None;
        if player.skip.pending && duration_s(&player) > 0.0 {
            player.skip.pending = false;
            let chapters = read_chapters(player.playback.mpv());
            player.skip.chapters = chapters;
            request = aniskip_request(player.episode_id, player.skip.load, duration_s(&player));
        }
        let position = position_s(&player);
        let state = &mut player.skip;
        let tick = state.machine.on_tick(&state.chapters, position);
        state.label = next_label(state.label.take(), tick.label);
        (state.label.clone(), request)
    };
    ui.show(label.as_deref());
    if let Some(request) = request {
        start_aniskip(shared, request);
    }
}

/// The button was pressed: playback jumps to the end of the segment, and the "skipped" text shows as a toast.
fn skip(shared: &Shared) {
    let mut player = shared.borrow_mut();
    let (position, duration) = (position_s(&player), duration_s(&player));
    let (tap, target) = {
        let state = &mut player.skip;
        let tap = state.machine.on_skip(&state.chapters, position);
        (tap, skip_target(&state.chapters, position, duration))
    };
    let Tap::Seek(seek) = tap else {
        return;
    };
    if let Err(e) = player.playback.seek_to_ms((f64::from(target) * 1000.0) as i64) {
        tracing::warn!(error = %e, "player: the skip seek failed");
    }
    drop(player);
    crate::toast::notify(seek.text);
}

/// Where a skip from `position` goes: the start of the next chapter, or the end of the file when the chapter now
/// playing is the last one.
pub fn skip_target(list: &[Segment], position: f32, duration: f32) -> f32 {
    chapters::current_chapter(list, position)
        .and_then(|index| list.get(index + 1))
        .map_or(duration, |next| next.start)
}

/// The label after one tick: `Set` replaces the text, where `None` hides the button, and `Keep` leaves it as it is.
pub fn next_label(current: Option<String>, label: Label) -> Option<String> {
    match label {
        Label::Keep => current,
        Label::Set(text) => text,
    }
}

/// The tracker bindings of an anime as AniSkip's MAL lookup reads them, in the bindings' order.
pub fn tracker_refs(tracks: &[Track]) -> Vec<TrackerRef> {
    tracks
        .iter()
        .map(|track| match track.tracker_id {
            id if id == TrackerId::MYANIMELIST.0 => TrackerRef::MyAnimeList(track.remote_id),
            id if id == TrackerId::ANILIST.0 => TrackerRef::AniList(track.remote_id),
            _ => TrackerRef::Other,
        })
        .collect()
}

/// What one AniSkip lookup needs: the episode, the file's load and the anime's bindings.
struct Request {
    episode_id: i64,
    anime_id: i64,
    /// The anime's title, for the MAL id lookup when it has no usable tracker binding.
    title: Option<String>,
    load: u64,
    episode_number: f64,
    tracks: Vec<TrackerRef>,
    duration_s: f64,
}

/// The lookup of an episode row, or `None` when the app data or the row is missing.
fn aniskip_request(episode_id: i64, load: u64, duration_s: f32) -> Option<Request> {
    if !crate::data::is_installed() {
        return None;
    }
    let db = crate::data::app().db.clone();
    let episode = ItemRepo::new(db.clone()).by_id(episode_id).ok()??;
    let title = EntryRepo::new(db.clone()).by_id(episode.entry_id).ok().map(|anime| anime.title);
    let tracks = TrackRepo::new(db).by_entry(episode.entry_id).ok()?;
    Some(Request {
        episode_id,
        anime_id: episode.entry_id,
        title,
        load,
        episode_number: episode.number,
        tracks: tracker_refs(&tracks),
        duration_s: f64::from(duration_s),
    })
}

/// Looks up the file's AniSkip intervals on the app's runtime, then merges them into the chapters on the main thread.
/// A missing runtime or client, a network or decode error, or no MAL id leaves the chapters as they are.
fn start_aniskip(shared: &Shared, request: Request) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let Ok(paths) = SystemPaths::system() else {
        return;
    };
    let prefs = NetworkPrefs::new(crate::data::app().base.store().clone());
    let Ok(client) = crate::net_client::build(&prefs, &paths) else {
        return;
    };
    let (episode_id, load, duration) = (request.episode_id, request.load, request.duration_s);
    let shared = Rc::clone(shared);
    crate::background::run_async_on(
        &handle,
        async move {
            let mut tracks = request.tracks;
            if aniskip::needs_title_lookup(&tracks) {
                if let Some(mal) = match &request.title {
                    Some(title) => aniskip::mal_id_by_title(&client, request.anime_id, title).await,
                    None => None,
                } {
                    tracks = vec![TrackerRef::MyAnimeList(mal)];
                }
            }
            let stamps = aniskip::lookup(&client, &tracks, request.episode_number, Some(request.duration_s)).await;
            (episode_id, load, duration, stamps)
        },
        move |answer| {
            let Some((episode_id, load, duration, Some(stamps))) = answer else {
                return;
            };
            let mut player = shared.borrow_mut();
            if player.closed || player.episode_id != episode_id || player.skip.load != load {
                return;
            }
            let merged = chapters::with_timestamps(&player.skip.chapters, &stamps, Some(duration as i32));
            if let Some(merged) = merged {
                player.skip.chapters = merged;
            }
        },
    );
}

/// mpv's chapters of the playing file. Each takes the kind its title names.
fn read_chapters(mpv: &MpvHandle) -> Vec<Segment> {
    let count = mpv.get_i64("chapter-list/count").unwrap_or(0);
    let raw: Vec<(Option<String>, f32)> = (0..count)
        .map(|index| {
            let title = mpv.get_string(&format!("chapter-list/{index}/title"));
            let start = mpv.get_f64(&format!("chapter-list/{index}/time")).unwrap_or(0.0);
            (title, start as f32)
        })
        .collect();
    let mut list = chapters::load(&raw);
    for segment in &mut list {
        segment.kind = chapters::kind_of_title(&segment.name);
    }
    list
}

fn position_s(player: &Player) -> f32 {
    player.playback.position_ms().unwrap_or(0) as f32 / 1000.0
}

fn duration_s(player: &Player) -> f32 {
    player.playback.duration_ms().unwrap_or(0) as f32 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_player::types::ChapterType;

    fn segment(name: &str, start: f32, kind: ChapterType) -> Segment {
        Segment {
            index: 0,
            name: name.to_owned(),
            start,
            kind,
        }
    }

    /// `[Other 0, Opening 10, Other 100, Ending 200]` in a 300 second file.
    fn chapters() -> Vec<Segment> {
        vec![
            segment("", 0.0, ChapterType::Other),
            segment("Opening", 10.0, ChapterType::Opening),
            segment("", 100.0, ChapterType::Other),
            segment("Ending", 200.0, ChapterType::Ending),
        ]
    }

    /// The label shows the segment's skip text inside an opening or an ending and nothing elsewhere.
    #[test]
    fn the_label_follows_the_segment_the_position_is_in() {
        let list = chapters();
        let mut machine = SkipIntro::new(SkipConfig::default());
        let mut label = None;
        let steps = [
            (5.0, None),
            (12.0, chapters::skip_label(ChapterType::Opening)),
            (150.0, None),
            (210.0, chapters::skip_label(ChapterType::Ending)),
            // the last chapter runs to the end of the file
            (260.0, chapters::skip_label(ChapterType::Ending)),
        ];
        for (position, expected) in steps {
            let tick = machine.on_tick(&list, position);
            label = next_label(label, tick.label);
            assert_eq!(label, expected, "at {position} s");
        }
    }

    #[test]
    fn keep_leaves_the_label_and_set_replaces_it() {
        assert_eq!(next_label(Some("a".into()), Label::Keep), Some("a".into()));
        assert_eq!(next_label(None, Label::Set(Some("b".into()))), Some("b".into()));
        assert_eq!(next_label(Some("b".into()), Label::Set(None)), None);
    }

    #[test]
    fn a_skip_goes_to_the_next_chapter_or_to_the_end_of_a_last_one() {
        let list = chapters();
        assert_eq!(skip_target(&list, 12.0, 300.0), 100.0);
        assert_eq!(skip_target(&list, 210.0, 300.0), 300.0);
        assert_eq!(skip_target(&[], 12.0, 300.0), 300.0);
    }

    #[test]
    fn tracker_bindings_map_to_the_mal_lookup_in_order() {
        let tracks = [
            Track { remote_id: 21, ..Track::new(kuro_platform::kind::Kind::Anime, TrackerId::MYANIMELIST.0) },
            Track { remote_id: 5, ..Track::new(kuro_platform::kind::Kind::Anime, TrackerId::ANILIST.0) },
            Track { remote_id: 9, ..Track::new(kuro_platform::kind::Kind::Anime, 7) },
        ];
        assert_eq!(
            tracker_refs(&tracks),
            vec![TrackerRef::MyAnimeList(21), TrackerRef::AniList(5), TrackerRef::Other]
        );
    }

    #[test]
    fn a_new_file_starts_its_skip_state_again() {
        let mut state = State::default();
        state.chapters = chapters();
        state.pending = false;
        state.label = Some("x".into());
        state.file_loaded();
        assert!(state.chapters.is_empty());
        assert!(state.pending);
        assert_eq!(state.label, None);
        assert_eq!(state.load, 1);
    }
}
