//! Anime episode list (T-14a-023).

use std::io;
use std::path::{Path, PathBuf};

use kuro_source::anime::{SAnime, SEpisode};

use crate::dates::parse_local_date;
use crate::error::LocalSourceError;
use crate::format::is_anime_episode;
use crate::fs::{FsEntry, file_url, find_child, list_dir};
use crate::images::{existing_thumbnail, find_image, write_image, write_thumbnail};
use crate::json_side::{EpisodeDetails, read_episodes_json, same_number, side_file};
use crate::names::{is_hidden, stem};
use crate::natsort::natural_cmp_ci;
use crate::recognition::episode_number;

/// The episodes of the folder named by `anime.url`, without frame extraction: see `episode_list_with`.
pub fn episode_list(root: &Path, anime: &SAnime) -> Result<Vec<SEpisode>, LocalSourceError> {
    episode_list_with(root, anime, |_| Ok(None))
}

/// The episodes of the folder named by `anime.url`: non-hidden files with an episode extension, newest number first,
/// then name descending. A missing folder gives an empty list.
///
/// `frame` makes one JPEG from a video file (ffmpeg, T-14a-026), `Ok(None)` when none came out. Auto-generation policy
/// (behavior 11.5 steps 4 and 5, E-ANIME-6): a missing or empty `<name>-thumbnail.jpg` is generated, an existing one is
/// kept. A missing cover or background falls back to the lowest episode. Frame errors are logged and never reach the
/// result.
///
/// The `episodes.json` overrides (T-14a-040) apply before the thumbnail step, as in Kotlin. A file that does not decode
/// fails the listing (E-ANIME-3).
pub fn episode_list_with<F>(
    root: &Path,
    anime: &SAnime,
    frame: F,
) -> Result<Vec<SEpisode>, LocalSourceError>
where
    F: Fn(&Path) -> io::Result<Option<Vec<u8>>>,
{
    let Some(dir) = find_child(root, &anime.url).filter(|p| p.is_dir()) else {
        return Err(LocalSourceError::folder_missing());
    };
    let entries = list_dir(&dir)?;
    let overrides = side_file(&entries, "episodes").and_then(|file| read_episodes_json(&file.path));
    let mut found: Vec<(SEpisode, PathBuf)> = entries
        .iter()
        .filter(|e| is_episode_entry(e))
        .map(|e| (episode_of(anime, e), e.path.clone()))
        .collect();
    for (episode, video) in &mut found {
        if let Some(data) = overrides
            .iter()
            .flatten()
            .find(|d| same_number(episode.episode_number, d.episode_number))
        {
            apply_override(episode, data);
        }
        ensure_thumbnail(&dir, episode, video, &frame);
    }
    found.sort_by(|(a, _), (b, _)| {
        b.episode_number
            .total_cmp(&a.episode_number)
            .then_with(|| natural_cmp_ci(&b.name, &a.name))
    });
    let lowest = found.last().map(|(_, video)| video.clone());
    if let Some(lowest) = lowest {
        if find_image(&dir, "cover").is_none() {
            fallback_image(&dir, &lowest, "cover", "cover.jpg", "cover", &frame);
        }
        if find_image(&dir, "background").is_none() {
            fallback_image(
                &dir,
                &lowest,
                "background",
                "background.jpg",
                "background",
                &frame,
            );
        }
    }
    Ok(found.into_iter().map(|(episode, _)| episode).collect())
}

/// Kotlin episode override: the name and date when present (a date that does not parse keeps the file date), and the
/// scanlator and summary replaced even when the override has none.
fn apply_override(episode: &mut SEpisode, data: &EpisodeDetails) {
    if let Some(name) = &data.name {
        episode.name = name.clone();
    }
    if let Some(millis) = data.date_upload.as_deref().and_then(parse_local_date) {
        episode.date_upload = millis;
    }
    episode.scanlator = data.scanlator.clone();
    episode.summary = data.summary.clone();
}

/// One episode thumbnail under the auto-generation policy. An existing non-empty file is kept and becomes the
/// episode's `preview_url`.
fn ensure_thumbnail<F>(dir: &Path, episode: &mut SEpisode, video: &Path, frame: &F)
where
    F: Fn(&Path) -> io::Result<Option<Vec<u8>>>,
{
    if let Some(existing) = existing_thumbnail(dir, &episode.name) {
        episode.preview_url = file_url(&existing);
        return;
    }
    match frame(video) {
        Ok(Some(bytes)) if !bytes.is_empty() => match write_thumbnail(dir, &episode.name, &bytes) {
            Ok(url) => episode.preview_url = Some(url),
            Err(error) => tracing::error!(
                "Couldn't extract thumbnail from video: {error} ({})",
                video.display()
            ),
        },
        Ok(_) => {}
        Err(error) => tracing::error!(
            "Couldn't extract thumbnail from video: {error} ({})",
            video.display()
        ),
    }
}

/// A cover or background made from the video frame. Errors are logged and swallowed.
fn fallback_image<F>(
    dir: &Path,
    video: &Path,
    stem_name: &str,
    default_name: &str,
    what: &str,
    frame: &F,
) where
    F: Fn(&Path) -> io::Result<Option<Vec<u8>>>,
{
    match frame(video) {
        Ok(Some(bytes)) if !bytes.is_empty() => {
            if let Err(error) = write_image(dir, stem_name, default_name, &bytes) {
                tracing::error!(
                    "Couldn't extract {what} from video: {error} ({})",
                    video.display()
                );
            }
        }
        Ok(_) => {}
        Err(error) => tracing::error!(
            "Couldn't extract {what} from video: {error} ({})",
            video.display()
        ),
    }
}

/// `ArchiveAnime.isSupported` on a non-hidden entry. A folder named `x.mkv` is not an episode (E-ANIME-9).
fn is_episode_entry(entry: &FsEntry) -> bool {
    !is_hidden(&entry.name) && !entry.is_dir && is_anime_episode(&entry.name)
}

fn episode_of(anime: &SAnime, entry: &FsEntry) -> SEpisode {
    let name = stem(&entry.name).to_owned();
    SEpisode {
        url: format!("{}/{}", anime.url, entry.name),
        episode_number: episode_number(&anime.title, &name),
        date_upload: entry.modified_ms,
        name,
        ..SEpisode::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io;

    fn show_with(files: &[&str]) -> io::Result<(tempfile::TempDir, SAnime)> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("Show"))?;
        for file in files {
            fs::write(root.path().join("Show").join(file), b"x")?;
        }
        Ok((
            root,
            SAnime {
                url: "Show".into(),
                title: "Show".into(),
                ..SAnime::default()
            },
        ))
    }

    #[test]
    fn episodes_are_numbered_sorted_and_keep_their_url() -> io::Result<()> {
        let (root, anime) = show_with(&[
            "Show - 2.mkv",
            "Show - 10.mp4",
            "Show - 1.mkv",
            "notes.txt",
            ".Show - 3.mkv",
        ])?;
        let got = episode_list(root.path(), &anime).map_err(io::Error::other)?;
        let numbers: Vec<f32> = got.iter().map(|e| e.episode_number).collect();
        assert_eq!(numbers, [10.0, 2.0, 1.0]);
        assert_eq!(got[0].url, "Show/Show - 10.mp4");
        assert_eq!(got[0].name, "Show - 10");
        assert!(got[0].date_upload > 0);
        Ok(())
    }

    #[test]
    fn the_episode_extension_is_case_insensitive() -> io::Result<()> {
        let (root, anime) = show_with(&["EP01.MKV", "EP02.mkv"])?;
        let got = episode_list(root.path(), &anime).map_err(io::Error::other)?;
        assert_eq!(got.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["EP02", "EP01"]);
        Ok(())
    }

    #[test]
    fn a_folder_named_like_an_episode_is_not_one() -> io::Result<()> {
        let (root, anime) = show_with(&[])?;
        fs::create_dir(root.path().join("Show").join("Extra.mkv"))?;
        let got = episode_list(root.path(), &anime).map_err(io::Error::other)?;
        assert!(got.is_empty());
        Ok(())
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_list() -> io::Result<()> {
        let (root, _) = show_with(&[])?;
        let gone = SAnime {
            url: "Gone".into(),
            ..SAnime::default()
        };
        assert!(episode_list(root.path(), &gone).is_err());
        Ok(())
    }

    fn frame_bytes(_: &Path) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(b"jpeg".to_vec()))
    }

    #[test]
    fn t_14_thumbnail_skip_existing() -> io::Result<()> {
        let (root, anime) = show_with(&["Show - 1.mkv", "Show - 2.mkv"])?;
        fs::write(
            root.path().join("Show").join("Show - 2-thumbnail.jpg"),
            b"kept",
        )?;
        let videos = std::cell::RefCell::new(Vec::new());
        let got = episode_list_with(root.path(), &anime, |video| {
            videos.borrow_mut().push(video.to_path_buf());
            frame_bytes(video)
        })
        .map_err(io::Error::other)?;
        let shown = root.path().join("Show");
        // Episode 2 has a thumbnail, so only the lowest episode is ever extracted (its thumbnail, cover, background).
        assert!(
            videos
                .borrow()
                .iter()
                .all(|v| *v == shown.join("Show - 1.mkv")),
            "{:?}",
            videos.borrow()
        );
        assert_eq!(
            fs::read(root.path().join("Show").join("Show - 2-thumbnail.jpg"))?,
            b"kept"
        );
        let urls: Vec<bool> = got.iter().map(|e| e.preview_url.is_some()).collect();
        assert_eq!(urls, [true, true]);
        Ok(())
    }

    #[test]
    fn t_14_thumbnail_name_sanitized() -> io::Result<()> {
        let (root, anime) = show_with(&["Show - 1.mkv"])?;
        let got = episode_list_with(root.path(), &anime, frame_bytes).map_err(io::Error::other)?;
        assert_eq!(got[0].name, "Show - 1");
        let thumb = root.path().join("Show").join("Show - 1-thumbnail.jpg");
        assert_eq!(fs::read(&thumb)?, b"jpeg");
        assert!(
            got[0]
                .preview_url
                .as_deref()
                .is_some_and(|u| u.starts_with("file://"))
        );
        Ok(())
    }

    #[test]
    fn a_frame_failure_never_reaches_the_listing() -> io::Result<()> {
        let (root, anime) = show_with(&["Show - 1.mkv"])?;
        let got = episode_list_with(root.path(), &anime, |_| {
            Err(io::Error::other("ffmpeg died"))
        })
        .map_err(io::Error::other)?;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].preview_url, None);
        assert!(!root.path().join("Show").join("cover.jpg").exists());
        Ok(())
    }

    #[test]
    fn t_14_anime_cover_from_lowest_episode() -> io::Result<()> {
        let (root, anime) = show_with(&["Show - 2.mkv", "Show - 1.mkv"])?;
        let videos = std::cell::RefCell::new(Vec::new());
        episode_list_with(root.path(), &anime, |video| {
            videos.borrow_mut().push(video.to_path_buf());
            frame_bytes(video)
        })
        .map_err(io::Error::other)?;
        let shown = root.path().join("Show");
        assert_eq!(fs::read(shown.join("cover.jpg"))?, b"jpeg");
        assert_eq!(fs::read(shown.join("background.jpg"))?, b"jpeg");
        assert!(shown.join(".nomedia").is_file());
        let last = videos.borrow().last().cloned();
        assert_eq!(
            last,
            Some(shown.join("Show - 1.mkv")),
            "the lowest episode feeds the fallback"
        );
        Ok(())
    }

    #[test]
    fn t_14_anime_existing_cover_and_background_are_never_overwritten() -> io::Result<()> {
        // Deliberate deviation from upstream (behavior 11.5 steps 4-5 read literally): the fallback is gated on
        // the file alone, so a user's cover.* / background.* survives a row with no thumbnail_url.
        let (root, anime) = show_with(&["Show - 1.mkv", "cover.png", "background.png"])?;
        let shown = root.path().join("Show");
        fs::write(shown.join("cover.png"), b"mine")?;
        fs::write(shown.join("background.png"), b"mine")?;
        episode_list_with(root.path(), &anime, frame_bytes).map_err(io::Error::other)?;
        assert_eq!(fs::read(shown.join("cover.png"))?, b"mine");
        assert_eq!(fs::read(shown.join("background.png"))?, b"mine");
        assert!(!shown.join("cover.jpg").exists());
        assert!(!shown.join("background.jpg").exists());
        Ok(())
    }

    #[test]
    fn episodes_json_overrides_name_summary_and_scanlator() -> io::Result<()> {
        let (root, anime) = show_with(&["Show - 1.mkv", "Show - 2.mkv"])?;
        fs::write(
            root.path().join("Show").join("episodes.json"),
            r#"[{"episode_number":1,"name":"Pilot","summary":"S","scanlator":"T","preview_url":"file:///x","fillermark":true}]"#,
        )?;
        let got = episode_list(root.path(), &anime).map_err(io::Error::other)?;
        let one = got
            .iter()
            .find(|e| e.episode_number == 1.0)
            .expect("episode 1");
        assert_eq!(one.name, "Pilot");
        assert_eq!(one.summary.as_deref(), Some("S"));
        assert_eq!(one.scanlator.as_deref(), Some("T"));
        assert!(!one.fillermark);
        let two = got
            .iter()
            .find(|e| e.episode_number == 2.0)
            .expect("episode 2");
        assert_eq!(two.name, "Show - 2");
        Ok(())
    }

    #[test]
    fn a_bad_episodes_json_is_ignored_and_the_files_still_list() -> io::Result<()> {
        let (root, anime) = show_with(&["Show - 1.mkv"])?;
        fs::write(
            root.path().join("Show").join("episodes.json"),
            r#"[{"name":"no number"}]"#,
        )?;
        let got = episode_list(root.path(), &anime).map_err(io::Error::other)?;
        assert_eq!(got.len(), 1);
        Ok(())
    }
}
