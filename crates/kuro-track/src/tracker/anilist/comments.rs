//! The comments and reviews AniList already has (docs/design/comments.md): an episode's or a chapter's discussion thread
//! with its comments, and a title's reviews. Read-only, sent without a token, asked only when the comments panel (player
//! or reader) or the Reviews page opens, and kept nowhere but in the caller's memory (AniList's terms forbid using the API as storage).
//!
//! What the live API was seen to do (2026-11-21):
//! - a thread has no episode field. Episode threads are user posts in the "Release Discussion" category (id 5), titled
//!   like `[Spoilers] Sousou No Frieren - Episode 10 Discussion`; a premiere can share one (`Premiere/Episodes 1-4`);
//! - chapter threads are the same, in the same category (2026-11-22): `[Spoilers] Chainsaw Man - Chapter 232
//!   Discussion`, `[Spoilers] Jujutsu Kaisen - Chapter 271 (Finale) Discussion`, and part chapters such as
//!   `Chapter 262.2`, which are not chapter 262;
//! - `threads(categoryId: 5, mediaCategoryId:)` lists a title's episode threads (Frieren: its 25 on one page), and
//!   `search:` finds an old one of a long show ("Episode 1100" for One Piece) but missed "Episode 10" for Frieren;
//! - `threadComments` sorts only by `ID` or `ID_DESC` and lists top-level comments; replies come nested in
//!   `childComments`, a JSON value with the same fields as asked, `null` when there are none;
//! - `comment(asHtml: true)` and `body(asHtml: true)` give AniList's own rendering: Markdown done, spoilers as
//!   `<span class='markdown_spoiler'>`, images as `<img>`;
//! - `perPage` above 50 is cut to 50, and `pageInfo.total` is not a count, so only `hasNextPage` is read.
//!
//! At most [`LIMIT`] comments and [`LIMIT`] reviews, one request each (user, 2026-11-21: a starting limit while the
//! feature is new).

use serde_json::{Value, json};

use super::api::Transport;
use super::error::AniListError;
use kuro_platform::kind::Kind;

/// The most comments of a thread, and the most reviews of a title, that are asked for: one page of each.
pub const LIMIT: usize = 50;
/// AniList's forum category of episode and chapter discussions.
pub const RELEASE_DISCUSSION: i64 = 5;
/// When neither the first page nor the search finds the episode's thread, the most pages of discussions read.
pub const SCAN_PAGES: u32 = 5;

/// One comment of a thread, with its replies.
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: i64,
    pub user: String,
    pub likes: i64,
    /// Unix seconds.
    pub created_at: i64,
    /// AniList's HTML of the comment.
    pub html: String,
    pub replies: Vec<Comment>,
}

/// One forum thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: i64,
    pub title: String,
    pub replies: i64,
    pub url: String,
}

/// One review of a title.
#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub id: i64,
    pub user: String,
    pub summary: String,
    /// Out of 100.
    pub score: i64,
    /// How many readers found it helpful, of `rating_amount`.
    pub rating: i64,
    pub rating_amount: i64,
    /// AniList's HTML of the whole review.
    pub html: String,
    pub url: String,
}

fn media_type(kind: Kind) -> &'static str {
    if kind.is_anime() { "ANIME" } else { "MANGA" }
}

/// The AniList id of the title MyAnimeList knows as `mal_id`.
pub fn media_id_request(mal_id: i64, kind: Kind) -> Value {
    json!({ "query": format!("query {{ Media(idMal: {mal_id}, type: {}) {{ id }} }}", media_type(kind)) })
}

/// The address of a title's page on AniList.
pub fn media_url(media_id: i64, kind: Kind) -> String {
    format!("https://anilist.co/{}/{media_id}", if kind.is_anime() { "anime" } else { "manga" })
}

const THREAD_FIELDS: &str = "id title replyCount siteUrl";

/// One page of a title's discussion threads, newest first.
pub fn threads_request(media_id: i64, page: u32) -> Value {
    json!({ "query": format!(
        "query {{ Page(page: {page}, perPage: {LIMIT}) {{ pageInfo {{ hasNextPage }} threads(categoryId: {RELEASE_DISCUSSION}, mediaCategoryId: {media_id}, sort: CREATED_AT_DESC) {{ {THREAD_FIELDS} }} }} }}"
    ) })
}

/// The word a thread title names a unit of `kind` with: `Episode` or `Chapter`.
fn unit_word(kind: Kind) -> &'static str {
    if kind.is_anime() { "Episode" } else { "Chapter" }
}

/// The title's discussion threads that AniList's search finds for unit `number` of `kind`.
pub fn thread_search_request(media_id: i64, kind: Kind, number: i64) -> Value {
    json!({
        "query": format!(
            "query ($search: String) {{ Page(perPage: {LIMIT}) {{ pageInfo {{ hasNextPage }} threads(categoryId: {RELEASE_DISCUSSION}, mediaCategoryId: {media_id}, search: $search) {{ {THREAD_FIELDS} }} }} }}"
        ),
        "variables": { "search": format!("{} {number}", unit_word(kind)) },
    })
}

/// The first [`LIMIT`] top-level comments of a thread, oldest first (AniList sorts comments by nothing else), with their
/// replies.
pub fn comments_request(thread_id: i64) -> Value {
    json!({ "query": format!(
        "query {{ Page(perPage: {LIMIT}) {{ threadComments(threadId: {thread_id}, sort: ID) {{ id likeCount createdAt user {{ name }} comment(asHtml: true) childComments }} }} }}"
    ) })
}

/// The [`LIMIT`] reviews readers found most helpful.
pub fn reviews_request(media_id: i64) -> Value {
    json!({ "query": format!(
        "query {{ Media(id: {media_id}) {{ reviews(sort: RATING_DESC, perPage: {LIMIT}) {{ nodes {{ id summary score rating ratingAmount siteUrl user {{ name }} body(asHtml: true) }} }} }} }}"
    ) })
}

/// The `data` of an answer. A body that is not JSON, or has no `data`, is an error.
fn data(body: &[u8]) -> Result<Value, AniListError> {
    let mut root: Value = serde_json::from_slice(body).map_err(AniListError::Decode)?;
    match root.get_mut("data").map(Value::take) {
        Some(data) if data.is_object() => Ok(data),
        _ => Err(AniListError::GraphQl(root.get("errors").map(Value::to_string).unwrap_or_else(|| "no data".to_owned()))),
    }
}

fn text(value: &Value, pointer: &str) -> String {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn number(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// Reads an answer to [`media_id_request`]: `None` when AniList does not know the title (it answers 404 in `errors`
/// and a `null` media).
pub fn parse_media_id(body: &[u8]) -> Result<Option<i64>, AniListError> {
    let root: Value = serde_json::from_slice(body).map_err(AniListError::Decode)?;
    Ok(root.pointer("/data/Media/id").and_then(Value::as_i64))
}

/// Reads an answer to [`threads_request`] or [`thread_search_request`]: the threads, and whether a next page exists.
pub fn parse_threads(body: &[u8]) -> Result<(Vec<Thread>, bool), AniListError> {
    let data = data(body)?;
    let more = data.pointer("/Page/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false);
    let threads = data
        .pointer("/Page/threads")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|t| {
                    Some(Thread { id: t.get("id")?.as_i64()?, title: text(t, "/title"), replies: number(t, "replyCount"), url: text(t, "/siteUrl") })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((threads, more))
}

/// One comment object, and its replies, from either the typed list or the JSON of `childComments`.
fn comment(value: &Value) -> Option<Comment> {
    let html = value.get("comment").and_then(Value::as_str)?.to_owned();
    let replies = value.get("childComments").and_then(Value::as_array).map(|list| list.iter().filter_map(comment).collect()).unwrap_or_default();
    Some(Comment {
        id: value.get("id")?.as_i64()?,
        user: text(value, "/user/name"),
        likes: number(value, "likeCount"),
        created_at: number(value, "createdAt"),
        html,
        replies,
    })
}

/// Reads an answer to [`comments_request`].
pub fn parse_comments(body: &[u8]) -> Result<Vec<Comment>, AniListError> {
    let data = data(body)?;
    Ok(data.pointer("/Page/threadComments").and_then(Value::as_array).map(|list| list.iter().filter_map(comment).collect()).unwrap_or_default())
}

/// Reads an answer to [`reviews_request`]. An unknown title has no reviews.
pub fn parse_reviews(body: &[u8]) -> Result<Vec<Review>, AniListError> {
    let data = data(body)?;
    let nodes = data.pointer("/Media/reviews/nodes").and_then(Value::as_array);
    Ok(nodes
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|r| {
                    Some(Review {
                        id: r.get("id")?.as_i64()?,
                        user: text(r, "/user/name"),
                        summary: text(r, "/summary"),
                        score: number(r, "score"),
                        rating: number(r, "rating"),
                        rating_amount: number(r, "ratingAmount"),
                        html: text(r, "/body"),
                        url: text(r, "/siteUrl"),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The whole number of an episode or a chapter the matching works on: `None` for an unknown (negative) or a part number
/// (10.5), which the matching does not tell from its whole one.
pub fn whole_number(number: f64) -> Option<i64> {
    (number >= 0.0 && number.fract() == 0.0 && number < 1e9).then_some(number as i64)
}

/// The number that starts `text` after any spaces, `#` or `.`, and the rest after it.
fn leading_number(text: &str) -> Option<(i64, &str)> {
    let text = text.trim_start_matches([' ', '#', '.', ':']);
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let number = text.get(..digits)?.parse().ok()?;
    Some((number, &text[digits..]))
}

/// The short word a title may name a unit of `kind` with, and the endings that make it whole: `ep`, `episode`,
/// `episodes`; `ch`, `chapter`, `chapters`.
fn unit_words(kind: Kind) -> (&'static str, [&'static str; 2]) {
    if kind.is_anime() { ("ep", ["isodes", "isode"]) } else { ("ch", ["apters", "apter"]) }
}

/// Whether a thread title names unit `number` of `kind`: after the kind's word (`episode`, `episodes` or `ep`; `chapter`,
/// `chapters` or `ch`) comes that number, or a range that holds it (`Episodes 1-4`, `Ep 1 ~ 2`, `Chapters 1 to 3`,
/// `Episodes 1 & 2`). The number must end there, so 1 does not match "Episode 10", and a part number is not its whole
/// one ("Chapter 262.2" is not chapter 262).
pub fn names_unit(title: &str, kind: Kind, number: i64) -> bool {
    let (short, endings) = unit_words(kind);
    let lower = title.to_lowercase();
    let mut rest = lower.as_str();
    while let Some(at) = rest.find(short) {
        let before_ok = at == 0 || !rest[..at].chars().next_back().is_some_and(char::is_alphanumeric);
        let after_word = &rest[at + short.len()..];
        let after_word = endings.iter().find_map(|ending| after_word.strip_prefix(ending)).unwrap_or(after_word);
        rest = &rest[at + short.len()..];
        if !before_ok {
            continue;
        }
        let Some((first, tail)) = leading_number(after_word) else { continue };
        if tail.chars().next().is_some_and(|c| c.is_ascii_digit() || c.is_alphabetic()) {
            continue;
        }
        // a part number: `262.2`, `10.5`
        if tail.strip_prefix('.').and_then(|t| t.chars().next()).is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let tail = tail.trim_start();
        let range_rest = ["-", "\u{2013}", "~", "&", "to ", "and "].iter().find_map(|sep| tail.strip_prefix(sep));
        let last = range_rest.and_then(leading_number).map_or(first, |(last, _)| last);
        if (first..=last.max(first)).contains(&number) {
            return true;
        }
    }
    false
}

/// The thread of unit `number` of `kind` among `threads`: one whose title names it; of several, the one with the most
/// replies.
pub fn pick_thread(threads: &[Thread], kind: Kind, number: i64) -> Option<Thread> {
    threads.iter().filter(|t| names_unit(&t.title, kind, number)).max_by_key(|t| t.replies).cloned()
}

/// One request; a status other than 2xx is an error (429 says when to ask again, when AniList says so).
async fn send(transport: &dyn Transport, body: &Value) -> Result<bytes::Bytes, AniListError> {
    let raw = transport.post(body, None).await?;
    match raw.status {
        200..=299 => Ok(raw.body),
        429 => Err(AniListError::RateLimited { retry_after: raw.retry_after }),
        status => Err(AniListError::Http(status)),
    }
}

/// The AniList id of a title from its MyAnimeList id; `None` when AniList does not know it.
pub async fn media_id(transport: &dyn Transport, mal_id: i64, kind: Kind) -> Result<Option<i64>, AniListError> {
    let raw = transport.post(&media_id_request(mal_id, kind), None).await?;
    match raw.status {
        // an unknown id answers 404 with a null media
        200..=299 | 404 => parse_media_id(&raw.body),
        429 => Err(AniListError::RateLimited { retry_after: raw.retry_after }),
        status => Err(AniListError::Http(status)),
    }
}

/// The discussion thread of unit `number` of `kind` (an episode or a chapter): the newest page of the title's
/// discussions, then AniList's search, then up to [`SCAN_PAGES`] pages in all. `None` when no title names it.
pub async fn find_thread(transport: &dyn Transport, media_id: i64, kind: Kind, number: i64) -> Result<Option<Thread>, AniListError> {
    let (first, mut more) = parse_threads(&send(transport, &threads_request(media_id, 1)).await?)?;
    if let Some(thread) = pick_thread(&first, kind, number) {
        return Ok(Some(thread));
    }
    if !more {
        return Ok(None);
    }
    let (found, _) = parse_threads(&send(transport, &thread_search_request(media_id, kind, number)).await?)?;
    if let Some(thread) = pick_thread(&found, kind, number) {
        return Ok(Some(thread));
    }
    let mut page = 2;
    while more && page <= SCAN_PAGES {
        let (threads, next) = parse_threads(&send(transport, &threads_request(media_id, page)).await?)?;
        if let Some(thread) = pick_thread(&threads, kind, number) {
            return Ok(Some(thread));
        }
        more = next;
        page += 1;
    }
    Ok(None)
}

/// The first [`LIMIT`] comments of a thread, with their replies.
pub async fn comments(transport: &dyn Transport, thread_id: i64) -> Result<Vec<Comment>, AniListError> {
    parse_comments(&send(transport, &comments_request(thread_id)).await?)
}

/// The [`LIMIT`] most helpful reviews of a title.
pub async fn reviews(transport: &dyn Transport, media_id: i64) -> Result<Vec<Review>, AniListError> {
    parse_reviews(&send(transport, &reviews_request(media_id)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::anilist::testkit::{FakeTransport, ok, status};

    fn thread(id: i64, title: &str, replies: i64) -> Thread {
        Thread { id, title: title.to_owned(), replies, url: format!("https://anilist.co/forum/thread/{id}") }
    }

    fn page(threads: &[(i64, &str, i64)], more: bool) -> String {
        let list: Vec<Value> = threads.iter().map(|(id, title, replies)| json!({ "id": id, "title": title, "replyCount": replies, "siteUrl": "u" })).collect();
        json!({ "data": { "Page": { "pageInfo": { "hasNextPage": more }, "threads": list } } }).to_string()
    }

    /// Trimmed from the live answer of 2026-11-21 (thread 69474, Frieren episode 10), with one reply as AniList nests it.
    const COMMENTS: &str = r#"{"data":{"Page":{"threadComments":[
      {"id":2448029,"likeCount":83,"createdAt":1699634000,"user":{"name":"SamuraiGinpachi"},
       "comment":"<p>FRIEREN BUILT DIFFERENT SHE MADE AURA KILL HERSELF</p>",
       "childComments":[{"id":2448045,"likeCount":1,"createdAt":1699634480,"user":{"name":"SamuraiGinpachi"},"comment":"<p>Facts</p>","childComments":null}]},
      {"id":2448034,"likeCount":16,"createdAt":1699634100,"user":{"name":"Gese11"},"comment":"<p>Great episode</p>","childComments":null},
      {"id":1,"likeCount":0,"createdAt":0,"user":null,"comment":null,"childComments":null}
    ]}}}"#;

    /// Trimmed from the live answer of 2026-11-21 (Frieren's most helpful review).
    const REVIEWS: &str = r#"{"data":{"Media":{"reviews":{"nodes":[
      {"id":23847,"summary":"Fascinating, fulfilling, and full of feelings","score":100,"rating":336,"ratingAmount":385,
       "siteUrl":"https://anilist.co/review/23847","user":{"name":"befalt"},"body":"<p>Have you ever felt...</p>"}
    ]}}}}"#;

    #[test]
    fn a_title_names_its_episode_and_only_that_one() {
        let ten = "[Spoilers] Sousou No Frieren - Episode 10 Discussion";
        assert!(names_unit(ten, Kind::Anime, 10));
        assert!(!names_unit(ten, Kind::Anime, 1), "episode 1 is not in episode 10");
        assert!(!names_unit(ten, Kind::Anime, 11));
        assert!(names_unit("[Spoilers] One Piece-Episode 1100 discussion", Kind::Anime, 1100));
        assert!(names_unit("Ep. 7 discussion", Kind::Anime, 7));
        assert!(names_unit("Ep 7 discussion", Kind::Anime, 7));
        assert!(names_unit("Episode #3", Kind::Anime, 3));
    }

    #[test]
    fn a_range_holds_every_episode_in_it() {
        let premiere = "[Spoilers] Sousou No Frieren - Premiere/Episodes 1-4 Discussion";
        assert!((1..=4).all(|n| names_unit(premiere, Kind::Anime, n)));
        assert!(!names_unit(premiere, Kind::Anime, 5));
        assert!(names_unit("Episodes 1 ~ 2", Kind::Anime, 2));
        assert!(names_unit("Episodes 5 to 7 discussion", Kind::Anime, 6));
        assert!(names_unit("Episodes 1 & 2", Kind::Anime, 2));
    }

    #[test]
    fn words_that_only_contain_ep_do_not_count() {
        assert!(!names_unit("Deep 10 thoughts", Kind::Anime, 10), "\"deep\" is not \"ep\"");
        assert!(!names_unit("Episode 10b recap", Kind::Anime, 10));
        assert!(!names_unit("Season 2 discussion", Kind::Anime, 2));
    }

    /// Titles from the live answers of 2026-11-22 (Chainsaw Man and Jujutsu Kaisen, category 5).
    #[test]
    fn a_chapter_thread_names_its_chapter_and_a_part_chapter_is_its_own() {
        assert!(names_unit("[Spoilers] Chainsaw Man - Chapter 232 Discussion ", Kind::Manga, 232));
        assert!(!names_unit("[Spoilers] Chainsaw Man - Chapter 232 Discussion ", Kind::Manga, 23));
        assert!(names_unit("[Spoilers] Jujutsu Kaisen - Chapter 271 (Finale) Discussion\n", Kind::Manga, 271));
        assert!(!names_unit("[Spoilers] Jujutsu Kaisen - Chapter 262.2 Discussion\n", Kind::Manga, 262), "262.2 is not 262");
        assert!(!names_unit("Episode 10 Discussion", Kind::Manga, 10), "a manga thread names a chapter");
        assert!(!names_unit("Chapter 10 Discussion", Kind::Anime, 10), "an anime thread names an episode");
        assert!(names_unit("Ch. 57 discussion", Kind::Manga, 57));
        assert!(names_unit("Chapters 1-3", Kind::Manga, 2));
        assert!(!names_unit("Episode 10.5 recap", Kind::Anime, 10), "a part episode is not its whole one");
    }

    #[test]
    fn of_several_matches_the_busiest_thread_wins() {
        let threads = [thread(1, "Episode 3 Discussion", 4), thread(2, "Episode 3 Discussion (mirror)", 40), thread(3, "Episode 30 Discussion", 99)];
        assert_eq!(pick_thread(&threads, Kind::Anime, 3).map(|t| t.id), Some(2));
        assert_eq!(pick_thread(&threads, Kind::Anime, 4), None);
    }

    #[test]
    fn only_whole_known_numbers_are_matched() {
        assert_eq!(whole_number(10.0), Some(10));
        assert_eq!(whole_number(10.5), None);
        assert_eq!(whole_number(-1.0), None);
    }

    #[test]
    fn comments_read_with_their_replies_and_a_broken_one_is_skipped() {
        let comments = parse_comments(COMMENTS.as_bytes()).unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!((comments[0].user.as_str(), comments[0].likes, comments[0].created_at), ("SamuraiGinpachi", 83, 1699634000));
        assert_eq!(comments[0].replies.len(), 1);
        assert_eq!(comments[0].replies[0].html, "<p>Facts</p>");
        assert!(comments[1].replies.is_empty(), "null childComments");
    }

    #[test]
    fn reviews_read_their_score_and_helpfulness() {
        let reviews = parse_reviews(REVIEWS.as_bytes()).unwrap();
        let review = &reviews[0];
        assert_eq!((review.user.as_str(), review.score, review.rating, review.rating_amount), ("befalt", 100, 336, 385));
        assert_eq!(review.url, "https://anilist.co/review/23847");
        assert!(parse_reviews(br#"{"data":{"Media":null}}"#).unwrap().is_empty(), "an unknown title has none");
    }

    #[test]
    fn the_requests_ask_for_fifty_at_most() {
        for body in [comments_request(1), reviews_request(1), threads_request(1, 1), thread_search_request(1, Kind::Anime, 10)] {
            let query = body["query"].as_str().unwrap();
            assert!(query.contains("perPage: 50"), "{query}");
        }
        assert!(comments_request(1)["query"].as_str().unwrap().contains("sort: ID"), "AniList sorts comments by id only");
        assert_eq!(thread_search_request(1, Kind::Anime, 10)["variables"]["search"], "Episode 10");
        assert_eq!(thread_search_request(1, Kind::Manga, 232)["variables"]["search"], "Chapter 232");
        assert!(threads_request(7, 2)["query"].as_str().unwrap().contains("Page(page: 2,"));
    }

    #[test]
    fn a_body_without_data_is_an_error() {
        assert!(parse_comments(br#"{"errors":[{"message":"down"}]}"#).is_err());
        assert!(parse_threads(b"not json").is_err());
    }

    #[tokio::test]
    async fn the_first_page_of_discussions_usually_holds_the_thread() {
        let transport = FakeTransport::new(vec![ok(&page(&[(5, "Episode 5 Discussion", 22), (10, "Episode 10 Discussion", 76)], true))]);
        let found = find_thread(transport.as_ref(), 154587, Kind::Anime, 10).await.unwrap();
        assert_eq!(found.map(|t| t.id), Some(10));
        assert_eq!(transport.calls(), 1);
        assert_eq!(transport.bearers(), [None], "no token");
    }

    #[tokio::test]
    async fn a_short_list_without_the_episode_asks_nothing_more() {
        let transport = FakeTransport::new(vec![ok(&page(&[(5, "Episode 5 Discussion", 22)], false))]);
        assert_eq!(find_thread(transport.as_ref(), 1, Kind::Anime, 10).await.unwrap(), None);
        assert_eq!(transport.calls(), 1);
    }

    #[tokio::test]
    async fn the_search_then_the_older_pages_are_tried_and_the_scan_stops_at_five_pages() {
        let busy = page(&[(1, "Episode 1179 Discussion", 9)], true);
        let none = page(&[], false);
        // page 1, the search, then pages 2 to 5 that each say a next page exists
        let transport = FakeTransport::new(vec![ok(&busy), ok(&none), ok(&busy), ok(&busy), ok(&busy), ok(&busy), ok(&busy)]);
        assert_eq!(find_thread(transport.as_ref(), 21, Kind::Anime, 3).await.unwrap(), None);
        assert_eq!(transport.calls(), 6, "page 1, the search, pages 2 to 5");
        let transport = FakeTransport::new(vec![ok(&busy), ok(&page(&[(72940, "[Spoilers] One Piece-Episode 1100 discussion", 8)], false))]);
        assert_eq!(find_thread(transport.as_ref(), 21, Kind::Anime, 1100).await.unwrap().map(|t| t.id), Some(72940));
        assert_eq!(transport.calls(), 2);
    }

    #[tokio::test]
    async fn a_429_is_a_rate_limit_and_an_unknown_mal_id_is_none() {
        let transport = FakeTransport::new(vec![status(429, "{}")]);
        assert!(matches!(comments(transport.as_ref(), 1).await, Err(AniListError::RateLimited { .. })));
        let transport = FakeTransport::new(vec![status(404, r#"{"errors":[{"message":"Not Found.","status":404}],"data":{"Media":null}}"#)]);
        assert_eq!(media_id(transport.as_ref(), 999_999_999, Kind::Anime).await.unwrap(), None);
        let transport = FakeTransport::new(vec![ok(r#"{"data":{"Media":{"id":154587}}}"#)]);
        assert_eq!(media_id(transport.as_ref(), 52991, Kind::Anime).await.unwrap(), Some(154587));
    }
}
