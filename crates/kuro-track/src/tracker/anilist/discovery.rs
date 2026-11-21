//! What Home suggests (docs/design/home.md, discovery rows): AniList's trending titles of both kinds, and the
//! recommendations of a few seed titles, found by their MyAnimeList id. One GraphQL request carries all of it, sent
//! without a token. Checked against the live API on 2026-11-21: `Page.media(sort: TRENDING_DESC)` for both kinds,
//! `Media(idMal:, type:).recommendations(sort: RATING_DESC)` with each node's `rating` (the users' votes).
//!
//! A seed AniList does not know comes back as an error for its alias next to the data of the others; the answer is read
//! from `data` whatever `errors` says, so one unknown title never empties the rows.

use kuro_platform::kind::Kind;
use serde_json::{Value, json};

use super::api::Transport;
use super::error::AniListError;

/// How many trending titles of each kind, and recommendations of each seed, are asked for.
pub const PER_PAGE: usize = 20;

/// The fields of every suggested title.
const MEDIA_FIELDS: &str = "id idMal type isAdult averageScore title { romaji english } coverImage { large }";

/// One title AniList suggests.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Suggestion {
    pub anilist_id: i64,
    pub mal_id: Option<i64>,
    pub kind: Kind,
    /// The romaji title, as most sources name titles.
    pub title: String,
    pub english: Option<String>,
    pub cover: Option<String>,
    /// AniList's average score on the app's 0 to 10 scale; `None` when AniList has none.
    pub score: Option<f64>,
    pub adult: bool,
    /// For a recommendation: the users' votes for it. 0 for a trending title.
    pub votes: i64,
}

/// One answer: the trending titles of each kind, and the recommendations of each seed in the seeds' order.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Discovery {
    pub trending_anime: Vec<Suggestion>,
    pub trending_manga: Vec<Suggestion>,
    pub recommended: Vec<Vec<Suggestion>>,
}

fn media_type(kind: Kind) -> &'static str {
    if kind.is_anime() { "ANIME" } else { "MANGA" }
}

/// The request for `seeds` (each a kind and a MyAnimeList id). Adult titles are left out of the trending lists unless
/// `adult`; recommendations carry their flag, and the caller filters them.
pub fn request(seeds: &[(Kind, i64)], adult: bool) -> Value {
    let adult_filter = if adult { String::new() } else { ", isAdult: false".to_owned() };
    let mut query = String::from("query {");
    for (alias, kind) in [("trendingAnime", Kind::Anime), ("trendingManga", Kind::Manga)] {
        query.push_str(&format!(
            " {alias}: Page(perPage: {PER_PAGE}) {{ media(type: {}, sort: TRENDING_DESC{adult_filter}) {{ {MEDIA_FIELDS} }} }}",
            media_type(kind)
        ));
    }
    for (index, (kind, mal_id)) in seeds.iter().enumerate() {
        query.push_str(&format!(
            " seed{index}: Media(idMal: {mal_id}, type: {}) {{ recommendations(sort: RATING_DESC, perPage: {PER_PAGE}) {{ nodes {{ rating mediaRecommendation {{ {MEDIA_FIELDS} }} }} }} }}",
            media_type(*kind)
        ));
    }
    query.push_str(" }");
    json!({ "query": query })
}

/// Reads an answer to [`request`] for `seeds` seeds. Only a body that is not JSON or has no `data` at all is an error.
pub fn parse(body: &[u8], seeds: usize) -> Result<Discovery, AniListError> {
    let root: Value = serde_json::from_slice(body).map_err(|error| AniListError::GraphQl(error.to_string()))?;
    let Some(data) = root.get("data").filter(|data| data.is_object()) else {
        let messages = root.get("errors").map(Value::to_string).unwrap_or_else(|| "no data".to_owned());
        return Err(AniListError::GraphQl(messages));
    };
    let page = |alias: &str| -> Vec<Suggestion> {
        data.get(alias).and_then(|page| page.get("media")).and_then(Value::as_array).map(|media| media.iter().filter_map(|m| suggestion(m, 0)).collect()).unwrap_or_default()
    };
    let recommended = (0..seeds)
        .map(|index| {
            data.get(format!("seed{index}"))
                .and_then(|media| media.pointer("/recommendations/nodes"))
                .and_then(Value::as_array)
                .map(|nodes| {
                    nodes
                        .iter()
                        .filter_map(|node| {
                            let votes = node.get("rating").and_then(Value::as_i64).unwrap_or(0);
                            node.get("mediaRecommendation").and_then(|m| suggestion(m, votes))
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    Ok(Discovery { trending_anime: page("trendingAnime"), trending_manga: page("trendingManga"), recommended })
}

/// One media object as a suggestion; `None` without an id, a known type or a title.
fn suggestion(media: &Value, votes: i64) -> Option<Suggestion> {
    let kind = match media.get("type")?.as_str()? {
        "ANIME" => Kind::Anime,
        "MANGA" => Kind::Manga,
        _ => return None,
    };
    let text = |pointer: &str| media.pointer(pointer).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Suggestion {
        anilist_id: media.get("id")?.as_i64()?,
        mal_id: media.get("idMal").and_then(Value::as_i64).filter(|id| *id > 0),
        kind,
        title: text("/title/romaji").or_else(|| text("/title/english"))?,
        english: text("/title/english"),
        cover: text("/coverImage/large"),
        score: media.get("averageScore").and_then(Value::as_f64).filter(|s| *s > 0.0).map(|s| s / 10.0),
        adult: media.get("isAdult").and_then(Value::as_bool).unwrap_or(false),
        votes,
    })
}

/// Asks AniList once for `seeds`, without a token.
pub async fn fetch(transport: &dyn Transport, seeds: &[(Kind, i64)], adult: bool) -> Result<Discovery, AniListError> {
    let raw = transport.post(&request(seeds, adult), None).await?;
    if !(200..300).contains(&raw.status) {
        return Err(AniListError::Http(raw.status));
    }
    parse(&raw.body, seeds.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::anilist::testkit::{FakeTransport, ok};

    /// Trimmed from the live answer of 2026-11-21, with an unknown seed as AniList answers it: an error beside the data.
    const ANSWER: &str = r#"{
      "errors": [{"message": "Not Found.", "status": 404, "path": ["seed1"]}],
      "data": {
        "trendingAnime": {"media": [
          {"id": 179965, "idMal": 61967, "type": "ANIME", "isAdult": false, "averageScore": 88,
           "title": {"romaji": "Black Clover 2nd Season", "english": "Black Clover Season 2"},
           "coverImage": {"large": "https://s4.anilist.co/file/anilistcdn/media/anime/cover/medium/a.jpg"}}
        ]},
        "trendingManga": {"media": [
          {"id": 170974, "idMal": null, "type": "MANGA", "isAdult": false, "averageScore": null,
           "title": {"romaji": "Mabeop Hakgyo Apjeommyeol Cheonjae-ga Doeeotda", "english": null},
           "coverImage": {"large": "https://s4.anilist.co/file/anilistcdn/media/manga/cover/medium/b.jpg"}}
        ]},
        "seed0": {"recommendations": {"nodes": [
          {"rating": 438, "mediaRecommendation": {"id": 437, "idMal": 437, "type": "ANIME", "isAdult": false, "averageScore": 85,
            "title": {"romaji": "PERFECT BLUE", "english": "Perfect Blue"}, "coverImage": {"large": "https://s4.anilist.co/c.jpg"}}},
          {"rating": 5, "mediaRecommendation": null}
        ]}},
        "seed1": null
      }
    }"#;

    #[test]
    fn the_request_asks_both_trending_lists_and_each_seed_by_its_mal_id() {
        let query = request(&[(Kind::Anime, 52034), (Kind::Manga, 2)], false)["query"].as_str().unwrap().to_owned();
        assert!(query.contains("trendingAnime: Page(perPage: 20) { media(type: ANIME, sort: TRENDING_DESC, isAdult: false)"), "{query}");
        assert!(query.contains("trendingManga: Page(perPage: 20) { media(type: MANGA, sort: TRENDING_DESC, isAdult: false)"));
        assert!(query.contains("seed0: Media(idMal: 52034, type: ANIME)") && query.contains("seed1: Media(idMal: 2, type: MANGA)"));
        assert!(!request(&[], true)["query"].as_str().unwrap().contains("isAdult: false"), "adult titles stay when allowed");
    }

    #[test]
    fn an_answer_reads_into_suggestions_and_an_unknown_seed_only_empties_its_own_list() {
        let found = parse(ANSWER.as_bytes(), 2).unwrap();
        let anime = &found.trending_anime[0];
        assert_eq!((anime.anilist_id, anime.mal_id, anime.kind, anime.title.as_str()), (179965, Some(61967), Kind::Anime, "Black Clover 2nd Season"));
        assert_eq!((anime.score, anime.english.as_deref()), (Some(8.8), Some("Black Clover Season 2")));
        let manga = &found.trending_manga[0];
        assert_eq!((manga.kind, manga.mal_id, manga.score, manga.english.as_deref()), (Kind::Manga, None, None, None), "no MAL id, no score, no English title");
        assert_eq!(found.recommended.len(), 2);
        assert_eq!(found.recommended[0].len(), 1, "a node without a title is skipped");
        assert_eq!((found.recommended[0][0].title.as_str(), found.recommended[0][0].votes), ("PERFECT BLUE", 438));
        assert!(found.recommended[1].is_empty(), "the unknown seed");
    }

    #[test]
    fn a_body_without_data_is_an_error() {
        assert!(parse(br#"{"errors": [{"message": "down"}]}"#, 0).is_err());
        assert!(parse(b"not json", 0).is_err());
    }

    #[tokio::test]
    async fn one_request_without_a_token_answers_everything() {
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let found = fetch(transport.as_ref(), &[(Kind::Anime, 52034), (Kind::Manga, 9)], false).await.unwrap();
        assert_eq!(found.recommended[0][0].anilist_id, 437);
        assert_eq!(transport.calls(), 1, "one request");
        assert_eq!(transport.bearers(), [None], "no token");
    }
}
