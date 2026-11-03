//! `cache_key` for an `ArtRef` and the MD5 hex used for file names.

use kuro_platform::kind::Kind;
use md5::{Digest, Md5};

use crate::art::ArtRef;

/// Lower-case hex MD5 of the UTF-8 bytes (`DiskUtil.hashKeyForDisk`). Every file name derived from a key or a
/// URL goes through this, so a key never becomes a path.
pub fn hash_key(s: &str) -> String {
    Md5::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The cache key of one image: `<owner>;<entry id>;<last_modified>` when the entry has a custom file, and
/// `<owner>;<url or null>;<last_modified>` otherwise. Changing the timestamp changes
/// the key, so stale entries are never shown after an update.
///
/// `custom_exists` is the result of the custom-file check (`cover_paths::has_custom`), passed in so the
/// caller decides where that `stat` runs.
pub fn cache_key(art: &ArtRef, custom_exists: bool) -> String {
    let owner = match art.owner {
        Kind::Manga => "manga",
        Kind::Anime => "anime",
    };
    if custom_exists {
        format!("{owner};{};{}", art.entry_id, art.last_modified)
    } else {
        format!(
            "{owner};{};{}",
            art.url.as_deref().unwrap_or("null"),
            art.last_modified
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::ArtKind;

    fn art(owner: Kind, kind: ArtKind, url: Option<&str>, lm: i64) -> ArtRef {
        ArtRef {
            owner,
            entry_id: 7,
            source_id: 1,
            kind,
            url: url.map(str::to_owned),
            favorite: true,
            last_modified: lm,
        }
    }

    #[test]
    fn md5_golden_vectors() {
        assert_eq!(hash_key(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hash_key("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash_key("1"), "c4ca4238a0b923820dcc509a6f75849b");
        assert_eq!(
            hash_key("https://example.invalid/covers/one.jpg"),
            "82437b03c2a148c30ab4597ab61a1d78"
        );
        assert_eq!(
            hash_key("manga;https://example.invalid/covers/one.jpg;1700000000000"),
            "e2f3dd2a802b66728114818dd7e3ea6d"
        );
        assert_eq!(hash_key("shared_image"), "e3b5554d5cecfbb32cc29f434a058731");
        assert_eq!(
            hash_key("chapter_disk_cache"),
            "9498aa7d3b71cc1a2addac211c0a1c66"
        );
    }

    #[test]
    fn manga_custom_key_uses_id() {
        let a = art(Kind::Manga, ArtKind::Cover, Some("U"), 100);
        assert_eq!(cache_key(&a, true), "manga;7;100");
    }

    #[test]
    fn manga_thumbnail_key_uses_url() {
        let a = art(Kind::Manga, ArtKind::Cover, Some("U"), 100);
        assert_eq!(cache_key(&a, false), "manga;U;100");
        let moved = art(Kind::Manga, ArtKind::Cover, Some("U"), 101);
        assert_ne!(cache_key(&a, false), cache_key(&moved, false));
    }

    #[test]
    fn null_url_renders_as_null() {
        let a = art(Kind::Manga, ArtKind::Cover, None, 100);
        assert_eq!(cache_key(&a, false), "manga;null;100");
    }

    #[test]
    fn anime_background_keys() {
        let custom = art(Kind::Anime, ArtKind::Background, Some("B"), 300);
        assert_eq!(cache_key(&custom, true), "anime;7;300");
        assert_eq!(cache_key(&custom, false), "anime;B;300");
    }

    #[test]
    fn c12_e_background_request_with_custom_background_keys_by_id() {
        let bg = ArtRef::anime_entry(
            true,
            7,
            1,
            true,
            (Some("T".to_owned()), 200),
            (Some("B".to_owned()), 300),
        );
        assert_eq!(bg.kind, ArtKind::Background);
        assert_eq!(cache_key(&bg, true), "anime;7;300");
    }

    #[test]
    fn c12_f_background_request_without_custom_keys_by_background_url() {
        let bg = ArtRef::anime_entry(
            true,
            7,
            1,
            true,
            (Some("T".to_owned()), 200),
            (Some("B".to_owned()), 300),
        );
        assert_eq!(cache_key(&bg, false), "anime;B;300");
    }

    #[test]
    fn thumbnail_request_ignores_the_background_fields() {
        let thumb = ArtRef::anime_entry(
            false,
            7,
            1,
            true,
            (Some("T".to_owned()), 200),
            (Some("B".to_owned()), 300),
        );
        assert_eq!(cache_key(&thumb, false), "anime;T;200");
        assert_eq!(cache_key(&thumb, true), "anime;7;200");
    }

    #[test]
    fn anime_cover_key_branches() {
        let a = art(Kind::Anime, ArtKind::Cover, Some("https://x/y.jpg"), 1);
        assert!(cache_key(&a, false).starts_with("anime;https://x/y.jpg;"));
        assert!(cache_key(&a, true).starts_with("anime;7;"));
    }
}
