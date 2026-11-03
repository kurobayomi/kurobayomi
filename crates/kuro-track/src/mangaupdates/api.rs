//! MangaUpdates REST calls M1 to M9. Authenticated calls take the bearer token
//! from [`MuAuth`] and fail with `NotAuthenticated` before any request when there is none. Search and login are
//! unauthenticated. Bodies are typed structs, so their key order is the declaration order (byte-compared in tests).

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, RequestBuilder, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::auth::MuAuth;
use super::dto::{ListItem, LoginContext, LoginResponse, Rating, Record, SearchResponse};
use super::error::MuError;

pub const BASE_URL: &str = "https://api.mangaupdates.com";
/// Content type of every JSON body (`MangaUpdatesApi.kt:195`).
const VND_API_JSON: &str = "application/vnd.api+json";
/// Sent on every request. Replaces the Aniyomi string (B-2.6).
pub const USER_AGENT: &str = concat!(
    "Kurobayomi v",
    env!("CARGO_PKG_VERSION"),
    " (io.github.kurobayomi.Kurobayomi)"
);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How much of an error reply is kept for the log (R-9).
const SNIPPET_BYTES: usize = 512;

/// Result of the rating GET (fix D-MU-3). `Unknown` means the fetch failed, so nothing may be decided from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RatingFetch {
    /// The server's rating, or `None` when the series has no rating.
    Known(Option<f64>),
    Unknown,
}

#[derive(Serialize)]
struct SeriesId {
    id: i64,
}

#[derive(Serialize)]
struct AddEntry {
    series: SeriesId,
    list_id: i64,
}

#[derive(Serialize)]
struct ChapterStatus {
    chapter: i64,
}

#[derive(Serialize)]
struct UpdateEntry {
    series: SeriesId,
    list_id: i64,
    status: ChapterStatus,
}

#[derive(Serialize)]
struct RatingBody {
    rating: f64,
}

#[derive(Serialize)]
struct SearchBody<'a> {
    search: &'a str,
    filter_types: [&'static str; 2],
}

#[derive(Serialize)]
struct LoginBody<'a> {
    username: &'a str,
    password: &'a str,
}

pub struct MuApi {
    http: reqwest::Client,
    base: String,
    auth: Arc<MuAuth>,
}

impl MuApi {
    pub fn new(base: &str, auth: Arc<MuAuth>) -> Result<Self, MuError> {
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

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn authed(&self, method: Method, path: &str) -> Result<RequestBuilder, MuError> {
        let token = self.auth.bearer()?;
        Ok(self.http.request(method, self.url(path)).bearer_auth(token))
    }

    /// M1: the list item of a series. A series that is not on the list is `Http { status: 404 }`.
    pub async fn list_item(&self, remote_id: i64) -> Result<ListItem, MuError> {
        let path = format!("/v1/lists/series/{remote_id}");
        let resp = self.authed(Method::GET, &path)?.send().await?;
        read_json(resp).await
    }

    /// M2: the rating of a series. Any failure is `Unknown`, never a zero rating.
    pub async fn rating(&self, remote_id: i64) -> RatingFetch {
        match self.get_rating(remote_id).await {
            Ok(r) => RatingFetch::Known(r.rating),
            Err(_) => RatingFetch::Unknown,
        }
    }

    async fn get_rating(&self, remote_id: i64) -> Result<Rating, MuError> {
        let path = format!("/v1/series/{remote_id}/rating");
        let resp = self.authed(Method::GET, &path)?.send().await?;
        read_json(resp).await
    }

    /// M3: adds a series to list `list_id`. Returns the HTTP status of the reply.
    pub async fn add_to_list(&self, remote_id: i64, list_id: i64) -> Result<u16, MuError> {
        let body = [AddEntry {
            series: SeriesId { id: remote_id },
            list_id,
        }];
        let req = json_body(self.authed(Method::POST, "/v1/lists/series")?, &body)?;
        let resp = checked(req.send().await?).await?;
        Ok(resp.status().as_u16())
    }

    /// M4: sets the list and the chapter. The chapter is truncated toward zero (`toInt()`, so 12.9 sends 12).
    pub async fn update_list_item(
        &self,
        remote_id: i64,
        list_id: i64,
        last_chapter_read: f64,
    ) -> Result<(), MuError> {
        let body = [UpdateEntry {
            series: SeriesId { id: remote_id },
            list_id,
            status: ChapterStatus {
                chapter: last_chapter_read.trunc() as i64,
            },
        }];
        let req = json_body(self.authed(Method::POST, "/v1/lists/series/update")?, &body)?;
        checked(req.send().await?).await.map(|_| ())
    }

    /// M5: sets the rating of a series.
    pub async fn set_rating(&self, remote_id: i64, rating: f64) -> Result<(), MuError> {
        let path = format!("/v1/series/{remote_id}/rating");
        let req = json_body(self.authed(Method::PUT, &path)?, &RatingBody { rating })?;
        checked(req.send().await?).await.map(|_| ())
    }

    /// M6: removes the rating of a series.
    pub async fn delete_rating(&self, remote_id: i64) -> Result<(), MuError> {
        let path = format!("/v1/series/{remote_id}/rating");
        let resp = self.authed(Method::DELETE, &path)?.send().await?;
        checked(resp).await.map(|_| ())
    }

    /// M7: removes a series from the list. The body is the one-element array `[remote_id]`.
    pub async fn delete_from_list(&self, remote_id: i64) -> Result<(), MuError> {
        let body = [remote_id];
        let req = json_body(self.authed(Method::POST, "/v1/lists/series/delete")?, &body)?;
        checked(req.send().await?).await.map(|_| ())
    }

    /// M8: series search, without authentication. The filter keeps only drama CDs and novels.
    pub async fn search(&self, query: &str) -> Result<Vec<Record>, MuError> {
        let body = SearchBody {
            search: query,
            filter_types: ["drama cd", "novel"],
        };
        let req = json_body(self.http.post(self.url("/v1/series/search")), &body)?;
        let res: SearchResponse = read_json(req.send().await?).await?;
        Ok(res.results.into_iter().map(|hit| hit.record).collect())
    }

    /// M9: exchanges the account name and password for a session. The password is sent and not kept.
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginContext, MuError> {
        let req = json_body(
            self.http.put(self.url("/v1/account/login")),
            &LoginBody { username, password },
        )?;
        let res: LoginResponse = read_json(req.send().await?).await?;
        Ok(res.context)
    }
}

fn json_body<B: Serialize + ?Sized>(
    req: RequestBuilder,
    body: &B,
) -> Result<RequestBuilder, MuError> {
    Ok(req
        .header(CONTENT_TYPE, VND_API_JSON)
        .body(serde_json::to_vec(body)?))
}

/// A non-2xx reply becomes `Http { status, body_snippet }`. The snippet is redacted, so no token text reaches an error.
async fn checked(resp: Response) -> Result<Response, MuError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(MuError::Http {
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

async fn read_json<T: DeserializeOwned>(resp: Response) -> Result<T, MuError> {
    let text = checked(resp).await?.text().await?;
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_32a_011_user_agent_names_the_app_and_its_version() {
        assert!(USER_AGENT.starts_with("Kurobayomi v"));
        assert!(USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
        assert!(!USER_AGENT.contains("Aniyomi"));
    }

    #[test]
    fn t_32a_011_base_url_loses_a_trailing_slash() {
        let api = MuApi::new("http://127.0.0.1:9/", Arc::new(MuAuth::new(None))).unwrap();
        assert_eq!(api.url("/v1/x"), "http://127.0.0.1:9/v1/x");
    }
}
