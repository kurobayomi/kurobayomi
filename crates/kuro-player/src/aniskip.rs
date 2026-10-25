//! AniSkip client and the AniList to MAL id lookup (area 45 `AniSkipApi.kt`, T-45b-020). Parsing is
//! pure and tested on fixtures. Any transport, status or decode failure is `None`, and nothing is
//! retried or shown to the user.

use std::future::Future;

use bytes::Bytes;
use kuro_net::{Body, NetworkClient};
use serde_json::Value;

use crate::types::{ChapterType, TimeStamp};

const SKIP_TIMES_URL: &str = "https://api.aniskip.com/v2/skip-times";
const ANILIST_URL: &str = "https://graphql.anilist.co";

/// The skip-times GET for one episode. `episode` is the whole episode number (the Kotlin truncates
/// the stored `Double`) and `episode_length` is the player duration in whole seconds.
pub fn skip_times_url(mal_id: i64, episode: i32, episode_length: i64) -> String {
    format!(
        "{SKIP_TIMES_URL}/{mal_id}/{episode}?types[]=ed&types[]=mixed-ed&types[]=mixed-op&types[]=op&types[]=recap&episodeLength={episode_length}"
    )
}

/// Decodes a skip-times body. `found: false`, a missing `results` list, or any malformed item gives
/// `None`. An unknown `skipType` (for example `mixed-ed`) is kept as `Other` with its interval, so
/// one new type does not drop the whole response (E-04).
pub fn parse_skip_times(body: &str) -> Option<Vec<TimeStamp>> {
    let value: Value = serde_json::from_str(body).ok()?;
    if value.get("found")?.as_bool() != Some(true) {
        return None;
    }
    value
        .get("results")?
        .as_array()?
        .iter()
        .map(parse_stamp)
        .collect()
}

fn parse_stamp(item: &Value) -> Option<TimeStamp> {
    let interval = item.get("interval")?;
    let start = interval.get("startTime")?.as_f64()?;
    let end = interval.get("endTime")?.as_f64()?;
    let (name, kind) = match item.get("skipType")?.as_str()? {
        "op" => ("Opening", ChapterType::Opening),
        "ed" => ("Ending", ChapterType::Ending),
        "recap" => ("Recap", ChapterType::Recap),
        "mixed-op" => ("Mixed-op", ChapterType::MixedOp),
        _ => ("", ChapterType::Other),
    };
    Some(TimeStamp {
        start,
        end,
        name: name.to_owned(),
        kind,
    })
}

/// The AniList GraphQL body that asks for an entry's MAL id. The id is interpolated, as the Kotlin
/// does, not sent as a variable.
pub fn anilist_body(anilist_id: i64) -> String {
    format!(r#"{{"query":"query{{\n Media(id:{anilist_id}){{idMal}}\n }}"}}"#)
}

/// The MAL id from an AniList response. `null` or a missing id is `None` (the Kotlin's `0`).
pub fn parse_mal_id(body: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(body).ok()?;
    value.pointer("/data/Media/idMal")?.as_i64()
}

/// A tracker entry of the anime, reduced to what the MAL id lookup needs (T-45b-021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerRef {
    /// MyAnimeList: the remote id is the MAL id.
    MyAnimeList(i64),
    /// AniList: the MAL id comes from the lookup.
    AniList(i64),
    /// Any other tracker has no MAL id.
    Other,
}

/// The MAL id for AniSkip. Only the first track counts: the Kotlin `return` sits inside `map`
/// (`PlayerViewModel.kt:2044`), so the first track's answer is the whole answer. A MAL track gives its remote id, an
/// AniList track asks `anilist_lookup`. A non-MAL first track, or a zero id, gives `None`, so AniSkip is not called.
/// This is kept on purpose as a quirk of the source (T-29a-033).
pub fn resolve_mal_id(
    tracks: &[TrackerRef],
    mut anilist_lookup: impl FnMut(i64) -> Option<i64>,
) -> Option<i64> {
    let id = match *tracks.first()? {
        TrackerRef::MyAnimeList(id) => Some(id),
        TrackerRef::AniList(id) => anilist_lookup(id),
        TrackerRef::Other => None,
    };
    id.filter(|id| *id != 0)
}

/// The `episodeLength` of the skip query: the player duration in whole seconds. A missing, zero or
/// non-finite duration gives `None`, and AniSkip is not called (T-45b-021).
pub fn episode_length(duration: Option<f64>) -> Option<i64> {
    let secs = duration.filter(|d| d.is_finite() && *d > 0.0)?;
    Some(secs as i64)
}

/// Fetches the skip intervals of one episode. Any error gives `None`.
pub async fn fetch_skip_times(
    client: &NetworkClient,
    mal_id: i64,
    episode: i32,
    episode_length: i64,
) -> Option<Vec<TimeStamp>> {
    let url = skip_times_url(mal_id, episode, episode_length);
    let response = client.get(&url).await.ok()?.await_success().ok()?;
    parse_skip_times(&response.text().await.ok()?)
}

/// Looks up the MAL id of an AniList entry. Any error gives `None`.
pub async fn fetch_mal_id(client: &NetworkClient, anilist_id: i64) -> Option<i64> {
    let body = Body::Bytes {
        content_type: "application/json; charset=utf-8".to_owned(),
        data: Bytes::from(anilist_body(anilist_id)),
    };
    let response = client
        .post(ANILIST_URL, body)
        .await
        .ok()?
        .await_success()
        .ok()?;
    parse_mal_id(&response.text().await.ok()?)
}

/// Whether the anime has no binding AniSkip can use, so its MAL id is looked up by title instead.
pub fn needs_title_lookup(tracks: &[TrackerRef]) -> bool {
    !matches!(
        tracks.first(),
        Some(TrackerRef::MyAnimeList(id) | TrackerRef::AniList(id)) if *id != 0
    )
}

/// The AniList search body: one page of the best matches for `title`.
pub fn title_search_body(title: &str) -> String {
    serde_json::json!({
        "query": "query($t:String){Page(perPage:5){media(search:$t,type:ANIME){id idMal title{romaji english native} synonyms seasonYear episodes}}}",
        "variables": { "t": title },
    })
    .to_string()
}

/// Lowercase words only, with "2nd season" written as "season 2" so both spellings compare equal.
fn canon(text: &str) -> String {
    let plain: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { ' ' })
        .collect();
    let words: Vec<&str> = plain.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let ordinal = words[i]
            .strip_suffix("st")
            .or_else(|| words[i].strip_suffix("nd"))
            .or_else(|| words[i].strip_suffix("rd"))
            .or_else(|| words[i].strip_suffix("th"))
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if let (Some(n), Some(&"season")) = (ordinal, words.get(i + 1)) {
            out.push("season".to_owned());
            out.push(n.to_owned());
            i += 2;
        } else {
            out.push(words[i].to_owned());
            i += 1;
        }
    }
    out.join(" ")
}

/// Dice coefficient over character bigrams, 0.0 to 1.0.
fn similarity(a: &str, b: &str) -> f64 {
    let bigrams = |s: &str| -> Vec<(char, char)> {
        let c: Vec<char> = s.chars().collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (mut x, y) = (bigrams(a), bigrams(b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    let total = x.len() + y.len();
    let mut hits = 0;
    for g in &y {
        if let Some(at) = x.iter().position(|h| h == g) {
            x.swap_remove(at);
            hits += 1;
        }
    }
    2.0 * hits as f64 / total as f64
}

/// The MAL id of the AniList search result that is `title`: a title or synonym that is the same text (case, punctuation
/// and "2nd season" / "season 2" spelling aside) in any result wins, else the first result when its closest title is
/// almost identical. Anything else is `None`: a wrong id would show skip times of another show. Outer `None` is a body
/// that did not decode, so the caller does not cache it.
pub fn parse_title_match(body: &str, title: &str) -> Option<Option<i64>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let media = value.pointer("/data/Page/media")?.as_array()?;
    let wanted = canon(title);
    let names = |m: &Value| -> Vec<String> {
        let mut out: Vec<String> = ["romaji", "english", "native"]
            .iter()
            .filter_map(|k| m.pointer(&format!("/title/{k}")).and_then(Value::as_str))
            .map(canon)
            .collect();
        out.extend(m.get("synonyms").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(canon));
        out.retain(|n| !n.is_empty());
        out
    };
    let mal = |m: &Value| m.get("idMal").and_then(Value::as_i64).filter(|id| *id != 0);
    if wanted.is_empty() {
        return Some(None);
    }
    if let Some(m) = media.iter().find(|m| names(m).contains(&wanted)) {
        return Some(mal(m));
    }
    let close = media
        .first()
        .filter(|m| names(m).iter().any(|n| similarity(n, &wanted) >= 0.9));
    Some(close.and_then(mal))
}

/// MAL id by anime title, cached per anime id for the run: a hit for good, a miss for a day. A transport or decode
/// failure is not cached, so the next episode tries again.
// ponytail: the cache lives in memory (one lookup per anime per run), a table if it must survive restarts.
pub async fn mal_id_by_title(client: &NetworkClient, anime_id: i64, title: &str) -> Option<i64> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static CACHE: OnceLock<Mutex<HashMap<i64, (Option<i64>, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((found, at)) = cache.lock().ok()?.get(&anime_id).copied() {
        if found.is_some() || at.elapsed() < Duration::from_secs(24 * 3600) {
            return found;
        }
    }
    let body = Body::Bytes {
        content_type: "application/json; charset=utf-8".to_owned(),
        data: Bytes::from(title_search_body(title)),
    };
    let found = async {
        let response = client.post(ANILIST_URL, body).await.ok()?.await_success().ok()?;
        parse_title_match(&response.text().await.ok()?, title)
    }
    .await;
    match found {
        Some(found) => {
            tracing::debug!(anime_id, ?found, "aniskip: MAL id by title");
            cache.lock().ok()?.insert(anime_id, (found, Instant::now()));
            found
        }
        None => {
            tracing::debug!(anime_id, "aniskip: the title lookup failed");
            None
        }
    }
}

/// The two AniSkip requests the lookup makes, so the orchestration is tested without a network.
/// `NetworkClient` is the real one.
pub trait SkipApi {
    fn mal_id_of(&self, anilist_id: i64) -> impl Future<Output = Option<i64>> + Send;
    fn skip_times(
        &self,
        mal_id: i64,
        episode: i32,
        episode_length: i64,
    ) -> impl Future<Output = Option<Vec<TimeStamp>>> + Send;
}

impl SkipApi for NetworkClient {
    fn mal_id_of(&self, anilist_id: i64) -> impl Future<Output = Option<i64>> + Send {
        fetch_mal_id(self, anilist_id)
    }

    fn skip_times(
        &self,
        mal_id: i64,
        episode: i32,
        episode_length: i64,
    ) -> impl Future<Output = Option<Vec<TimeStamp>>> + Send {
        fetch_skip_times(self, mal_id, episode, episode_length)
    }
}

/// The AniSkip lookup of one episode (T-45b-021): the skip intervals for the first track only, from one skip-times
/// request. Like the Kotlin (`PlayerViewModel.kt:2044`, `return` inside `map`), the first track's answer is the whole
/// answer, so a non-MAL first track makes no AniSkip call even when a MAL track follows (T-29a-033). Nothing is
/// requested when the duration is missing, and no AniSkip request follows a failed AniList lookup. The caller merges
/// the result into the chapters (`Session::apply_aniskip`).
pub async fn lookup(
    api: &impl SkipApi,
    tracks: &[TrackerRef],
    episode_number: f64,
    duration: Option<f64>,
) -> Option<Vec<TimeStamp>> {
    let length = episode_length(duration)?;
    let id = match *tracks.first()? {
        TrackerRef::MyAnimeList(id) => Some(id),
        TrackerRef::AniList(id) => api.mal_id_of(id).await,
        TrackerRef::Other => None,
    };
    let mal_id = id.filter(|id| *id != 0)?;
    api.skip_times(mal_id, episode_number as i32, length).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = r#"{"found":true,"results":[{"interval":{"startTime":1387.996,"endTime":1500},"skipType":"ed","skipId":"b60b5bf4-7fc9-40e9-b178-71d2a4f0e6c9","episodeLength":1500.146},{"interval":{"startTime":28.783,"endTime":118.783},"skipType":"op","skipId":"bf98fb07-c8f8-4399-baa4-1579f03b7e90","episodeLength":1500.04}],"message":"Successfully found skip times","statusCode":200}"#;

    #[test]
    fn url_matches_the_api_template() {
        assert_eq!(
            skip_times_url(21, 1, 1500),
            "https://api.aniskip.com/v2/skip-times/21/1?types[]=ed&types[]=mixed-ed&types[]=mixed-op&types[]=op&types[]=recap&episodeLength=1500"
        );
    }

    #[test]
    fn t36_decodes_the_live_sample() {
        let stamps = parse_skip_times(LIVE).unwrap();
        assert_eq!(stamps.len(), 2);
        assert_eq!(stamps[0].name, "Ending");
        assert_eq!(stamps[0].kind, ChapterType::Ending);
        assert_eq!(stamps[0].start, 1387.996);
        assert_eq!(stamps[1].name, "Opening");
        assert_eq!(stamps[1].kind, ChapterType::Opening);
        assert_eq!(stamps[1].end, 118.783);
    }

    #[test]
    fn t37_not_found_is_none() {
        let body = r#"{"found":false,"results":null,"message":"Not found","statusCode":404}"#;
        assert_eq!(parse_skip_times(body), None);
    }

    #[test]
    fn t38_unknown_skip_type_is_other_and_keeps_the_interval() {
        let body = r#"{"found":true,"results":[{"interval":{"startTime":5,"endTime":9},"skipType":"mixed-ed"}]}"#;
        let stamps = parse_skip_times(body).unwrap();
        assert_eq!(stamps.len(), 1);
        assert_eq!(stamps[0].kind, ChapterType::Other);
        assert_eq!(stamps[0].name, "");
        assert_eq!((stamps[0].start, stamps[0].end), (5.0, 9.0));
    }

    #[test]
    fn malformed_body_is_none() {
        assert_eq!(parse_skip_times("<html>"), None);
        assert_eq!(parse_skip_times(r#"{"found":true}"#), None);
    }

    #[test]
    fn mal_id_from_anilist_body() {
        assert_eq!(parse_mal_id(r#"{"data":{"Media":{"idMal":21}}}"#), Some(21));
        assert_eq!(parse_mal_id(r#"{"data":{"Media":{"idMal":null}}}"#), None);
        assert_eq!(parse_mal_id("nope"), None);
    }

    #[test]
    fn the_first_track_gives_the_mal_id() {
        use TrackerRef::*;
        let never = |_| panic!("no lookup needed");
        assert_eq!(
            resolve_mal_id(&[MyAnimeList(21), AniList(9)], never),
            Some(21)
        );
        assert_eq!(resolve_mal_id(&[], never), None);
        assert_eq!(resolve_mal_id(&[AniList(5)], |id| Some(id * 2)), Some(10));
        assert_eq!(resolve_mal_id(&[AniList(5)], |_| Some(0)), None);
    }

    /// The quirk of T-29a-033: a non-MAL first track returns nothing, even when a MAL track follows.
    #[test]
    fn a_non_mal_first_track_returns_nothing() {
        use TrackerRef::*;
        let never = |_| panic!("no lookup needed");
        assert_eq!(resolve_mal_id(&[Other, MyAnimeList(21)], never), None);
        assert_eq!(resolve_mal_id(&[Other], never), None);
        assert_eq!(resolve_mal_id(&[MyAnimeList(0), MyAnimeList(7)], never), None);
    }

    #[test]
    fn episode_length_is_whole_seconds_or_no_call() {
        assert_eq!(episode_length(Some(1500.9)), Some(1500));
        assert_eq!(episode_length(None), None);
        assert_eq!(episode_length(Some(0.0)), None);
        assert_eq!(episode_length(Some(f64::NAN)), None);
    }

    #[test]
    fn anilist_request_interpolates_the_id() {
        let body: Value = serde_json::from_str(&anilist_body(154587)).unwrap();
        assert_eq!(
            body["query"].as_str().unwrap(),
            "query{\n Media(id:154587){idMal}\n }"
        );
    }

    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        mal_of: Option<i64>,
        calls: Mutex<Vec<String>>,
    }

    impl SkipApi for Fake {
        fn mal_id_of(&self, anilist_id: i64) -> impl Future<Output = Option<i64>> + Send {
            self.calls.lock().unwrap().push(format!("anilist {anilist_id}"));
            std::future::ready(self.mal_of)
        }

        fn skip_times(
            &self,
            mal_id: i64,
            episode: i32,
            length: i64,
        ) -> impl Future<Output = Option<Vec<TimeStamp>>> + Send {
            self.calls
                .lock()
                .unwrap()
                .push(format!("skip {mal_id} {episode} {length}"));
            std::future::ready(parse_skip_times(LIVE))
        }
    }

    #[tokio::test]
    async fn lookup_queries_aniskip_once_for_the_first_track() {
        let api = Fake {
            mal_of: Some(21),
            ..Fake::default()
        };
        let tracks = [TrackerRef::AniList(9), TrackerRef::MyAnimeList(5)];
        let stamps = lookup(&api, &tracks, 3.0, Some(1500.9)).await.unwrap();
        assert_eq!(stamps.len(), 2);
        assert_eq!(*api.calls.lock().unwrap(), vec!["anilist 9", "skip 21 3 1500"]);
    }

    /// T-29a-033: the production lookup keeps the Kotlin quirk. A non-MAL first track makes no call at all.
    #[tokio::test]
    async fn a_non_mal_first_track_makes_no_aniskip_call() {
        let api = Fake {
            mal_of: Some(21),
            ..Fake::default()
        };
        let tracks = [TrackerRef::Other, TrackerRef::MyAnimeList(21)];
        assert_eq!(lookup(&api, &tracks, 3.0, Some(1500.0)).await, None);
        let tracks = [TrackerRef::Other, TrackerRef::AniList(9)];
        assert_eq!(lookup(&api, &tracks, 3.0, Some(1500.0)).await, None);
        assert!(api.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn lookup_makes_no_call_when_the_anilist_lookup_fails_or_the_duration_is_missing() {
        let api = Fake::default();
        assert_eq!(lookup(&api, &[TrackerRef::AniList(9)], 1.0, Some(100.0)).await, None);
        assert_eq!(*api.calls.lock().unwrap(), vec!["anilist 9"]);
        let api = Fake::default();
        assert_eq!(lookup(&api, &[TrackerRef::MyAnimeList(5)], 1.0, None).await, None);
        assert_eq!(lookup(&api, &[], 1.0, Some(100.0)).await, None);
        assert!(api.calls.lock().unwrap().is_empty());
    }

    fn media(id_mal: Option<i64>, romaji: &str, english: &str, synonyms: &[&str]) -> Value {
        serde_json::json!({"id": 1, "idMal": id_mal, "title": {"romaji": romaji, "english": english, "native": null},
            "synonyms": synonyms, "seasonYear": 2020, "episodes": 12})
    }

    fn page(items: Vec<Value>) -> String {
        serde_json::json!({"data": {"Page": {"media": items}}}).to_string()
    }

    #[test]
    fn title_match_is_exact_and_ignores_case() {
        let body = page(vec![media(Some(1), "Other Show", "", &[]), media(Some(21), "One Piece", "ONE PIECE", &[])]);
        assert_eq!(parse_title_match(&body, "one piece!"), Some(Some(21)));
    }

    #[test]
    fn title_match_reads_synonyms() {
        let body = page(vec![media(Some(5114), "Hagane no Renkinjutsushi", "", &["Fullmetal Alchemist: Brotherhood"])]);
        assert_eq!(parse_title_match(&body, "Fullmetal Alchemist Brotherhood"), Some(Some(5114)));
    }

    #[test]
    fn season_spellings_match_each_other_but_not_the_first_season() {
        let body = page(vec![
            media(Some(10), "Foo", "Foo", &[]),
            media(Some(20), "Foo 2nd Season", "Foo Season 2", &[]),
        ]);
        assert_eq!(parse_title_match(&body, "Foo Season 2"), Some(Some(20)));
        assert_eq!(parse_title_match(&body, "Foo 2nd Season"), Some(Some(20)));
        assert_eq!(parse_title_match(&body, "Foo"), Some(Some(10)));
        // only season 1 came back for a season 2 title: no guess
        let only_first = page(vec![media(Some(10), "Foo", "Foo", &[])]);
        assert_eq!(parse_title_match(&only_first, "Foo Season 2"), Some(None));
    }

    #[test]
    fn title_match_never_guesses_wildly() {
        let body = page(vec![media(Some(7), "Totally Different", "", &[])]);
        assert_eq!(parse_title_match(&body, "Foo Bar"), Some(None));
        assert_eq!(parse_title_match(&page(vec![]), "Foo"), Some(None));
        // a near-identical first result is accepted, a MAL-less one gives nothing
        let near = page(vec![media(Some(8), "Kono Subarashii Sekai ni Shukufuku wo!", "", &[])]);
        assert_eq!(parse_title_match(&near, "Kono Subarashii Sekai ni Shukufuku o!"), Some(Some(8)));
        let no_mal = page(vec![media(None, "Foo", "", &[])]);
        assert_eq!(parse_title_match(&no_mal, "Foo"), Some(None));
        assert_eq!(parse_title_match("nope", "Foo"), None);
    }

    #[test]
    fn only_unusable_bindings_need_the_title_lookup() {
        use TrackerRef::*;
        assert!(needs_title_lookup(&[]));
        assert!(needs_title_lookup(&[Other]));
        assert!(needs_title_lookup(&[MyAnimeList(0)]));
        assert!(!needs_title_lookup(&[MyAnimeList(21)]));
        assert!(!needs_title_lookup(&[AniList(5), Other]));
    }

    #[test]
    fn title_search_body_sends_the_title_as_a_variable() {
        let body: Value = serde_json::from_str(&title_search_body("A \"b\"")).unwrap();
        assert_eq!(body["variables"]["t"], "A \"b\"");
    }
}
