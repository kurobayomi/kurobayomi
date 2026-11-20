//! The MyAnimeList score from MyAnimeList's own API (`api.myanimelist.net/v2`): anime and manga, no login, the app's
//! client id in a header (the one the MyAnimeList tracker uses; without it the API answers 403). It answers one title
//! per request and publishes no limit, so every request here waits its turn on one pace of a request each 1.5 seconds;
//! a library fills in over minutes, in the background.
//!
//! It replaced Jikan, an unofficial API that scraped MyAnimeList: Jikan's public API was shut down on 2026-10-01.
//!
//! An entry is asked by the most exact thing known about it, in this order:
//! 1. the title the user chose for it;
//! 2. its MyAnimeList binding, whose id is the id this API takes;
//! 3. the match stored before;
//! 4. its Shikimori binding, or its AniList binding turned into a MyAnimeList id by AniList itself. Neither id is
//!    known to be a MyAnimeList id for certain, so the answer counts only when its names include the entry's title;
//! 5. a search by name: the entry's title, then its shorter forms (`title_queries`), each accepted only when exactly
//!    one result has the same name (`pick`).
//!
//! What the API was seen to do (real requests, 2026-11-20):
//! - a search text must be 3 to 64 characters: 2 and 65 answer 400;
//! - a search leaves out the titles MyAnimeList marks as adult unless `nsfw=true` is sent; by id they answer either way;
//! - a title nobody rated has no `mean` field; a missing id answers 404;
//! - a season (`anime/season/{year}/{season}`) is one list of the titles on air in it, continuing ones included: 219
//!   titles for fall 2026, with no further page;
//! - the popularity list (`{kind}/ranking?ranking_type=bypopularity`) gives 500 titles a request, for anime and manga,
//!   and takes an `offset`; a `limit` of 501 answers 400.
//!
//! The bulk lists ([`ScoreProvider::catalogue`]) are those: the season of today and the one before (anime), and the
//! most popular titles. Of 19 titles on one page of a source's "Popular" listing, the 2,000 most popular held 10, so the
//! lists are a first answer for a Browse page and not the whole of it.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kuro_data::community_score::MatchedBy;
use kuro_platform::kind::Kind;
use kuro_track::auto_bind::{pick, title_queries};
use kuro_track::mal::consts::API_BASE;
use kuro_track::model::TrackerId;
use kuro_track::tracker::anilist::api::{RateLimiter, Transport, anilist_to_mal_id, user_agent};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::provider::{Hit, Outcome, Query, ScoreProvider, Stop};

/// The name stored with a score from here.
pub const ID: &str = "myanimelist";
/// One request each. MyAnimeList publishes no limit; this is the pace Jikan's limits asked for, kept.
pub const PACE: Duration = Duration::from_millis(1500);
/// The header that carries the client id.
const CLIENT_ID_HEADER: &str = "X-MAL-CLIENT-ID";
/// The fields asked of a title: its score and its other names. Its id and title always come.
const FIELDS: &str = "mean,alternative_titles";
/// Results asked of a search by name: enough to see two titles of one name, which is then no match.
const MATCH_RESULTS: &str = "5";
/// Results asked of a search the user reads.
const SHOWN_RESULTS: &str = "15";
/// The shortest and the longest search text the API takes, in characters.
const QUERY_MIN: usize = 3;
const QUERY_MAX: usize = 64;
/// Titles asked of a list in one request: the most the API gives.
const LIST_PAGE: usize = 500;
/// How far down the popularity list the bulk lists go: four requests a kind.
const POPULAR_TITLES: usize = 2000;

/// The season of a month as the API names it, with its year.
fn season_of(year: i32, month: u32) -> (i32, &'static str) {
    let season = match month {
        1..=3 => "winter",
        4..=6 => "spring",
        7..=9 => "summer",
        _ => "fall",
    };
    (year, season)
}

/// The season before `season`.
fn season_before(season: (i32, &'static str)) -> (i32, &'static str) {
    match season {
        (year, "winter") => (year - 1, "fall"),
        (year, "spring") => (year, "winter"),
        (year, "summer") => (year, "spring"),
        (year, _) => (year, "summer"),
    }
}

/// Adds the titles of `list` that `hits` does not hold yet: a title on air is in its season's list and may be in the
/// popularity list too.
fn keep_new(hits: &mut Vec<Hit>, seen: &mut HashSet<i64>, list: Vec<Hit>) {
    hits.extend(list.into_iter().filter(|hit| seen.insert(hit.remote_id)));
}

pub struct Mal {
    client: reqwest::Client,
    base: String,
    client_id: String,
    limiter: RateLimiter,
    anilist: Arc<dyn Transport>,
}

/// The answer of a search: each result is wrapped in a `node`.
#[derive(Deserialize)]
struct Many {
    #[serde(default)]
    data: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    node: Item,
}

/// The fields read of the API's anime and manga objects. Everything but the id may be missing or null.
#[derive(Deserialize)]
struct Item {
    id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    mean: Option<f64>,
    #[serde(default)]
    alternative_titles: Option<OtherNames>,
}

#[derive(Deserialize)]
struct OtherNames {
    #[serde(default)]
    synonyms: Option<Vec<String>>,
    #[serde(default)]
    en: Option<String>,
    #[serde(default)]
    ja: Option<String>,
}

impl From<Item> for Hit {
    fn from(item: Item) -> Self {
        let (english, japanese, synonyms) = match item.alternative_titles {
            Some(other) => (other.en, other.ja, other.synonyms.unwrap_or_default()),
            None => (None, None, Vec::new()),
        };
        let mut names: Vec<String> = Vec::new();
        // the API gives an empty text, not null, for a name a title does not have
        for name in item.title.into_iter().chain(english).chain(japanese).chain(synonyms) {
            let name = name.trim().to_owned();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        let title = if names.is_empty() { String::new() } else { names.remove(0) };
        Hit { remote_id: item.id, title, alt_titles: names, score: item.mean.filter(|mean| *mean > 0.0) }
    }
}

/// A search text the API takes: 3 to 64 characters. A longer one is cut to the whole words that fit; `None` when what
/// is left is too short to send.
fn search_text(text: &str) -> Option<String> {
    let text = text.trim();
    let cut = if text.chars().count() > QUERY_MAX {
        let head: String = text.chars().take(QUERY_MAX).collect();
        let cut_in_a_word = text.chars().nth(QUERY_MAX).is_some_and(|next| !next.is_whitespace());
        match head.rfind(char::is_whitespace) {
            Some(at) if cut_in_a_word => head[..at].trim_end().to_owned(),
            _ => head.trim_end().to_owned(),
        }
    } else {
        text.to_owned()
    };
    (cut.chars().count() >= QUERY_MIN).then_some(cut)
}

impl Mal {
    /// `client_id` is the MyAnimeList client id of the app (`kuro_track::mal::store::configured_client_id`).
    /// `anilist` turns an AniList id into a MyAnimeList id, without a login.
    pub fn new(client_id: &str, anilist: Arc<dyn Transport>) -> Result<Self, reqwest::Error> {
        Self::with_pace(API_BASE, client_id, anilist, PACE)
    }

    /// `pace` is the time kept between two requests (the tests, against a local server, keep none).
    pub fn with_pace(base: &str, client_id: &str, anilist: Arc<dyn Transport>, pace: Duration) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).user_agent(user_agent()).build()?;
        Ok(Self { client, base: base.trim_end_matches('/').to_owned(), client_id: client_id.trim().to_owned(), limiter: RateLimiter::new(1, pace), anilist })
    }

    /// One paced GET. `None` for a title the site does not have (404) or a request it does not take (400).
    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<Option<T>, Stop> {
        self.limiter.acquire().await;
        let request = self.client.get(format!("{}/{path}", self.base)).header(CLIENT_ID_HEADER, self.client_id.as_str()).query(query);
        let response = request.send().await.map_err(|error| {
            tracing::debug!(%error, "myanimelist: no answer");
            Stop::Unreachable
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
            return Err(Stop::RateLimited { retry_after });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // every later request would be refused the same way: stop the run instead of recording a miss per entry
            tracing::warn!(%status, "myanimelist: the client id was refused; no score is read until it is accepted");
            return Err(Stop::Unreachable);
        }
        if status.is_server_error() {
            return Err(Stop::Unreachable);
        }
        if !status.is_success() {
            return Ok(None);
        }
        response.json::<T>().await.map(Some).map_err(|error| {
            tracing::debug!(%error, "myanimelist: an answer that cannot be read");
            Stop::Unreachable
        })
    }

    async fn by_id(&self, kind: Kind, id: i64) -> Result<Option<Hit>, Stop> {
        Ok(self.get::<Item>(&format!("{}/{id}", kind.as_str()), &[("fields", FIELDS)]).await?.map(Hit::from))
    }

    /// The titles of one list answer: a search, a season or a page of the popularity list. They share one shape.
    async fn list(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Hit>, Stop> {
        let found = self.get::<Many>(path, query).await?;
        Ok(found.map(|many| many.data.into_iter().map(|result| Hit::from(result.node)).collect()).unwrap_or_default())
    }

    /// The titles `text` finds. A text the API would refuse for its length is cut or not sent ([`search_text`]).
    async fn search_n(&self, kind: Kind, text: &str, results: &str) -> Result<Vec<Hit>, Stop> {
        let Some(text) = search_text(text) else { return Ok(Vec::new()) };
        self.list(kind.as_str(), &[("q", text.as_str()), ("limit", results), ("fields", FIELDS), ("nsfw", "true")]).await
    }

    /// The title of `id` when its names include one of `titles`: an id that came from another site is not trusted alone.
    async fn by_id_named(&self, kind: Kind, id: i64, titles: &[&str]) -> Result<Option<Hit>, Stop> {
        let Some(hit) = self.by_id(kind, id).await? else { return Ok(None) };
        let same = titles.iter().any(|title| pick(title, [(hit.title.as_str(), hit.alt_titles.as_slice())]).is_some());
        Ok(same.then_some(hit))
    }
}

#[async_trait]
impl ScoreProvider for Mal {
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

    async fn catalogue(&self, kind: Kind, today: (i32, u32)) -> Result<Vec<Hit>, Stop> {
        let mut hits: Vec<Hit> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();
        let page = LIST_PAGE.to_string();
        if kind == Kind::Anime {
            let this_season = season_of(today.0, today.1);
            for (year, season) in [this_season, season_before(this_season)] {
                let list = self.list(&format!("anime/season/{year}/{season}"), &[("limit", page.as_str()), ("fields", FIELDS), ("nsfw", "true")]).await?;
                keep_new(&mut hits, &mut seen, list);
            }
        }
        for offset in (0..POPULAR_TITLES).step_by(LIST_PAGE) {
            let offset = offset.to_string();
            let query = [("ranking_type", "bypopularity"), ("limit", page.as_str()), ("offset", offset.as_str()), ("fields", FIELDS), ("nsfw", "true")];
            let list = self.list(&format!("{}/ranking", kind.as_str()), &query).await?;
            // a page that is not full is the last one
            let last = list.len() < LIST_PAGE;
            keep_new(&mut hits, &mut seen, list);
            if last {
                break;
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use kuro_track::tracker::anilist::api::RawResponse;
    use kuro_track::tracker::anilist::error::AniListError;
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path, query_param};
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

    async fn mal(server: &MockServer, anilist: Option<i64>) -> Mal {
        Mal::with_pace(&server.uri(), "the-client-id", Arc::new(AniList(anilist)), Duration::ZERO).expect("client")
    }

    /// A title as the API gives it: an empty text for a name it does not have, and no `mean` while nobody rated it.
    fn item(id: i64, title: &str, mean: Option<f64>) -> Value {
        let mut item = json!({ "id": id, "title": title, "alternative_titles": { "synonyms": [], "en": "", "ja": "" } });
        if let Some(mean) = mean {
            item["mean"] = json!(mean);
        }
        item
    }

    /// The answer of a search with these results.
    fn results(items: Vec<Value>) -> Value {
        json!({ "data": items.into_iter().map(|node| json!({ "node": node })).collect::<Vec<_>>() })
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
        serve(&server, "/anime/16498", item(16498, "Shingeki no Kyojin", Some(8.56))).await;
        serve(&server, "/manga/2", item(2, "Berserk", None)).await;
        let mal = mal(&server, None).await;
        let anime = Query { tracks: vec![track(TrackerId::MYANIMELIST, 16498, "Attack on Titan")], ..query("Some other name") };
        assert_eq!(score_of(&mal.lookup(&anime).await.expect("answer")), Some((16498, Some(8.56), MatchedBy::Tracker)), "an exact id needs no name");
        let manga = Query { kind: Kind::Manga, known: Some((2, MatchedBy::User)), ..query("Berserk") };
        assert_eq!(score_of(&mal.lookup(&manga).await.expect("answer")), Some((2, None, MatchedBy::User)), "found, not rated yet");
    }

    #[tokio::test]
    async fn every_request_carries_the_client_id_and_asks_for_the_score_and_the_names() {
        let server = MockServer::start().await;
        // only a request with the header and the fields is answered; any other gets the mock server's 404
        Mock::given(method("GET"))
            .and(path("/anime/5"))
            .and(header("X-MAL-CLIENT-ID", "the-client-id"))
            .and(query_param("fields", "mean,alternative_titles"))
            .respond_with(ResponseTemplate::new(200).set_body_json(item(5, "Kimetsu no Yaiba", Some(8.4))))
            .mount(&server)
            .await;
        let mal = mal(&server, None).await;
        let by_id = Query { known: Some((5, MatchedBy::User)), ..query("T") };
        assert_eq!(score_of(&mal.lookup(&by_id).await.expect("answer")), Some((5, Some(8.4), MatchedBy::User)));
    }

    #[tokio::test]
    async fn a_name_search_takes_the_one_result_of_that_name_and_tries_the_shorter_title() {
        let server = MockServer::start().await;
        let long = "Sousou no Frieren: Tabi no Owari";
        let search = |text: &str, body: Value| {
            Mock::given(method("GET"))
                .and(path("/anime"))
                .and(query_param("q", text))
                .and(query_param("nsfw", "true"))
                .and(query_param("limit", "5"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
        };
        search(long, results(Vec::new())).mount(&server).await;
        let named = json!({ "id": 52991, "title": "Sousou no Frieren", "mean": 9.3, "alternative_titles": { "synonyms": [long], "en": "Frieren: Beyond Journey's End", "ja": "葬送のフリーレン" } });
        search("Sousou no Frieren", results(vec![item(1, "Sousou no Frieren 2nd Season", Some(9.1)), named])).mount(&server).await;
        let mal = mal(&server, None).await;
        assert_eq!(score_of(&mal.lookup(&query(long)).await.expect("answer")), Some((52991, Some(9.3), MatchedBy::Title)), "the shorter query widens the search, the full name still decides");
    }

    #[test]
    fn a_search_text_is_three_to_sixty_four_characters() {
        assert_eq!(search_text(" ab "), None, "the API answers 400 to two characters");
        assert_eq!(search_text("abc").as_deref(), Some("abc"));
        let sixty_four = "a".repeat(64);
        assert_eq!(search_text(&sixty_four).as_deref(), Some(sixty_four.as_str()), "64 is taken as it is");
        // 65 characters and more: the whole words that fit
        let long = "Sekai Saikyou no Majo Hajimemashita Watashi dake Kouryaku Site wo Mireru Sekai de";
        let cut = search_text(long).expect("a text");
        assert_eq!(cut, "Sekai Saikyou no Majo Hajimemashita Watashi dake Kouryaku Site");
        assert!(cut.chars().count() <= 64 && long.starts_with(&cut));
        // a cut that falls between two words keeps the last word
        let even = format!("{} {}", "a".repeat(64), "tail");
        assert_eq!(search_text(&even).as_deref(), Some("a".repeat(64).as_str()));
        // one word longer than the limit is cut where the limit is, and counts characters, not bytes
        assert_eq!(search_text(&"漫".repeat(70)).map(|text| text.chars().count()), Some(64));
    }

    #[tokio::test]
    async fn a_title_too_short_to_search_sends_nothing() {
        let server = MockServer::start().await;
        let mal = mal(&server, None).await;
        assert_eq!(mal.lookup(&query("Ab")).await, Ok(Outcome::NotFound));
        assert_eq!(mal.search(Kind::Anime, "x").await, Ok(Vec::new()));
        assert!(server.received_requests().await.expect("recorded").is_empty(), "the API would answer 400");
    }

    #[tokio::test]
    async fn two_results_of_one_name_are_no_match() {
        let server = MockServer::start().await;
        serve(&server, "/anime", results(vec![item(1, "Hunter x Hunter", Some(8.4)), item(2, "Hunter x Hunter", Some(9.0))])).await;
        let mal = mal(&server, None).await;
        assert_eq!(mal.lookup(&query("Hunter x Hunter")).await, Ok(Outcome::NotFound));
    }

    #[tokio::test]
    async fn an_id_from_another_tracker_counts_only_with_the_same_name() {
        let server = MockServer::start().await;
        serve(&server, "/anime/20", item(20, "Naruto", Some(8.0))).await;
        serve(&server, "/anime", results(Vec::new())).await;
        // AniList says its media 99 is MyAnimeList 20
        let mal = mal(&server, Some(20)).await;
        let named = Query { tracks: vec![track(TrackerId::ANILIST, 99, "NARUTO")], ..query("Naruto (Dub)") };
        assert_eq!(score_of(&mal.lookup(&named).await.expect("answer")), Some((20, Some(8.0), MatchedBy::Tracker)));
        let shikimori = Query { tracks: vec![track(TrackerId::SHIKIMORI, 20, "Naruto")], ..query("Naruto") };
        assert_eq!(score_of(&mal.lookup(&shikimori).await.expect("answer")), Some((20, Some(8.0), MatchedBy::Tracker)));
        let other = Query { tracks: vec![track(TrackerId::SHIKIMORI, 20, "Bleach")], ..query("Bleach") };
        assert_eq!(mal.lookup(&other).await, Ok(Outcome::NotFound), "the id names another title there, and the search finds none");
    }

    #[tokio::test]
    async fn a_missing_title_is_not_found_and_a_busy_broken_or_refusing_site_stops_the_run() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/anime/404")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/429")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30")).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/500")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/403")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
        Mock::given(method("GET")).and(path("/anime/7")).respond_with(ResponseTemplate::new(200).set_body_string("not json")).mount(&server).await;
        // a search the API refuses (400 "invalid q") is a title not found, not a reason to stop
        Mock::given(method("GET")).and(path("/anime")).respond_with(ResponseTemplate::new(400)).mount(&server).await;
        let mal = mal(&server, None).await;
        let by_id = |id| Query { known: Some((id, MatchedBy::User)), ..query("T") };
        assert_eq!(mal.lookup(&by_id(404)).await, Ok(Outcome::NotFound));
        assert_eq!(mal.lookup(&by_id(429)).await, Err(Stop::RateLimited { retry_after: Some(Duration::from_secs(30)) }));
        assert_eq!(mal.lookup(&by_id(500)).await, Err(Stop::Unreachable));
        assert_eq!(mal.lookup(&by_id(403)).await, Err(Stop::Unreachable), "a refused client id is refused for every title");
        assert_eq!(mal.lookup(&by_id(7)).await, Err(Stop::Unreachable));
        assert_eq!(mal.lookup(&query("Some title")).await, Ok(Outcome::NotFound));
    }

    #[test]
    fn a_season_is_named_by_its_months_and_the_one_before_winter_is_last_years_fall() {
        assert_eq!([season_of(2026, 1), season_of(2026, 3)], [(2026, "winter"); 2]);
        assert_eq!([season_of(2026, 4), season_of(2026, 6)], [(2026, "spring"); 2]);
        assert_eq!([season_of(2026, 7), season_of(2026, 9)], [(2026, "summer"); 2]);
        assert_eq!([season_of(2026, 10), season_of(2026, 12)], [(2026, "fall"); 2]);
        assert_eq!(season_before((2026, "fall")), (2026, "summer"));
        assert_eq!(season_before((2026, "summer")), (2026, "spring"));
        assert_eq!(season_before((2026, "spring")), (2026, "winter"));
        assert_eq!(season_before((2026, "winter")), (2025, "fall"));
    }

    #[tokio::test]
    async fn the_bulk_lists_of_anime_are_two_seasons_and_the_popular_titles_each_title_once() {
        let server = MockServer::start().await;
        // on air this season and still on air from the one before: in both lists
        serve(&server, "/anime/season/2026/fall", results(vec![item(21, "One Piece", Some(8.7)), item(64084, "Sekai Saikyou no Majo, Hajimemashita", Some(7.07))])).await;
        serve(&server, "/anime/season/2026/summer", results(vec![item(21, "One Piece", Some(8.7)), item(7, "A summer title", None)])).await;
        let popular = |offset: &str, body: Value| {
            Mock::given(method("GET"))
                .and(path("/anime/ranking"))
                .and(query_param("ranking_type", "bypopularity"))
                .and(query_param("limit", "500"))
                .and(query_param("offset", offset))
                .and(header("X-MAL-CLIENT-ID", "the-client-id"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
        };
        // a full page, then a page that is not full: the last one
        let first: Vec<Value> = (0..500).map(|i| item(1000 + i, &format!("Popular {i}"), Some(7.5))).collect();
        popular("0", results(first)).mount(&server).await;
        popular("500", results(vec![item(21, "One Piece", Some(8.7)), item(9000, "The last popular one", Some(6.0))])).mount(&server).await;
        let mal = mal(&server, None).await;
        let hits = mal.catalogue(Kind::Anime, (2026, 11)).await.expect("lists");
        assert_eq!(hits.len(), 3 + 500 + 1, "One Piece once, though three lists hold it");
        assert_eq!(hits.iter().filter(|hit| hit.remote_id == 21).count(), 1);
        assert_eq!(hits[1].score, Some(7.07));
        assert_eq!(hits[2].score, None, "listed, not rated yet");
        assert_eq!(server.received_requests().await.expect("recorded").len(), 4, "two seasons and two pages: no third page after a short one");
    }

    #[tokio::test]
    async fn the_bulk_lists_of_manga_have_no_season_and_a_stop_ends_them() {
        let server = MockServer::start().await;
        serve(&server, "/manga/ranking", results(vec![item(2, "Berserk", Some(9.46))])).await;
        let api = mal(&server, None).await;
        let hits = api.catalogue(Kind::Manga, (2026, 11)).await.expect("lists");
        assert_eq!(hits.iter().map(|hit| hit.remote_id).collect::<Vec<_>>(), [2]);
        assert_eq!(server.received_requests().await.expect("recorded").len(), 1, "no season is asked for manga");
        // the site asks to slow down in the middle of the lists: nothing is returned, the caller tries again later
        let busy = MockServer::start().await;
        serve(&busy, "/anime/season/2026/fall", results(vec![item(21, "One Piece", Some(8.7))])).await;
        Mock::given(method("GET")).and(path("/anime/season/2026/summer")).respond_with(ResponseTemplate::new(429)).mount(&busy).await;
        let api = mal(&busy, None).await;
        assert_eq!(api.catalogue(Kind::Anime, (2026, 11)).await, Err(Stop::RateLimited { retry_after: None }));
    }

    #[test]
    fn the_names_of_a_title_are_its_title_first_then_the_others_once() {
        let item: Item = serde_json::from_value(json!({
            "id": 5, "title": "Kimetsu no Yaiba", "mean": 0,
            "alternative_titles": { "synonyms": ["Blade of Demon Destruction", "Demon Slayer"], "en": "Demon Slayer", "ja": "" }
        }))
        .expect("item");
        let hit = Hit::from(item);
        assert_eq!((hit.remote_id, hit.title.as_str(), hit.score), (5, "Kimetsu no Yaiba", None), "a score of 0 is no score");
        assert_eq!(hit.alt_titles, ["Demon Slayer", "Blade of Demon Destruction"], "an empty name is none, a repeated one counts once");
        let bare: Item = serde_json::from_value(json!({ "id": 6 })).expect("item");
        let hit = Hit::from(bare);
        assert_eq!((hit.title.as_str(), hit.score), ("", None));
    }
}
