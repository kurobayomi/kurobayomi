//! The Jellyfin tracking URL: an item URL whose fragment says what the item is
//! (T-33a-011, T-33a-014).

use url::Url;

use crate::trackers::enhanced::EnhancedError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingKind {
    /// Fragment `movie…`: the item is a movie.
    Movie,
    /// Fragment `seriesId,{s}`: the item is a season of the series `series_id`.
    Season { series_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackingUrl {
    pub kind: TrackingKind,
    /// The segment after `Users` (`pathSegments[1]` when there is no proxy prefix, E-81).
    pub user_id: String,
    /// The last path segment: the movie id, or the season id for a season.
    pub item_id: String,
    /// `scheme://host[:port]` plus any reverse-proxy prefix. No `Users` part, no query, no fragment (E-81).
    pub server: Url,
}

impl TrackingUrl {
    /// Parses a tracking URL. A missing fragment is `MissingFragment` (E-67). An unknown fragment is `Decode`.
    pub fn parse(raw: &str) -> Result<Self, EnhancedError> {
        let url =
            Url::parse(raw).map_err(|_| EnhancedError::Decode("invalid tracking URL".into()))?;
        let fragment = url
            .fragment()
            .ok_or(EnhancedError::MissingFragment)?
            .to_owned();
        let segments: Vec<String> = url
            .path_segments()
            .map(|s| s.map(str::to_owned).collect())
            .unwrap_or_default();
        let users = segments
            .iter()
            .position(|s| s == "Users")
            .ok_or_else(|| EnhancedError::Decode("no Users segment".into()))?;
        let user_id = segments
            .get(users + 1)
            .filter(|s| !s.is_empty())
            .cloned()
            .ok_or_else(|| EnhancedError::Decode("no user id".into()))?;
        let item_id = segments
            .last()
            .filter(|s| !s.is_empty() && segments.len() > users + 2)
            .cloned()
            .ok_or_else(|| EnhancedError::Decode("no item id".into()))?;
        let kind = if fragment.starts_with("movie") {
            TrackingKind::Movie
        } else if fragment.starts_with("seriesId") {
            let series_id = fragment.rsplit(',').next().unwrap_or_default().to_owned();
            if series_id.is_empty() {
                return Err(EnhancedError::Decode("no series id".into()));
            }
            TrackingKind::Season { series_id }
        } else {
            return Err(EnhancedError::Decode("unknown fragment".into()));
        };
        let mut server = url.clone();
        server.set_query(None);
        server.set_fragment(None);
        {
            let mut path = server
                .path_segments_mut()
                .map_err(|_| EnhancedError::Decode("tracking URL cannot be a base".into()))?;
            path.clear();
            path.extend(segments[..users].iter());
        }
        Ok(Self {
            kind,
            user_id,
            item_id,
            server,
        })
    }

    /// The server URL with `segments` appended. Ids are percent-encoded as path segments.
    pub fn child(&self, segments: &[&str]) -> Result<Url, EnhancedError> {
        let mut url = self.server.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| EnhancedError::Decode("tracking URL cannot be a base".into()))?;
            path.pop_if_empty().extend(segments);
        }
        Ok(url)
    }

    /// `{server}/Users/{user}/Items/{item}`: the item read URL, with no query, so no key is in it.
    pub fn item_url(&self) -> Result<Url, EnhancedError> {
        self.child(&[
            "Users",
            self.user_id.as_str(),
            "Items",
            self.item_id.as_str(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_33a_011_a_movie_fragment_parses_user_and_item() {
        let t = TrackingUrl::parse("https://h/Users/U1/Items/M1#movie").unwrap();
        assert_eq!(t.kind, TrackingKind::Movie);
        assert_eq!(t.user_id, "U1");
        assert_eq!(t.item_id, "M1");
    }

    #[test]
    fn t_33a_011_a_season_fragment_takes_the_series_id_after_the_comma() {
        let t = TrackingUrl::parse("https://h/Users/U1/Items/S1#seriesId,R9").unwrap();
        assert_eq!(
            t.kind,
            TrackingKind::Season {
                series_id: "R9".into()
            }
        );
        assert_eq!(t.item_id, "S1");
    }

    #[test]
    fn t_33a_011_e67_a_missing_fragment_is_an_error() {
        let r = TrackingUrl::parse("https://h/Users/U1/Items/M1");
        assert!(matches!(r, Err(EnhancedError::MissingFragment)));
    }

    #[test]
    fn t_33a_011_an_unknown_fragment_is_a_decode_error() {
        let r = TrackingUrl::parse("https://h/Users/U1/Items/M1#other");
        assert!(matches!(r, Err(EnhancedError::Decode(_))));
    }

    #[test]
    fn t_33a_014_e81_a_proxy_prefix_is_kept_in_the_server_url() {
        let t = TrackingUrl::parse("https://h/jf/Users/U1/Items/S1#seriesId,R9").unwrap();
        assert_eq!(t.user_id, "U1");
        assert_eq!(t.server.as_str(), "https://h/jf");
        let episodes = t.child(&["Shows", "R9", "Episodes"]).unwrap();
        assert_eq!(episodes.path(), "/jf/Shows/R9/Episodes");
    }

    #[test]
    fn t_33a_014_no_prefix_gives_the_plain_root_path() {
        let t = TrackingUrl::parse("https://h:8096/Users/U1/Items/S1#seriesId,R9").unwrap();
        assert_eq!(t.server.as_str(), "https://h:8096/");
        let episodes = t.child(&["Shows", "R9", "Episodes"]).unwrap();
        assert_eq!(episodes.path(), "/Shows/R9/Episodes");
    }

    #[test]
    fn t_33a_011_the_item_url_has_no_query_and_no_fragment() {
        let t = TrackingUrl::parse("https://h/jf/Users/U1/Items/M1?x=1#movie").unwrap();
        let item = t.item_url().unwrap();
        assert_eq!(item.as_str(), "https://h/jf/Users/U1/Items/M1");
    }
}
