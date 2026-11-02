//! Headless check of the player core: real libmpv (`vo=null`, `ao=null`) plays a generated clip, and the real anime
//! database holds the progress. No display is needed. The clip comes from ffmpeg; without ffmpeg the test says so
//! and returns.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kuro_data::entry::{Entry, EntryRepo};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use kuro_player::mpv::MpvHandle;
use kuro_player::options::{MpvSetting, PlayerPrefs, startup_settings};
use kuro_testkit::{TempDir, TestDb};

use super::playback::Playback;
use super::store::Tracker;

/// Writes a 4 second 160x120 test pattern at 10 fps. Returns `None` when ffmpeg is not installed.
fn make_clip(dir: &Path) -> Option<PathBuf> {
    let path = dir.join("clip.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=4:size=160x120:rate=10"])
        .args(["-c:v", "mpeg4", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .status()
        .ok()?;
    status.success().then_some(path)
}

/// A libmpv core with no window and no sound output.
fn headless(data_dir: &Path) -> Playback {
    let prefs = PlayerPrefs {
        data_dir: data_dir.to_owned(),
        ..PlayerPrefs::default()
    };
    let mut settings = startup_settings(&prefs);
    settings.push(MpvSetting {
        name: "ao",
        value: "null".to_owned(),
    });
    kuro_platform::locale::use_c_numeric_locale();
    let mpv = MpvHandle::create(&settings).expect("libmpv must initialise headless");
    mpv.set_property_str("vo", "null").expect("vo=null");
    Playback::new(mpv)
}

/// Polls `check` until it holds or 15 seconds pass.
fn wait_for(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    false
}

/// One anime with one episode whose url is the clip. Returns the database and the episode id.
fn library(dir: &TempDir, clip: &Path) -> (Arc<Db>, i64) {
    let db = TestDb::handle(dir);
    let anime_id = EntryRepo::new(Arc::clone(&db)).insert(&Entry::create(Kind::Anime)).expect("anime row");
    let episode = Item {
        entry_id: anime_id,
        url: clip.display().to_string(),
        name: "Episode 1".to_owned(),
        ..Item::create(Kind::Anime)
    };
    let id = ItemRepo::new(Arc::clone(&db)).add_all(&[episode]).expect("episode row")[0].id;
    (db, id)
}

fn stored(db: &Arc<Db>, id: i64) -> Item {
    ItemRepo::new(Arc::clone(db)).by_id(id).expect("read").expect("row")
}

#[test]
fn a_clip_plays_seeks_pauses_and_its_progress_survives_a_reopen() {
    let dir = TempDir::new().unwrap();
    let Some(clip) = make_clip(dir.path()) else {
        eprintln!("skipped: ffmpeg is not installed, so no test clip was made");
        return;
    };
    let (db, id) = library(&dir, &clip);
    let playback = headless(dir.path());

    let mut tracker = Tracker::open(Arc::clone(&db), id, false).expect("episode row");
    assert_eq!(tracker.start_ms(0), 0, "an unseen episode with no progress starts at zero");
    playback.load(clip.to_str().expect("UTF-8 path"), None, tracker.start_ms(0)).expect("loadfile");

    assert!(wait_for(|| playback.duration_ms().is_some_and(|d| d > 0)), "mpv reports the duration");
    let duration = playback.duration_ms().unwrap();
    assert!((3_500..=4_500).contains(&duration), "a 4 second clip, got {duration} ms");
    assert!(wait_for(|| playback.position_ms().is_some_and(|p| p > 300)), "playback advances");
    assert!(!playback.is_paused(), "a load starts playing");

    assert!(playback.toggle_pause().expect("pause"), "toggle reports paused");
    assert!(playback.is_paused());
    assert!(!playback.toggle_pause().expect("play"), "toggle reports playing");

    playback.seek_to_ms(2_500).expect("seek");
    assert!(wait_for(|| playback.position_ms().is_some_and(|p| p >= 2_400)), "seek moves the playhead");

    playback.set_volume(40.0).expect("volume");
    assert_eq!(playback.volume(), 40.0);

    let position = playback.position_ms().unwrap();
    tracker.save(1_000, position, duration, true);
    let saved = stored(&db, id);
    assert_eq!(saved.progress, position / 1000, "the forced save writes the playhead");
    assert_eq!(saved.total_seconds(), duration / 1000);
    assert!(!saved.done, "two thirds through is not finished");

    let reopened = Tracker::open(Arc::clone(&db), id, false).expect("reopen");
    assert_eq!(reopened.start_ms(0), saved.progress * 1000, "reopen restores the saved position");

    playback.seek_to_ms(3_800).expect("seek near the end");
    assert!(wait_for(|| playback.eof_reached()), "playback reaches the end of the clip");
    tracker.save(2_000, playback.duration_ms().unwrap(), playback.duration_ms().unwrap(), true);
    assert!(stored(&db, id).done, "reaching the end marks the episode seen");

    let finished = Tracker::open(db, id, false).expect("reopen after finishing");
    assert_eq!(finished.start_ms(0), 0, "a seen episode restarts at zero");

    playback.stop();
}

/// A source stream's load on real libmpv, over local files: the headers land on the core before `loadfile`, and the
/// subtitle added once the file is loaded becomes a track.
#[test]
fn a_stream_load_sets_its_headers_and_adds_its_subtitle_after_the_file_loads() {
    let dir = TempDir::new().unwrap();
    let Some(clip) = make_clip(dir.path()) else {
        eprintln!("skipped: ffmpeg is not installed, so no test clip was made");
        return;
    };
    let subtitle = dir.path().join("en.srt");
    std::fs::write(&subtitle, "1\n00:00:00,000 --> 00:00:03,000\nHello\n").unwrap();
    let mut playback = headless(dir.path());
    playback.load(clip.to_str().unwrap(), Some("Origin: https://player.test,X-Note: a\\, b"), 0).expect("loadfile");
    let headers = playback.mpv().get_string("http-header-fields").unwrap_or_default();
    assert!(headers.contains("Origin: https://player.test") && headers.contains("X-Note: a"), "{headers}");

    assert!(wait_for(|| playback.mpv_mut().drain_events().contains(&kuro_player::observer::MpvEvent::FileLoaded)), "the file loads");
    let before = playback.mpv().get_i64("track-list/count").unwrap_or(0);
    playback.add_tracks(&[vec!["sub-add".into(), subtitle.display().to_string(), "auto".into(), "English".into()]], None);
    // the side track is added on a worker thread
    assert!(wait_for(|| playback.mpv().get_i64("track-list/count") == Some(before + 1)), "sub-add adds the subtitle track");
    let lists = kuro_player::tracks::load_track_lists(&super::menu::MpvTracks(playback.mpv()));
    assert!(lists.subtitles.iter().any(|t| t.name == "English"), "the tracks menu lists the added subtitle: {lists:?}");
    playback.stop();
}

/// Runs the main loop for `ms`.
fn pump_for(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Escape, Alt+Left and the mouse back button leave the embedded page through `ui::pop`, which starts the slide
/// back and drops it after. Playback must stop and save at that call, not at the removal. Needs a display and ffmpeg.
#[test]
fn popping_the_embedded_player_stops_playback_at_once() {
    crate::testing::with_display(|| {
        use gtk::prelude::*;
        let dir = TempDir::new().unwrap();
        let Some(clip) = make_clip(dir.path()) else {
            return;
        };
        let (_stack, window) = crate::testing::nav();
        let was_visible = window.is_visible();
        window.present();
        crate::ui::push(&gtk::Label::new(Some("below")));
        let media = super::Media { target: clip.display().to_string(), title: crate::widgets::page_header::MediaTitle::new("clip", ""), headers: None, track_headers: Vec::new(), tracks: Vec::new() };
        let shared = super::open_in(dir.path().to_owned(), media, 0, 0).expect("the player opens");
        pump_for(500);
        assert!(!shared.borrow().closed, "playing");

        crate::ui::pop();
        assert!(shared.borrow().closed, "closed by the pop itself, before the slide ends");
        assert_eq!(shared.borrow().playback.mpv().get_flag("idle-active"), Some(true), "mpv stopped");
        pump_for(500);
        window.set_visible(was_visible);
    });
}

/// Closing the player frees everything: after Back the page goes away and nothing holds the player any more, so the
/// mpv handle (and its render context and timers) is destroyed. A leak would keep the weak reference alive.
/// Needs a display and ffmpeg.
#[test]
fn a_closed_player_is_freed_and_leaves_no_leak() {
    crate::testing::with_display(|| {
        use gtk::prelude::*;
        let dir = TempDir::new().unwrap();
        let Some(clip) = make_clip(dir.path()) else {
            return;
        };
        let (_stack, window) = crate::testing::nav();
        let was_visible = window.is_visible();
        window.present();
        crate::ui::push(&gtk::Label::new(Some("below")));
        let media = super::Media { target: clip.display().to_string(), title: crate::widgets::page_header::MediaTitle::new("clip", ""), headers: None, track_headers: Vec::new(), tracks: Vec::new() };
        let shared = super::open_in(dir.path().to_owned(), media, 0, 0).expect("the player opens");
        pump_for(400);
        let weak = std::rc::Rc::downgrade(&shared);
        crate::ui::pop();
        drop(shared);
        pump_for(1200);
        assert!(weak.upgrade().is_none(), "nothing keeps the closed player (and its mpv handle) alive");
        assert!(!super::is_open(), "no player counts as open any more");
        window.set_visible(was_visible);
    });
}

/// F11 and Escape act on the main window, which holds the page. Needs a display, a compositor that honours fullscreen,
/// and ffmpeg; without them it says so and returns.
#[test]
fn fullscreen_and_escape_act_on_the_window_that_holds_the_player() {
    crate::testing::with_display(|| {
        use gtk::prelude::*;
        let dir = TempDir::new().unwrap();
        let Some(clip) = make_clip(dir.path()) else {
            return;
        };
        let (_nav, window) = crate::testing::nav();
        let was_visible = window.is_visible();
        window.present();
        // a page under the player, so that popping the player has somewhere to go back to
        crate::ui::push(&gtk::Box::new(gtk::Orientation::Vertical, 0));
        let media = super::Media { target: clip.display().to_string(), title: crate::widgets::page_header::MediaTitle::new("clip", ""), headers: None, track_headers: Vec::new(), tracks: Vec::new() };
        let shared = super::open_in(dir.path().to_owned(), media, 0, 0).expect("the player opens");
        let page = crate::ui::visible_child().and_downcast::<gtk::Box>().expect("the page is the visible child");

        // the header is the page's first child: fullscreen hides it, and leaving fullscreen brings it back
        // (inside the revealer that auto-hide slides; the header is its child)
        let first = page.first_child().expect("the page has its header");
        let header = first.downcast_ref::<gtk::Revealer>().and_then(gtk::Revealer::child).unwrap_or(first);
        super::toggle_fullscreen(&page);
        pump_for(700);
        if window.is_fullscreen() {
            assert!(!header.is_visible(), "fullscreen hides the header");
            assert!(super::apply_action(&shared, &page, kuro_player::shortcuts::Action::Escape), "Escape leaves fullscreen");
            pump_for(700);
            assert!(!window.is_fullscreen(), "back to normal");
            assert!(header.is_visible(), "leaving fullscreen shows the header again");
        } else {
            eprintln!("skipped: the compositor did not fullscreen the window");
        }
        crate::ui::pop();
        pump_for(300);
        assert!(shared.borrow().closed);
        pump_for(300);
        window.set_visible(was_visible);
    });
}

/// The control bar is one capsule of seven controls, each named for screen readers (with its key), and no error page
/// covers a video that plays. `KB_PLAYER_SHOT=<file.png>` saves a picture of the window. Needs a display and ffmpeg.
#[test]
fn the_control_bar_names_every_control_and_a_playing_video_shows_no_error() {
    crate::testing::with_display(|| {
        use gtk::prelude::*;
        let dir = TempDir::new().unwrap();
        let Some(clip) = make_clip(dir.path()) else {
            return;
        };
        let (_stack, window) = crate::testing::nav();
        let was_visible = window.is_visible();
        window.set_default_size(1280, 720);
        window.present();
        let media = super::Media { target: clip.display().to_string(), title: crate::widgets::page_header::MediaTitle::new("Clip", "Episode 1"), headers: None, track_headers: Vec::new(), tracks: Vec::new() };
        let shared = super::open_in(dir.path().to_owned(), media, 0, 0).expect("the player opens");
        pump_for(1500);
        let page = crate::ui::visible_child().expect("the player page");
        let mut all = Vec::new();
        let mut stack = vec![page.clone()];
        while let Some(w) = stack.pop() {
            stack.extend(std::iter::successors(w.first_child(), |c| c.next_sibling()));
            all.push(w);
        }
        let bar = all.iter().find(|w| w.has_css_class("osd") && w.has_css_class("toolbar")).expect("the control bar");
        let controls: Vec<gtk::Widget> = std::iter::successors(bar.first_child(), |c| c.next_sibling()).collect();
        assert_eq!(controls.len(), 7, "previous, play, next, seek, time, volume, fullscreen");
        for control in controls.iter().filter(|c| !c.is::<gtk::Label>()) {
            assert!(control.tooltip_text().is_some_and(|t| !t.is_empty()), "{} has a tooltip", control.type_().name());
        }
        let failure = all.iter().find(|w| w.is::<adw::StatusPage>()).expect("the error page");
        assert!(!failure.is_visible(), "a video that plays shows no error");
        // the skip button sits above the bar, not under it
        let skip = all.iter().find(|w| w.is::<gtk::Revealer>() && w.first_child().is_some_and(|b| b.has_css_class("pill"))).expect("the skip button");
        let bar_top = bar.compute_bounds(&window).map(|b| b.y()).expect("the bar on screen");
        let skip_bottom = window.height() as f32 - skip.margin_bottom() as f32;
        assert!(skip.margin_bottom() > 0 && skip_bottom <= bar_top, "skip button bottom {skip_bottom} above the bar top {bar_top}");
        if let Ok(path) = std::env::var("KB_PLAYER_SHOT") {
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snap = gtk::Snapshot::new();
            paintable.snapshot(&snap, f64::from(window.width()), f64::from(window.height()));
            if let (Some(node), Some(renderer)) = (snap.to_node(), window.renderer()) {
                renderer.render_texture(node, None).save_to_png(path).unwrap();
            }
        }
        crate::ui::pop();
        drop(shared);
        pump_for(300);
        window.set_visible(was_visible);
    });
}
