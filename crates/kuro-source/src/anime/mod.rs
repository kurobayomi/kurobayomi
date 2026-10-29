//! The anime source API (area 16, `p1-rust-design.md` sections 2 and 3). It lives in this crate, next to the manga
//! API, as `ARCHITECTURE.md` and `CRATE-MAP.md` place it. No GTK and no HTTP here: the HTTP adapter lands in
//! `kuro-source-http`, the same as for manga.

/// The thumbnail a listing gives for an anime whose poster it does not know yet: `kb-cover:<anime url>`. The image
/// service asks the source's `cover_url` when the picture is needed (`kuro_images::fetch::COVER_MARKER` is the same text).
pub const COVER_MARKER: &str = "kb-cover:";

pub mod codec;
pub mod hoster;
pub mod loader;
pub mod model;
pub mod registry;
pub mod thumbnail;
pub mod traits;
pub mod url;
pub mod video;

pub use hoster::{Hoster, HosterState, NO_HOSTER_LIST, downloaded_hosters, http_hosters, local_hosters};
pub use loader::{best_video, load_hoster, resolve_once, select_best};
pub use model::{
    AnimeRelation, AnimeStatus, AnimesPage, FetchType, SAnime, SAnimeEpisodeUpdate, SAnimeSeasonUpdate, SEpisode,
};
pub use thumbnail::{ThumbnailInfo, TileInfo};
pub use traits::{
    AnimeSource, ConfigurableSource, ResolvableAnimeSource, ResolvedAnime, UriType, fetch_each_season, related_animes,
    resolve_anime_uri,
};
pub use url::{url_without_domain, url_without_domain_manga};
pub use video::{ChapterType, Headers, TimeStamp, Track, Video, VideoEntry, VideoState, is_placeholder};
