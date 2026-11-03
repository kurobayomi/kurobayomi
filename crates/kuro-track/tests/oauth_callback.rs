//! OAuth callback tests T-01, T-25 and T-26, through the public parser.

use kuro_track::model::TrackerId;
use kuro_track::oauth::{Redirect, parse_callback};

fn credential(
    tracker: TrackerId,
    value: &str,
    state: Option<&str>,
    expires: Option<i64>,
) -> Redirect {
    Redirect::Credential {
        tracker,
        value: value.into(),
        state: state.map(str::to_owned),
        expires_in: expires,
    }
}

#[test]
fn t_01_parse_callback_table() {
    let rows: [(&str, Redirect); 9] = [
        ("https://example.test/", Redirect::Ignored),
        ("aniyomi://unknown-auth?code=x", Redirect::Unexpected),
        ("aniyomi://kitsu-auth?code=x", Redirect::Unexpected),
        ("aniyomi://", Redirect::Unexpected),
        ("aniyomi://simkl-auth?code=", Redirect::Malformed),
        (
            "aniyomi://simkl-auth?error=access_denied",
            Redirect::Cancelled(TrackerId::SIMKL),
        ),
        (
            "aniyomi://bangumi-auth?code=x&state=s",
            credential(TrackerId::BANGUMI, "x", Some("s"), None),
        ),
        (
            "aniyomi://anilist-auth#error=access_denied",
            Redirect::Cancelled(TrackerId::ANILIST),
        ),
        (
            "aniyomi://myanimelist-auth?code=x",
            Redirect::MyAnimeListLoopback,
        ),
    ];
    for (uri, expected) in rows {
        assert_eq!(parse_callback(uri), expected, "{uri}");
    }
}

#[test]
fn t_25_anilist_fragment_tokens_are_raw_substrings() {
    let anilist = |value: &str, expires| credential(TrackerId::ANILIST, value, None, expires);
    assert_eq!(
        parse_callback("aniyomi://anilist-auth#access_token=abc&token_type=Bearer"),
        anilist("abc", None)
    );
    assert_eq!(
        parse_callback("aniyomi://anilist-auth#access_token=abc"),
        anilist("abc", None)
    );
    assert_eq!(
        parse_callback("aniyomi://anilist-auth#xaccess_token=abc"),
        Redirect::Cancelled(TrackerId::ANILIST)
    );
    assert_eq!(
        parse_callback("aniyomi://anilist-auth#access_token=a%2Bb"),
        anilist("a%2Bb", None)
    );
    assert_eq!(
        parse_callback("aniyomi://anilist-auth#access_token=abc&expires_in=3600"),
        anilist("abc", Some(3600))
    );
}

#[test]
fn t_26_query_percent_decode_keeps_plus_and_takes_the_first_code() {
    let bangumi = |value: &str| credential(TrackerId::BANGUMI, value, None, None);
    assert_eq!(
        parse_callback("aniyomi://bangumi-auth?code=a%2Bb"),
        bangumi("a+b")
    );
    assert_eq!(
        parse_callback("aniyomi://bangumi-auth?code=a+b"),
        bangumi("a+b")
    );
    assert_eq!(
        parse_callback("aniyomi://bangumi-auth?code=x&code=y"),
        bangumi("x")
    );
}
