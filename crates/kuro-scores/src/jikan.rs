//! The MyAnimeList score through Jikan (`api.jikan.moe`, an unofficial MyAnimeList API: no login, anime and manga). It
//! answers one title per request and allows 60 requests a minute, so every request here waits its turn on one pace of
//! a request each 1.5 seconds; a library fills in over minutes, in the background.
//!
//! An entry is asked by the most exact thing known about it, in this order:
//! 1. the title the user chose for it;
//! 2. its MyAnimeList binding, whose id is the id Jikan takes;
//! 3. the match stored before;
//! 4. its Shikimori binding, or its AniList binding turned into a MyAnimeList id by AniList itself. Neither id is
//!    known to be a MyAnimeList id for certain, so the answer counts only when its names include the entry's title;
//! 5. a search by name: the entry's title, then its shorter forms (`title_queries`), each accepted only when exactly
//!    one result has the same name (`pick`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kuro_data::community_score::MatchedBy;
use kuro_platform::kind::Kind;
use kuro_track::auto_bind::{pick, title_queries};
use kuro_track::model::TrackerId;
use kuro_track::tracker::anilist::api::{RateLimiter, Transport, anilist_to_mal_id, user_agent};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::provider::{Hit, Outcome, Query, ScoreProvider, Stop};

/// The name stored with a score from here.
pub const ID: &str = "jikan";
pub const BASE: &str = "https://api.jikan.moe/v4";
/// One request each: 40 a minute, under Jikan's 60 a minute and 3 a second.
pub const PACE: Duration = Duration::from_millis(1500);
/// Results asked of a search by name: enough to see two titles of one name, which is then no match.
const MATCH_RESULTS: &str = "5";
/// Results asked of a search the user reads.
const SHOWN_RESULTS: &str = "15";

pub struct Jikan {
    client: reqwest::Client,
    base: String,
    limiter: RateLimiter,
    anilist: Arc<dyn Transport>,
}

#[derive(Deserialize)]
struct One {
    data: Item,
}

#[derive(Deserialize)]
struct Many {
    #[serde(default)]
    data: Vec<Item>,
}

/// The fields read of Jikan's `anime` and `manga` objects. Everything but the id may be missing or null.
#[derive(Deserialize)]
struct Item {
    mal_id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    title_english: Option<String>,
    #[serde(default)]
    title_japanese: Option<String>,
    #[serde(default)]
    title_synonyms: Option<Vec<String>>,
    #[serde(default)]
    titles: Option<Vec<Name>>,
    #[serde(default)]
    score: Option<f64>,
}

#[derive(Deserialize)]
struct Name {
    #[serde(default)]
    title: Option<String>,
}

impl From<Item> for Hit {
    fn from(item: Item) -> Self {
        let named = item.titles.unwrap_or_default().into_iter().filter_map(|name| name.title);
        let mut names: Vec<String> = Vec::new();
        for name in item.title.into_iter().chain(item.title_english).chain(item.title_japanese).chain(item.title_synonyms.unwrap_or_default()).chain(named) {
            let name = name.trim().to_owned();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        let title = if names.is_empty() { String::new() } else { names.remove(0) };
        // an unrated title comes as null, or as 0 on some pages
        Hit { remote_id: item.mal_id, title, alt_titles: names, score: item.score.filter(|score| *score > 0.0) }
    }
}

impl Jikan {
    /// `anilist` turns an AniList id into a MyAnimeList id, without a login.
    pub fn new(anilist: Arc<dyn Transport>) -> Result<Self, reqwest::Error> {
        Self::with_base(BASE, anilist)
    }

    pub fn with_base(base: &str, anilist: Arc<dyn Transport>) -> Result<Self, reqwest::Error> {
        Self::with_pace(base, anilist, PACE)
    }

    /// `pace` is the time kept between two requests (the tests, against a local server, keep none).
    pub fn with_pace(base: &str, anilist: Arc<dyn Transport>, pace: Duration) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).user_agent(user_agent()).build()?;
        Ok(Self { client, base: base.trim_end_matches('/').to_owned(), limiter: RateLimiter::new(1, pace), anilist })
    }

    /// One paced GET. `None` for a title the site does not have (404) or a request it does not take (another 4xx).
    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<T>, Stop> {
        self.limiter.acquire().await;
        let response = self.client.get(format!("{}/{path}", self.base)).query(query).send().await.map_err(|error| {
            tracing::debug!(%error, "jikan: no answer");
            Stop::Unreachable
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
            return Err(Stop::RateLimited { retry_after });
        }
        if status.is_server_error() {
            return Err(Stop::Unreachable);
        }
        if !status.is_success() {
            return Ok(None);
        }
        response.json::<T>().await.map(Some).map_err(|error| {
            tracing::debug!(%error, "jikan: an answer that cannot be read");
            Stop::Unreachable
        })
    }

    async fn by_id(&self, kind: Kind, id: i64) -> Result<Option<Hit>, Stop> {
        Ok(self.get::<One>(&format!("{}/{id}", kind.as_str()), &[]).await?.map(|one| Hit::from(one.data)))
    }

    async fn search_n(&self, kind: Kind, text: &str, results: &str) -> Result<Vec<Hit>, Stop> {
        let found = self.get::<Many>(kind.as_str(), &[("q", text), ("limit", results)]).await?;
        Ok(found.map(|many| many.data.into_iter().map(Hit::from).collect()).unwrap_or_default())
    }

    /// The title of `id` when its names include one of `titles`: an id that came from another site is not trusted alone.
    async fn by_id_named(&self, kind: Kind, id: i64, titles: &[&str]) -> Result<Option<Hit>, Stop> {
        let Some(hit) = self.by_id(kind, id).await? else { return Ok(None) };
        let same = titles.iter().any(|title| pick(title, [(hit.title.as_str(), hit.alt_titles.as_slice())]).is_some());
        Ok(same.then_some(hit))
    }
}

#[async_trait]
impl ScoreProvider for Jikan {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports(&self, _kind: Kind) -> bool {
        true
    }

    async fn lookup(&self, query: &Query) -> Result<Outcome, Stop> {
        let kind = query.kind;
        let found = |hit: Hit, by: MatchedBy| Ok(Outcome::Found { hit, by });
        if let Some((id, MatchedBy::User)) = query.known {
            return match self.by_id(kind, id).await? {
                Some(hit) => found(hit, MatchedBy::User),
                None => Ok(Outcome::NotFound),
            };
        }
        if let Some(track) = query.tracks.iter().find(|t| t.tracker == TrackerId::MYANIMELIST && t.remote_id > 0) {
            if let Some(hit) = self.by_id(kind, track.remote_id).await? {
                return found(hit, MatchedBy::Tracker);
            }
        }
        if let Some((id, by)) = query.known {
            if let Some(hit) = self.by_id(kind, id).await? {
                return found(hit, by);
            }
        }
        for track in query.tracks.iter().filter(|t| t.remote_id > 0) {
            let id = match track.tracker {
                TrackerId::SHIKIMORI => Some(track.remote_id),
                TrackerId::ANILIST => {
                    self.limiter.acquire().await;
                    anilist_to_mal_id(self.anilist.as_ref(), track.remote_id).await
                }
                _ => None,
            };
            let Some(id) = id else { continue };
            if let Some(hit) = self.by_id_named(kind, id, &[query.title.as_str(), track.title.as_str()]).await? {
                return found(hit, MatchedBy::Tracker);
            }
        }
        for text in title_queries(&query.title) {
            let mut hits = self.search_n(kind, &text, MATCH_RESULTS).await?;
            let one = pick(&query.title, hits.iter().map(|hit| (hit.title.as_str(), hit.alt_titles.as_slice())));
            if let Some(index) = one {
                return found(hits.swap_remove(index), MatchedBy::Title);
            }
        }
        Ok(Outcome::NotFound)
    }

    async fn search(&self, kind: Kind, text: &str) -> Result<Vec<Hit>, Stop> {
        self.search_n(kind, text, SHOWN_RESULTS).await
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use kuro_track::tracker::anilist::api::RawResponse;
    use kuro_track::tracker::anilist::error::AniListError;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::provider::TrackRef;

    /// AniList answering every id with one MyAnimeList id, or with none.
    struct AniList(Option<i64>);

    #[async_trait]
    impl Transport for AniList {
        async fn post(&self, _body: &Value, bearer: Option<&str>) -> Result<RawResponse, AniListError> {
            assert_eq!(bearer, None, "asked without a login");
            let body = json!({ "data": { "Media": { "idMal": self.0 } } });
            Ok(RawResponse { status: 200, body: Bytes::from(body.to_string()), retry_after: None })
        }
    }

    async fn jikan(server: &MockServer, anilist: Option<i64>) -> Jikan {
        Jikan::with_pace(&server.uri(), Arc::new(AniList(anilist)), Duration::ZERO).expect("client")
    }

    fn item(id: i64, title: &str, score: Value) -> Value {
        json!({ "mal_id": id, "title": title, "title_english": null, "title_synonyms": [], "titles": [{ "type": "Default", "title": title }], "score": score })
    }

    fn query(title: &str) -> Query {
        Query { entry_id: 1, kind: Kind::Anime, title: title.into(), known: None, tracks: Vec::new() }
    }

    fn track(tracker: TrackerId, remote_id: i64, title: &str) -> TrackRef {
        TrackRef { tracker, remote_id, title: title.into() }
    }

    async fn serve(server: &MockServer, route: &str, body: Value) {
        Mock::given(method("GET")).and(path(route)).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(server).await;
    }

    fn score_of(outcome: &Outcome) -> Option<(i64, Option<f64>, MatchedBy)> {
        match outcome {
            Outcome::Found { hit, by } => Some((hit.remote_id, hit.score, *by)),
            Outcome::NotFound => None,
        }
    }

    #[tokio::test]
    async fn a_myanimelist_binding_is_asked_by_its_id_and_an_unrated_title_has_no_score() {
        let server = MockServer::start().await;
        serve(&server, "/anime/16498", json!({ "data": item(16498, "Shingeki no Kyojin", json!(8.56)) })).await;
        serve(&server, "/manga/2", json!({ "data": item(2, "Berserk", Value::Null) })).await;
        let jikan = jikan(&server, None).await;
        let anime = Query { tracks: vec![track(TrackerId::MYANIMELIST, 16498, "Attack on Titan")], ..query("Some other name") };
        assert_eq!(score_of(&jikan.lookup(&anime).await.expect("answer")), Some((16498, Some(8.56), MatchedBy::Tracker)), "an exact id needs no name");
        let manga = Query { kind: Kind::Manga, known: Some((2, MatchedBy::User)), ..query("Berserk") };
        assert_eq!(score_of(&jikan.lookup(&manga).await.expect("answer")), Some((2, None, MatchedBy::User)), "found, not rated yet");
    }

    #[tokio::test]
    async fn a_name_search_takes_the_one_result_of_that_name_and_tries_the_shorter_title() {
        let server = MockServer::start().await;
        let long = "Sousou no Frieren: Tabi no Owari";
        Mock::given(method("GET")).and(path("/anime")).and(query_param("q", long)).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] }))).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/anime"))
            .and(query_param("q", "Sousou no Frieren"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [item(1, "Sousou no Frieren 2nd Season", json!(9.1)), {
                "mal_id": 52991, "title": "Sousou no Frieren", "title_synonyms": [long], "titles": [], "score": 9.3
            }] })))
            .mount(&server)
            .await;
        let jikan = jikan(&server, None).await;
        assert_eq!(score_of(&jikan.lookup(&query(long)).await.expect("answer")), Some((52991, Some(9.3), MatchedBy::Title)), "the shorter query widens the search, the full name still decides");
    }

    #[tokio::test]
    async fn two_results_of_one_name_are_no_match() {
        let server = MockServer::start().await;
        serve(&server, "/anime", json!({ "data": [item(1, "Hunter x Hunter", json!(8.4)), item(2, "Hunter x Hunter", json!(9.0))] })).await;
        let jikan = jikan(&server, None).await;
        assert_eq!(jikan.lookup(&query("Hunter x Hunter")).await, Ok(Outcome::NotFound));
    }

    #[tokio::test]
    async fn an_id_from_another_tracker_counts_only_with_the_same_name() {
        let server = MockServer::start().await;
        serve(&server, "/anime/20", json!({ "data": item(20, "Naruto", json!(8.0)) })).await;
        serve(&server, "/anime", json!({ "data": [] })).await;
        // AniList says its media 99 is MyAnimeList 20
        let jikan = jikan(&server, Some(20)).await;
        let named = Query { tracks: vec![track(TrackerId::ANILIST, 99, "NARUTO")], ..query("Naruto (Dub)") };
        assert_eq!(score_of(&jikan.lookup(&named).await.expect("answer")), Some((20, Some(8.0), MatchedBy::Tracker)));
        let shikimori = Query { tracks: vec![track(TrackerId::SHIKIMORI, 20, "Naruto")], ..query("Naruto") };
        assert_eq!(score_of(&jikan.lookup(&shikimori).await.expect("answer")), Some((20, Some(8.0), MatchedBy::Tracker)));
        let other = Query { tracks: vec![track(TrackerId::SHIKIMORI, 20, "Bleach")], ..query("Bleach") };
        assert_eq!(jikan.lookup(&other).await, Ok(Outcome::NotFound), "the id names another title there, and the search finds none");
    }

    #[tokio::test]
    async fn a_missing_title_is_not_found_and_a_busy_or_broken_site_stops_the_run() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/anime/404")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/429")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30")).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/500")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/7")).respond_with(ResponseTemplate::new(200).set_body_string("not json")).mount(&server).await;
        let jikan = jikan(&server, None).await;
        let by_id = |id| Query { known: Some((id, MatchedBy::User)), ..query("T") };
        assert_eq!(jikan.lookup(&by_id(404)).await, Ok(Outcome::NotFound));
        assert_eq!(jikan.lookup(&by_id(429)).await, Err(Stop::RateLimited { retry_after: Some(Duration::from_secs(30)) }));
        assert_eq!(jikan.lookup(&by_id(500)).await, Err(Stop::Unreachable));
        assert_eq!(jikan.lookup(&by_id(7)).await, Err(Stop::Unreachable));
    }

    #[test]
    fn the_pace_stays_under_the_limits_of_the_site() {
        let a_minute = Duration::from_secs(60).as_millis() / PACE.as_millis();
        assert!(a_minute <= 60 && PACE >= Duration::from_millis(334), "{a_minute} a minute: under 60 a minute and 3 a second");
    }

    #[test]
    fn the_names_of_a_title_are_its_title_first_then_the_others_once() {
        let item: Item = serde_json::from_value(json!({
            "mal_id": 5, "title": "Kimetsu no Yaiba", "title_english": "Demon Slayer", "title_japanese": null,
            "title_synonyms": ["Blade of Demon Destruction"], "titles": [{ "type": "Default", "title": "Kimetsu no Yaiba" }, { "type": "English", "title": "Demon Slayer" }], "score": 0
        }))
        .expect("item");
        let hit = Hit::from(item);
        assert_eq!((hit.remote_id, hit.title.as_str(), hit.score), (5, "Kimetsu no Yaiba", None), "a score of 0 is no score");
        assert_eq!(hit.alt_titles, ["Demon Slayer", "Blade of Demon Destruction"]);
        let bare: Item = serde_json::from_value(json!({ "mal_id": 6 })).expect("item");
        assert_eq!(Hit::from(bare).title, "");
    }
}
