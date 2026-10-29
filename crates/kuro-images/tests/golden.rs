//! Golden tables of area:34 (`p1-edge-cases-and-tests.md` section 6), through the public API only. No network
//! and no display.

use kuro_platform::kind::Kind;
use kuro_images::art::{ArtKind, ArtRef};
use kuro_images::exception::{AppError, user_message};
use kuro_images::format::item_number_text;
use kuro_images::keys::{cache_key, hash_key};
use kuro_images::options::{RequestSize, Scale};
use kuro_images::sample::{Dimension, Size, in_sample_size, sample_for, target_dim};

#[test]
fn g_md5() {
    let rows = [
        ("", "d41d8cd98f00b204e9800998ecf8427e"),
        ("abc", "900150983cd24fb0d6963f7d28e17f72"),
        ("1", "c4ca4238a0b923820dcc509a6f75849b"),
        (
            "https://example.invalid/covers/one.jpg",
            "82437b03c2a148c30ab4597ab61a1d78",
        ),
        (
            "manga;https://example.invalid/covers/one.jpg;1700000000000",
            "e2f3dd2a802b66728114818dd7e3ea6d",
        ),
        ("shared_image", "e3b5554d5cecfbb32cc29f434a058731"),
        ("chapter_disk_cache", "9498aa7d3b71cc1a2addac211c0a1c66"),
    ];
    for (input, want) in rows {
        assert_eq!(hash_key(input), want, "{input:?}");
    }
}

#[test]
fn g_md5_of_a_real_cache_key() {
    let art = ArtRef {
        owner: Kind::Manga,
        entry_id: 7,
        source_id: 1,
        kind: ArtKind::Cover,
        url: Some("https://example.invalid/covers/one.jpg".into()),
        favorite: false,
        last_modified: 1_700_000_000_000,
    };
    assert_eq!(
        hash_key(&cache_key(&art, false)),
        "e2f3dd2a802b66728114818dd7e3ea6d"
    );
}

#[test]
fn g_sample() {
    use Scale::{Fill, Fit};
    let px = |w: u32, h: u32| RequestSize::Pixels {
        width: w,
        height: h,
    };
    let rows: [((u32, u32), RequestSize, Scale, i32); 14] = [
        ((1000, 1500), px(500, 500), Fit, 2),
        ((1000, 1500), px(500, 500), Fill, 2),
        ((1000, 1500), px(300, 400), Fit, 2),
        ((4000, 6000), px(500, 750), Fit, 8),
        ((4000, 6000), px(500, 750), Fill, 8),
        ((100, 100), px(100, 100), Fit, 1),
        ((1000, 1000), px(600, 400), Fit, 2),
        ((1000, 1000), px(600, 400), Fill, 1),
        ((5000, 100), px(1000, 1000), Fit, 4),
        ((5000, 100), px(1000, 1000), Fill, 1),
        ((2048, 2048), px(1024, 1024), Fit, 2),
        ((4000, 6000), px(0, 0), Fit, 1),
        ((4000, 6000), RequestSize::Original, Fit, 1),
        ((4000, 6000), RequestSize::Original, Fill, 1),
    ];
    for (src, size, scale, want) in rows {
        assert_eq!(
            sample_for(size, scale, src),
            want,
            "{src:?} {size:?} {scale:?}"
        );
    }
    for scale in [Fill, Fit] {
        let undefined = Size::Dims {
            width: Dimension::Undefined,
            height: Dimension::Undefined,
        };
        let (w, h) = target_dim(undefined, scale, (4000, 6000));
        assert_eq!(in_sample_size(4000, 6000, w, h, scale), 1);
    }
}

#[test]
fn g_num() {
    let rows: [(f64, &str); 16] = [
        (0.0, "0"),
        (0.5, "0.5"),
        (1.0, "1"),
        (1.5, "1.5"),
        (1.3333, "1.333"),
        (1.0005, "1"),
        (1.0015, "1.002"),
        (1.0025, "1.002"),
        (2.25, "2.25"),
        (-0.5, "-0.5"),
        (10.0, "10"),
        (0.0005, "0"),
        (0.0004, "0"),
        (100.1234, "100.123"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "∞"),
    ];
    for (n, want) in rows {
        assert_eq!(item_number_text(n), want, "{n}");
    }
}

#[test]
fn g_exc() {
    let host = |offline| AppError::UnknownHost {
        host: "example.invalid".into(),
        offline,
    };
    let other = |kind: &'static str, message: Option<&str>| AppError::Other {
        kind,
        message: message.map(str::to_owned),
    };
    let rows = [
        (
            AppError::Http { code: 404 },
            "HTTP 404, check website in WebView",
        ),
        (host(true), "No Internet connection"),
        (host(false), "Couldn't reach example.invalid"),
        (AppError::NoResults, "No results found"),
        (AppError::SourceNotInstalled, "Source not found"),
        (other("IOException", Some("disk full")), "disk full"),
        (other("IOException", None), "IOException"),
        (other("Exception", Some("boom")), "boom"),
        (
            other("IllegalStateException", Some("bad")),
            "IllegalStateException: bad",
        ),
        (
            other("IllegalStateException", None),
            "IllegalStateException: null",
        ),
    ];
    for (error, want) in rows {
        assert_eq!(user_message(&error), want, "{error:?}");
    }
}
