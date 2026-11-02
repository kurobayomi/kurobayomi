//! Track flows of AniList (`p1-behavior.md` section 10): the progress-driven status rules, the update that
//! resolves a missing `library_id` first, bind (register) and refresh. The functions change the track in
//! memory only. The caller stores the result in the same write, so a `library_id` learnt here is never lost
//! (E-11). Every operation is over one entry; nothing here touches the local database.

use crate::model::{AnimeTrack, MangaTrack, copy_personal_from_anime, copy_personal_from_manga};

use super::AniList;
use super::convert::{anime_url, manga_url};
use super::error::AniListError;
use super::status::{AnimeStatus, MangaStatus};

/// Progress-driven rules for a manga read event (`p1-behavior.md` 10.1, E-24, E-25). Only applies when
/// `did_read` is true and the status is not COMPLETED.
pub fn apply_manga_read(t: &mut MangaTrack, did_read: bool, now_ms: i64) {
    if t.status == MangaStatus::Completed.to_db() || !did_read {
        return;
    }
    let total = t.total_chapters;
    if t.last_chapter_read as i64 == total && total > 0 {
        t.status = MangaStatus::Completed.to_db();
        t.finish_date = now_ms;
    } else if t.status != MangaStatus::Rereading.to_db() {
        t.status = MangaStatus::Reading.to_db();
        if t.last_chapter_read == 1.0 {
            t.start_date = now_ms;
        }
    }
}

/// Anime form of [`apply_manga_read`], with WATCHING, REWATCHING and COMPLETED.
pub fn apply_anime_watch(t: &mut AnimeTrack, did_watch: bool, now_ms: i64) {
    if t.status == AnimeStatus::Completed.to_db() || !did_watch {
        return;
    }
    let total = t.total_episodes;
    if t.last_episode_seen as i64 == total && total > 0 {
        t.status = AnimeStatus::Completed.to_db();
        t.finish_date = now_ms;
    } else if t.status != AnimeStatus::Rewatching.to_db() {
        t.status = AnimeStatus::Watching.to_db();
        if t.last_episode_seen == 1.0 {
            t.start_date = now_ms;
        }
    }
}

/// Fills `library_id` from the user's list when it is missing (T-28a-027). Not on the list is `NotInList`
/// and no write is attempted.
async fn resolve_manga_library_id(api: &AniList, t: &mut MangaTrack) -> Result<(), AniListError> {
    if t.library_id.is_some_and(|id| id != 0) {
        return Ok(());
    }
    let found = api
        .find_manga_entry(t.remote_id)
        .await?
        .ok_or(AniListError::NotInList)?;
    t.library_id = Some(found.library_id.ok_or(AniListError::NotInList)?);
    Ok(())
}

async fn resolve_anime_library_id(api: &AniList, t: &mut AnimeTrack) -> Result<(), AniListError> {
    if t.library_id.is_some_and(|id| id != 0) {
        return Ok(());
    }
    let found = api
        .find_anime_entry(t.remote_id)
        .await?
        .ok_or(AniListError::NotInList)?;
    t.library_id = Some(found.library_id.ok_or(AniListError::NotInList)?);
    Ok(())
}

/// Update flow for a manga: resolve `library_id`, apply the read rules, then save.
pub async fn update_manga(
    api: &AniList,
    t: &mut MangaTrack,
    did_read: bool,
    now_ms: i64,
) -> Result<(), AniListError> {
    resolve_manga_library_id(api, t).await?;
    apply_manga_read(t, did_read, now_ms);
    api.save_manga(t).await?;
    t.remote_url = manga_url(t.remote_id);
    Ok(())
}

/// Update flow for an anime: resolve `library_id`, apply the watch rules, then save.
pub async fn update_anime(
    api: &AniList,
    t: &mut AnimeTrack,
    did_watch: bool,
    now_ms: i64,
) -> Result<(), AniListError> {
    resolve_anime_library_id(api, t).await?;
    apply_anime_watch(t, did_watch, now_ms);
    api.save_anime(t).await?;
    t.remote_url = anime_url(t.remote_id);
    Ok(())
}

/// Bind of a manga to the local entry (`p1-behavior.md` 10.3). A remote entry copies the personal fields,
/// keeps the local `private` (E-18) unless the remote entry is private, which stays private (a bind, auto-track's
/// included, never makes a hidden entry public), and is saved. No remote entry adds it, and the new id becomes
/// `library_id`.
pub async fn bind_manga(
    api: &AniList,
    t: &mut MangaTrack,
    has_read: bool,
) -> Result<(), AniListError> {
    match api.find_manga_entry(t.remote_id).await? {
        Some(remote) => {
            copy_personal_from_manga(t, &remote, false);
            t.private |= remote.private;
            t.library_id = remote.library_id;
            if t.status != MangaStatus::Completed.to_db() {
                let rereading = t.status == MangaStatus::Rereading.to_db();
                if !rereading && has_read {
                    t.status = MangaStatus::Reading.to_db();
                }
            }
            api.save_manga(t).await?;
        }
        None => {
            let status = if has_read {
                MangaStatus::Reading
            } else {
                MangaStatus::PlanToRead
            };
            t.status = status.to_db();
            t.score = 0.0;
            t.library_id = Some(api.add_manga(t).await?);
        }
    }
    t.remote_url = manga_url(t.remote_id);
    Ok(())
}

/// Bind of an anime. Same rules as [`bind_manga`], with WATCHING, PLAN_TO_WATCH and REWATCHING.
pub async fn bind_anime(
    api: &AniList,
    t: &mut AnimeTrack,
    has_seen: bool,
) -> Result<(), AniListError> {
    match api.find_anime_entry(t.remote_id).await? {
        Some(remote) => {
            copy_personal_from_anime(t, &remote, false);
            t.private |= remote.private;
            t.library_id = remote.library_id;
            if t.status != AnimeStatus::Completed.to_db() {
                let rewatching = t.status == AnimeStatus::Rewatching.to_db();
                if !rewatching && has_seen {
                    t.status = AnimeStatus::Watching.to_db();
                }
            }
            api.save_anime(t).await?;
        }
        None => {
            let status = if has_seen {
                AnimeStatus::Watching
            } else {
                AnimeStatus::PlanToWatch
            };
            t.status = status.to_db();
            t.score = 0.0;
            t.library_id = Some(api.add_anime(t).await?);
        }
    }
    t.remote_url = anime_url(t.remote_id);
    Ok(())
}

/// Remote delete of a manga entry (T-28a-028). The MediaList id is `library_id`, looked up first when it is
/// missing (E-20). An entry that is not on the list is already absent: `Ok`, with an info log (E-21).
pub async fn delete_manga(api: &AniList, t: &MangaTrack) -> Result<(), AniListError> {
    let list_id = match t.library_id.filter(|id| *id != 0) {
        Some(id) => id,
        None => match api.find_manga_entry(t.remote_id).await? {
            Some(found) => match found.library_id {
                Some(id) => id,
                None => return Ok(absent_on_remote(t.remote_id)),
            },
            None => return Ok(absent_on_remote(t.remote_id)),
        },
    };
    if !api.delete_entry(list_id).await? {
        return Ok(absent_on_remote(t.remote_id));
    }
    Ok(())
}

/// Anime form of [`delete_manga`].
pub async fn delete_anime(api: &AniList, t: &AnimeTrack) -> Result<(), AniListError> {
    let list_id = match t.library_id.filter(|id| *id != 0) {
        Some(id) => id,
        None => match api.find_anime_entry(t.remote_id).await? {
            Some(found) => match found.library_id {
                Some(id) => id,
                None => return Ok(absent_on_remote(t.remote_id)),
            },
            None => return Ok(absent_on_remote(t.remote_id)),
        },
    };
    if !api.delete_entry(list_id).await? {
        return Ok(absent_on_remote(t.remote_id));
    }
    Ok(())
}

fn absent_on_remote(media_id: i64) {
    tracing::info!(target: "kurobayomi::tracker::anilist", media_id, "entry already absent on AniList");
}

/// Refresh of a manga from the list (`p1-behavior.md` 10.4). Copies the personal fields including
/// `private`, the title and the total. `library_id` and the link are kept.
pub async fn refresh_manga(api: &AniList, t: &mut MangaTrack) -> Result<(), AniListError> {
    let remote = api
        .find_manga_entry(t.remote_id)
        .await?
        .ok_or(AniListError::NotInList)?;
    copy_personal_from_manga(t, &remote, true);
    t.title = remote.title;
    t.total_chapters = remote.total_chapters;
    Ok(())
}

/// Refresh of an anime. Same rules as [`refresh_manga`].
pub async fn refresh_anime(api: &AniList, t: &mut AnimeTrack) -> Result<(), AniListError> {
    let remote = api
        .find_anime_entry(t.remote_id)
        .await?
        .ok_or(AniListError::NotInList)?;
    copy_personal_from_anime(t, &remote, true);
    t.title = remote.title;
    t.total_episodes = remote.total_episodes;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kuro_prefs::store::PrefStore;
    use serde_json::{Value, json};

    use super::*;
    use crate::fakes::MemSecrets;
    use crate::model::TrackerId;
    use crate::tracker::anilist::api::RawResponse;
    use crate::tracker::anilist::auth::{AccessToken, AuthState};
    use crate::tracker::anilist::date::fuzzy_to_ms;
    use crate::tracker::anilist::score::ScoreFormat;
    use crate::tracker::anilist::testkit::{FakeTransport, ok};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const FIND_FOUND: &str = include_str!("../../../tests/fixtures/anilist/find_manga_found.json");
    const FIND_EMPTY: &str = include_str!("../../../tests/fixtures/anilist/find_manga_empty.json");
    const SAVE_OK: &str = include_str!("../../../tests/fixtures/anilist/save_entry_ok.json");
    const UPDATE_OK: &str = include_str!("../../../tests/fixtures/anilist/update_entry_ok.json");
    const DELETE_OK: &str = include_str!("../../../tests/fixtures/anilist/delete_entry_ok.json");
    const DELETE_ABSENT: &str =
        include_str!("../../../tests/fixtures/anilist/delete_entry_absent.json");
    const FIND_ANIME: &str = include_str!("../../../tests/fixtures/anilist/find_anime_found.json");

    fn client(responses: Vec<RawResponse>) -> (AniList, Arc<FakeTransport>) {
        let fake = FakeTransport::new(responses);
        let state = AuthState::LoggedIn {
            token: AccessToken::new("tok"),
            viewer_id: 4242,
            expires_at: i64::MAX,
            score_format: ScoreFormat::Point10,
        };
        let api = AniList::from_parts(
            fake.clone(),
            MemSecrets::new(),
            &PrefStore::in_memory(),
            state,
        );
        (api, fake)
    }

    fn manga(status: MangaStatus, last: f64, total: i64) -> MangaTrack {
        MangaTrack {
            id: Some(1),
            manga_id: 7,
            tracker_id: TrackerId::ANILIST,
            remote_id: 30001,
            library_id: Some(9100),
            title: "Story Art Manga".into(),
            last_chapter_read: last,
            total_chapters: total,
            status: status.to_db(),
            score: 0.0,
            remote_url: "https://anilist.co/manga/30001".into(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    fn anime(status: AnimeStatus, last: f64, total: i64) -> AnimeTrack {
        AnimeTrack {
            id: Some(1),
            anime_id: 7,
            tracker_id: TrackerId::ANILIST,
            remote_id: 40001,
            library_id: Some(9200),
            title: "Main Show".into(),
            last_episode_seen: last,
            total_episodes: total,
            status: status.to_db(),
            score: 0.0,
            remote_url: "https://anilist.co/anime/40001".into(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    fn body(fake: &FakeTransport, index: usize) -> Value {
        fake.bodies().get(index).cloned().unwrap_or(Value::Null)
    }

    const NOW: i64 = 1_750_000_000_000;

    #[test]
    fn t_28a_024_manga_last_chapter_completes_and_sets_finish() {
        let mut t = manga(MangaStatus::Reading, 12.0, 12);
        apply_manga_read(&mut t, true, NOW);
        assert_eq!(t.status, MangaStatus::Completed.to_db());
        assert_eq!(t.finish_date, NOW, "test 33");
    }

    #[test]
    fn t_28a_024_first_chapter_sets_start_and_a_later_one_does_not() {
        let mut t = manga(MangaStatus::PlanToRead, 1.0, 12);
        apply_manga_read(&mut t, true, NOW);
        assert_eq!(t.status, MangaStatus::Reading.to_db());
        assert_eq!(t.start_date, NOW, "test 34: progress 1.0 sets the start");
        let mut later = manga(MangaStatus::Reading, 5.0, 12);
        apply_manga_read(&mut later, true, NOW);
        assert_eq!(later.start_date, 0);
    }

    #[test]
    fn t_28a_024_rereading_is_kept_on_a_non_final_chapter() {
        let mut t = manga(MangaStatus::Rereading, 5.0, 12);
        apply_manga_read(&mut t, true, NOW);
        assert_eq!(t.status, MangaStatus::Rereading.to_db(), "test 35, E-24");
    }

    #[test]
    fn t_28a_024_paused_anime_becomes_watching_on_an_event() {
        let mut t = anime(AnimeStatus::OnHold, 3.0, 24);
        apply_anime_watch(&mut t, true, NOW);
        assert_eq!(t.status, AnimeStatus::Watching.to_db(), "test 36, E-25");
    }

    #[test]
    fn t_28a_038_rewatching_on_the_last_episode_completes_e26() {
        // E-26 documents the quirk: the final episode completes the entry even while rewatching.
        let mut t = anime(AnimeStatus::Rewatching, 24.0, 24);
        apply_anime_watch(&mut t, true, NOW);
        assert_eq!(t.status, AnimeStatus::Completed.to_db());
        assert_eq!(t.finish_date, NOW);
    }

    #[test]
    fn t_28a_024_completed_anime_is_left_alone() {
        let mut t = anime(AnimeStatus::Completed, 24.0, 24);
        t.finish_date = 5;
        apply_anime_watch(&mut t, true, NOW);
        assert_eq!(t.status, AnimeStatus::Completed.to_db(), "test 37");
        assert_eq!(t.finish_date, 5);
        assert_eq!(t.start_date, 0);
    }

    #[test]
    fn t_28a_024_no_read_event_changes_nothing() {
        let mut t = manga(MangaStatus::PlanToRead, 1.0, 12);
        apply_manga_read(&mut t, false, NOW);
        assert_eq!(t.status, MangaStatus::PlanToRead.to_db());
    }

    #[tokio::test]
    async fn t_28a_027_update_resolves_library_id_and_writes_it_back() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_FOUND), ok(UPDATE_OK)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = None;
        update_manga(&api, &mut t, false, NOW).await?;
        assert_eq!(t.library_id, Some(9100));
        assert_eq!(body(&fake, 1)["variables"]["listId"], json!(9100));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_027_missing_on_the_list_is_not_in_list_and_no_save() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_EMPTY)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = None;
        let r = update_manga(&api, &mut t, false, NOW).await;
        assert!(matches!(r, Err(AniListError::NotInList)), "test 39");
        assert_eq!(fake.calls(), 1, "test 38: no save call");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_021_update_payload_is_integer_and_omits_zero_dates() -> TestResult {
        let (api, fake) = client(vec![ok(UPDATE_OK)]);
        let mut t = manga(MangaStatus::Reading, 3.7, 40);
        t.score = 85.0;
        update_manga(&api, &mut t, false, NOW).await?;
        let vars = body(&fake, 0)["variables"].clone();
        assert_eq!(vars["progress"], json!(3), "E-33 truncates");
        assert_eq!(vars["score"], json!(85));
        assert_eq!(vars["status"], json!("CURRENT"));
        assert_eq!(vars["listId"], json!(9100));
        assert!(vars.get("startedAt").is_none(), "E-29: zero date omitted");
        assert!(vars.get("completedAt").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_021_update_sends_dates_as_local_ymd_when_set() -> TestResult {
        let (api, fake) = client(vec![ok(UPDATE_OK)]);
        let mut t = manga(MangaStatus::Completed, 40.0, 40);
        t.start_date = fuzzy_to_ms(Some(2025), Some(3), Some(10));
        t.finish_date = fuzzy_to_ms(Some(2025), Some(4), Some(2));
        update_manga(&api, &mut t, false, NOW).await?;
        let vars = body(&fake, 0)["variables"].clone();
        assert_eq!(
            vars["startedAt"],
            json!({"year": 2025, "month": 3, "day": 10})
        );
        assert_eq!(
            vars["completedAt"],
            json!({"year": 2025, "month": 4, "day": 2})
        );
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_021_mutation_errors_are_surfaced_on_http_200() -> TestResult {
        let (api, _fake) = client(vec![ok(include_str!(
            "../../../tests/fixtures/anilist/errors_http200.json"
        ))]);
        let t = manga(MangaStatus::Reading, 3.0, 40);
        let r = api.save_manga(&t).await;
        assert!(matches!(r, Err(AniListError::GraphQl(_))), "E-22");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_020_add_sends_four_variables_and_returns_the_list_id() -> TestResult {
        let (api, fake) = client(vec![ok(SAVE_OK)]);
        let mut t = manga(MangaStatus::PlanToRead, 0.0, 0);
        t.library_id = None;
        t.private = true;
        let id = api.add_manga(&t).await?;
        assert_eq!(id, 9001, "E-11");
        let vars = body(&fake, 0)["variables"].clone();
        let keys: Vec<&str> = vec!["mediaId", "progress", "status", "private"];
        assert_eq!(vars.as_object().map(|o| o.len()), Some(keys.len()));
        for k in keys {
            assert!(vars.get(k).is_some(), "missing {k}");
        }
        assert_eq!(vars["mediaId"], json!(30001));
        assert_eq!(vars["status"], json!("PLANNING"));
        assert_eq!(vars["private"], json!(true));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_025_bind_found_copies_remote_and_keeps_a_private_remote_private() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_FOUND), ok(UPDATE_OK)]);
        let mut t = manga(MangaStatus::PlanToRead, 0.0, 40);
        t.library_id = None;
        t.private = false;
        bind_manga(&api, &mut t, false).await?;
        assert_eq!(t.last_chapter_read, 12.0, "copies progress");
        assert_eq!(t.score, 85.0, "copies score");
        assert_eq!(t.status, MangaStatus::Rereading.to_db(), "copies status");
        assert!(t.private, "the remote entry is private, so a public bind does not expose it");
        assert_eq!(body(&fake, 1)["variables"]["private"], json!(true), "the save keeps it private");
        assert_eq!(t.library_id, Some(9100));
        assert_eq!(fake.calls(), 2, "find then save, no add");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_025_bind_found_keeps_a_local_private_choice_over_a_public_remote() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_ANIME), ok(UPDATE_OK)]);
        let mut t = anime(AnimeStatus::PlanToWatch, 0.0, 12);
        t.library_id = None;
        t.private = true;
        bind_anime(&api, &mut t, false).await?;
        assert!(t.private, "E-18: the local private choice is kept");
        assert_eq!(body(&fake, 1)["variables"]["private"], json!(true));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_025_bind_found_rereading_is_kept() -> TestResult {
        let (api, _fake) = client(vec![ok(FIND_FOUND), ok(UPDATE_OK)]);
        let mut t = manga(MangaStatus::PlanToRead, 0.0, 40);
        bind_manga(&api, &mut t, true).await?;
        assert_eq!(t.status, MangaStatus::Rereading.to_db(), "test 41");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_025_bind_not_found_plans_or_reads_and_scores_zero() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_EMPTY), ok(SAVE_OK)]);
        let mut t = manga(MangaStatus::Dropped, 0.0, 40);
        t.library_id = None;
        t.score = 50.0;
        bind_manga(&api, &mut t, false).await?;
        assert_eq!(t.status, MangaStatus::PlanToRead.to_db(), "test 42");
        assert_eq!(t.score, 0.0);
        assert_eq!(body(&fake, 1)["variables"]["status"], json!("PLANNING"));

        let (api, fake) = client(vec![ok(FIND_EMPTY), ok(SAVE_OK)]);
        let mut read = manga(MangaStatus::Dropped, 2.0, 40);
        bind_manga(&api, &mut read, true).await?;
        assert_eq!(read.status, MangaStatus::Reading.to_db());
        assert_eq!(body(&fake, 1)["variables"]["status"], json!("CURRENT"));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_025_bind_add_persists_library_id() -> TestResult {
        let (api, _fake) = client(vec![ok(FIND_EMPTY), ok(SAVE_OK)]);
        let mut t = manga(MangaStatus::PlanToRead, 0.0, 40);
        t.library_id = None;
        bind_manga(&api, &mut t, false).await?;
        assert_eq!(t.library_id, Some(9001), "test 43, E-11");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_025_anime_bind_not_found_uses_watching_rules() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_EMPTY), ok(SAVE_OK)]);
        let mut t = anime(AnimeStatus::Dropped, 0.0, 12);
        t.library_id = None;
        bind_anime(&api, &mut t, true).await?;
        assert_eq!(t.status, AnimeStatus::Watching.to_db());
        assert_eq!(body(&fake, 1)["variables"]["status"], json!("CURRENT"));
        assert_eq!(t.library_id, Some(9001));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_026_refresh_copies_personal_fields_and_keeps_link_and_library_id() -> TestResult
    {
        let (api, _fake) = client(vec![ok(FIND_FOUND)]);
        let mut t = manga(MangaStatus::Reading, 1.0, 3);
        t.remote_url = "old-url".into();
        t.title = "Old".into();
        t.private = false;
        refresh_manga(&api, &mut t).await?;
        assert_eq!(t.last_chapter_read, 12.0);
        assert_eq!(t.score, 85.0);
        assert!(t.private, "copyRemotePrivate defaults to true");
        assert_eq!(t.title, "Story Art Manga");
        assert_eq!(t.total_chapters, 40);
        assert_eq!(t.remote_url, "old-url", "test 44");
        assert_eq!(t.library_id, Some(9100));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_026_refresh_of_a_missing_entry_is_not_in_list() -> TestResult {
        let (api, _fake) = client(vec![ok(FIND_EMPTY)]);
        let mut t = manga(MangaStatus::Reading, 1.0, 3);
        let r = refresh_manga(&api, &mut t).await;
        assert!(matches!(r, Err(AniListError::NotInList)), "test 45");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_026_anime_refresh_copies_episode_total() -> TestResult {
        let (api, _fake) = client(vec![ok(FIND_ANIME)]);
        let mut t = anime(AnimeStatus::Watching, 1.0, 2);
        refresh_anime(&api, &mut t).await?;
        assert_eq!(t.total_episodes, 24);
        assert_eq!(t.last_episode_seen, 4.0);
        assert_eq!(t.status, AnimeStatus::OnHold.to_db());
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_016_find_sends_the_viewer_id() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_EMPTY)]);
        let _ = api.find_manga_entry(30001).await?;
        let vars = body(&fake, 0)["variables"].clone();
        assert_eq!(vars["id"], json!(4242));
        assert_eq!(vars["manga_id"], json!(30001));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_028_delete_sends_the_media_list_id_from_library_id() -> TestResult {
        let (api, fake) = client(vec![ok(DELETE_OK)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = Some(9100);
        delete_manga(&api, &t).await?;
        assert_eq!(fake.calls(), 1);
        assert_eq!(body(&fake, 0)["variables"]["listId"], json!(9100));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_028_missing_library_id_is_found_before_the_delete() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_FOUND), ok(DELETE_OK)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = None;
        delete_manga(&api, &t).await?;
        assert_eq!(body(&fake, 1)["variables"]["listId"], json!(9100));
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_028_entry_not_on_the_list_is_ok_with_no_delete() -> TestResult {
        let (api, fake) = client(vec![ok(FIND_EMPTY)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = None;
        delete_manga(&api, &t).await?;
        assert_eq!(fake.calls(), 1, "only the lookup was sent");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_028_remote_reports_nothing_deleted_is_ok() -> TestResult {
        let (api, _fake) = client(vec![ok(DELETE_ABSENT)]);
        let mut t = anime(AnimeStatus::Watching, 2.0, 12);
        t.library_id = Some(7);
        delete_anime(&api, &t).await?;
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_028_graphql_error_on_delete_is_returned() -> TestResult {
        let (api, _fake) = client(vec![ok(r#"{"errors":[{"message":"denied"}]}"#)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = Some(9100);
        let r = delete_manga(&api, &t).await;
        assert!(matches!(r, Err(AniListError::GraphQl(_))), "{r:?}");
        Ok(())
    }

    #[tokio::test]
    async fn t_28a_039_a_write_sets_the_link_from_the_remote_id() -> TestResult {
        let (api, _fake) = client(vec![ok(FIND_FOUND), ok(UPDATE_OK)]);
        let mut t = manga(MangaStatus::Reading, 3.0, 40);
        t.library_id = None;
        t.remote_url = "stale".into();
        update_manga(&api, &mut t, false, NOW).await?;
        assert_eq!(t.remote_url, manga_url(t.remote_id));
        assert!(t.remote_url.starts_with("https://anilist.co/manga/"));
        Ok(())
    }
}
