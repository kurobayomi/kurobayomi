//! The anime episode engine: one episode from the hoster to the finished folder (T-24a-016, with T-24a-049, T-24a-062,
//! T-24a-066 and T-24a-067; behaviour 5.1 to 5.9, edge cases E-F01 to E-F08).
//!
//! The queue actor runs one [`EpisodeEngine`] per active item through the [`EpisodeRunner`] trait. The engine works in
//! a staging folder `<episode folder>_tmp` under the anime folder. ffmpeg writes `<name>.tmp`, which becomes
//! `<name>.mkv` when the remux ends. Finalise renames the staging folder to the episode folder and writes `.nomedia`.
//! A finished `<name>.mkv` in the staging folder is kept, so a restart resumes without running ffmpeg again.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use kuro_platform::kind::Kind;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::anime::ffmpeg::{FFMPEG, FFPROBE, build_remux_args, check_on_path, hls_input_options, probe_duration, run_remux, with_playlist_options};
use crate::anime::model::{AnimeDownload, Video};
use crate::anime::progress::ProgressCell;
use crate::naming::{UnitName, fat_safe, file_component, unit_dir_name, unit_dir_names};
use crate::anime::resolve::{HosterResolver, resolve_video};
use crate::anime::torrent::check_torrent;
use crate::error::DownloadError;
use crate::naming::TMP_DIR_SUFFIX;
use crate::layout::{Layout, rename_dir};
use crate::redact::redacted_headers;
use crate::retry::with_retry;

/// The name ffmpeg's output is finished under (`<name>.mkv`), and the partial name it is written under (`<name>.tmp`).
const FINISHED_EXT: &str = ".mkv";
const PARTIAL_EXT: &str = ".tmp";
/// Written into every finished episode folder so media indexers skip it (behaviour 5.9, T-24a-056).
const NOMEDIA: &str = ".nomedia";

/// Copies a direct video file to `out` when ffmpeg is not installed. The app implements it over its HTTP client. It
/// reports the percent through `progress` and stops when `cancel` fires.
#[async_trait]
pub trait VideoFetch: Send + Sync {
    async fn fetch_to(
        &self,
        url: &str,
        headers: &[(String, String)],
        out: &Path,
        progress: &ProgressCell,
        cancel: &CancellationToken,
    ) -> Result<(), DownloadError>;
}

/// `true` for an HLS playlist, which only ffmpeg can turn into a file. The URL path ends in `.m3u8`.
pub fn is_hls(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".m3u8")
}

/// The extension a direct copy of `url` is finished under: the container of the URL path when it is a known video
/// container, else `.mkv`. A copy only moves bytes, so the name must not claim a container the file does not have.
fn copy_ext(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    [".mkv", ".mp4", ".webm", ".m4v", ".mov", ".avi", ".ts"].into_iter().find(|ext| path.ends_with(ext)).unwrap_or(FINISHED_EXT)
}

/// Downloads one episode. Video through ffmpeg when ffmpeg and ffprobe are on `PATH` (HLS and direct files). Without
/// them a direct file is copied through the [`VideoFetch`] port, and an HLS stream fails with `FfmpegMissing`.
/// `ffmpeg` and `ffprobe` are program names or paths.
pub struct EpisodeEngine {
    resolver: Arc<dyn HosterResolver>,
    ffmpeg: OsString,
    ffprobe: OsString,
    ffmpeg_ready: bool,
    fetch: Option<Arc<dyn VideoFetch>>,
}

impl EpisodeEngine {
    pub fn new(resolver: Arc<dyn HosterResolver>) -> Self {
        Self {
            resolver,
            ffmpeg: FFMPEG.into(),
            ffprobe: FFPROBE.into(),
            ffmpeg_ready: check_on_path().is_ok(),
            fetch: None,
        }
    }

    /// Replaces the ffmpeg and ffprobe programs. Used by tests and by a user-chosen binary.
    pub fn with_programs(mut self, ffmpeg: impl Into<OsString>, ffprobe: impl Into<OsString>) -> Self {
        self.ffmpeg = ffmpeg.into();
        self.ffprobe = ffprobe.into();
        self.ffmpeg_ready = true;
        self
    }

    /// Sets the port that copies a direct video file when ffmpeg is not installed.
    pub fn with_fetch(mut self, fetch: Arc<dyn VideoFetch>) -> Self {
        self.fetch = Some(fetch);
        self
    }

    /// One attempt: probe the duration, then remux into the partial file. A failure is retried by the caller.
    async fn remux_once(
        &self,
        item: &AnimeDownload,
        video: &Video,
        partial: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), DownloadError> {
        let headers = video.headers.clone().unwrap_or_default();
        // Dropping the probe on cancel kills ffprobe (`kill_on_drop`), so a paused run never reaches ffmpeg.
        let duration = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            duration = probe_duration(&self.ffprobe, &video.url, &headers) => duration?,
        };
        // every playlist input (the video and its separate tracks) gets the lenient segment option when it applies
        let lenient = hls_input_options(&self.ffprobe, "x.m3u8");
        let args = with_playlist_options(build_remux_args(video, &[], partial), &lenient);
        debug!(target: "kuro_downloads", episode = item.episode.id, headers = %redacted_headers(&headers), "remuxing episode");
        run_remux(&self.ffmpeg, &args, duration, partial, &item.progress, cancel).await
    }

    /// Makes sure `<stem>.mkv` exists in the staging folder: a finished file from an earlier run is kept (resume), else
    /// the video is remuxed under the retry schedule and the partial file is renamed when it ends. Without ffmpeg a direct
    /// file is copied and keeps its own container extension ([`copy_ext`]), so it is not resumed, only fetched again.
    async fn fetch(
        &self,
        item: &AnimeDownload,
        video: &Video,
        staging: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), DownloadError> {
        let stem = fat_safe(&item.episode.name);
        let mut finished = staging.join(file_component(&stem, FINISHED_EXT));
        if finished.is_file() {
            debug!(target: "kuro_downloads", episode = item.episode.id, "resuming a finished remux");
            return Ok(());
        }
        let partial = staging.join(file_component(&stem, PARTIAL_EXT));
        let _ = fs::remove_file(&partial);
        let headers = video.headers.clone().unwrap_or_default();
        match (self.ffmpeg_ready, self.fetch.as_deref()) {
            (true, _) => with_retry(cancel, || self.remux_once(item, video, &partial, cancel)).await?,
            (false, Some(fetch)) if !is_hls(&video.url) => {
                finished = staging.join(file_component(&stem, copy_ext(&video.url)));
                with_retry(cancel, || fetch.fetch_to(&video.url, &headers, &partial, &item.progress, cancel)).await?
            }
            (false, _) => return Err(DownloadError::FfmpegMissing),
        }
        fs::rename(&partial, &finished)?;
        Ok(())
    }

    /// The staging folder's episode name. A staging folder left by an earlier attempt keeps its name, so the resume finds
    /// it. Otherwise the plain name is used: a finished folder of that name is replaced at the end (B12).
    fn target_name(&self, item: &AnimeDownload, anime_dir: &Path) -> String {
        let unit = UnitName::new(&item.episode.name, item.episode.scanlator.as_deref());
        for candidate in unit_dir_names(Kind::Anime, unit) {
            if anime_dir.join(format!("{candidate}{TMP_DIR_SUFFIX}")).is_dir() {
                return candidate;
            }
        }
        unit_dir_name(Kind::Anime, unit)
    }

    /// Runs one episode inside `anime_dir`: resolve the video, fetch it into the staging folder, finalise. Returns the
    /// name of the finished folder. `anime::runner` calls this.
    pub(crate) async fn download(
        &self,
        item: &AnimeDownload,
        anime_dir: &Path,
        cancel: &CancellationToken,
    ) -> Result<String, DownloadError> {
        let target = self.target_name(item, anime_dir);
        let staging = anime_dir.join(format!("{target}{TMP_DIR_SUFFIX}"));

        let video = resolve_video(item, self.resolver.as_ref(), cancel).await?;
        check_torrent(&video.url)?;
        fs::create_dir_all(&staging)?;
        self.fetch(item, &video, &staging, cancel).await?;
        if !is_download_successful(&staging) {
            return Err(DownloadError::DownloadIncomplete(format!("{} is not a single video file", staging.display())));
        }
        finalise(&staging, &anime_dir.join(&target))?;
        Ok(target)
    }
}

/// `isDownloadSuccessful` (T-24a-049, E-F06): leftover `.tmp` files are removed first, then the folder must hold
/// exactly one entry, a file whose name does not end in `.tmp`. A `.mkv` alone is a success, two `.mkv` files a failure.
pub fn is_download_successful(staging: &Path) -> bool {
    let Ok(entries) = fs::read_dir(staging) else {
        return false;
    };
    let mut kept: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(PARTIAL_EXT)) {
            let _ = fs::remove_file(&path);
        } else {
            kept.push(path);
        }
    }
    match kept.as_slice() {
        [only] => only.is_file() && !only.to_string_lossy().ends_with(PARTIAL_EXT),
        _ => false,
    }
}

/// Renames the staging folder to its episode folder and writes `.nomedia` into it (behaviour 5.9). A finished folder or
/// file of the same name is replaced: the user asked for the episode again (decision 3 of 1b, B12).
fn finalise(staging: &Path, dest: &Path) -> Result<(), DownloadError> {
    Layout::remove_path(dest)?;
    rename_dir(staging, dest)?;
    if let Err(err) = fs::write(dest.join(NOMEDIA), b"") {
        warn!(target: "kuro_downloads", error = %err, "could not write .nomedia");
    }
    Ok(())
}

/// Startup sweep (T-24a-062, E-F05). Under `root`, every `<name>_tmp` staging folder is removed, except the ones in
/// `keep`, which belong to restored queue items. In a kept folder only the partial `.tmp` files go, so a finished
/// `.mkv` is left for the resume. Returns how many entries were removed.
pub fn sweep_staging(root: &Path, keep: &[PathBuf]) -> usize {
    let mut removed = 0;
    for source in subdirs(root) {
        for anime in subdirs(&source) {
            for staging in subdirs(&anime) {
                if !staging.file_name().is_some_and(|n| n.to_string_lossy().ends_with(TMP_DIR_SUFFIX)) {
                    continue;
                }
                if keep.contains(&staging) {
                    for file in subdirs_and_files(&staging) {
                        if file.is_file()
                            && file.to_string_lossy().ends_with(PARTIAL_EXT)
                            && fs::remove_file(&file).is_ok()
                        {
                            removed += 1;
                        }
                    }
                } else if fs::remove_dir_all(&staging).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    removed
}

fn subdirs(path: &Path) -> Vec<PathBuf> {
    subdirs_and_files(path).into_iter().filter(|p| p.is_dir()).collect()
}

fn subdirs_and_files(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anime::model::{AnimeRef, AnimeSource, EpisodeRef};
    use crate::model::DownloadState;
    use async_trait::async_trait;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    static SPAWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[cfg(unix)]
    async fn spawn_guard() -> tokio::sync::MutexGuard<'static, ()> {
        SPAWN_LOCK.lock().await
    }

    /// One download of `item` into its anime folder under `root`, as `anime::runner` does it.
    async fn run(engine: &EpisodeEngine, root: &Path, item: Arc<AnimeDownload>, cancel: CancellationToken) -> Result<(), DownloadError> {
        let layout = Layout::new(Kind::Anime, root);
        let anime_dir = layout.entry_dir(&Layout::source_dir_name(&item.source.name, &item.source.lang), &item.anime.title)?;
        engine.download(&item, &anime_dir, &cancel).await.map(|_| ())
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    struct Hoster {
        answer: Option<Video>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl HosterResolver for Hoster {
        async fn best_video(
            &self,
            _: &AnimeRef,
            _: &EpisodeRef,
            _: &AnimeSource,
        ) -> Result<Option<Video>, DownloadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.answer.clone())
        }
    }

    fn video() -> Video {
        Video { url: "/local/source.mkv".into(), title: "v".into(), ..Default::default() }
    }

    #[test]
    fn a_direct_copy_keeps_the_container_of_its_url() {
        assert_eq!(copy_ext("https://cdn.example/ep1.MP4?x=1"), ".mp4");
        assert_eq!(copy_ext("https://cdn.example/ep1.webm#t"), ".webm");
        assert_eq!(copy_ext("https://cdn.example/stream"), ".mkv");
    }

    #[test]
    fn only_a_playlist_url_is_hls() {
        assert!(is_hls("https://cdn.example/master.m3u8?token=1"));
        assert!(is_hls("https://cdn.example/A.M3U8#x"));
        assert!(!is_hls("https://cdn.example/episode.mp4?name=master.m3u8.bak"));
        assert!(!is_hls("/local/source.mkv"));
    }

    fn source(root_name: &str) -> AnimeSource {
        AnimeSource { id: 1, name: root_name.into(), lang: "en".into(), is_http: true, unmetered: false }
    }

    fn anime() -> AnimeRef {
        AnimeRef { id: 10, source_id: 1, url: "/a".into(), title: "Show".into() }
    }

    fn episode(id: i64, name: &str) -> EpisodeRef {
        EpisodeRef {
            id,
            anime_id: 10,
            url: format!("/e/{id}"),
            name: name.into(),
            scanlator: None,
            seen: false,
            bookmark: false,
            fillermark: false,
            source_order: id,
            upload_date: 0,
            episode_number: id as f64,
        }
    }

    fn download(id: i64, name: &str) -> Arc<AnimeDownload> {
        Arc::new(AnimeDownload::new(source("Src"), anime(), episode(id, name), false, None))
    }

    /// A fake ffprobe that prints a duration, and a fake ffmpeg that logs each run and writes the partial file.
    /// `exit_code` is the ffmpeg exit status.
    #[cfg(unix)]
    fn programs(dir: &Path, exit_code: i32, partial: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let log = dir.join("ffmpeg.log");
        let ffprobe = script(dir, "ffprobe", "echo 30.0");
        let body = format!(
            "echo run >> '{}'\nprintf 'video' > '{}'\nprintf 'out_time_us=30000000\\nprogress=end\\n'\nexit {exit_code}",
            log.display(),
            partial.display()
        );
        let ffmpeg = script(dir, "ffmpeg", &body);
        (ffmpeg, ffprobe, log)
    }

    #[cfg(unix)]
    fn runs(log: &Path) -> usize {
        fs::read_to_string(log).map(|s| s.lines().count()).unwrap_or(0)
    }

    fn engine(_root: &Path, ffmpeg: &Path, ffprobe: &Path) -> (EpisodeEngine, Arc<Hoster>) {
        let hoster = Arc::new(Hoster { answer: Some(video()), calls: AtomicUsize::new(0) });
        let engine = EpisodeEngine::new(hoster.clone())
            .with_programs(ffmpeg.as_os_str(), ffprobe.as_os_str());
        (engine, hoster)
    }

    /// The anime folder that the engine uses for `download(…, "Show")` under the fake source.
    #[cfg(unix)]
    fn anime_folder(root: &Path) -> PathBuf {
        root.join("Src (EN)").join("Show")
    }

    #[test]
    fn a_single_mkv_is_a_successful_download() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("Episode 1.mkv"), b"x").unwrap();
        assert!(is_download_successful(tmp.path()));
    }

    #[test]
    fn a_leftover_tmp_is_removed_before_the_check() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("Episode 1.mkv"), b"x").unwrap();
        fs::write(tmp.path().join("Episode 1.tmp"), b"partial").unwrap();
        assert!(is_download_successful(tmp.path()));
        assert!(!tmp.path().join("Episode 1.tmp").exists(), "the leftover is gone");
    }

    #[test]
    fn two_files_or_only_a_tmp_is_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.mkv"), b"x").unwrap();
        fs::write(tmp.path().join("b.mkv"), b"x").unwrap();
        assert!(!is_download_successful(tmp.path()));
        let only_tmp = tempfile::tempdir().unwrap();
        fs::write(only_tmp.path().join("a.tmp"), b"x").unwrap();
        assert!(!is_download_successful(only_tmp.path()));
    }

    #[test]
    fn the_startup_sweep_removes_orphans_and_keeps_restored_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let anime_dir = root.join("Src (EN)").join("Show");
        let kept = anime_dir.join("Episode 1_tmp");
        let orphan = anime_dir.join("Episode 2_tmp");
        let finished_dir = anime_dir.join("Episode 3");
        fs::create_dir_all(&kept).unwrap();
        fs::create_dir_all(&orphan).unwrap();
        fs::create_dir_all(&finished_dir).unwrap();
        fs::write(kept.join("Episode 1.mkv"), b"x").unwrap();
        fs::write(kept.join("Episode 1.tmp"), b"partial").unwrap();
        fs::write(orphan.join("Episode 2.mkv"), b"x").unwrap();
        fs::write(finished_dir.join("Episode 3.mkv"), b"x").unwrap();

        let removed = sweep_staging(root, std::slice::from_ref(&kept));

        assert!(!orphan.exists(), "a staging folder of no restored item is removed");
        assert!(kept.join("Episode 1.mkv").is_file(), "the finished remux of a restored item stays for resume");
        assert!(!kept.join("Episode 1.tmp").exists(), "the partial file goes");
        assert!(finished_dir.join("Episode 3.mkv").is_file(), "finished episodes are never touched");
        assert_eq!(removed, 2, "one orphan folder and one partial file");
    }

    #[test]
    fn the_startup_sweep_on_the_tmp_fixture_tree_keeps_the_finished_remux_of_a_restored_item() {
        let tree = crate::testing::dl_fixture("tree_tmp");
        let show = tree.path().join("Example (EN)").join("Show");
        let kept = show.join("Episode 4_tmp");

        let removed = sweep_staging(tree.path(), std::slice::from_ref(&kept));

        assert!(!show.join("Episode 3_tmp").exists(), "the orphan with a partial file is removed");
        assert!(kept.join("Episode 4.mkv").is_file(), "the restored item keeps its finished remux");
        assert_eq!(removed, 1);
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn a_download_is_remuxed_then_finalised_with_nomedia() {
        let _guard = spawn_guard().await;
        let tmp = tempfile::tempdir().unwrap();
        let folder = anime_folder(tmp.path());
        fs::create_dir_all(&folder).unwrap();
        let partial = folder.join("Episode 1_tmp").join("Episode 1.tmp");
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        let (ffmpeg, ffprobe, log) = programs(tmp.path(), 0, &partial);
        let (engine, hoster) = engine(tmp.path(), &ffmpeg, &ffprobe);
        let item = download(1, "Episode 1");

        run(&engine, tmp.path(), item.clone(), CancellationToken::new()).await.unwrap();

        assert_eq!(runs(&log), 1);
        assert_eq!(hoster.calls.load(Ordering::SeqCst), 1);
        assert!(folder.join("Episode 1").join("Episode 1.mkv").is_file(), "the finished file is in the episode folder");
        assert!(folder.join("Episode 1").join(NOMEDIA).is_file(), "the .nomedia file is written");
        assert!(!folder.join("Episode 1_tmp").exists(), "staging is gone");
        assert_eq!(item.status(), DownloadState::Queue, "the engine does not set the queue status itself");
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn a_failing_ffmpeg_is_tried_four_times_and_keeps_no_partial() {
        let _guard = spawn_guard().await;
        let tmp = tempfile::tempdir().unwrap();
        let folder = anime_folder(tmp.path());
        let partial = folder.join("Episode 1_tmp").join("Episode 1.tmp");
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        let (ffmpeg, ffprobe, log) = programs(tmp.path(), 1, &partial);
        let (engine, _) = engine(tmp.path(), &ffmpeg, &ffprobe);

        let err = run(&engine, tmp.path(), download(1, "Episode 1"), CancellationToken::new()).await.unwrap_err();

        assert_eq!(err.to_string(), "Error in ffmpeg!");
        assert_eq!(runs(&log), 4, "one attempt and three retries");
        assert!(!partial.exists(), "the partial file is removed");
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn a_finished_remux_from_an_earlier_run_is_resumed_without_ffmpeg() {
        let _guard = spawn_guard().await;
        let tmp = tempfile::tempdir().unwrap();
        let folder = anime_folder(tmp.path());
        let staging = folder.join("Episode 1_tmp");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("Episode 1.mkv"), b"done").unwrap();
        let (ffmpeg, ffprobe, log) = programs(tmp.path(), 1, &staging.join("Episode 1.tmp"));
        let (engine, _) = engine(tmp.path(), &ffmpeg, &ffprobe);

        run(&engine, tmp.path(), download(1, "Episode 1"), CancellationToken::new()).await.unwrap();

        assert_eq!(runs(&log), 0, "ffmpeg is not run again");
        assert!(folder.join("Episode 1").join("Episode 1.mkv").is_file());
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn an_existing_non_empty_episode_folder_is_replaced_by_the_finished_one() {
        let _guard = spawn_guard().await;
        let tmp = tempfile::tempdir().unwrap();
        let folder = anime_folder(tmp.path());
        fs::create_dir_all(folder.join("Episode 1")).unwrap();
        fs::write(folder.join("Episode 1").join("old.mkv"), b"old").unwrap();
        let partial = folder.join("Episode 1_tmp").join("Episode 1.tmp");
        let (ffmpeg, ffprobe, _) = programs(tmp.path(), 0, &partial);
        let (engine, _) = engine(tmp.path(), &ffmpeg, &ffprobe);

        // The plain name is taken by another non-empty folder: the new episode replaces it (B12, E-F03 flipped).
        let outcome = run(&engine, tmp.path(), download(1, "Episode 1"), CancellationToken::new()).await;
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(folder.join("Episode 1").join("Episode 1.mkv").is_file());
        assert!(!folder.join("Episode 1").join("old.mkv").exists(), "the old copy is gone");
        assert!(!folder.join("Episode 1 (1)").exists(), "no second folder named with the episode id");

        // A finalise onto a non-empty folder replaces it too (E-F02 flipped).
        let staging = folder.join("Episode 2_tmp");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("Episode 2.mkv"), b"new").unwrap();
        fs::create_dir_all(folder.join("Episode 2")).unwrap();
        fs::write(folder.join("Episode 2").join("old.mkv"), b"old").unwrap();
        finalise(&staging, &folder.join("Episode 2")).unwrap();
        assert!(folder.join("Episode 2").join("Episode 2.mkv").is_file());
        assert!(!folder.join("Episode 2").join("old.mkv").exists());
        assert!(!staging.exists(), "the staging folder was moved into place");
    }

    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn the_file_component_is_cut_to_200_bytes_and_the_folder_keeps_its_name() {
        let _guard = spawn_guard().await;
        let tmp = tempfile::tempdir().unwrap();
        let folder = anime_folder(tmp.path());
        // 120 two-byte characters: a 240-byte folder name, but a file name that must be cut to 200 bytes.
        let long = "\u{e9}".repeat(120);
        let partial = folder.join(format!("{long}_tmp")).join(file_component(&long, PARTIAL_EXT));
        let (ffmpeg, ffprobe, _) = programs(tmp.path(), 0, &partial);
        let (engine, _) = engine(tmp.path(), &ffmpeg, &ffprobe);

        run(&engine, tmp.path(), download(1, &long), CancellationToken::new()).await.unwrap();

        let finished = folder.join(&long).join(file_component(&long, FINISHED_EXT));
        assert!(finished.is_file(), "the finished file exists under the cut name");
        assert!(finished.file_name().unwrap().len() <= 200, "the file component is at most 200 bytes");
    }

    #[tokio::test(start_paused = true)]
    async fn a_missing_ffmpeg_fails_at_once_with_ffmpeg_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let (engine, _) =
            engine(tmp.path(), Path::new("/nonexistent/kuro-ffmpeg"), Path::new("/nonexistent/kuro-ffprobe"));
        let err = run(&engine, tmp.path(), download(1, "Episode 1"), CancellationToken::new()).await.unwrap_err();
        assert!(matches!(err, DownloadError::FfmpegMissing), "got {err:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn no_video_from_the_hoster_is_no_video() {
        let tmp = tempfile::tempdir().unwrap();
        let hoster = Arc::new(Hoster { answer: None, calls: AtomicUsize::new(0) });
        let engine = EpisodeEngine::new(hoster);
        let err = run(&engine, tmp.path(), download(1, "Episode 1"), CancellationToken::new()).await.unwrap_err();
        assert!(matches!(err, DownloadError::NoVideo));
    }

    /// A pause during the probe kills ffprobe: ffmpeg never starts, and the partial file that a newer run has started
    /// meanwhile is left alone (T-24a-058).
    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_during_the_probe_starts_no_ffmpeg_and_leaves_a_newer_partial() {
        let _guard = spawn_guard().await;
        let tmp = tempfile::tempdir().unwrap();
        let folder = anime_folder(tmp.path());
        let partial = folder.join("Episode 1_tmp").join("Episode 1.tmp");
        let (ffmpeg, _, log) = programs(tmp.path(), 0, &partial);
        let slow_probe = script(tmp.path(), "slow-ffprobe", "exec sleep 5");
        let (engine, _) = engine(tmp.path(), &ffmpeg, &slow_probe);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let newer = partial.clone();
        tokio::spawn(async move {
            // The newer run's partial appears while the older run is still probing, then the older run is cancelled.
            tokio::time::sleep(Duration::from_millis(100)).await;
            fs::write(&newer, b"newer run").unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            trigger.cancel();
        });

        let started = std::time::Instant::now();
        let err = run(&engine, tmp.path(), download(1, "Episode 1"), cancel).await.unwrap_err();

        assert!(matches!(err, DownloadError::Cancelled), "got {err:?}");
        assert!(started.elapsed() < Duration::from_secs(4), "the probe is killed, not waited for");
        assert_eq!(runs(&log), 0, "ffmpeg was never started");
        assert_eq!(fs::read(&partial).unwrap(), b"newer run", "the newer run's partial is untouched");
    }
}
