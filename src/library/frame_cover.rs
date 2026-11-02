//! The cover of a local anime that has none (docs/design/main-window.md): an image named `cover.*` in the series
//! folder, else the middle frame of the lowest episode written there as `cover.jpg`, as kuro-local's episode listing
//! and Aniyomi's local source do. The row's `thumbnail_url` then points at it, so the work runs once per entry.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kuro_data::entry::{EntryRepo, EntryUpdate};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::handle::Db;
use kuro_local::media::FrameExtractor;
use kuro_platform::kind::Kind;

thread_local! {
    /// Entries already tried this run, so a card that binds again starts no second ffmpeg.
    static TRIED: RefCell<HashSet<i64>> = RefCell::new(HashSet::new());
}

/// The cover image of the series folder `dir`: an existing `cover.*`, else one made from `lowest_episode`. None when
/// neither works (no episode, no ffmpeg, a frame that failed).
pub fn find_or_make(dir: &Path, lowest_episode: Option<&Path>, frames: &FrameExtractor) -> Option<PathBuf> {
    if let Some(found) = kuro_local::images::find_image(dir, "cover") {
        return Some(found);
    }
    let bytes = frames.frame(lowest_episode?).inspect_err(|e| tracing::warn!(error = %e, "no video frame for a local anime cover")).ok()??;
    kuro_local::images::write_image(dir, "cover", "cover.jpg", &bytes)
        .inspect_err(|e| tracing::warn!(error = %e, dir = %dir.display(), "the local anime cover could not be written"))
        .ok()
}

/// The lowest episode: the smallest number, then the first name (an unnumbered -1 comes first, as in kuro-local).
pub fn lowest(episodes: &[Item]) -> Option<&Item> {
    episodes.iter().min_by(|a, b| a.number.total_cmp(&b.number).then_with(|| a.name.cmp(&b.name)))
}

/// Finds or makes the cover of local anime `anime_id` off the GTK thread, stores it as the row's thumbnail, then runs
/// `done` on the GTK thread. Each entry is tried once per run.
pub fn ensure(db: Arc<Db>, anime_id: i64, done: impl FnOnce() + 'static) {
    if !TRIED.with(|t| t.borrow_mut().insert(anime_id)) {
        return;
    }
    // one cover at a time, so a library of uncovered series never runs a crowd of video decoders at once
    // ponytail: one thread per waiting entry; a single queue thread if libraries get huge
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    crate::background::run(
        move || {
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            store_cover(&db, anime_id)
        },
        move |stored| {
            if stored {
                done();
            }
        },
    );
}

fn store_cover(db: &Arc<Db>, anime_id: i64) -> bool {
    let Ok(anime) = EntryRepo::new(db.clone()).by_id(anime_id) else { return false };
    let episodes = ItemRepo::new(db.clone()).by_entry_id(anime_id, false).unwrap_or_default();
    let video = lowest(&episodes).and_then(|e| crate::local_flow::path_of(Kind::Anime, &e.url));
    let Some(folder) = crate::local_flow::path_of(Kind::Anime, &anime.url) else { return false };
    let frames = FrameExtractor::from_path(std::env::temp_dir());
    let Some(url) = find_or_make(&folder, video.as_deref(), &frames).and_then(|p| kuro_local::fs::file_url(&p)) else { return false };
    let update = EntryUpdate { thumbnail_url: Some(url), cover_last_modified: Some(kuro_data::time::now_millis()), ..EntryUpdate::new(anime_id) };
    EntryRepo::new(db.clone()).update(&update).inspect_err(|e| tracing::warn!(error = %e, "the local anime cover was not stored")).is_ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use kuro_local::media::Tool;

    use super::*;

    fn episode(number: f64, name: &str) -> Item {
        Item { number, name: name.into(), url: format!("/s/{name}.mkv"), ..Item::create(kuro_platform::kind::Kind::Anime) }
    }

    #[test]
    fn the_lowest_episode_is_the_smallest_number_then_the_first_name() {
        let eps = [episode(2.0, "b"), episode(1.0, "z"), episode(1.0, "a")];
        assert_eq!(lowest(&eps).map(|e| e.name.as_str()), Some("a"));
        assert_eq!(lowest(&[]).map(|e| e.name.clone()), None);
    }

    #[test]
    fn an_existing_cover_is_used_without_any_tool() {
        let dir = tempfile::tempdir().expect("dir");
        fs::write(dir.path().join("Cover.PNG"), b"\x89PNG\r\n\x1a\n").expect("cover");
        let none = FrameExtractor::new(None, None, dir.path());
        assert_eq!(find_or_make(dir.path(), None, &none), Some(dir.path().join("Cover.PNG")));
    }

    #[cfg(unix)]
    #[test]
    fn without_a_cover_the_lowest_episode_frame_becomes_cover_jpg() {
        let tools = tempfile::tempdir().expect("tools");
        let (probe, ffmpeg) = (tools.path().join("probe.sh"), tools.path().join("ffmpeg.sh"));
        fs::write(&probe, "echo 10\n").expect("probe");
        // ffmpeg's output path is its ninth argument
        fs::write(&ffmpeg, "printf 'jpeg' > \"$9\"\n").expect("ffmpeg");
        let temp = tempfile::tempdir().expect("temp");
        let frames = FrameExtractor::new(Some(Tool::new("sh").with_args([probe])), Some(Tool::new("sh").with_args([ffmpeg])), temp.path());
        let dir = tempfile::tempdir().expect("series");
        let video = dir.path().join("Show - 1.mkv");
        fs::write(&video, b"video").expect("video");
        let made = find_or_make(dir.path(), Some(&video), &frames).expect("a cover");
        assert_eq!(made, dir.path().join("cover.jpg"));
        assert_eq!(fs::read(made).expect("read"), b"jpeg");
    }

    #[test]
    fn no_cover_and_no_tool_or_episode_gives_none() {
        let dir = tempfile::tempdir().expect("dir");
        let none = FrameExtractor::new(None, None, dir.path());
        assert_eq!(find_or_make(dir.path(), None, &none), None);
        assert_eq!(find_or_make(dir.path(), Some(&dir.path().join("e.mkv")), &none), None);
    }
}
