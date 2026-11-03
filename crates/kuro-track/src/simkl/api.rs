//! Simkl REST calls (S2 to S11). Authenticated calls take the bearer token from
//! [`SimklAuth`] and fail with `NotAuthenticated` before any request when there is none. The token exchange and search
//! are unauthenticated. Bodies are typed structs, so their key order is the declaration order.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{IntoUrl, Method, RequestBuilder, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use super::auth::SimklAuth;
use super::dto::{
    SimklOAuth, SimklSearchResult, SimklSyncResult, SimklSyncWatched, SimklUser, WatchedQuery,
};
use super::error::SimklError;

pub const CLIENT_ID: &str = "aa62a7da32518aae5d5049a658b87fa4837c3b739e06ed250b315aab6af82b0e";
pub const CLIENT_SECRET: &str = "2bec9c1d0c00a1e9b0e9e096a71f88d555a6f52da7923df07906df3b21351783";
pub const SITE_URL: &str = "https://simkl.com";
pub const API_URL: &str = "https://api.simkl.com";
/// The redirect the Simkl app registration expects (B-3.7). Linux handles it through the desktop scheme (O-6).
pub const REDIRECT_URL: &str = "kurobayomi://simkl-auth";
/// Content type of the JSON bodies (`jsonMime`, `OkHttpExtensions.kt:22`).
const JSON_MIME: &str = "application/json; charset=utf-8";
/// Sent on every request. Replaces the Aniyomi string (B-3.13).
pub const USER_AGENT: &str = concat!(
    "Kurobayomi v",
    env!("CARGO_PKG_VERSION"),
    " (io.github.anas1412.Kurobayomi)"
);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How much of an error reply is kept for the log (R-9).
const SNIPPET_BYTES: usize = 512;

/// The list a write goes to: `shows` or `movies` (B-3.8, `if (type == "movies") "movies" else "shows"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Shows,
    Movies,
}

/// Search path of S11 (`anime`, `tv`, `movie`; singular for movies, B-3.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Anime,
    Tv,
    Movie,
}

impl SearchKind {
    pub fn path(self) -> &'static str {
        match self {
            SearchKind::Anime => "anime",
            SearchKind::Tv => "tv",
            SearchKind::Movie => "movie",
        }
    }

    /// The segment stored in a tracking URL when the result URL has no known one. A movie is stored as `movies`
    /// (R-5.2).
    pub fn stored_segment(self) -> &'static str {
        match self {
            SearchKind::Anime => "anime",
            SearchKind::Tv => "tv",
            SearchKind::Movie => "movies",
        }
    }
}

/// The one-key body of the list, progress and rating calls: `{"shows": [...]}` or `{"movies": [...]}`.
#[derive(Serialize)]
struct MediaBody<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    shows: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    movies: Option<T>,
}

impl<T> MediaBody<T> {
    fn new(kind: MediaKind, items: T) -> Self {
        match kind {
            MediaKind::Shows => Self {
                shows: Some(items),
                movies: None,
            },
            MediaKind::Movies => Self {
                shows: None,
                movies: Some(items),
            },
        }
    }
}

#[derive(Serialize)]
struct Ids {
    simkl: i64,
}

#[derive(Serialize)]
struct ListEntry<'a> {
    ids: Ids,
    to: &'a str,
}

#[derive(Serialize)]
struct RatingEntry {
    ids: Ids,
    rating: i32,
}

#[derive(Serialize)]
struct Episode {
    number: u32,
}

/// A season of a progress body. `episodes` is absent for the remove call and present (possibly empty) for the add.
#[derive(Serialize)]
struct Season {
    number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    episodes: Option<Vec<Episode>>,
}

#[derive(Serialize)]
struct ShowProgress {
    ids: Ids,
    seasons: Vec<Season>,
}

/// The body of S2. Declaration order is the wire order (`code`, `client_id`, `client_secret`, `redirect_uri`,
/// `grant_type`).
#[derive(Serialize)]
struct TokenRequest<'a> {
    code: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    redirect_uri: &'a str,
    grant_type: &'a str,
}

pub struct SimklApi {
    http: reqwest::Client,
    base: String,
    auth: Arc<SimklAuth>,
}

impl SimklApi {
    pub fn new(base: &str, auth: Arc<SimklAuth>) -> Result<Self, SimklError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            auth,
        })
    }

    /// `base` with the given path segments appended. Each segment is percent-encoded as one segment.
    fn endpoint(&self, segments: &[&str]) -> Result<Url, SimklError> {
        let mut url = Url::parse(&self.base).map_err(|e| SimklError::Transport(e.to_string()))?;
        url.path_segments_mut()
            .map_err(|()| SimklError::Transport("base URL cannot take a path".into()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// An authenticated request: the bearer token and the client key. `NotAuthenticated` before any request when no
    /// token is held.
    fn authed(&self, method: Method, url: impl IntoUrl) -> Result<RequestBuilder, SimklError> {
        let token = self.auth.bearer()?;
        Ok(self
            .http
            .request(method, url)
            .bearer_auth(token)
            .header("simkl-api-key", CLIENT_ID))
    }

    /// S2: exchanges the one-time code for an access token (`accessToken`, SimklApi.kt:215-234).
    pub async fn access_token(&self, code: &str) -> Result<SimklOAuth, SimklError> {
        let body = TokenRequest {
            code,
            client_id: CLIENT_ID,
            client_secret: CLIENT_SECRET,
            redirect_uri: REDIRECT_URL,
            grant_type: "authorization_code",
        };
        let resp = self
            .http
            .post(self.endpoint(&["oauth", "token"])?)
            .header(CONTENT_TYPE, JSON_MIME)
            .body(serde_json::to_vec(&body)?)
            .send()
            .await?;
        read_json(resp).await
    }

    /// S3: the account id of the stored token's owner (`getCurrentUser`, SimklApi.kt:204-213). The id is required in
    /// the reply, so a missing account is an error and not id 0. The token must already be held.
    pub async fn current_user_id(&self) -> Result<i64, SimklError> {
        let req = self.authed(Method::GET, self.endpoint(&["users", "settings"])?)?;
        let user: SimklUser = read_json(req.send().await?).await?;
        Ok(user.account.id)
    }

    /// S4: the list state of one title (`findLibAnime`, SimklApi.kt:171-174). `None` when the reply is empty.
    pub async fn find_watched(
        &self,
        remote_id: i64,
    ) -> Result<Option<SimklSyncWatched>, SimklError> {
        let body = [WatchedQuery { simkl: remote_id }];
        let req = json_body(
            self.authed(Method::POST, self.endpoint(&["sync", "watched"])?)?,
            &body,
        )?;
        let list: Vec<SimklSyncWatched> = read_json(req.send().await?).await?;
        Ok(list.into_iter().next())
    }

    /// S5: the user's items of one kind and status changed since `date_from`. `date_from` is sent unchanged, encoded
    /// once (`Uri.Builder`, TST-48).
    pub async fn all_items(
        &self,
        path: &str,
        status: &str,
        date_from: &str,
    ) -> Result<SimklSyncResult, SimklError> {
        let url = self.endpoint(&["sync", "all-items", path, status])?;
        let req = self
            .authed(Method::GET, url)?
            .query(&[("date_from", date_from)]);
        read_json(req.send().await?).await
    }

    /// S6: sets the list status of a title. The same call adds a title and changes its status.
    pub async fn add_to_list(
        &self,
        kind: MediaKind,
        remote_id: i64,
        to: &str,
    ) -> Result<(), SimklError> {
        let entries = [ListEntry {
            ids: Ids { simkl: remote_id },
            to,
        }];
        self.post(&["sync", "add-to-list"], &MediaBody::new(kind, entries))
            .await
    }

    /// S7: removes the title's history. Always season 1 (B-3.8.1).
    pub async fn history_remove(&self, remote_id: i64) -> Result<(), SimklError> {
        let show = ShowProgress {
            ids: Ids { simkl: remote_id },
            seasons: vec![Season {
                number: 1,
                episodes: None,
            }],
        };
        self.post(
            &["sync", "history", "remove"],
            &MediaBody::new(MediaKind::Shows, [show]),
        )
        .await
    }

    /// S8: marks episodes 1 to `last_episode` of season 1 as watched. Zero sends an empty list.
    pub async fn history_add(&self, remote_id: i64, last_episode: u32) -> Result<(), SimklError> {
        let episodes = (1..=last_episode)
            .map(|number| Episode { number })
            .collect();
        let show = ShowProgress {
            ids: Ids { simkl: remote_id },
            seasons: vec![Season {
                number: 1,
                episodes: Some(episodes),
            }],
        };
        self.post(
            &["sync", "history"],
            &MediaBody::new(MediaKind::Shows, [show]),
        )
        .await
    }

    /// S9: sets the user's rating, an integer from 0 to 10.
    pub async fn rating_set(
        &self,
        kind: MediaKind,
        remote_id: i64,
        rating: i32,
    ) -> Result<(), SimklError> {
        let entries = [RatingEntry {
            ids: Ids { simkl: remote_id },
            rating,
        }];
        self.post(&["sync", "ratings"], &MediaBody::new(kind, entries))
            .await
    }

    /// S10: removes the rating. The body still carries `rating: 0` (B-3.8.3).
    pub async fn rating_remove(&self, kind: MediaKind, remote_id: i64) -> Result<(), SimklError> {
        let entries = [RatingEntry {
            ids: Ids { simkl: remote_id },
            rating: 0,
        }];
        self.post(
            &["sync", "ratings", "remove"],
            &MediaBody::new(kind, entries),
        )
        .await
    }

    /// S11: one search of a kind. Unauthenticated, with the client id as a query parameter (B-3.14).
    pub async fn search(
        &self,
        kind: SearchKind,
        query: &str,
    ) -> Result<Vec<SimklSearchResult>, SimklError> {
        let url = self.endpoint(&["search", kind.path()])?;
        let resp = self
            .http
            .get(url)
            .query(&[("q", query), ("extended", "full"), ("client_id", CLIENT_ID)])
            .send()
            .await?;
        read_json(resp).await
    }

    /// An authenticated POST whose reply is ignored.
    async fn post<B: Serialize + ?Sized>(
        &self,
        segments: &[&str],
        body: &B,
    ) -> Result<(), SimklError> {
        let req = json_body(self.authed(Method::POST, self.endpoint(segments)?)?, body)?;
        checked(req.send().await?).await.map(|_| ())
    }
}

fn json_body<B: Serialize + ?Sized>(
    req: RequestBuilder,
    body: &B,
) -> Result<RequestBuilder, SimklError> {
    Ok(req
        .header(CONTENT_TYPE, JSON_MIME)
        .body(serde_json::to_vec(body)?))
}

/// A non-2xx reply becomes `Http { status, body_snippet }`. The snippet is the start of the body with tokens redacted.
async fn checked(resp: Response) -> Result<Response, SimklError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(SimklError::Http {
        status: status.as_u16(),
        body_snippet: snippet(&body),
    })
}

/// What of an error body reaches the log: its first 512 bytes, or a placeholder when those bytes name a credential.
// ponytail: a snippet that names a credential word is dropped whole. That is coarser than a redaction but cannot leak.
fn snippet(body: &str) -> String {
    const CREDENTIAL_WORDS: [&str; 5] = ["token", "password", "secret", "session", "code"];
    let mut end = body.len().min(SNIPPET_BYTES);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let head = &body[..end];
    let lower = head.to_ascii_lowercase();
    if CREDENTIAL_WORDS.iter().any(|w| lower.contains(w)) {
        return "[withheld]".to_owned();
    }
    head.to_owned()
}

async fn read_json<T: DeserializeOwned>(resp: Response) -> Result<T, SimklError> {
    let text = checked(resp).await?.text().await?;
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_32a_031_the_client_constants_are_the_source_values() {
        assert_eq!(CLIENT_ID.len(), 64);
        assert_eq!(CLIENT_SECRET.len(), 64);
        assert_eq!(REDIRECT_URL, "kurobayomi://simkl-auth");
        assert_eq!(API_URL, "https://api.simkl.com");
    }

    #[test]
    fn t_32a_032_the_user_agent_names_the_app_and_its_version() {
        assert!(USER_AGENT.starts_with("Kurobayomi v"));
        assert!(!USER_AGENT.contains("Aniyomi"));
    }

    #[test]
    fn t_32a_035_an_endpoint_keeps_the_base_and_encodes_each_segment() {
        let api = SimklApi::new("http://127.0.0.1:9/", Arc::new(SimklAuth::new(None))).unwrap();
        assert_eq!(
            api.endpoint(&["sync", "all-items", "shows", "a b"])
                .unwrap()
                .as_str(),
            "http://127.0.0.1:9/sync/all-items/shows/a%20b"
        );
    }

    #[test]
    fn t_32a_036_search_kinds_use_the_source_paths_and_stored_segments() {
        assert_eq!(SearchKind::Movie.path(), "movie");
        assert_eq!(SearchKind::Movie.stored_segment(), "movies");
        assert_eq!(SearchKind::Tv.stored_segment(), "tv");
    }

    #[test]
    fn t_32a_042_the_progress_bodies_match_the_source_shape() {
        let remove = MediaBody::new(
            MediaKind::Shows,
            [ShowProgress {
                ids: Ids { simkl: 5 },
                seasons: vec![Season {
                    number: 1,
                    episodes: None,
                }],
            }],
        );
        assert_eq!(
            serde_json::to_string(&remove).unwrap(),
            r#"{"shows":[{"ids":{"simkl":5},"seasons":[{"number":1}]}]}"#
        );
        let add = MediaBody::new(
            MediaKind::Shows,
            [ShowProgress {
                ids: Ids { simkl: 5 },
                seasons: vec![Season {
                    number: 1,
                    episodes: Some(vec![Episode { number: 1 }, Episode { number: 2 }]),
                }],
            }],
        );
        assert_eq!(
            serde_json::to_string(&add).unwrap(),
            r#"{"shows":[{"ids":{"simkl":5},"seasons":[{"number":1,"episodes":[{"number":1},{"number":2}]}]}]}"#
        );
    }

    #[test]
    fn t_32a_040_a_movie_list_body_uses_the_movies_key_only() {
        let body = MediaBody::new(
            MediaKind::Movies,
            [ListEntry {
                ids: Ids { simkl: 9 },
                to: "watching",
            }],
        );
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"movies":[{"ids":{"simkl":9},"to":"watching"}]}"#
        );
    }
}
