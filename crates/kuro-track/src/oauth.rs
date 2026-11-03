//! OAuth login plumbing: authorise URLs,
//! PKCE, the `state` value, pending attempts with a TTL, and redirect parsing. The random source is the only
//! side effect. The caller owns the token exchange and the secret store.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use url::Url;

use crate::messages::MsgKey;
use crate::model::TrackerId;

/// A login attempt stays valid this long after its authorise URL was opened.
pub const PENDING_TTL_MS: i64 = 5 * 60 * 1000;

/// `state` bytes. 32 bytes encode to 43 URL-safe characters.
const STATE_BYTES: usize = 32;

/// Kurobayomi's own AniList client (redirect `kurobayomi://anilist-auth`); the implicit grant needs no secret.
const ANILIST_CLIENT_ID: &str = "53424";
const SIMKL_CLIENT_ID: &str = "aa62a7da32518aae5d5049a658b87fa4837c3b739e06ed250b315aab6af82b0e";

/// The client secrets of the trackers that exchange a code with one (T-27b-022). They come from configuration, never
/// from the source. A missing or empty secret turns that tracker's browser login off. Shikimori and Bangumi are not
/// here: their client (id and secret) is the tracker's own `OAuthClient`, and their login gate is
/// `Tracker::setup_message` (T-31a-039).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientSecrets {
    pub simkl: Option<String>,
}

impl ClientSecrets {
    /// The secret of `tracker`. `None` when it is missing, empty, or the tracker uses none.
    pub fn get(&self, tracker: TrackerId) -> Option<&str> {
        let secret = match tracker {
            TrackerId::SIMKL => &self.simkl,
            _ => return None,
        };
        secret.as_deref().filter(|s| !s.trim().is_empty())
    }

    /// True when `tracker` can start a browser login: it needs no secret, or its secret is set.
    pub fn allows_login(&self, tracker: TrackerId) -> bool {
        tracker != TrackerId::SIMKL || self.get(tracker).is_some()
    }
}

/// `n` random bytes as URL-safe base64 without padding.
fn random_token(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The authorise URL of a browser login, with the source's query order and `state` last. `None` for trackers
/// without a browser login here (Kitsu, MangaUpdates, enhanced trackers). MyAnimeList is not listed: its URL needs the
/// user's client id and a PKCE verifier, and comes from `mal::MalTracker::begin_login` (T-29a-040). Shikimori and
/// Bangumi are not listed either: their URL needs the user's client and the redirect URI area 27 chose, and comes from
/// `Tracker::browser_auth_url` through [`PendingLogins::begin_configured`] (T-31a-038, T-31a-039).
pub fn authorize_url(tracker: TrackerId, state: &str) -> Option<String> {
    let (base, params): (&str, Vec<(&str, &str)>) = match tracker {
        TrackerId::ANILIST => (
            "https://anilist.co/api/v2/oauth/authorize",
            vec![("client_id", ANILIST_CLIENT_ID), ("response_type", "token")],
        ),
        TrackerId::SIMKL => (
            "https://simkl.com/oauth/authorize",
            vec![
                ("response_type", "code"),
                ("client_id", SIMKL_CLIENT_ID),
                ("redirect_uri", "kurobayomi://simkl-auth"),
            ],
        ),
        _ => return None,
    };
    let mut url = Url::parse(base).ok()?;
    url.query_pairs_mut()
        .extend_pairs(params)
        .append_pair("state", state);
    Some(url.into())
}

/// One login attempt, from the moment its authorise URL was opened. Lives in memory only, so a restart forgets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLogin {
    pub tracker: TrackerId,
    pub state: String,
    /// MyAnimeList only. Never written anywhere but this struct.
    pub verifier: Option<String>,
    pub started_at_ms: i64,
}

/// What a redirect carried, with the pending attempt applied. `Accepted` consumes the attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Not a `kurobayomi://` (or `aniyomi://`) URI. Ignored.
    Ignored,
    /// Unknown host or unreadable link. The caller shows "Unrecognized login link".
    Unrecognized,
    /// The user refused, or the redirect had no credential. The stored login stays.
    Cancelled(TrackerId),
    /// No attempt is pending for this tracker. No network call is made.
    NotAllowed(TrackerId),
    /// The attempt is older than [`PENDING_TTL_MS`]. It is dropped.
    Expired(TrackerId),
    /// The echoed `state` differs from the attempt. The attempt stays pending and the stored login stays.
    StateMismatch(TrackerId),
    /// The attempt is consumed. The caller exchanges `value` for a token (authorisation code) or stores it (implicit).
    Accepted(Credential),
}

/// The credential of an accepted redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub tracker: TrackerId,
    /// The authorisation code, or the AniList access token.
    pub value: String,
    /// MyAnimeList's PKCE verifier from the attempt.
    pub verifier: Option<String>,
    /// AniList's `expires_in` in seconds, when the redirect carried it.
    pub expires_in: Option<i64>,
}

impl Route {
    /// The toast of a route that stops the login. `name` is the tracker's display name.
    pub fn message(&self, name: &str) -> Option<MsgKey> {
        Some(match self {
            Route::Unrecognized => MsgKey::UnrecognizedLoginLink,
            Route::Cancelled(_) => MsgKey::TrackerLoginCancelled(name.to_owned()),
            Route::NotAllowed(_) | Route::Expired(_) => MsgKey::LoginExpired,
            Route::StateMismatch(_) => MsgKey::TrackerLoginFailed(name.to_owned()),
            Route::Ignored | Route::Accepted(_) => return None,
        })
    }
}

/// Pending attempts, one per tracker. A new attempt replaces the old one (last attempt wins).
#[derive(Debug, Default)]
pub struct PendingLogins(HashMap<TrackerId, PendingLogin>);

impl PendingLogins {
    /// Opens an attempt for `tracker` and returns its authorise URL. `None` when the tracker has no browser login.
    pub fn begin(&mut self, tracker: TrackerId, now_ms: i64) -> Option<String> {
        let state = random_token(STATE_BYTES);
        let url = authorize_url(tracker, &state)?;
        self.open(tracker, state, now_ms);
        Some(url)
    }

    /// Opens an attempt for `tracker` on an authorise URL the tracker built from its own client and redirect URI, and
    /// returns that URL with `state` added last. `None` when `url` is not a URL.
    pub fn begin_with_url(&mut self, tracker: TrackerId, url: &str, now_ms: i64) -> Option<String> {
        let mut url = Url::parse(url).ok()?;
        let state = random_token(STATE_BYTES);
        url.query_pairs_mut().append_pair("state", &state);
        self.open(tracker, state, now_ms);
        Some(url.into())
    }

    fn open(&mut self, tracker: TrackerId, state: String, now_ms: i64) {
        self.0.insert(
            tracker,
            PendingLogin {
                tracker,
                state,
                verifier: None,
                started_at_ms: now_ms,
            },
        );
    }

    /// [`PendingLogins::begin`] for a login the user starts. A tracker that is not configured gets the
    /// "<name> is not configured" toast and no authorise URL, and no attempt is opened (T-27b-022, T-31a-037).
    /// Shikimori and Bangumi are gated by their registered tracker (`Tracker::setup_message`, the same check that
    /// disables the row) and use its `browser_auth_url`; the others by `secrets`.
    pub fn begin_configured(
        &mut self,
        tracker: TrackerId,
        name: &str,
        secrets: &ClientSecrets,
        trackers: &crate::manager::TrackerManager,
        now_ms: i64,
    ) -> Result<String, MsgKey> {
        if matches!(tracker, TrackerId::SHIKIMORI | TrackerId::BANGUMI) {
            let not_configured = || MsgKey::TrackerNotConfigured(name.to_owned());
            let registered = trackers.get(tracker).ok_or_else(not_configured)?;
            if let Some(hint) = registered.setup_message() {
                return Err(hint);
            }
            let url = registered.browser_auth_url().ok_or_else(not_configured)?;
            return self
                .begin_with_url(tracker, &url, now_ms)
                .ok_or(MsgKey::UnrecognizedLoginLink);
        }
        if !secrets.allows_login(tracker) {
            return Err(MsgKey::TrackerNotConfigured(name.to_owned()));
        }
        self.begin(tracker, now_ms)
            .ok_or(MsgKey::UnrecognizedLoginLink)
    }

    /// The live attempt of `tracker`. An attempt past its TTL is dropped and reported as `Expired`.
    fn live(&mut self, tracker: TrackerId, now_ms: i64) -> Result<&PendingLogin, Route> {
        let expired = match self.0.get(&tracker) {
            None => return Err(Route::NotAllowed(tracker)),
            Some(attempt) => now_ms - attempt.started_at_ms > PENDING_TTL_MS,
        };
        if expired {
            self.0.remove(&tracker);
            return Err(Route::Expired(tracker));
        }
        self.0.get(&tracker).ok_or(Route::NotAllowed(tracker))
    }

    /// Applies a redirect URI to the pending attempts. Nothing here touches the network or the store.
    pub fn route(&mut self, uri: &str, now_ms: i64) -> Route {
        match parse_callback(uri) {
            Redirect::Ignored => Route::Ignored,
            Redirect::Unexpected | Redirect::Malformed => Route::Unrecognized,
            Redirect::MyAnimeListLoopback => Route::Ignored,
            Redirect::Cancelled(tracker) => match self.live(tracker, now_ms) {
                Ok(_) => {
                    self.0.remove(&tracker);
                    Route::Cancelled(tracker)
                }
                Err(stop) => stop,
            },
            Redirect::Credential {
                tracker,
                value,
                state,
                expires_in,
            } => {
                let attempt = match self.live(tracker, now_ms) {
                    Ok(attempt) => attempt,
                    Err(stop) => return stop,
                };
                if let Some(echoed) = state {
                    if echoed != attempt.state {
                        return Route::StateMismatch(tracker);
                    }
                }
                let verifier = attempt.verifier.clone();
                self.0.remove(&tracker);
                Route::Accepted(Credential {
                    tracker,
                    value,
                    verifier,
                    expires_in,
                })
            }
        }
    }
}

/// What the open handler does with a redirect (T-27a-022).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handled {
    /// Not ours. Nothing happens.
    Ignored,
    /// Show this toast. The login stops here.
    Toast(MsgKey),
    /// Exchange the credential for a token (or store it, for an implicit grant), then save it with
    /// [`crate::base::BaseTracker::save_oauth`].
    Exchange(Credential),
    /// The Simkl redirect `kurobayomi://simkl-auth...`. Simkl's login carries no `state` and no pending attempt, so it is
    /// not checked here: pass the URI to `simkl::SimklTracker::handle_launch_uri` (or the open dialog's
    /// `simkl::login::SimklLogin::on_launch_uri`), which exchanges the code or logs out when there is none (T-32a-031).
    SimklRedirect(String),
}

/// Routes a `kurobayomi://<tracker>-auth` URI received by the application's open handler: the GUI passes every opened
/// URI here, shows the toast of [`Handled::Toast`], and starts the token exchange for [`Handled::Exchange`].
pub fn handle_redirect(
    pending: &mut PendingLogins,
    trackers: &crate::manager::TrackerManager,
    uri: &str,
    now_ms: i64,
) -> Handled {
    if crate::simkl::oauth::is_simkl_redirect(uri) {
        // A repeated redirect after the login already worked is ignored, as for the other trackers (R-5).
        let live = trackers
            .get(TrackerId::SIMKL)
            .is_some_and(|t| t.is_logged_in() && !t.auth_expired());
        return if live {
            Handled::Ignored
        } else {
            Handled::SimklRedirect(uri.to_owned())
        };
    }
    let route = pending.route(uri, now_ms);
    // A repeated redirect after the login already succeeded is ignored (R-5), not reported as expired.
    if let Route::NotAllowed(t) = &route {
        if trackers.get(*t).is_some_and(|tr| tr.is_logged_in()) {
            return Handled::Ignored;
        }
    }
    let name = match &route {
        Route::Cancelled(t)
        | Route::NotAllowed(t)
        | Route::Expired(t)
        | Route::StateMismatch(t) => trackers
            .get(*t)
            .map_or_else(|| t.0.to_string(), |tr| tr.name().to_owned()),
        _ => String::new(),
    };
    if let Some(message) = route.message(&name) {
        return Handled::Toast(message);
    }
    match route {
        Route::Accepted(credential) => Handled::Exchange(credential),
        _ => Handled::Ignored,
    }
}

/// What a redirect URI carried, before any pending check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// Not a `kurobayomi://` (or `aniyomi://`) URI. Ignored.
    Ignored,
    /// `kurobayomi://<unknown>-auth`: "Unrecognized login link".
    Unexpected,
    /// A known host with an empty `code`, so the link is malformed: "Unrecognized login link" with no network call.
    Malformed,
    /// `myanimelist-auth`: MyAnimeList logs in through its loopback redirect (`mal::redirect`), so this scheme is not
    /// routed here and nothing happens.
    MyAnimeListLoopback,
    /// The user refused, or the redirect carried no code or token: "Login cancelled". Old credentials stay.
    Cancelled(TrackerId),
    /// An authorisation code (query) or an implicit token (fragment), with the echoed `state` if any.
    Credential {
        tracker: TrackerId,
        value: String,
        state: Option<String>,
        expires_in: Option<i64>,
    },
}

fn tracker_of(host: &str) -> Option<TrackerId> {
    Some(match host.strip_suffix("-auth")? {
        "anilist" => TrackerId::ANILIST,
        "shikimori" => TrackerId::SHIKIMORI,
        "bangumi" => TrackerId::BANGUMI,
        "simkl" => TrackerId::SIMKL,
        _ => return None,
    })
}

/// The raw value of `key` in a `&`-separated fragment or query. The key must start a segment (the fragment start or
/// after `&`), and the value runs to the next `&`. The value is not percent-decoded (E-50, E-51, E-55).
fn raw_value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    source
        .split('&')
        .find_map(|segment| segment.strip_prefix(key)?.strip_prefix('='))
}

/// Parses a redirect URI. AniList answers in the fragment (implicit grant, `access_token`), the others in the query
/// (`code`).
pub fn parse_callback(uri: &str) -> Redirect {
    let Ok(url) = Url::parse(uri) else {
        return Redirect::Ignored;
    };
    // Every login redirect Kurobayomi asks for is `kurobayomi://<tracker>-auth`; `aniyomi://` is still accepted as input.
    if !matches!(url.scheme(), "aniyomi" | "kurobayomi") {
        return Redirect::Ignored;
    }
    if url.host_str() == Some("myanimelist-auth") {
        return Redirect::MyAnimeListLoopback;
    }
    let Some(tracker) = url.host_str().and_then(tracker_of) else {
        return Redirect::Unexpected;
    };
    if tracker == TrackerId::ANILIST {
        let fragment = url.fragment().unwrap_or("");
        return match raw_value(fragment, "access_token").filter(|token| !token.is_empty()) {
            Some(token) => Redirect::Credential {
                tracker,
                value: token.to_owned(),
                state: raw_value(fragment, "state").map(str::to_owned),
                expires_in: raw_value(fragment, "expires_in").and_then(|v| v.parse().ok()),
            },
            None => Redirect::Cancelled(tracker),
        };
    }
    // The first `code` counts. It is percent-decoded, and `+` stays `+` (E-52, E-54).
    let query = url.query().unwrap_or("");
    let code = query
        .split('&')
        .find_map(|segment| segment.strip_prefix("code="));
    let state = query
        .split('&')
        .find_map(|segment| segment.strip_prefix("state="))
        .map(percent_decode);
    match code {
        None => Redirect::Cancelled(tracker),
        Some("") => Redirect::Malformed,
        Some(raw) => Redirect::Credential {
            tracker,
            value: percent_decode(raw),
            state,
            expires_in: None,
        },
    }
}

/// Decodes `%XX` escapes and leaves every other byte alone, so `+` stays `+`. Invalid UTF-8 is replaced.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Routes text the user pasted into the "Finish login by pasting the link" field (T-27b-029). The text goes through
/// the same parser and `state` guard as a routed link. Empty text, or text that is not a login link, shows the
/// "Unrecognized login link" toast.
pub fn handle_pasted(
    pending: &mut PendingLogins,
    trackers: &crate::manager::TrackerManager,
    text: &str,
    now_ms: i64,
) -> Handled {
    let text = text.trim();
    match handle_redirect(pending, trackers, text, now_ms) {
        Handled::Ignored => Handled::Toast(MsgKey::UnrecognizedLoginLink),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An authorise URL as a tracker builds it from its own client.
    const BGM_URL: &str = "https://bgm.tv/oauth/authorize?client_id=cid&response_type=code";

    #[tokio::test]
    async fn tr_27a_022_the_open_handler_gives_a_toast_or_a_credential() {
        let h = crate::fakes::Harness::new();
        let trackers = &h.ctx.trackers;
        let mut pending = PendingLogins::default();
        assert_eq!(
            handle_redirect(&mut pending, trackers, "https://example.test/", 0),
            Handled::Ignored
        );
        assert_eq!(
            handle_redirect(&mut pending, trackers, "aniyomi://unknown-auth?code=x", 0),
            Handled::Toast(MsgKey::UnrecognizedLoginLink)
        );
        assert_eq!(
            handle_redirect(&mut pending, trackers, "aniyomi://shikimori-auth?code=x", 0),
            Handled::Toast(MsgKey::LoginExpired),
            "no attempt is pending"
        );
        let url = pending.begin(TrackerId::ANILIST, 0).unwrap();
        let state = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let uri = format!("aniyomi://anilist-auth#access_token=abc&state={state}");
        let Handled::Exchange(c) = handle_redirect(&mut pending, trackers, &uri, 1) else {
            panic!("an accepted redirect is exchanged");
        };
        assert_eq!((c.tracker, c.value.as_str()), (TrackerId::ANILIST, "abc"));
    }

    #[tokio::test]
    async fn t_32a_031_the_simkl_redirect_is_handed_over_without_a_pending_attempt() {
        let h = crate::fakes::Harness::new();
        let mut pending = PendingLogins::default();
        assert_eq!(
            handle_redirect(
                &mut pending,
                &h.ctx.trackers,
                "aniyomi://simkl-auth?code=x",
                0
            ),
            Handled::SimklRedirect("aniyomi://simkl-auth?code=x".into()),
            "Simkl carries no state: the redirect goes to the Simkl login, not to the LoginExpired toast"
        );
        assert_eq!(
            handle_redirect(&mut pending, &h.ctx.trackers, "aniyomi://simkl-auth", 0),
            Handled::SimklRedirect("aniyomi://simkl-auth".into()),
            "a redirect without a code is handled there too (logout, TST-57)"
        );
    }

    #[tokio::test]
    async fn t_27b_022_a_missing_client_secret_gives_a_toast_and_no_url() {
        let trackers = fake_registry();
        let mut pending = PendingLogins::default();
        let none = ClientSecrets::default();
        assert_eq!(
            pending.begin_configured(TrackerId::SIMKL, "Simkl", &none, &trackers, 0),
            Err(MsgKey::TrackerNotConfigured("Simkl".into()))
        );
        assert_eq!(
            pending.route("aniyomi://simkl-auth?code=x", 1),
            Route::NotAllowed(TrackerId::SIMKL),
            "no attempt was opened"
        );
        let blank = ClientSecrets {
            simkl: Some("  ".into()),
        };
        assert!(
            pending
                .begin_configured(TrackerId::SIMKL, "Simkl", &blank, &trackers, 0)
                .is_err()
        );
    }

    #[tokio::test]
    async fn t_31a_039_shikimori_and_bangumi_have_no_url_without_their_tracker_client() {
        // The fake registry has no client for either, so neither builds an authorise URL.
        let trackers = fake_registry();
        let mut pending = PendingLogins::default();
        for (tracker, name) in [
            (TrackerId::SHIKIMORI, "Shikimori"),
            (TrackerId::BANGUMI, "Bangumi"),
        ] {
            assert!(
                authorize_url(tracker, "S").is_none(),
                "no literal client id"
            );
            assert!(pending.begin(tracker, 0).is_none());
            assert_eq!(
                pending.begin_configured(tracker, name, &ClientSecrets::default(), &trackers, 0),
                Err(MsgKey::TrackerNotConfigured(name.into()))
            );
            assert_eq!(
                pending.route(&format!("aniyomi://{}-auth?code=x", name.to_lowercase()), 1),
                Route::NotAllowed(tracker),
                "no attempt was opened"
            );
        }
    }

    #[tokio::test]
    async fn t_27b_022_a_set_secret_or_a_tracker_without_one_starts_the_login() {
        let trackers = fake_registry();
        let mut pending = PendingLogins::default();
        let secrets = ClientSecrets {
            simkl: Some("s".into()),
        };
        assert!(
            pending
                .begin_configured(TrackerId::SIMKL, "Simkl", &secrets, &trackers, 0)
                .unwrap()
                .starts_with("https://simkl.com/oauth/authorize")
        );
        assert!(
            pending
                .begin_configured(
                    TrackerId::ANILIST,
                    "AniList",
                    &ClientSecrets::default(),
                    &trackers,
                    0
                )
                .is_ok(),
            "AniList uses no client secret"
        );
        assert_eq!(secrets.get(TrackerId::SIMKL), Some("s"));
        assert_eq!(secrets.get(TrackerId::BANGUMI), None);
    }

    #[test]
    fn t_31a_038_begin_with_url_keeps_the_tracker_url_and_adds_state_last() {
        let mut pending = PendingLogins::default();
        let url = pending
            .begin_with_url(
                TrackerId::SHIKIMORI,
                "https://shikimori.io/oauth/authorize?client_id=c&redirect_uri=kurobayomi%3A%2F%2Fshikimori-auth&response_type=code",
                0,
            )
            .unwrap();
        let state = pending.0[&TrackerId::SHIKIMORI].state.clone();
        assert_eq!(
            url,
            format!(
                "https://shikimori.io/oauth/authorize?client_id=c&redirect_uri=kurobayomi%3A%2F%2Fshikimori-auth&response_type=code&state={state}"
            )
        );
        assert!(
            pending
                .begin_with_url(TrackerId::SHIKIMORI, "not a url", 0)
                .is_none()
        );
    }

    fn credential(tracker: TrackerId, value: &str, state: Option<&str>) -> Redirect {
        Redirect::Credential {
            tracker,
            value: value.into(),
            state: state.map(str::to_owned),
            expires_in: None,
        }
    }

    #[test]
    fn tr_27_d17_an_unknown_host_is_an_unexpected_response() {
        assert_eq!(parse_callback("aniyomi://foo?code=x"), Redirect::Unexpected);
        assert_eq!(
            parse_callback("aniyomi://unknown-auth?code=x"),
            Redirect::Unexpected
        );
    }

    #[test]
    fn tr_27_d18_a_fragment_token_without_a_trailing_ampersand_parses() {
        assert_eq!(
            parse_callback("aniyomi://anilist-auth#access_token=abc"),
            credential(TrackerId::ANILIST, "abc", None)
        );
    }

    #[test]
    fn tr_27_d18_a_fragment_token_with_extra_fields_parses() {
        assert_eq!(
            parse_callback(
                "aniyomi://anilist-auth#access_token=abc&token_type=Bearer&expires_in=3600"
            ),
            Redirect::Credential {
                tracker: TrackerId::ANILIST,
                value: "abc".into(),
                state: None,
                expires_in: Some(3600),
            }
        );
    }

    #[test]
    fn tr_27b_021_the_fragment_token_is_the_raw_substring_not_decoded() {
        assert_eq!(
            parse_callback("aniyomi://anilist-auth#access_token=a+b%20c&token_type=Bearer"),
            credential(TrackerId::ANILIST, "a+b%20c", None)
        );
    }

    #[test]
    fn tr_27b_021_access_token_is_matched_only_at_a_segment_start() {
        assert_eq!(
            parse_callback("aniyomi://anilist-auth#old_access_token=no&access_token=yes"),
            credential(TrackerId::ANILIST, "yes", None)
        );
        assert_eq!(
            parse_callback("aniyomi://anilist-auth#xaccess_token=no"),
            Redirect::Cancelled(TrackerId::ANILIST)
        );
    }

    #[test]
    fn tr_27b_021_a_fragment_without_a_token_is_denied_not_a_token() {
        assert_eq!(
            parse_callback("aniyomi://anilist-auth#access_token=&token_type=Bearer"),
            Redirect::Cancelled(TrackerId::ANILIST)
        );
    }

    #[test]
    fn redirect_table_cancelled_rows() {
        assert_eq!(
            parse_callback("aniyomi://anilist-auth#error=access_denied"),
            Redirect::Cancelled(TrackerId::ANILIST)
        );
        assert_eq!(
            parse_callback("aniyomi://bangumi-auth?error=access_denied"),
            Redirect::Cancelled(TrackerId::BANGUMI)
        );
        assert_eq!(
            parse_callback("aniyomi://myanimelist-auth"),
            Redirect::MyAnimeListLoopback,
            "MyAnimeList logs in through its loopback redirect, not this scheme"
        );
    }

    #[test]
    fn redirect_table_code_rows_carry_the_state() {
        assert_eq!(
            parse_callback("aniyomi://bangumi-auth?code=xyz&state=s1"),
            credential(TrackerId::BANGUMI, "xyz", Some("s1"))
        );
        assert_eq!(
            parse_callback("aniyomi://simkl-auth?code=a%20b"),
            credential(TrackerId::SIMKL, "a b", None)
        );
    }

    #[test]
    fn t_27b_022_the_first_code_is_percent_decoded_and_plus_stays_plus() {
        assert_eq!(
            parse_callback("aniyomi://bangumi-auth?code=a+b%2Bc&code=later"),
            credential(TrackerId::BANGUMI, "a+b+c", None)
        );
    }

    #[test]
    fn t_27b_022_a_missing_code_is_a_denial_and_an_empty_code_is_malformed() {
        assert_eq!(
            parse_callback("aniyomi://simkl-auth?error=access_denied"),
            Redirect::Cancelled(TrackerId::SIMKL)
        );
        assert_eq!(
            parse_callback("aniyomi://simkl-auth?code="),
            Redirect::Malformed
        );
        let mut pending = PendingLogins::default();
        pending.begin(TrackerId::SIMKL, 0).unwrap();
        assert_eq!(
            pending.route("aniyomi://simkl-auth?code=", 0),
            Route::Unrecognized
        );
        assert_eq!(
            pending.route("aniyomi://simkl-auth?error=x", 0),
            Route::Cancelled(TrackerId::SIMKL),
            "a denial keeps the stored login and consumes the attempt"
        );
    }

    #[test]
    fn t_27b_022_the_percent_decoder_leaves_bad_escapes_alone() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
        assert_eq!(percent_decode("%E3%81%82"), "あ");
    }

    /// The eleven registry ids, each backed by a fake manga tracker.
    fn fake_registry() -> crate::manager::TrackerManager {
        use crate::fakes::{FakeManga, MemSecrets, new_log};
        use crate::tracker::Tracker;
        let secrets = MemSecrets::new();
        let log = new_log();
        let ids = [
            TrackerId::MYANIMELIST,
            TrackerId::ANILIST,
            TrackerId::KITSU,
            TrackerId::SHIKIMORI,
            TrackerId::BANGUMI,
            TrackerId::KOMGA,
            TrackerId::MANGAUPDATES,
            TrackerId::KAVITA,
            TrackerId::SUWAYOMI,
            TrackerId::SIMKL,
            TrackerId::JELLYFIN,
        ];
        let trackers: Vec<std::sync::Arc<dyn Tracker>> = ids
            .iter()
            .map(|id| {
                std::sync::Arc::new(FakeManga::new(*id, "x", secrets.clone(), log.clone()))
                    as std::sync::Arc<dyn Tracker>
            })
            .collect();
        crate::manager::TrackerManager::new(trackers).unwrap()
    }

    #[tokio::test]
    async fn t_27b_029_pasted_links_use_the_same_guard_and_toasts() {
        let trackers = fake_registry();
        let mut pending = PendingLogins::default();
        assert_eq!(
            handle_pasted(&mut pending, &trackers, "   ", 0),
            Handled::Toast(MsgKey::UnrecognizedLoginLink)
        );
        assert_eq!(
            handle_pasted(&mut pending, &trackers, "https://example.test/x", 0),
            Handled::Toast(MsgKey::UnrecognizedLoginLink)
        );
        // No attempt is pending, so a pasted code is refused like a routed one.
        assert_eq!(
            handle_pasted(&mut pending, &trackers, "aniyomi://bangumi-auth?code=x", 0),
            Handled::Toast(MsgKey::LoginExpired)
        );
        pending
            .begin_with_url(TrackerId::BANGUMI, BGM_URL, 0)
            .unwrap();
        assert!(matches!(
            handle_pasted(
                &mut pending,
                &trackers,
                "aniyomi://bangumi-auth?code=xyz&state=wrong",
                0
            ),
            Handled::Toast(MsgKey::TrackerLoginFailed(_))
        ));
    }

    #[test]
    fn redirect_table_other_schemes_are_ignored() {
        assert_eq!(
            parse_callback("http://anilist-auth?code=x"),
            Redirect::Ignored
        );
        assert_eq!(parse_callback("not a uri"), Redirect::Ignored);
    }

    #[test]
    fn tr_27a_021_state_is_43_url_safe_characters_per_request() {
        let mut pending = PendingLogins::default();
        let first = pending.begin(TrackerId::ANILIST, 0).unwrap();
        let state = pending.0[&TrackerId::ANILIST].state.clone();
        assert_eq!(state.len(), 43);
        assert!(first.contains(&format!("state={state}")));
        pending.begin(TrackerId::ANILIST, 0).unwrap();
        assert_ne!(pending.0[&TrackerId::ANILIST].state, state);
    }

    #[test]
    fn tr_27a_021_authorize_urls_keep_the_source_order_and_add_state_last() {
        assert_eq!(
            authorize_url(TrackerId::ANILIST, "S").unwrap(),
            "https://anilist.co/api/v2/oauth/authorize?client_id=53424&response_type=token&state=S"
        );
        assert_eq!(
            authorize_url(TrackerId::SIMKL, "S").unwrap(),
            "https://simkl.com/oauth/authorize?response_type=code&client_id=aa62a7da32518aae5d5049a658b87fa4837c3b739e06ed250b315aab6af82b0e&redirect_uri=kurobayomi%3A%2F%2Fsimkl-auth&state=S"
        );
    }

    #[test]
    fn tr_27a_021_mal_has_no_url_here_and_no_embedded_client_id() {
        assert!(authorize_url(TrackerId::MYANIMELIST, "S").is_none());
        assert!(
            PendingLogins::default()
                .begin(TrackerId::MYANIMELIST, 0)
                .is_none()
        );
    }

    #[test]
    fn tr_27a_021_trackers_without_a_browser_login_have_no_url() {
        assert!(authorize_url(TrackerId::KITSU, "S").is_none());
        assert!(authorize_url(TrackerId::MANGAUPDATES, "S").is_none());
        assert!(
            PendingLogins::default()
                .begin(TrackerId::KITSU, 0)
                .is_none()
        );
    }

    #[test]
    fn tr_27b_026_a_code_without_a_pending_attempt_is_not_allowed() {
        let mut pending = PendingLogins::default();
        assert_eq!(
            pending.route("aniyomi://bangumi-auth?code=x&state=s", 0),
            Route::NotAllowed(TrackerId::BANGUMI)
        );
    }

    #[test]
    fn tr_27b_026_an_attempt_expires_after_five_minutes() {
        let mut pending = PendingLogins::default();
        pending.begin(TrackerId::SIMKL, 1_000).unwrap();
        let state = pending.0[&TrackerId::SIMKL].state.clone();
        let uri = format!("aniyomi://simkl-auth?code=x&state={state}");
        assert_eq!(
            pending.route(&uri, 1_000 + PENDING_TTL_MS + 1),
            Route::Expired(TrackerId::SIMKL)
        );
        assert_eq!(
            pending.route(&uri, 1_000 + PENDING_TTL_MS + 2),
            Route::NotAllowed(TrackerId::SIMKL),
            "an expired attempt is dropped"
        );
    }

    #[test]
    fn tr_27b_026_the_attempt_is_still_live_at_exactly_the_ttl() {
        let mut pending = PendingLogins::default();
        pending.begin(TrackerId::SIMKL, 0).unwrap();
        let state = pending.0[&TrackerId::SIMKL].state.clone();
        let uri = format!("aniyomi://simkl-auth?code=x&state={state}");
        assert!(matches!(
            pending.route(&uri, PENDING_TTL_MS),
            Route::Accepted(_)
        ));
    }

    #[test]
    fn tr_27b_026_last_attempt_wins_and_a_mismatched_state_keeps_it_pending() {
        let mut pending = PendingLogins::default();
        pending
            .begin_with_url(TrackerId::BANGUMI, BGM_URL, 0)
            .unwrap();
        let old = pending.0[&TrackerId::BANGUMI].state.clone();
        pending
            .begin_with_url(TrackerId::BANGUMI, BGM_URL, 0)
            .unwrap();
        let new = pending.0[&TrackerId::BANGUMI].state.clone();
        assert_eq!(
            pending.route(&format!("aniyomi://bangumi-auth?code=x&state={old}"), 0),
            Route::StateMismatch(TrackerId::BANGUMI)
        );
        assert!(matches!(
            pending.route(&format!("aniyomi://bangumi-auth?code=x&state={new}"), 0),
            Route::Accepted(_)
        ));
    }

    #[test]
    fn tr_27b_025_mal_redirects_are_not_routed_by_this_module() {
        let mut pending = PendingLogins::default();
        assert_eq!(
            pending.route("aniyomi://myanimelist-auth?code=abc", 0),
            Route::Ignored
        );
    }

    #[test]
    fn tr_27b_021_anilist_accepts_without_state_and_carries_expires_in() {
        let mut pending = PendingLogins::default();
        pending.begin(TrackerId::ANILIST, 0).unwrap();
        assert_eq!(
            pending.route("aniyomi://anilist-auth#access_token=t&expires_in=60", 0),
            Route::Accepted(Credential {
                tracker: TrackerId::ANILIST,
                value: "t".into(),
                verifier: None,
                expires_in: Some(60),
            })
        );
    }

    #[test]
    fn tr_27b_024_a_cancelled_redirect_consumes_the_attempt_and_names_the_tracker() {
        let mut pending = PendingLogins::default();
        pending
            .begin_with_url(TrackerId::SHIKIMORI, BGM_URL, 0)
            .unwrap();
        let route = pending.route("aniyomi://shikimori-auth?error=access_denied", 0);
        assert_eq!(route, Route::Cancelled(TrackerId::SHIKIMORI));
        assert_eq!(
            route.message("Shikimori"),
            Some(MsgKey::TrackerLoginCancelled("Shikimori".into()))
        );
        assert!(pending.0.is_empty());
    }

    #[test]
    fn tr_27b_023_unknown_hosts_and_stopped_logins_have_their_toasts() {
        let mut pending = PendingLogins::default();
        let route = pending.route("aniyomi://foo-auth?code=x", 0);
        assert_eq!(
            route.message(""),
            Some(MsgKey::UnrecognizedLoginLink),
            "no hanging state"
        );
        assert_eq!(
            Route::Expired(TrackerId::ANILIST).message("AniList"),
            Some(MsgKey::LoginExpired)
        );
        assert_eq!(Route::Ignored.message("x"), None);
    }

    #[tokio::test]
    async fn t_28a_040_the_kurobayomi_scheme_carries_the_anilist_fragment() {
        assert_eq!(
            parse_callback("kurobayomi://anilist-auth#access_token=abc"),
            Redirect::Credential {
                tracker: TrackerId::ANILIST,
                value: "abc".into(),
                state: None,
                expires_in: None,
            }
        );
        let h = crate::fakes::Harness::new();
        let mut pending = PendingLogins::default();
        assert_eq!(
            handle_redirect(
                &mut pending,
                &h.ctx.trackers,
                "kurobayomi://simkl-auth?code=x",
                0
            ),
            Handled::SimklRedirect("kurobayomi://simkl-auth?code=x".into()),
            "routed like aniyomi://, with no attempt pending"
        );
    }

    #[tokio::test]
    async fn t_28a_040_a_repeat_redirect_after_a_login_is_ignored() {
        let h = crate::fakes::Harness::new();
        h.login(TrackerId::SIMKL);
        let mut pending = PendingLogins::default();
        assert_eq!(
            handle_redirect(
                &mut pending,
                &h.ctx.trackers,
                "aniyomi://simkl-auth?code=x",
                0
            ),
            Handled::Ignored
        );
    }
}
