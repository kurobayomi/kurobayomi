//! Token state, refresh with single flight, and the auth-expired flag.
//! The state lock is a `std` mutex that is never held across an await. The
//! refresh gate is a `tokio` mutex, so only one refresh is in flight.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use reqwest::StatusCode;
use url::Url;

use super::dates::now_secs;
use super::error::MalError;
use super::store::{MalSettings, TokenStore};
use super::token::MalToken;

/// Seconds after a refresh that failed without a 401 during which no new refresh is tried (D-06).
pub const REFRESH_BACKOFF_S: i64 = 30;

#[derive(Default)]
struct AuthState {
    token: Option<MalToken>,
    /// Bumped by logout. A refresh that started before a logout does not store its result.
    epoch: u64,
    /// When the last refresh failed without a 401, in epoch seconds. Cleared by a successful refresh.
    refresh_failed_at: Option<i64>,
}

pub struct MalAuth {
    tokens: Arc<dyn TokenStore>,
    settings: Arc<MalSettings>,
    state: Mutex<AuthState>,
    refresh_gate: tokio::sync::Mutex<()>,
    refresh_url: Url,
    client_id: String,
}

impl MalAuth {
    /// Loads the saved token. A corrupt token is treated as logged out by the store.
    pub fn new(
        tokens: Arc<dyn TokenStore>,
        settings: Arc<MalSettings>,
        refresh_url: Url,
        client_id: String,
    ) -> Result<Self, MalError> {
        let token = tokens.load()?;
        Ok(Self {
            tokens,
            settings,
            state: Mutex::new(AuthState {
                token,
                epoch: 0,
                refresh_failed_at: None,
            }),
            refresh_gate: tokio::sync::Mutex::new(()),
            refresh_url,
            client_id,
        })
    }

    pub fn has_token(&self) -> bool {
        self.lock().token.is_some()
    }

    pub fn token(&self) -> Option<MalToken> {
        self.lock().token.clone()
    }

    /// The persisted auth-expired flag (`TrackPreferences.kt:24-27`).
    pub fn expired(&self) -> bool {
        self.settings.auth_expired()
    }

    pub fn set_expired(&self, value: bool) -> Result<(), MalError> {
        self.settings.set_auth_expired(value)
    }

    /// Stores a token (`setAuth`). The token is written to the store before memory changes.
    pub fn set_token(&self, token: MalToken) -> Result<(), MalError> {
        let mut state = self.lock();
        self.tokens.save(&token)?;
        state.token = Some(token);
        Ok(())
    }

    /// Removes the token from memory and from the store, and invalidates any refresh in flight.
    pub fn clear_token(&self) -> Result<(), MalError> {
        let mut state = self.lock();
        state.epoch += 1;
        state.token = None;
        state.refresh_failed_at = None;
        self.tokens.clear()
    }

    /// The Bearer value for one request, refreshing first when the token is past its margin.
    pub async fn bearer(&self, http: &reqwest::Client) -> Result<String, MalError> {
        self.bearer_at(http, now_secs()).await
    }

    /// [`MalAuth::bearer`] at the epoch second `now`. The clock is a parameter so the refresh backoff can be tested.
    pub(crate) async fn bearer_at(
        &self,
        http: &reqwest::Client,
        now: i64,
    ) -> Result<String, MalError> {
        if self.expired() {
            return Err(MalError::LoginExpired);
        }
        let token = self.token().ok_or(MalError::NotAuthenticated)?;
        if !token.is_expired(now) {
            return Ok(token.access_token);
        }

        let _gate = self.refresh_gate.lock().await;
        // Re-read under the gate: a caller that waited may find a fresh token already stored.
        let (token, epoch, failed_at) = self.snapshot()?;
        if !token.is_expired(now) {
            return Ok(token.access_token);
        }
        if self.expired() {
            return Err(MalError::LoginExpired);
        }
        self.refresh_gated(http, &token, epoch, failed_at, now).await
    }

    /// Refreshes after the server refused `refused` with 401, whatever the token's expiry (T-29a-053). When another
    /// caller already replaced `refused` while this one waited for the gate, that token is returned without a request.
    pub(crate) async fn refresh_after_refusal(
        &self,
        http: &reqwest::Client,
        refused: &str,
    ) -> Result<String, MalError> {
        let _gate = self.refresh_gate.lock().await;
        let (token, epoch, failed_at) = self.snapshot()?;
        if token.access_token != refused {
            return Ok(token.access_token);
        }
        self.refresh_gated(http, &token, epoch, failed_at, now_secs())
            .await
    }

    fn snapshot(&self) -> Result<(MalToken, u64, Option<i64>), MalError> {
        let state = self.lock();
        Ok((
            state.token.clone().ok_or(MalError::NotAuthenticated)?,
            state.epoch,
            state.refresh_failed_at,
        ))
    }

    /// Runs one refresh while the gate is held. After a refresh that failed without a 401, no new refresh is tried
    /// for [`REFRESH_BACKOFF_S`] seconds, and the call returns `RefreshFailed` at once (D-06).
    async fn refresh_gated(
        &self,
        http: &reqwest::Client,
        old: &MalToken,
        epoch: u64,
        failed_at: Option<i64>,
        now: i64,
    ) -> Result<String, MalError> {
        if failed_at.is_some_and(|at| now - at < REFRESH_BACKOFF_S) {
            return Err(MalError::RefreshFailed);
        }
        match self.refresh(http, old, epoch).await {
            Err(MalError::RefreshFailed) => {
                self.lock().refresh_failed_at = Some(now);
                Err(MalError::RefreshFailed)
            }
            other => other,
        }
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        old: &MalToken,
        epoch: u64,
    ) -> Result<String, MalError> {
        let response = http
            .post(self.refresh_url.clone())
            .bearer_auth(&old.access_token)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("refresh_token", old.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "MAL token refresh did not complete");
                MalError::RefreshFailed
            })?;

        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            self.set_expired(true)?;
            return Err(MalError::LoginExpired);
        }
        if !status.is_success() {
            tracing::warn!(status = status.as_u16(), "MAL token refresh was refused");
            return Err(MalError::RefreshFailed);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| MalError::RefreshFailed)?;
        let fresh: MalToken =
            serde_json::from_slice(&bytes).map_err(|_| MalError::RefreshFailed)?;

        let mut state = self.lock();
        if state.epoch != epoch {
            // Logout ran while the refresh was pending: the result must not bring the token back.
            return Err(MalError::NotAuthenticated);
        }
        self.tokens.save(&fresh)?;
        let access = fresh.access_token.clone();
        state.token = Some(fresh);
        state.refresh_failed_at = None;
        Ok(access)
    }

    fn lock(&self) -> MutexGuard<'_, AuthState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
