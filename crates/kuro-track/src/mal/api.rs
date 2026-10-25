//! HTTP endpoints of MAL (`p1-data-and-config.md` section 7, `p1-rust-design.md` section 3.4). Every call except the
//! token exchange sends a Bearer value from [`MalAuth::bearer`]. Non-2xx responses become [`MalError::Http`].

use std::sync::Arc;
use std::time::Duration;

use reqwest::Response;
use serde::de::DeserializeOwned;
use url::Url;

use super::auth::MalAuth;
use super::consts::{API_BASE, OAUTH_BASE, PAGE_SIZE};
use super::dto::{
    AnimeDetails, AnimeListEntry, AnimeListStatus, MangaDetails, MangaListEntry, MangaListStatus,
    SearchPage, User, UserListPage,
};
use super::error::MalError;
use super::token::MalToken;

/// The `fields` values of the detail and list requests, as `MyAnimeListApi.kt` sends them, plus `alternative_titles` for
/// the auto-track match.
pub const MANGA_FIELDS: &str =
    "id,title,synopsis,num_chapters,mean,main_picture,status,media_type,start_date,alternative_titles";
pub const ANIME_FIELDS: &str =
    "id,title,synopsis,num_episodes,mean,main_picture,status,media_type,start_date,alternative_titles";
pub const MANGA_LIST_ENTRY_FIELDS: &str = "num_chapters,my_list_status{start_date,finish_date}";
pub const ANIME_LIST_ENTRY_FIELDS: &str = "num_episodes,my_list_status{start_date,finish_date}";
pub const USER_LIST_FIELDS: &str = "list_status{start_date,finish_date}";

/// Which of the user's lists a `my:` search reads (D-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Manga,
    Anime,
}

impl ListKind {
    fn path(self) -> &'static str {
        match self {
            Self::Manga => "mangalist",
            Self::Anime => "animelist",
        }
    }
}

/// The client: 30 s connect, 30 s read, 120 s total, no cookie store, no HTTP cache (`NetworkHelper.kt:23-25`).
pub fn build_http_client() -> Result<reqwest::Client, MalError> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()?)
}

pub struct MalApi {
    pub(crate) http: reqwest::Client,
    pub(crate) auth: Arc<MalAuth>,
    oauth_base: String,
    api_base: String,
    client_id: String,
}

impl MalApi {
    pub fn new(auth: Arc<MalAuth>, client_id: String) -> Result<Self, MalError> {
        Self::with_bases(auth, client_id, OAUTH_BASE, API_BASE)
    }

    /// Same client with other base URLs, for tests against a local server.
    pub fn with_bases(
        auth: Arc<MalAuth>,
        client_id: String,
        oauth_base: &str,
        api_base: &str,
    ) -> Result<Self, MalError> {
        Ok(Self {
            http: build_http_client()?,
            auth,
            oauth_base: oauth_base.trim_end_matches('/').to_owned(),
            api_base: api_base.trim_end_matches('/').to_owned(),
            client_id,
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    fn api_url(&self, path: &str, query: &[(&str, String)]) -> Result<Url, MalError> {
        let mut url = Url::parse(&format!("{}{}", self.api_base, path))?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        Ok(url)
    }

    /// `POST oauth/token` with the code (E2). No Authorization header.
    pub async fn exchange_code(&self, code: &str, verifier: &str) -> Result<MalToken, MalError> {
        let url = Url::parse(&format!("{}/token", self.oauth_base))?;
        let response = self
            .http
            .post(url)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("code", code),
                ("code_verifier", verifier),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await?;
        decode(check(response)?).await
    }

    /// `GET users/@me` (E4). Returns the user name.
    pub async fn current_user(&self) -> Result<String, MalError> {
        let user: User = self.get(self.api_url("/users/@me", &[])?).await?;
        Ok(user.name)
    }

    /// `GET {manga,anime}?q=&nsfw=true` (E5, E6). Returns the ids in MAL order. Only one `q` is sent.
    pub async fn search_ids(&self, kind: &str, q: &str) -> Result<Vec<i64>, MalError> {
        let url = self.api_url(
            &format!("/{kind}"),
            &[("q", q.to_owned()), ("nsfw", "true".to_owned())],
        )?;
        let page: SearchPage = self.get(url).await?;
        Ok(page.data.into_iter().map(|n| n.node.id).collect())
    }

    /// `GET manga/{id}` with the detail fields (E7).
    pub async fn manga_details(&self, id: i64) -> Result<MangaDetails, MalError> {
        self.get(self.api_url(
            &format!("/manga/{id}"),
            &[("fields", MANGA_FIELDS.to_owned())],
        )?)
        .await
    }

    /// `GET anime/{id}` with the detail fields (E8).
    pub async fn anime_details(&self, id: i64) -> Result<AnimeDetails, MalError> {
        self.get(self.api_url(
            &format!("/anime/{id}"),
            &[("fields", ANIME_FIELDS.to_owned())],
        )?)
        .await
    }

    /// `GET manga/{id}` for the user's list status (E9).
    pub async fn manga_list_entry(&self, id: i64) -> Result<MangaListEntry, MalError> {
        self.get(self.api_url(
            &format!("/manga/{id}"),
            &[("fields", MANGA_LIST_ENTRY_FIELDS.to_owned())],
        )?)
        .await
    }

    /// `GET anime/{id}` for the user's list status (E10).
    pub async fn anime_list_entry(&self, id: i64) -> Result<AnimeListEntry, MalError> {
        self.get(self.api_url(
            &format!("/anime/{id}"),
            &[("fields", ANIME_LIST_ENTRY_FIELDS.to_owned())],
        )?)
        .await
    }

    /// One page of `users/@me/{mangalist,animelist}` (E11). `offset` is sent only when above 0.
    pub async fn user_list_page(
        &self,
        kind: ListKind,
        offset: u32,
    ) -> Result<UserListPage, MalError> {
        let mut query = vec![
            ("fields", USER_LIST_FIELDS.to_owned()),
            ("limit", PAGE_SIZE.to_string()),
        ];
        if offset > 0 {
            query.push(("offset", offset.to_string()));
        }
        self.get(self.api_url(&format!("/users/@me/{}", kind.path()), &query)?)
            .await
    }

    /// `PUT manga/{id}/my_list_status` with the form in order (E12).
    pub async fn put_manga(
        &self,
        id: i64,
        form: &[(&str, String)],
    ) -> Result<MangaListStatus, MalError> {
        let url = self.api_url(&format!("/manga/{id}/my_list_status"), &[])?;
        self.send_form(reqwest::Method::PUT, url, form).await
    }

    /// `PUT anime/{id}/my_list_status` with the form in order (E13).
    pub async fn put_anime(
        &self,
        id: i64,
        form: &[(&str, String)],
    ) -> Result<AnimeListStatus, MalError> {
        let url = self.api_url(&format!("/anime/{id}/my_list_status"), &[])?;
        self.send_form(reqwest::Method::PUT, url, form).await
    }

    /// `DELETE {manga,anime}/{id}/my_list_status` (E14, E15). The response body is ignored.
    pub async fn delete_list_item(&self, kind: &str, id: i64) -> Result<(), MalError> {
        let url = self.api_url(&format!("/{kind}/{id}/my_list_status"), &[])?;
        self.authed(|token| self.http.delete(url.clone()).bearer_auth(token))
            .await?;
        Ok(())
    }

    async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T, MalError> {
        let response = self
            .authed(|token| self.http.get(url.clone()).bearer_auth(token))
            .await?;
        decode(response).await
    }

    async fn send_form<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: Url,
        form: &[(&str, String)],
    ) -> Result<T, MalError> {
        let response = self
            .authed(|token| {
                self.http
                    .request(method.clone(), url.clone())
                    .bearer_auth(token)
                    .form(form)
            })
            .await?;
        decode(response).await
    }

    /// Sends one request with the Bearer value. A 401 refreshes the token once and sends the request again (T-29a-053).
    /// A 401 on that retry is `Http(401)`, and nothing loops. A refresh that MAL refuses with 401 sets the
    /// auth-expired flag and fails with [`MalError::LoginExpired`] (T-27a-074).
    async fn authed(
        &self,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> Result<Response, MalError> {
        let token = self.auth.bearer(&self.http).await?;
        let response = build(&token).send().await?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return check(response);
        }
        let fresh = self
            .auth
            .refresh_after_refusal(&self.http, &token)
            .await?;
        check(build(&fresh).send().await?)
    }
}

/// Non-2xx responses become `Http(code)`.
fn check(response: Response) -> Result<Response, MalError> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else {
        Err(MalError::Http(status.as_u16()))
    }
}

/// Decodes the body with serde_json, so decode errors are [`MalError::Decode`].
async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, MalError> {
    let bytes = response.bytes().await?;
    Ok(serde_json::from_slice(&bytes)?)
}
