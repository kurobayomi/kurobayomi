//! Remote setter rules (`AnimeTracker.kt`, `MangaTracker.kt`). These are free
//! functions so that trackers cannot change the rules. `now_ms` is passed in so tests fix the clock.

use crate::error::{TrackError, redact};
use crate::model::{AnimeTrack, MangaTrack};
use crate::repo::{AnimeTrackRepo, MangaTrackRepo};
use crate::tracker::{AnimeTracker, MangaTracker};

/// Behaviour 3.2. A completed status with a known total sets the progress to the total.
pub fn set_anime_status(t: &mut AnimeTrack, tr: &dyn AnimeTracker, status: i64) {
    t.status = status;
    if status == tr.completion_status() && t.total_episodes != 0 {
        t.last_episode_seen = t.total_episodes as f64;
    }
}

/// Manga form of [`set_anime_status`], over `last_chapter_read` and `total_chapters`.
pub fn set_manga_status(t: &mut MangaTrack, tr: &dyn MangaTracker, status: i64) {
    t.status = status;
    if status == tr.completion_status() && t.total_chapters != 0 {
        t.last_chapter_read = t.total_chapters as f64;
    }
}

/// Behaviour 3.3. `episode` is an `i32` (callers truncate, as the source does). Moves an unstarted entry
/// to watching, unless it is being rewatched. Reaching the total completes it and sets the finish date.
pub fn set_last_episode_seen(t: &mut AnimeTrack, tr: &dyn AnimeTracker, episode: i32, now_ms: i64) {
    if t.last_episode_seen == 0.0
        && t.last_episode_seen < episode as f64
        && t.status != tr.rewatching_status()
    {
        t.status = tr.watching_status();
    }
    t.last_episode_seen = episode as f64;
    if t.total_episodes != 0 && t.last_episode_seen as i64 == t.total_episodes {
        t.status = tr.completion_status();
        t.finish_date = now_ms;
    }
}

/// Manga form of [`set_last_episode_seen`] with `reading_status` and `rereading_status`.
pub fn set_last_chapter_read(t: &mut MangaTrack, tr: &dyn MangaTracker, chapter: i32, now_ms: i64) {
    if t.last_chapter_read == 0.0
        && t.last_chapter_read < chapter as f64
        && t.status != tr.rereading_status()
    {
        t.status = tr.reading_status();
    }
    t.last_chapter_read = chapter as f64;
    if t.total_chapters != 0 && t.last_chapter_read as i64 == t.total_chapters {
        t.status = tr.completion_status();
        t.finish_date = now_ms;
    }
}

/// Behaviour 3.4. The index of `score_text` in the score list becomes the score. A string that is not in
/// the list returns [`TrackError::UnknownScore`] and leaves `t` unchanged (D-8, T-13).
pub fn set_anime_score(
    t: &mut AnimeTrack,
    tr: &dyn AnimeTracker,
    score_text: &str,
) -> Result<(), TrackError> {
    match tr.score_list().iter().position(|s| s == score_text) {
        Some(i) => {
            t.score = tr.index_to_score(i);
            Ok(())
        }
        None => Err(TrackError::UnknownScore(score_text.to_owned())),
    }
}

/// Manga form of [`set_anime_score`].
pub fn set_manga_score(
    t: &mut MangaTrack,
    tr: &dyn MangaTracker,
    score_text: &str,
) -> Result<(), TrackError> {
    match tr.score_list().iter().position(|s| s == score_text) {
        Some(i) => {
            t.score = tr.index_to_score(i);
            Ok(())
        }
        None => Err(TrackError::UnknownScore(score_text.to_owned())),
    }
}

/// Behaviour 3.5. Millis of 0 clears the date.
pub fn set_anime_start_date(t: &mut AnimeTrack, millis: i64) {
    t.start_date = millis;
}

/// See [`set_anime_start_date`].
pub fn set_anime_finish_date(t: &mut AnimeTrack, millis: i64) {
    t.finish_date = millis;
}

/// See [`set_anime_start_date`].
pub fn set_manga_start_date(t: &mut MangaTrack, millis: i64) {
    t.start_date = millis;
}

/// See [`set_anime_start_date`].
pub fn set_manga_finish_date(t: &mut MangaTrack, millis: i64) {
    t.finish_date = millis;
}

/// `t` with the status, progress and total of the track `update` returned, so the stored binding keeps the
/// server's verdict (E-92) and the local personal fields.
pub(crate) fn with_server_anime(t: &AnimeTrack, server: &AnimeTrack) -> AnimeTrack {
    AnimeTrack {
        status: server.status,
        last_episode_seen: server.last_episode_seen,
        total_episodes: server.total_episodes,
        ..t.clone()
    }
}

/// Manga form of [`with_server_anime`].
pub(crate) fn with_server_manga(t: &MangaTrack, server: &MangaTrack) -> MangaTrack {
    MangaTrack {
        status: server.status,
        last_chapter_read: server.last_chapter_read,
        total_chapters: server.total_chapters,
        ..t.clone()
    }
}

/// Behaviour 3.6, the one write path of every setter. The remote `update` runs first. Only then is `t` upserted locally,
/// with the status, progress and total the server returned. A remote failure writes nothing and returns the error.
/// Returns the stored binding with its row id.
pub async fn update_remote_anime(
    t: &AnimeTrack,
    tr: &dyn AnimeTracker,
    repo: &dyn AnimeTrackRepo,
) -> Result<AnimeTrack, TrackError> {
    let server = match tr.update(t, false).await {
        Ok(server) => server,
        Err(e) => {
            tracing::error!(tracker = t.tracker_id.0, error = %redact(&e.to_string()), "remote update failed");
            return Err(e);
        }
    };
    let stored = with_server_anime(t, &server);
    let id = repo.upsert(&stored).await?;
    Ok(AnimeTrack {
        id: Some(id),
        ..stored
    })
}

/// Manga form of [`update_remote_anime`].
pub async fn update_remote_manga(
    t: &MangaTrack,
    tr: &dyn MangaTracker,
    repo: &dyn MangaTrackRepo,
) -> Result<MangaTrack, TrackError> {
    let server = match tr.update(t, false).await {
        Ok(server) => server,
        Err(e) => {
            tracing::error!(tracker = t.tracker_id.0, error = %redact(&e.to_string()), "remote update failed");
            return Err(e);
        }
    };
    let stored = with_server_manga(t, &server);
    let id = repo.upsert(&stored).await?;
    Ok(MangaTrack {
        id: Some(id),
        ..stored
    })
}

/// Behaviour 3.5 private setter: stores the flag, then runs [`update_remote_anime`].
pub async fn set_anime_private(
    t: &AnimeTrack,
    tr: &dyn AnimeTracker,
    repo: &dyn AnimeTrackRepo,
    private: bool,
) -> Result<AnimeTrack, TrackError> {
    let next = AnimeTrack {
        private,
        ..t.clone()
    };
    update_remote_anime(&next, tr, repo).await
}

/// Manga form of [`set_anime_private`].
pub async fn set_manga_private(
    t: &MangaTrack,
    tr: &dyn MangaTracker,
    repo: &dyn MangaTrackRepo,
    private: bool,
) -> Result<MangaTrack, TrackError> {
    let next = MangaTrack {
        private,
        ..t.clone()
    };
    update_remote_manga(&next, tr, repo).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{
        FakeAnime, FakeManga, MemAnimeRepo, MemMangaRepo, MemSecrets, STATUS_COMPLETED,
        STATUS_PLANNED, STATUS_REWATCHING, STATUS_WATCHING, new_log,
    };
    use crate::model::TrackerId;

    const NOW: i64 = 1_700_000_000_000;

    fn anime(last: f64, total: i64, status: i64) -> AnimeTrack {
        AnimeTrack {
            id: Some(1),
            anime_id: 5,
            tracker_id: TrackerId::ANILIST,
            remote_id: 77,
            library_id: None,
            title: "Show".into(),
            last_episode_seen: last,
            total_episodes: total,
            status,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    fn manga(last: f64, total: i64, status: i64) -> MangaTrack {
        MangaTrack {
            id: Some(1),
            manga_id: 5,
            tracker_id: TrackerId::ANILIST,
            remote_id: 77,
            library_id: None,
            title: "Book".into(),
            last_chapter_read: last,
            total_chapters: total,
            status,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    fn tracker(log: crate::fakes::Log) -> FakeAnime {
        FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), log)
    }

    #[test]
    fn tr_27_100_completion_sets_last_to_the_total_only_when_known() {
        let tr = tracker(new_log());
        let mut t = anime(3.0, 12, STATUS_WATCHING);
        set_anime_status(&mut t, &tr, STATUS_COMPLETED);
        assert_eq!((t.status, t.last_episode_seen), (STATUS_COMPLETED, 12.0));
        let mut unknown = anime(3.0, 0, STATUS_WATCHING);
        set_anime_status(&mut unknown, &tr, STATUS_COMPLETED);
        assert_eq!(unknown.last_episode_seen, 3.0);
        let mut other = anime(3.0, 12, STATUS_WATCHING);
        set_anime_status(&mut other, &tr, STATUS_PLANNED);
        assert_eq!(other.last_episode_seen, 3.0);
    }

    #[test]
    fn tr_27_101_first_episode_moves_planned_to_watching() {
        let tr = tracker(new_log());
        let mut t = anime(0.0, 12, STATUS_PLANNED);
        set_last_episode_seen(&mut t, &tr, 1, NOW);
        assert_eq!((t.status, t.last_episode_seen), (STATUS_WATCHING, 1.0));
    }

    #[test]
    fn tr_27_101_episode_zero_changes_nothing() {
        let tr = tracker(new_log());
        let mut t = anime(0.0, 12, STATUS_PLANNED);
        set_last_episode_seen(&mut t, &tr, 0, NOW);
        assert_eq!((t.status, t.last_episode_seen), (STATUS_PLANNED, 0.0));
    }

    #[test]
    fn tr_27_101_rewatching_is_not_moved_to_watching() {
        let tr = tracker(new_log());
        let mut t = anime(0.0, 12, STATUS_REWATCHING);
        set_last_episode_seen(&mut t, &tr, 1, NOW);
        assert_eq!(t.status, STATUS_REWATCHING);
    }

    #[test]
    fn tr_27_102_reaching_the_total_completes_and_stamps_the_finish_date() {
        let tr = tracker(new_log());
        let mut t = anime(11.0, 12, STATUS_WATCHING);
        set_last_episode_seen(&mut t, &tr, 12, NOW);
        assert_eq!((t.status, t.finish_date), (STATUS_COMPLETED, NOW));
        let mut short = anime(10.0, 12, STATUS_WATCHING);
        set_last_episode_seen(&mut short, &tr, 11, NOW);
        assert_eq!((short.status, short.finish_date), (STATUS_WATCHING, 0));
    }

    #[test]
    fn tr_27_103_a_decimal_episode_is_truncated_by_the_caller() {
        let tr = tracker(new_log());
        let mut t = anime(10.0, 12, STATUS_WATCHING);
        set_last_episode_seen(&mut t, &tr, 11.9_f64 as i32, NOW);
        assert_eq!(t.last_episode_seen, 11.0);
        assert_eq!(t.status, STATUS_WATCHING, "11 of 12 does not complete");
    }

    #[test]
    fn tr_27_104_score_maps_the_list_index() {
        let tr = tracker(new_log());
        let mut t = anime(0.0, 0, STATUS_PLANNED);
        set_anime_score(&mut t, &tr, "a").unwrap();
        assert_eq!(t.score, 0.0);
        set_anime_score(&mut t, &tr, "c").unwrap();
        assert_eq!(t.score, 2.0);
    }

    #[test]
    fn t_33a_042_the_server_status_and_progress_replace_the_local_ones() {
        let mut local = manga(5.0, 0, STATUS_WATCHING);
        local.score = 8.0;
        let server = manga(12.0, 12, STATUS_COMPLETED);
        let stored = with_server_manga(&local, &server);
        assert_eq!(
            (
                stored.status,
                stored.last_chapter_read,
                stored.total_chapters
            ),
            (STATUS_COMPLETED, 12.0, 12)
        );
        assert_eq!(stored.score, 8.0, "personal fields stay");
        let mut local = anime(5.0, 0, STATUS_WATCHING);
        local.score = 8.0;
        let stored = with_server_anime(&local, &anime(12.0, 12, STATUS_COMPLETED));
        assert_eq!(
            (
                stored.status,
                stored.last_episode_seen,
                stored.total_episodes,
                stored.score
            ),
            (STATUS_COMPLETED, 12.0, 12, 8.0)
        );
    }

    #[test]
    fn tr_27_009_unknown_score_reports_it_and_leaves_the_record_unchanged() {
        let tr = tracker(new_log());
        let mut t = anime(3.0, 0, STATUS_PLANNED);
        t.score = 7.0;
        let err = set_anime_score(&mut t, &tr, "x").unwrap_err();
        assert_eq!(t.score, 7.0, "T-13: the record is unchanged");
        assert!(matches!(err, TrackError::UnknownScore(ref s) if s == "x"));
    }

    #[test]
    fn tr_27_105_dates_are_stored_as_given_and_zero_clears() {
        let mut t = anime(0.0, 0, STATUS_PLANNED);
        set_anime_start_date(&mut t, 5);
        assert_eq!(t.start_date, 5);
        set_anime_start_date(&mut t, 0);
        assert_eq!(t.start_date, 0);
        set_anime_finish_date(&mut t, NOW);
        assert_eq!(t.finish_date, NOW);
    }

    #[test]
    fn tr_27_008_manga_completion_and_chapter_rules() {
        let tr = FakeManga::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log());
        let mut t = manga(11.0, 12, STATUS_WATCHING);
        set_manga_status(&mut t, &tr, STATUS_COMPLETED);
        assert_eq!(t.last_chapter_read, 12.0);
        let mut r = manga(11.0, 12, STATUS_WATCHING);
        set_last_chapter_read(&mut r, &tr, 12, NOW);
        assert_eq!((r.status, r.finish_date), (STATUS_COMPLETED, NOW));
        let mut p = manga(0.0, 12, STATUS_PLANNED);
        set_last_chapter_read(&mut p, &tr, 1, NOW);
        assert_eq!(p.status, STATUS_WATCHING);
        set_manga_score(&mut p, &tr, "b").unwrap();
        assert_eq!(p.score, 1.0);
        assert!(set_manga_score(&mut p, &tr, "z").is_err());
        assert_eq!(
            p.score, 1.0,
            "D-8: an unknown label leaves the score unchanged"
        );
    }

    #[tokio::test]
    async fn tr_27_106_remote_update_runs_before_the_local_write() {
        let log = new_log();
        let tr = tracker(log.clone());
        let repo = MemAnimeRepo {
            log: Some(log.clone()),
            ..Default::default()
        };
        let t = anime(3.0, 12, STATUS_WATCHING);
        let stored = update_remote_anime(&t, &tr, &repo).await.unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            vec!["update:false".to_string(), "upsert".to_string()]
        );
        assert_eq!(stored.id, Some(1));
    }

    #[tokio::test]
    async fn tr_27_010_remote_failure_writes_nothing_and_keeps_the_raw_message() {
        let log = new_log();
        let tr = tracker(log.clone());
        *tr.fail.lock().unwrap() = Some("rate limited".into());
        let repo = MemAnimeRepo::default();
        let t = anime(3.0, 12, STATUS_WATCHING);
        let err = update_remote_anime(&t, &tr, &repo).await.unwrap_err();
        assert_eq!(err.to_string(), "rate limited");
        assert!(repo.rows.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tr_27_010_set_private_stores_the_flag_through_update() {
        let log = new_log();
        let tr = tracker(log.clone());
        let repo = MemAnimeRepo::default();
        let t = anime(3.0, 12, STATUS_WATCHING);
        let stored = set_anime_private(&t, &tr, &repo, true).await.unwrap();
        assert!(stored.private);
        assert!(repo.rows.lock().unwrap()[0].private);
        assert_eq!(
            log.lock().unwrap().first().map(String::as_str),
            Some("update:false")
        );
    }

    #[tokio::test]
    async fn tr_27_010_manga_update_remote_persists_after_the_remote_call() {
        let log = new_log();
        let tr = FakeManga::new(
            TrackerId::ANILIST,
            "AniList",
            MemSecrets::new(),
            log.clone(),
        );
        let repo = MemMangaRepo {
            log: Some(log.clone()),
            ..Default::default()
        };
        let t = manga(3.0, 12, STATUS_WATCHING);
        let stored = set_manga_private(&t, &tr, &repo, true).await.unwrap();
        assert!(stored.private);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["update:false".to_string(), "upsert".to_string()]
        );
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::fakes::{
        FakeAnime, MemAnimeRepo, MemSecrets, SCORES, STATUS_COMPLETED, STATUS_PLANNED, new_log,
    };
    use crate::model::TrackerId;

    const NOW: i64 = 1_700_000_000_000;

    /// Deterministic generator, so every run checks the same sequences.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn tracker() -> FakeAnime {
        FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log())
    }

    fn planned(total: i64) -> AnimeTrack {
        AnimeTrack {
            id: None,
            anime_id: 1,
            tracker_id: TrackerId::ANILIST,
            remote_id: 1,
            library_id: None,
            title: String::new(),
            last_episode_seen: 0.0,
            total_episodes: total,
            status: STATUS_PLANNED,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    /// TT-27-300. The episodes of a viewing run only move forward, as the picker and the player produce them. Then
    /// completion always means progress equals the total. (A backwards move keeps the status, which is the
    /// source's own rule, behaviour 3.3 table row "completed, 9, 12".)
    #[test]
    fn tr_27_300_a_forward_run_is_completed_exactly_when_it_reaches_the_total() {
        let tr = tracker();
        let mut rng = Lcg(7);
        for _ in 0..200 {
            let total = 1 + rng.below(30) as i64;
            let mut t = planned(total);
            let mut episode = 0_i64;
            while episode < total {
                episode = (episode + 1 + rng.below(3) as i64).min(total);
                set_last_episode_seen(&mut t, &tr, episode as i32, NOW);
                if t.status == STATUS_COMPLETED {
                    assert_eq!(t.last_episode_seen, total as f64);
                }
            }
            assert_eq!(t.status, STATUS_COMPLETED);
            assert_eq!(t.finish_date, NOW);
        }
    }

    /// TT-27-301. Every score string of the list maps to its index, never to -1.0.
    #[test]
    fn tr_27_301_a_score_from_the_list_never_stores_minus_one() {
        let tr = tracker();
        let mut rng = Lcg(3);
        for _ in 0..100 {
            let text = SCORES[rng.below(SCORES.len() as u64) as usize];
            let mut t = planned(12);
            set_anime_score(&mut t, &tr, text).unwrap();
            assert_ne!(t.score, -1.0, "{text}");
            assert_eq!(
                t.score,
                SCORES.iter().position(|s| *s == text).unwrap() as f64
            );
        }
    }

    /// TT-27-067. The failure text carries a code, a state and a token in its redirect query. The log line names the
    /// failure and none of the secrets.
    #[tokio::test]
    async fn tr_27_067_captured_log_never_contains_a_token() {
        let (captured, _guard) = crate::fakes::capture_logs(tracing::Level::DEBUG);

        let tr = tracker();
        *tr.fail.lock().unwrap() = Some(
            "redirect https://x.test/cb?code=SECRET-CODE&state=SECRET-STATE then access_token=SECRET-TOKEN"
                .into(),
        );
        let repo = MemAnimeRepo::default();
        assert!(update_remote_anime(&planned(12), &tr, &repo).await.is_err());

        let text = captured.text();
        assert!(text.contains("remote update failed"), "{text}");
        for secret in ["SECRET-CODE", "SECRET-STATE", "SECRET-TOKEN"] {
            assert!(!text.contains(secret), "{text}");
        }
    }
}
