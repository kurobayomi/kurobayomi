//! MangaUpdates wire shapes. A missing nullable key parses as `None`. A missing
//! required key (for example `context` in the login reply) is a decode error.

use serde::Deserialize;

/// `MULoginResponse`: `context` is required.
#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub context: LoginContext,
}

/// `MUContext`: the account id and its session token.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LoginContext {
    pub session_token: String,
    pub uid: i64,
}

/// `MUListItem`: the series' place on the user's list.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListItem {
    #[serde(default)]
    pub series: Option<SeriesRef>,
    #[serde(default)]
    pub list_id: Option<i64>,
    #[serde(default)]
    pub status: Option<ListStatus>,
    #[serde(default)]
    pub priority: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeriesRef {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
}

/// `MUStatus`: reading progress on the list item.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListStatus {
    #[serde(default)]
    pub volume: Option<i64>,
    #[serde(default)]
    pub chapter: Option<i64>,
}

/// `MURating`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rating {
    #[serde(default)]
    pub rating: Option<f64>,
}

/// `MUSearchResult`: both keys are required.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchHit {
    pub record: Record,
}

/// `MURecord`: one series in a search result.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub series_id: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub image: Option<Image>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub bayesian_rating: Option<f64>,
    #[serde(default)]
    pub rating_votes: Option<i64>,
    #[serde(default)]
    pub latest_chapter: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Image {
    #[serde(default)]
    pub url: Option<ImageUrl>,
    #[serde(default)]
    pub height: Option<i64>,
    #[serde(default)]
    pub width: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImageUrl {
    #[serde(default)]
    pub original: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_32a_012_list_item_fixture_round_trips_with_nullable_keys() {
        let text = r#"{"series":{"id":101,"title":"Example"},"list_id":0,"status":{"volume":1,"chapter":12},"priority":0}"#;
        let item: ListItem = serde_json::from_str(text).unwrap();
        assert_eq!(item.list_id, Some(0));
        assert_eq!(item.series.unwrap().id, Some(101));
        assert_eq!(item.status.unwrap().chapter, Some(12));
    }

    #[test]
    fn t_32a_012_a_null_status_and_missing_keys_parse_as_none() {
        let item: ListItem = serde_json::from_str(r#"{"list_id":2,"status":null}"#).unwrap();
        assert!(item.status.is_none());
        assert!(item.series.is_none());
        assert_eq!(item.list_id, Some(2));
    }

    #[test]
    fn t_32a_012_rating_null_and_value_parse() {
        let v: Rating = serde_json::from_str(r#"{"rating":7.5}"#).unwrap();
        assert_eq!(v.rating, Some(7.5));
        let n: Rating = serde_json::from_str(r#"{"rating":null}"#).unwrap();
        assert_eq!(n.rating, None);
    }

    #[test]
    fn t_32a_012_login_without_context_is_a_parse_error() {
        assert!(serde_json::from_str::<LoginResponse>("{}").is_err());
        let ok: LoginResponse =
            serde_json::from_str(r#"{"context":{"session_token":"T","uid":42}}"#).unwrap();
        assert_eq!(ok.context.uid, 42);
        assert_eq!(ok.context.session_token, "T");
    }

    #[test]
    fn t_32a_012_search_record_reads_type_year_and_nested_image() {
        let text = r#"{"results":[{"record":{"series_id":5,"title":"A","url":"https://x/5","description":"d","image":{"url":{"original":"o.jpg","thumb":"t.jpg"},"height":1,"width":2},"type":"Manga","year":"2020","bayesian_rating":8.1,"rating_votes":3,"latest_chapter":9}},{"record":{"series_id":null,"title":null,"url":null,"description":null,"image":null,"type":null,"year":null}}]}"#;
        let res: SearchResponse = serde_json::from_str(text).unwrap();
        assert_eq!(res.results.len(), 2);
        let first = &res.results[0].record;
        assert_eq!(first.kind.as_deref(), Some("Manga"));
        assert_eq!(
            first
                .image
                .as_ref()
                .unwrap()
                .url
                .as_ref()
                .unwrap()
                .original
                .as_deref(),
            Some("o.jpg")
        );
        let second = &res.results[1].record;
        assert!(second.kind.is_none() && second.year.is_none());
    }

    #[test]
    fn t_32a_012_empty_search_result_parses() {
        let res: SearchResponse = serde_json::from_str(r#"{"results":[]}"#).unwrap();
        assert!(res.results.is_empty());
    }
}
