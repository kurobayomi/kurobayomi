//! Jellyfin response shapes (`dto/JFItem.kt`). Unknown fields are ignored.

use serde::Deserialize;

/// One Jellyfin item: a movie, a season or an episode.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JFItem {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Id")]
    pub id: String,
    /// Required: a missing `UserData` is a parse error (E-87).
    #[serde(rename = "UserData")]
    pub user_data: JFUserData,
    /// Absent for movies and series (E-86). Episodes carry it.
    #[serde(rename = "IndexNumber", default)]
    pub index_number: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct JFUserData {
    #[serde(rename = "Played")]
    pub played: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JFItemList {
    #[serde(rename = "Items")]
    pub items: Vec<JFItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_33a_010_e86_a_movie_without_index_number_parses() {
        let item: JFItem = serde_json::from_str(
            r#"{"Name":"Film","Id":"m1","UserData":{"Played":true},"Type":"Movie"}"#,
        )
        .unwrap();
        assert_eq!(item.index_number, None);
        assert!(item.user_data.played);
        assert_eq!(item.name, "Film");
    }

    #[test]
    fn t_33a_010_e87_user_data_is_required() {
        let r = serde_json::from_str::<JFItem>(r#"{"Name":"x","Id":"1"}"#);
        assert!(r.is_err());
    }

    #[test]
    fn t_33a_010_item_list_reads_items_and_index_numbers() {
        let list: JFItemList = serde_json::from_str(
            r#"{"Items":[{"Name":"E1","Id":"e1","IndexNumber":1,"UserData":{"Played":false}}],"TotalRecordCount":1}"#,
        )
        .unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].index_number, Some(1));
    }
}
