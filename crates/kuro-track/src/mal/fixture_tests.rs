//! DTO tests on the fixtures in `tests/fixtures/mal/` (T-29a-036, T-29a-041). The fixtures are synthetic, written
//! from the DTO shapes. They are not recorded
//! responses: the recording from a test account is still open (T-29a-041, T-29a-058). Replace each file with a
//! recorded response that keeps the same shape, and the tests below stay as they are.

use super::dto::{
    AnimeDetails, AnimeListEntry, MangaDetails, MangaListEntry, MangaListStatus, SearchPage, User,
    UserListPage,
};
use super::token::MalToken;

const OAUTH_TOKEN: &str = include_str!("../../tests/fixtures/mal/oauth_token.json");
const OAUTH_TOKEN_WITH_CREATED: &str =
    include_str!("../../tests/fixtures/mal/oauth_token_with_created.json");
const USER_ME: &str = include_str!("../../tests/fixtures/mal/user_me.json");
const MANGA_SEARCH: &str = include_str!("../../tests/fixtures/mal/manga_search.json");
const MANGA_DETAILS_1: &str = include_str!("../../tests/fixtures/mal/manga_details_1.json");
const MANGA_DETAILS_2: &str = include_str!("../../tests/fixtures/mal/manga_details_2.json");
const ANIME_DETAILS_1: &str = include_str!("../../tests/fixtures/mal/anime_details_1.json");
const MANGA_LIST_ENTRY_PRESENT: &str =
    include_str!("../../tests/fixtures/mal/manga_list_entry_present.json");
const MANGA_LIST_ENTRY_ABSENT: &str =
    include_str!("../../tests/fixtures/mal/manga_list_entry_absent.json");
const ANIME_LIST_ENTRY_PRESENT: &str =
    include_str!("../../tests/fixtures/mal/anime_list_entry_present.json");
const MANGALIST_PAGE1: &str = include_str!("../../tests/fixtures/mal/mangalist_page1.json");
const MANGALIST_PAGE2: &str = include_str!("../../tests/fixtures/mal/mangalist_page2.json");
const PUT_MANGA_RESPONSE: &str = include_str!("../../tests/fixtures/mal/put_manga_response.json");
const ERROR_401: &str = include_str!("../../tests/fixtures/mal/error_401.json");
const ERROR_400: &str = include_str!("../../tests/fixtures/mal/error_400_invalid_grant.json");

#[test]
fn every_fixture_deserialises_into_its_dto() {
    let token: MalToken = serde_json::from_str(OAUTH_TOKEN).expect("token");
    assert_eq!(token.access_token, "fixture-access");
    let token: MalToken =
        serde_json::from_str(OAUTH_TOKEN_WITH_CREATED).expect("token with created");
    assert_eq!(token.created_at, 1_700_000_000);
    let user: User = serde_json::from_str(USER_ME).expect("user");
    assert_eq!(user.name, "fixture_user");
    let search: SearchPage = serde_json::from_str(MANGA_SEARCH).expect("search");
    assert_eq!(search.data.len(), 2);
    serde_json::from_str::<MangaDetails>(MANGA_DETAILS_1).expect("manga 1");
    serde_json::from_str::<MangaDetails>(MANGA_DETAILS_2).expect("manga 2");
    serde_json::from_str::<AnimeDetails>(ANIME_DETAILS_1).expect("anime 1");
    serde_json::from_str::<MangaListEntry>(MANGA_LIST_ENTRY_PRESENT).expect("manga entry");
    serde_json::from_str::<MangaListEntry>(MANGA_LIST_ENTRY_ABSENT).expect("manga entry absent");
    serde_json::from_str::<AnimeListEntry>(ANIME_LIST_ENTRY_PRESENT).expect("anime entry");
    serde_json::from_str::<MangaListStatus>(PUT_MANGA_RESPONSE).expect("put reply");
    let page: UserListPage = serde_json::from_str(MANGALIST_PAGE1).expect("page 1");
    assert_eq!(page.data.len(), 250);
    let last: UserListPage = serde_json::from_str(MANGALIST_PAGE2).expect("page 2");
    assert!(last.paging.next.is_none());
    for body in [ERROR_401, ERROR_400] {
        serde_json::from_str::<serde_json::Value>(body).expect("error bodies are JSON");
    }
}

#[test]
fn manga_details_defaults_follow_the_kotlin_dto() {
    let d: MangaDetails = serde_json::from_str(MANGA_DETAILS_2).expect("decodes");
    assert_eq!(d.mean, -1.0, "mean absent is -1.0");
    assert_eq!(d.synopsis, "", "synopsis absent is empty");
    assert!(d.main_picture.is_none());
    assert!(d.start_date.is_none());
}

#[test]
fn null_synopsis_and_null_mean_take_the_defaults() {
    let d: AnimeDetails = serde_json::from_str(
        r#"{"id":1,"title":"t","synopsis":null,"num_episodes":1,"mean":null,"main_picture":null,"status":"s","media_type":"tv"}"#,
    )
    .expect("null is tolerated");
    assert_eq!(d.synopsis, "");
    assert_eq!(d.mean, -1.0);
    assert!(d.main_picture.is_none());
}

#[test]
fn unknown_keys_are_ignored() {
    let user: User = serde_json::from_str(r#"{"name":"ann","location":"x","joined":"y"}"#)
        .expect("extra keys are ignored");
    assert_eq!(user.name, "ann");
}

#[test]
fn missing_required_keys_are_a_decode_error_not_a_panic() {
    assert!(serde_json::from_str::<User>("{}").is_err());
    assert!(serde_json::from_str::<MangaDetails>(r#"{"id":1,"title":"t"}"#).is_err());
    assert!(serde_json::from_str::<MangaListStatus>(r#"{"status":"reading"}"#).is_err());
}
