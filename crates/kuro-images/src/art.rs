//! `ArtRef`, `ArtKind` and `Kind`: one input type for manga and anime covers.

use kuro_platform::kind::Kind;
use crate::cover_paths::Family;

/// Source id of the local source, the same for manga and anime (`LocalMangaSource.ID`, `LocalAnimeSource.ID`).
/// Local entries keep their images in the entry folder, so they are never fetched or deleted here.
pub const LOCAL_SOURCE_ID: i64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtKind {
    Cover,
    /// Anime only. Manga entries have no background.
    Background,
}

/// One image of one entry: its cover, or an anime's background.
///
/// Library entries, list projections (`MangaCover`, `AnimeCover`) and anime requests with `use_background`
/// all reduce to this shape. `ArtRef::anime_entry` picks `kind` from `use_background`, so `Background` is only
/// set for an anime request with `use_background`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtRef {
    pub owner: Kind,
    pub entry_id: i64,
    pub source_id: i64,
    pub kind: ArtKind,
    /// Thumbnail URL for covers, background URL for backgrounds. `None` when the source gave none.
    pub url: Option<String>,
    /// `favorite` of the entry. Library entries write and promote cover files.
    pub favorite: bool,
    /// `cover_last_modified` or `background_last_modified`, in epoch milliseconds.
    pub last_modified: i64,
}

impl ArtRef {
    /// The image an anime entry shows for one request (`AnimeFactory`). With
    /// `use_background` it is the background: its URL, its timestamp and `ArtKind::Background`. Otherwise it is
    /// the thumbnail. `cover` and `background` are `(url, last_modified)` pairs. Cover projections
    /// (`AnimeCover`) build `ArtKind::Cover` directly and ignore the option, as Kotlin's cover-only factory does.
    pub fn anime_entry(
        use_background: bool,
        entry_id: i64,
        source_id: i64,
        favorite: bool,
        cover: (Option<String>, i64),
        background: (Option<String>, i64),
    ) -> Self {
        let (kind, (url, last_modified)) = if use_background {
            (ArtKind::Background, background)
        } else {
            (ArtKind::Cover, cover)
        };
        Self {
            owner: Kind::Anime,
            entry_id,
            source_id,
            kind,
            url,
            favorite,
            last_modified,
        }
    }

    /// The folder family the image lives in. Manga has no background, so a manga `Background` maps to the
    /// manga cover family.
    pub fn family(&self) -> Family {
        match (self.owner, self.kind) {
            (Kind::Anime, ArtKind::Background) => Family::AnimeBackground,
            (Kind::Anime, ArtKind::Cover) => Family::AnimeCover,
            (Kind::Manga, _) => Family::MangaCover,
        }
    }

    pub fn is_local(&self) -> bool {
        self.source_id == LOCAL_SOURCE_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(owner: Kind, kind: ArtKind, source_id: i64) -> ArtRef {
        ArtRef {
            owner,
            entry_id: 7,
            source_id,
            kind,
            url: None,
            favorite: false,
            last_modified: 0,
        }
    }

    #[test]
    fn family_follows_owner_and_kind() {
        assert_eq!(
            art(Kind::Manga, ArtKind::Cover, 1).family(),
            Family::MangaCover
        );
        assert_eq!(
            art(Kind::Anime, ArtKind::Cover, 1).family(),
            Family::AnimeCover
        );
        assert_eq!(
            art(Kind::Anime, ArtKind::Background, 1).family(),
            Family::AnimeBackground
        );
    }

    #[test]
    fn anime_entry_picks_the_background_only_when_asked() {
        let cover = (Some("thumb".to_owned()), 200);
        let background = (Some("bg".to_owned()), 300);
        let thumb = ArtRef::anime_entry(false, 7, 1, true, cover.clone(), background.clone());
        assert_eq!(thumb.kind, ArtKind::Cover);
        assert_eq!(thumb.url.as_deref(), Some("thumb"));
        assert_eq!(thumb.last_modified, 200);
        assert_eq!(thumb.family(), Family::AnimeCover);
        let bg = ArtRef::anime_entry(true, 7, 1, true, cover, background);
        assert_eq!(bg.kind, ArtKind::Background);
        assert_eq!(bg.url.as_deref(), Some("bg"));
        assert_eq!(bg.last_modified, 300);
        assert_eq!(bg.family(), Family::AnimeBackground);
        assert!(bg.favorite && bg.entry_id == 7 && bg.source_id == 1);
    }

    #[test]
    fn local_is_source_zero() {
        assert!(art(Kind::Manga, ArtKind::Cover, LOCAL_SOURCE_ID).is_local());
        assert!(!art(Kind::Manga, ArtKind::Cover, 42).is_local());
    }
}
