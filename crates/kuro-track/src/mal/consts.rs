//! Tracker id and endpoint constants.
//! The app's own OAuth client id is built in; the user's config or `KUROBAYOMI_MAL_CLIENT_ID` can replace it.

use crate::model::TrackerId;

/// `TrackerManager.kt:27`: MAL is tracker 1.
pub const MAL_TRACKER_ID: TrackerId = TrackerId::MYANIMELIST;

/// Kurobayomi's MAL API client (app type "other": no secret, PKCE). Its one registered redirect is
/// `http://localhost:8765/myanimelist-auth` (`redirect::redirect_uri(DEFAULT_PORT)`).
pub const BUILT_IN_CLIENT_ID: &str = "d13312aeeb4264a8c7ce633952a615fb";

pub const OAUTH_BASE: &str = "https://myanimelist.net/v1/oauth2";
pub const API_BASE: &str = "https://api.myanimelist.net/v2";

/// Page size of `users/@me/{mangalist,animelist}` requests (`MyAnimeListApi.kt:388`).
pub const PAGE_SIZE: u32 = 250;
/// Longest search query sent, counted in UTF-16 units (`MyAnimeListApi.kt:85`).
pub const QUERY_MAX: usize = 64;
/// Detail requests running at once for search and `my:` results (D-13 proposal).
pub const DETAIL_CONCURRENCY: usize = 4;

pub const SEARCH_ID_PREFIX: &str = "id:";
pub const SEARCH_LIST_PREFIX: &str = "my:";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_bases_are_fixed_strings() {
        assert_eq!(MAL_TRACKER_ID.0, 1);
        assert_eq!(OAUTH_BASE, "https://myanimelist.net/v1/oauth2");
        assert_eq!(API_BASE, "https://api.myanimelist.net/v2");
        assert_eq!(PAGE_SIZE, 250);
        assert_eq!(QUERY_MAX, 64);
    }
}
