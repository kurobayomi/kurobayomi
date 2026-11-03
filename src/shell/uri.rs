//! The link routes of the `kurobayomi://` scheme (T-53a-004) and the URL decoding the launch parsing shares (T-53a-010).
//! The route table leaves out Aniyomi's and Tachiyomi's own schemes: their
//! repository links name repositories of Android extensions, which Kurobayomi cannot use (user, 2026-10-24). Pure: the
//! window maps a [`UriRoute`] to a screen and an error to a toast.

use kuro_i18n::{MsgKey, tr};
use std::cell::RefCell;

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UriRoute {
    /// The one repository list with the repository address `url` (already decoded): `kurobayomi://add-repo?url=`.
    Repos { url: String },
    /// A tracker sign-in callback. `host` is the `*-auth` host and `uri` the whole link, which the tracker lane parses.
    TrackerAuth { host: String, uri: String },
}

/// The hosts of the tracker sign-in callbacks the tracker lane handles.
pub const TRACKER_AUTH_HOSTS: [&str; 5] = ["anilist-auth", "myanimelist-auth", "bangumi-auth", "shikimori-auth", "simkl-auth"];

/// A link the route table refuses. Each one becomes a toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UriError {
    /// A scheme other than `kurobayomi` (EC-17), Aniyomi's and Tachiyomi's included.
    UnsupportedScheme(String),
    /// A host that is no route (EC-18).
    UnknownHost(String),
    /// `add-repo` without `url=` (EC-16).
    MissingRepositoryAddress,
}

impl std::fmt::Display for UriError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme(s) => write!(f, "Unsupported link type {s}"),
            Self::UnknownHost(h) => write!(f, "Unknown link {h}"),
            Self::MissingRepositoryAddress => f.write_str(&tr(MsgKey::ui_missing_repository_address)),
        }
    }
}

/// The route of a link, by its host: `add-repo` (a repository to add) or a tracker's `*-auth` sign-in callback.
pub fn parse_uri(uri: &str) -> Result<UriRoute, UriError> {
    let (scheme, rest) = uri.split_once("://").unwrap_or((uri, ""));
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "kurobayomi" {
        return Err(UriError::UnsupportedScheme(scheme));
    }
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..host_end].to_ascii_lowercase();
    match host.as_str() {
        "add-repo" => {}
        h if h.ends_with("-auth") => return Ok(UriRoute::TrackerAuth { host, uri: uri.to_string() }),
        _ => return Err(UriError::UnknownHost(host)),
    }
    let url = query_value(rest[host_end..].split('#').next().unwrap_or(""), "url").filter(|u| !u.is_empty()).ok_or(UriError::MissingRepositoryAddress)?;
    Ok(UriRoute::Repos { url })
}

/// The decoded value of query parameter `name`, the first one when it repeats.
fn query_value(tail: &str, name: &str) -> Option<String> {
    let query = tail.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| percent_decode(value, true))
    })
}

/// Decodes `%XX` escapes (a bad escape stays as written). `plus_is_space` is form decoding, for query values.
pub fn percent_decode(text: &str, plus_is_space: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() => {
                out.push(hex(bytes[i + 1]).unwrap_or(0) * 16 + hex(bytes[i + 2]).unwrap_or(0));
                i += 3;
                continue;
            }
            b'+' if plus_is_space => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// The settings screen a route opens.
pub fn screen_of(route: &UriRoute) -> Option<&'static str> {
    match route {
        UriRoute::Repos { .. } => Some("settings_extension_repos"),
        UriRoute::TrackerAuth { .. } => None,
    }
}

thread_local! {
    static PENDING_REPO: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Parks the address of a repository link for the page `screen_of` opens, which takes it once.
pub fn park_repo_url(url: String) {
    PENDING_REPO.with(|slot| *slot.borrow_mut() = Some(url));
}

/// The parked repository address, handed out once.
pub fn take_repo_url() -> Option<String> {
    PENDING_REPO.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https%3A%2F%2Frepo.example.test%2Findex.min.json";

    fn repo_url() -> String {
        "https://repo.example.test/index.min.json".into()
    }

    #[test]
    fn ec12_an_add_repo_link_opens_the_repository_list() {
        assert_eq!(parse_uri(&format!("kurobayomi://add-repo?url={REPO}")), Ok(UriRoute::Repos { url: repo_url() }));
    }

    #[test]
    fn aniyomi_and_tachiyomi_links_are_refused_as_their_repositories_hold_android_extensions() {
        for link in [format!("aniyomi://add-repo?url={REPO}"), format!("tachiyomi://add-repo?url={REPO}"), format!("aniyomi://extension-store?url={REPO}")] {
            assert!(matches!(parse_uri(&link), Err(UriError::UnsupportedScheme(_))), "{link}");
        }
        assert_eq!(parse_uri(&format!("kurobayomi://extension-store?url={REPO}")), Err(UriError::UnknownHost("extension-store".into())));
    }

    #[test]
    fn ec16_a_repo_link_without_an_address_is_an_error_with_a_message() {
        assert_eq!(parse_uri("kurobayomi://add-repo"), Err(UriError::MissingRepositoryAddress));
        assert_eq!(parse_uri("kurobayomi://add-repo?url="), Err(UriError::MissingRepositoryAddress));
        assert_eq!(parse_uri("kurobayomi://add-repo?other=1"), Err(UriError::MissingRepositoryAddress));
        assert_eq!(UriError::MissingRepositoryAddress.to_string(), "Missing repository address");
    }

    #[test]
    fn ec17_an_unsupported_scheme_and_ec18_an_unknown_host_are_errors() {
        assert_eq!(parse_uri("ftp://x/y"), Err(UriError::UnsupportedScheme("ftp".into())));
        assert_eq!(parse_uri("kurobayomi://unknown-host"), Err(UriError::UnknownHost("unknown-host".into())));
        assert!(UriError::UnknownHost("h".into()).to_string().contains('h'));
    }

    #[test]
    fn auth_hosts_are_tracker_routes_and_keep_the_whole_link() {
        let uri = "kurobayomi://anilist-auth#access_token=abc";
        assert_eq!(parse_uri(uri), Ok(UriRoute::TrackerAuth { host: "anilist-auth".into(), uri: uri.into() }));
    }

    #[test]
    fn the_host_is_matched_in_any_case_and_a_fragment_does_not_hide_the_address() {
        assert_eq!(parse_uri("KuroBayomi://ADD-REPO?url=a%20b#frag"), Ok(UriRoute::Repos { url: "a b".into() }));
        assert_eq!(parse_uri("kurobayomi://add-repo/?x=1&url=u"), Ok(UriRoute::Repos { url: "u".into() }));
    }

    #[test]
    fn t53a_009_tracker_callbacks_route_to_the_tracker_lane_with_the_whole_link() {
        // EC-19 to EC-23: the token is in the fragment (last or not), the code in the query, or no code at all; the
        // shell only picks the route, the tracker lane reads the parameters
        let links = [
            "kurobayomi://anilist-auth#access_token=abc&token_type=Bearer",
            "kurobayomi://anilist-auth#access_token=abc",
            "kurobayomi://bangumi-auth?code=xyz",
            "kurobayomi://simkl-auth",
            "kurobayomi://myanimelist-auth?code=%20x",
            "kurobayomi://shikimori-auth?code=xyz",
        ];
        for link in links {
            let host = link.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap();
            assert_eq!(parse_uri(link), Ok(UriRoute::TrackerAuth { host: host.into(), uri: link.into() }), "{link}");
        }
        // an unknown `*-auth` host is still a tracker route; the tracker lane answers it with the toast
        assert!(matches!(parse_uri("kurobayomi://nosuch-auth#access_token=a"), Ok(UriRoute::TrackerAuth { .. })));
    }

    #[test]
    fn percent_decoding_handles_utf8_bad_escapes_and_plus() {
        assert_eq!(percent_decode("Copia%20de%20seguridad", false), "Copia de seguridad");
        assert_eq!(percent_decode("%C3%B1", false), "ñ");
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_decode("%zz%4", false), "%zz%4");
        assert_eq!(percent_decode("a+b", true), "a b");
        assert_eq!(percent_decode("a+b", false), "a+b");
        assert_eq!(percent_decode("%41", false), "A", "an escape that ends the text");
    }

    #[test]
    fn routes_name_their_screen_and_a_parked_address_is_taken_once() {
        assert_eq!(screen_of(&UriRoute::Repos { url: "u".into() }), Some("settings_extension_repos"));
        park_repo_url("u".into());
        assert_eq!(take_repo_url().as_deref(), Some("u"));
        assert_eq!(take_repo_url(), None);
    }
}
