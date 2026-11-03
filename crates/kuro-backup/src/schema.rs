//! Message types of the `.tachibk` format (area 21, T-21a-002..006, T-21a-009).
//!
//! Field numbers, wire types and defaults follow Aniyomi's backup protobuf.
//! Entry messages are plain structs; the SQL-side conversion lives in `creator`.

use kuro_db::codecs::UpdateStrategy;
use kuro_db::codec::FetchType;

use crate::wire::{Message, Reader, WireError, Writer, required};

/// Serial names of the `PreferenceValue` subclasses. kotlinx polymorphic encoding writes the fully qualified class
/// name as field 1 and the subclass message (its `value` is field 1) as field 2. Confirmed on a real Aniyomi
/// `.tachibk` (xyz.jmir.tachiyomi.mi_2026-05-26, 34 app preferences, 7 source preference groups; T-21a-034). The
/// records and the dump are in `fixtures/restore/prefs_wire.md` and `tests/prefs_wire.rs`, so these constants are
/// verified, not guessed.
pub mod pref_tag {
    pub const INT: &str = "eu.kanade.tachiyomi.data.backup.models.IntPreferenceValue";
    pub const LONG: &str = "eu.kanade.tachiyomi.data.backup.models.LongPreferenceValue";
    pub const FLOAT: &str = "eu.kanade.tachiyomi.data.backup.models.FloatPreferenceValue";
    pub const STRING: &str = "eu.kanade.tachiyomi.data.backup.models.StringPreferenceValue";
    pub const BOOLEAN: &str = "eu.kanade.tachiyomi.data.backup.models.BooleanPreferenceValue";
    pub const STRING_SET: &str = "eu.kanade.tachiyomi.data.backup.models.StringSetPreferenceValue";
}

/// Unknown polymorphic type name in a preference value (Aniyomi throws a serialization error too).
pub const PREF_VALUE_UNKNOWN: &str = "unknown preference value type";

fn opt_str(w: &mut Writer, field: u32, v: &Option<String>) {
    if let Some(s) = v {
        w.string(field, s);
    }
}

fn update_strategy_of(v: u64) -> Result<UpdateStrategy, WireError> {
    match v {
        0 => Ok(UpdateStrategy::AlwaysUpdate),
        1 => Ok(UpdateStrategy::OnlyFetchOnce),
        ordinal => Err(WireError::BadEnum { name: "UpdateStrategy", ordinal }),
    }
}

/// `Seasons` 0, `Episodes` 1: Kotlin declaration order; the real dump has no field 507 to confirm it (`fixtures/MANIFEST.md`).
fn fetch_type_of(v: u64) -> Result<FetchType, WireError> {
    match v {
        0 => Ok(FetchType::Seasons),
        1 => Ok(FetchType::Episodes),
        ordinal => Err(WireError::BadEnum { name: "FetchType", ordinal }),
    }
}

/// The current container, field numbers 1, 2, 101, 104, 105, 106, 500..506.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Backup {
    pub manga: Vec<BackupManga>,
    pub categories: Vec<BackupCategory>,
    pub sources: Vec<BackupSource>,
    pub preferences: Vec<BackupPreference>,
    pub source_preferences: Vec<BackupSourcePreferences>,
    pub manga_extension_repos: Vec<BackupExtensionRepos>,
    pub is_legacy: bool,
    pub anime: Vec<BackupAnime>,
    pub anime_categories: Vec<BackupCategory>,
    pub anime_sources: Vec<BackupAnimeSource>,
    pub extensions: Vec<BackupExtension>,
    pub anime_extension_stores: Vec<BackupExtensionStore>,
    pub custom_buttons: Vec<BackupCustomButtons>,
}

impl Message for Backup {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        for m in &self.manga {
            w.message(1, m)?;
        }
        for c in &self.categories {
            w.message(2, c)?;
        }
        for s in &self.sources {
            w.message(101, s)?;
        }
        for p in &self.preferences {
            w.message(104, p)?;
        }
        for p in &self.source_preferences {
            w.message(105, p)?;
        }
        for r in &self.manga_extension_repos {
            w.message(106, r)?;
        }
        w.boolean(500, self.is_legacy);
        for a in &self.anime {
            w.message(501, a)?;
        }
        for c in &self.anime_categories {
            w.message(502, c)?;
        }
        for s in &self.anime_sources {
            w.message(503, s)?;
        }
        for e in &self.extensions {
            w.message(504, e)?;
        }
        for s in &self.anime_extension_stores {
            w.message(505, s)?;
        }
        for b in &self.custom_buttons {
            w.message(506, b)?;
        }
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        // Kotlin default for `isLegacy` when the field is absent is `true`.
        let mut b = Self { is_legacy: true, ..Self::default() };
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => b.manga.push(r.message(f, wt)?),
                2 => b.categories.push(r.message(f, wt)?),
                101 => b.sources.push(r.message(f, wt)?),
                104 => b.preferences.push(r.message(f, wt)?),
                105 => b.source_preferences.push(r.message(f, wt)?),
                106 => b.manga_extension_repos.push(r.message(f, wt)?),
                500 => b.is_legacy = r.boolean(f, wt)?,
                501 => b.anime.push(r.message(f, wt)?),
                502 => b.anime_categories.push(r.message(f, wt)?),
                503 => b.anime_sources.push(r.message(f, wt)?),
                504 => b.extensions.push(r.message(f, wt)?),
                505 => b.anime_extension_stores.push(r.message(f, wt)?),
                506 => b.custom_buttons.push(r.message(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        Ok(b)
    }
}

/// Pre-1.0 Aniyomi container, read only. Field numbers 1, 2, 3, 4, 101, 103..109.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LegacyBackup {
    pub backup_manga: Vec<BackupManga>,
    pub backup_categories: Vec<BackupCategory>,
    pub backup_anime: Vec<BackupAnime>,
    pub backup_anime_categories: Vec<BackupCategory>,
    pub backup_sources: Vec<BackupSource>,
    pub backup_anime_sources: Vec<BackupAnimeSource>,
    pub backup_preferences: Vec<BackupPreference>,
    pub backup_source_preferences: Vec<BackupSourcePreferences>,
    pub backup_extensions: Vec<BackupExtension>,
    pub backup_anime_extension_stores: Vec<BackupExtensionStore>,
    pub backup_manga_extension_repo: Vec<BackupExtensionRepos>,
    pub backup_custom_button: Vec<BackupCustomButtons>,
}

impl LegacyBackup {
    /// Lossless mapping of the legacy lists to the current container (B-12, data 1.2). Field 106 here is
    /// extensions, and field 108 is the manga repos.
    pub fn into_current(self) -> Backup {
        Backup {
            manga: self.backup_manga,
            categories: self.backup_categories,
            sources: self.backup_sources,
            preferences: self.backup_preferences,
            source_preferences: self.backup_source_preferences,
            manga_extension_repos: self.backup_manga_extension_repo,
            is_legacy: false,
            anime: self.backup_anime,
            anime_categories: self.backup_anime_categories,
            anime_sources: self.backup_anime_sources,
            extensions: self.backup_extensions,
            anime_extension_stores: self.backup_anime_extension_stores,
            custom_buttons: self.backup_custom_button,
        }
    }
}

impl Message for LegacyBackup {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        for m in &self.backup_manga {
            w.message(1, m)?;
        }
        for c in &self.backup_categories {
            w.message(2, c)?;
        }
        for a in &self.backup_anime {
            w.message(3, a)?;
        }
        for c in &self.backup_anime_categories {
            w.message(4, c)?;
        }
        for s in &self.backup_sources {
            w.message(101, s)?;
        }
        for s in &self.backup_anime_sources {
            w.message(103, s)?;
        }
        for p in &self.backup_preferences {
            w.message(104, p)?;
        }
        for p in &self.backup_source_preferences {
            w.message(105, p)?;
        }
        for e in &self.backup_extensions {
            w.message(106, e)?;
        }
        for s in &self.backup_anime_extension_stores {
            w.message(107, s)?;
        }
        for r in &self.backup_manga_extension_repo {
            w.message(108, r)?;
        }
        for b in &self.backup_custom_button {
            w.message(109, b)?;
        }
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut b = Self::default();
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => b.backup_manga.push(r.message(f, wt)?),
                2 => b.backup_categories.push(r.message(f, wt)?),
                3 => b.backup_anime.push(r.message(f, wt)?),
                4 => b.backup_anime_categories.push(r.message(f, wt)?),
                101 => b.backup_sources.push(r.message(f, wt)?),
                103 => b.backup_anime_sources.push(r.message(f, wt)?),
                104 => b.backup_preferences.push(r.message(f, wt)?),
                105 => b.backup_source_preferences.push(r.message(f, wt)?),
                106 => b.backup_extensions.push(r.message(f, wt)?),
                107 => b.backup_anime_extension_stores.push(r.message(f, wt)?),
                108 => b.backup_manga_extension_repo.push(r.message(f, wt)?),
                109 => b.backup_custom_button.push(r.message(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        Ok(b)
    }
}

/// Manga entry (`Backup.manga`, field 1).
#[derive(Debug, Clone, PartialEq)]
pub struct BackupManga {
    pub source: i64,
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Vec<String>,
    pub status: i32,
    pub thumbnail_url: Option<String>,
    pub date_added: i64,
    /// Field 14. Written as `viewer_flags & 0x7` when `viewer_flags` is set.
    pub viewer: i32,
    pub chapters: Vec<BackupChapter>,
    /// Category `order` values, never ids. Unpacked on the wire.
    pub categories: Vec<i64>,
    pub tracking: Vec<BackupTracking>,
    pub favorite: bool,
    pub chapter_flags: i32,
    /// Field 103. Takes precedence over `viewer` when present.
    pub viewer_flags: Option<i32>,
    pub history: Vec<BackupHistory>,
    pub update_strategy: UpdateStrategy,
    pub last_modified_at: i64,
    pub favorite_modified_at: Option<i64>,
    pub excluded_scanlators: Vec<String>,
    pub version: i64,
}

impl Default for BackupManga {
    fn default() -> Self {
        Self {
            source: 0,
            url: String::new(),
            title: String::new(),
            artist: None,
            author: None,
            description: None,
            genre: Vec::new(),
            status: 0,
            thumbnail_url: None,
            date_added: 0,
            viewer: 0,
            chapters: Vec::new(),
            categories: Vec::new(),
            tracking: Vec::new(),
            favorite: true,
            chapter_flags: 0,
            viewer_flags: None,
            history: Vec::new(),
            update_strategy: UpdateStrategy::AlwaysUpdate,
            last_modified_at: 0,
            favorite_modified_at: None,
            excluded_scanlators: Vec::new(),
            version: 0,
        }
    }
}

impl BackupManga {
    /// Effective reader flags: `viewer_flags` (103) when present, else the legacy `viewer` (14).
    pub fn effective_viewer_flags(&self) -> i32 {
        self.viewer_flags.unwrap_or(self.viewer)
    }
}

impl Message for BackupManga {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.int64(1, self.source);
        w.string(2, &self.url);
        w.string(3, &self.title);
        opt_str(w, 4, &self.artist);
        opt_str(w, 5, &self.author);
        opt_str(w, 6, &self.description);
        for g in &self.genre {
            w.string(7, g);
        }
        w.int32(8, self.status);
        opt_str(w, 9, &self.thumbnail_url);
        w.int64(13, self.date_added);
        w.int32(14, self.viewer_flags.map_or(self.viewer, |v| v & 0x7));
        for c in &self.chapters {
            w.message(16, c)?;
        }
        for c in &self.categories {
            w.int64(17, *c);
        }
        for t in &self.tracking {
            w.message(18, t)?;
        }
        w.boolean(100, self.favorite);
        w.int32(101, self.chapter_flags);
        if let Some(v) = self.viewer_flags {
            w.int32(103, v);
        }
        for h in &self.history {
            w.message(104, h)?;
        }
        w.enum_ordinal(105, self.update_strategy as u8);
        w.int64(106, self.last_modified_at);
        if let Some(v) = self.favorite_modified_at {
            w.int64(107, v);
        }
        for s in &self.excluded_scanlators {
            w.string(108, s);
        }
        w.int64(109, self.version);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut source, mut url) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => source = Some(r.int64(f, wt)?),
                2 => url = Some(r.string(f, wt)?),
                3 => m.title = r.string(f, wt)?,
                4 => m.artist = Some(r.string(f, wt)?),
                5 => m.author = Some(r.string(f, wt)?),
                6 => m.description = Some(r.string(f, wt)?),
                7 => m.genre.push(r.string(f, wt)?),
                8 => m.status = r.int32(f, wt)?,
                9 => m.thumbnail_url = Some(r.string(f, wt)?),
                13 => m.date_added = r.int64(f, wt)?,
                14 => m.viewer = r.int32(f, wt)?,
                16 => m.chapters.push(r.message(f, wt)?),
                17 => r.repeated_int64(f, wt, &mut m.categories)?,
                18 => m.tracking.push(r.message(f, wt)?),
                100 => m.favorite = r.boolean(f, wt)?,
                101 => m.chapter_flags = r.int32(f, wt)?,
                103 => m.viewer_flags = Some(r.int32(f, wt)?),
                104 => m.history.push(r.message(f, wt)?),
                105 => m.update_strategy = update_strategy_of(r.enum_ordinal(f, wt)?)?,
                106 => m.last_modified_at = r.int64(f, wt)?,
                107 => m.favorite_modified_at = Some(r.int64(f, wt)?),
                108 => m.excluded_scanlators.push(r.string(f, wt)?),
                109 => m.version = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.source = required(source, "source")?;
        m.url = required(url, "url")?;
        Ok(m)
    }
}

/// Chapter of a manga (`BackupManga.chapters`, field 16).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupChapter {
    pub url: String,
    pub name: String,
    pub scanlator: Option<String>,
    pub read: bool,
    pub bookmark: bool,
    pub last_page_read: i64,
    pub date_fetch: i64,
    pub date_upload: i64,
    pub chapter_number: f32,
    pub source_order: i64,
    pub last_modified_at: i64,
    pub version: i64,
}

impl Message for BackupChapter {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.url);
        w.string(2, &self.name);
        opt_str(w, 3, &self.scanlator);
        w.boolean(4, self.read);
        w.boolean(5, self.bookmark);
        w.int64(6, self.last_page_read);
        w.int64(7, self.date_fetch);
        w.int64(8, self.date_upload);
        w.float32(9, self.chapter_number);
        w.int64(10, self.source_order);
        w.int64(11, self.last_modified_at);
        w.int64(12, self.version);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut url, mut name) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => url = Some(r.string(f, wt)?),
                2 => name = Some(r.string(f, wt)?),
                3 => m.scanlator = Some(r.string(f, wt)?),
                4 => m.read = r.boolean(f, wt)?,
                5 => m.bookmark = r.boolean(f, wt)?,
                6 => m.last_page_read = r.int64(f, wt)?,
                7 => m.date_fetch = r.int64(f, wt)?,
                8 => m.date_upload = r.int64(f, wt)?,
                9 => m.chapter_number = r.float32(f, wt)?,
                10 => m.source_order = r.int64(f, wt)?,
                11 => m.last_modified_at = r.int64(f, wt)?,
                12 => m.version = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.url = required(url, "url")?;
        m.name = required(name, "name")?;
        Ok(m)
    }
}

/// Manga read history (`BackupManga.history`, field 104). `url` is the chapter URL.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupHistory {
    pub url: String,
    pub last_read: i64,
    pub read_duration: i64,
}

impl Message for BackupHistory {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.url);
        w.int64(2, self.last_read);
        w.int64(3, self.read_duration);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut url, mut last_read) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => url = Some(r.string(f, wt)?),
                2 => last_read = Some(r.int64(f, wt)?),
                3 => m.read_duration = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.url = required(url, "url")?;
        m.last_read = required(last_read, "lastRead")?;
        Ok(m)
    }
}

/// Manga tracking (`BackupManga.tracking`, field 18). Decodes `remote_id` from `media_id_int` when non-zero,
/// else from `media_id`. Creation writes `media_id` (100) only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupTracking {
    pub sync_id: i32,
    pub library_id: i64,
    /// Field 3, deprecated. Read only.
    pub media_id_int: i32,
    pub tracking_url: String,
    pub title: String,
    pub last_chapter_read: f32,
    pub total_chapters: i32,
    pub score: f32,
    pub status: i32,
    pub started_reading_date: i64,
    pub finished_reading_date: i64,
    pub private: bool,
    pub media_id: i64,
}

impl BackupTracking {
    /// The remote id a restore uses (E-19): `media_id_int` if non-zero, else `media_id`.
    pub fn remote_id(&self) -> i64 {
        if self.media_id_int != 0 { i64::from(self.media_id_int) } else { self.media_id }
    }
}

impl Message for BackupTracking {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.int32(1, self.sync_id);
        w.int64(2, self.library_id);
        w.string(4, &self.tracking_url);
        w.string(5, &self.title);
        w.float32(6, self.last_chapter_read);
        w.int32(7, self.total_chapters);
        w.float32(8, self.score);
        w.int32(9, self.status);
        w.int64(10, self.started_reading_date);
        w.int64(11, self.finished_reading_date);
        w.boolean(12, self.private);
        w.int64(100, self.media_id);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut sync_id, mut library_id) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => sync_id = Some(r.int32(f, wt)?),
                2 => library_id = Some(r.int64(f, wt)?),
                3 => m.media_id_int = r.int32(f, wt)?,
                4 => m.tracking_url = r.string(f, wt)?,
                5 => m.title = r.string(f, wt)?,
                6 => m.last_chapter_read = r.float32(f, wt)?,
                7 => m.total_chapters = r.int32(f, wt)?,
                8 => m.score = r.float32(f, wt)?,
                9 => m.status = r.int32(f, wt)?,
                10 => m.started_reading_date = r.int64(f, wt)?,
                11 => m.finished_reading_date = r.int64(f, wt)?,
                12 => m.private = r.boolean(f, wt)?,
                100 => m.media_id = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.sync_id = required(sync_id, "syncId")?;
        m.library_id = required(library_id, "libraryId")?;
        Ok(m)
    }
}

/// Anime entry (`Backup.anime`, field 501).
#[derive(Debug, Clone, PartialEq)]
pub struct BackupAnime {
    pub source: i64,
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Vec<String>,
    pub status: i32,
    pub thumbnail_url: Option<String>,
    pub date_added: i64,
    pub episodes: Vec<BackupEpisode>,
    pub categories: Vec<i64>,
    pub tracking: Vec<BackupAnimeTracking>,
    pub favorite: bool,
    pub episode_flags: i32,
    pub viewer_flags: i32,
    pub history: Vec<BackupAnimeHistory>,
    pub update_strategy: UpdateStrategy,
    pub last_modified_at: i64,
    pub favorite_modified_at: Option<i64>,
    pub version: i64,
    /// JSON text bytes. `{}` when empty.
    pub memo: Vec<u8>,
    /// Field 500. Aniyomi never writes it; Kurobayomi writes it when the anime has a background.
    pub background_url: Option<String>,
    pub parent_id: Option<i64>,
    pub id: Option<i64>,
    pub season_flags: i64,
    pub season_number: f64,
    pub season_source_order: i64,
    pub fetch_type: FetchType,
}

impl Default for BackupAnime {
    fn default() -> Self {
        Self {
            source: 0,
            url: String::new(),
            title: String::new(),
            artist: None,
            author: None,
            description: None,
            genre: Vec::new(),
            status: 0,
            thumbnail_url: None,
            date_added: 0,
            episodes: Vec::new(),
            categories: Vec::new(),
            tracking: Vec::new(),
            favorite: true,
            episode_flags: 0,
            viewer_flags: 0,
            history: Vec::new(),
            update_strategy: UpdateStrategy::AlwaysUpdate,
            last_modified_at: 0,
            favorite_modified_at: None,
            version: 0,
            memo: b"{}".to_vec(),
            background_url: None,
            parent_id: None,
            id: None,
            season_flags: 0,
            season_number: -1.0,
            season_source_order: 0,
            fetch_type: FetchType::Episodes,
        }
    }
}

impl Message for BackupAnime {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.int64(1, self.source);
        w.string(2, &self.url);
        w.string(3, &self.title);
        opt_str(w, 4, &self.artist);
        opt_str(w, 5, &self.author);
        opt_str(w, 6, &self.description);
        for g in &self.genre {
            w.string(7, g);
        }
        w.int32(8, self.status);
        opt_str(w, 9, &self.thumbnail_url);
        w.int64(13, self.date_added);
        for e in &self.episodes {
            w.message(16, e)?;
        }
        for c in &self.categories {
            w.int64(17, *c);
        }
        for t in &self.tracking {
            w.message(18, t)?;
        }
        w.boolean(100, self.favorite);
        w.int32(101, self.episode_flags);
        w.int32(103, self.viewer_flags);
        for h in &self.history {
            w.message(104, h)?;
        }
        w.enum_ordinal(105, self.update_strategy as u8);
        w.int64(106, self.last_modified_at);
        if let Some(v) = self.favorite_modified_at {
            w.int64(107, v);
        }
        w.int64(109, self.version);
        w.bytes(112, &self.memo);
        opt_str(w, 500, &self.background_url);
        if let Some(v) = self.parent_id {
            w.int64(502, v);
        }
        if let Some(v) = self.id {
            w.int64(503, v);
        }
        w.int64(504, self.season_flags);
        w.float64(505, self.season_number);
        w.int64(506, self.season_source_order);
        w.enum_ordinal(507, self.fetch_type as u8);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut source, mut url) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => source = Some(r.int64(f, wt)?),
                2 => url = Some(r.string(f, wt)?),
                3 => m.title = r.string(f, wt)?,
                4 => m.artist = Some(r.string(f, wt)?),
                5 => m.author = Some(r.string(f, wt)?),
                6 => m.description = Some(r.string(f, wt)?),
                7 => m.genre.push(r.string(f, wt)?),
                8 => m.status = r.int32(f, wt)?,
                9 => m.thumbnail_url = Some(r.string(f, wt)?),
                13 => m.date_added = r.int64(f, wt)?,
                16 => m.episodes.push(r.message(f, wt)?),
                17 => r.repeated_int64(f, wt, &mut m.categories)?,
                18 => m.tracking.push(r.message(f, wt)?),
                100 => m.favorite = r.boolean(f, wt)?,
                101 => m.episode_flags = r.int32(f, wt)?,
                103 => m.viewer_flags = r.int32(f, wt)?,
                104 => m.history.push(r.message(f, wt)?),
                105 => m.update_strategy = update_strategy_of(r.enum_ordinal(f, wt)?)?,
                106 => m.last_modified_at = r.int64(f, wt)?,
                107 => m.favorite_modified_at = Some(r.int64(f, wt)?),
                109 => m.version = r.int64(f, wt)?,
                112 => m.memo = r.bytes(f, wt)?,
                500 => m.background_url = Some(r.string(f, wt)?),
                502 => m.parent_id = Some(r.int64(f, wt)?),
                503 => m.id = Some(r.int64(f, wt)?),
                504 => m.season_flags = r.int64(f, wt)?,
                505 => m.season_number = r.float64(f, wt)?,
                506 => m.season_source_order = r.int64(f, wt)?,
                507 => m.fetch_type = fetch_type_of(r.enum_ordinal(f, wt)?)?,
                _ => r.skip(wt)?,
            }
        }
        m.source = required(source, "source")?;
        m.url = required(url, "url")?;
        Ok(m)
    }
}

/// Episode of an anime (`BackupAnime.episodes`, field 16).
#[derive(Debug, Clone, PartialEq)]
pub struct BackupEpisode {
    pub url: String,
    pub name: String,
    pub scanlator: Option<String>,
    pub seen: bool,
    pub bookmark: bool,
    pub last_second_seen: i64,
    pub total_seconds: i64,
    pub date_fetch: i64,
    pub date_upload: i64,
    pub episode_number: f32,
    pub source_order: i64,
    pub last_modified_at: i64,
    pub version: i64,
    pub memo: Vec<u8>,
    pub fillermark: bool,
    pub summary: Option<String>,
    pub preview_url: Option<String>,
}

impl Default for BackupEpisode {
    fn default() -> Self {
        Self {
            url: String::new(),
            name: String::new(),
            scanlator: None,
            seen: false,
            bookmark: false,
            last_second_seen: 0,
            total_seconds: 0,
            date_fetch: 0,
            date_upload: 0,
            episode_number: 0.0,
            source_order: 0,
            last_modified_at: 0,
            version: 0,
            memo: b"{}".to_vec(),
            fillermark: false,
            summary: None,
            preview_url: None,
        }
    }
}

impl Message for BackupEpisode {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.url);
        w.string(2, &self.name);
        opt_str(w, 3, &self.scanlator);
        w.boolean(4, self.seen);
        w.boolean(5, self.bookmark);
        w.int64(6, self.last_second_seen);
        w.int64(7, self.date_fetch);
        w.int64(8, self.date_upload);
        w.float32(9, self.episode_number);
        w.int64(10, self.source_order);
        w.int64(11, self.last_modified_at);
        w.int64(12, self.version);
        w.bytes(13, &self.memo);
        w.int64(16, self.total_seconds);
        w.boolean(501, self.fillermark);
        opt_str(w, 502, &self.summary);
        opt_str(w, 503, &self.preview_url);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut url, mut name) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => url = Some(r.string(f, wt)?),
                2 => name = Some(r.string(f, wt)?),
                3 => m.scanlator = Some(r.string(f, wt)?),
                4 => m.seen = r.boolean(f, wt)?,
                5 => m.bookmark = r.boolean(f, wt)?,
                6 => m.last_second_seen = r.int64(f, wt)?,
                7 => m.date_fetch = r.int64(f, wt)?,
                8 => m.date_upload = r.int64(f, wt)?,
                9 => m.episode_number = r.float32(f, wt)?,
                10 => m.source_order = r.int64(f, wt)?,
                11 => m.last_modified_at = r.int64(f, wt)?,
                12 => m.version = r.int64(f, wt)?,
                13 => m.memo = r.bytes(f, wt)?,
                16 => m.total_seconds = r.int64(f, wt)?,
                501 => m.fillermark = r.boolean(f, wt)?,
                502 => m.summary = Some(r.string(f, wt)?),
                503 => m.preview_url = Some(r.string(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        m.url = required(url, "url")?;
        m.name = required(name, "name")?;
        Ok(m)
    }
}

/// Anime tracking (`BackupAnime.tracking`, field 18). Same numbers as `BackupTracking` with the episode fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupAnimeTracking {
    pub sync_id: i32,
    pub library_id: i64,
    pub media_id_int: i32,
    pub tracking_url: String,
    pub title: String,
    pub last_episode_seen: f32,
    pub total_episodes: i32,
    pub score: f32,
    pub status: i32,
    pub started_watching_date: i64,
    pub finished_watching_date: i64,
    pub private: bool,
    pub media_id: i64,
}

impl BackupAnimeTracking {
    /// The remote id a restore uses (E-19): `media_id_int` if non-zero, else `media_id`.
    pub fn remote_id(&self) -> i64 {
        if self.media_id_int != 0 { i64::from(self.media_id_int) } else { self.media_id }
    }
}

impl Message for BackupAnimeTracking {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.int32(1, self.sync_id);
        w.int64(2, self.library_id);
        w.string(4, &self.tracking_url);
        w.string(5, &self.title);
        w.float32(6, self.last_episode_seen);
        w.int32(7, self.total_episodes);
        w.float32(8, self.score);
        w.int32(9, self.status);
        w.int64(10, self.started_watching_date);
        w.int64(11, self.finished_watching_date);
        w.boolean(12, self.private);
        w.int64(100, self.media_id);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut sync_id, mut library_id) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => sync_id = Some(r.int32(f, wt)?),
                2 => library_id = Some(r.int64(f, wt)?),
                3 => m.media_id_int = r.int32(f, wt)?,
                4 => m.tracking_url = r.string(f, wt)?,
                5 => m.title = r.string(f, wt)?,
                6 => m.last_episode_seen = r.float32(f, wt)?,
                7 => m.total_episodes = r.int32(f, wt)?,
                8 => m.score = r.float32(f, wt)?,
                9 => m.status = r.int32(f, wt)?,
                10 => m.started_watching_date = r.int64(f, wt)?,
                11 => m.finished_watching_date = r.int64(f, wt)?,
                12 => m.private = r.boolean(f, wt)?,
                100 => m.media_id = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.sync_id = required(sync_id, "syncId")?;
        m.library_id = required(library_id, "libraryId")?;
        Ok(m)
    }
}

/// Anime history (`BackupAnime.history`, field 104). `url` is the episode URL.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupAnimeHistory {
    pub url: String,
    pub last_read: i64,
    pub read_duration: i64,
}

impl Message for BackupAnimeHistory {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.url);
        w.int64(2, self.last_read);
        w.int64(3, self.read_duration);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut url, mut last_read) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => url = Some(r.string(f, wt)?),
                2 => last_read = Some(r.int64(f, wt)?),
                3 => m.read_duration = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.url = required(url, "url")?;
        m.last_read = required(last_read, "lastRead")?;
        Ok(m)
    }
}

/// Category (fields 2 and 502 of `Backup`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupCategory {
    pub name: String,
    pub order: i64,
    pub id: i64,
    pub flags: i64,
}

impl Message for BackupCategory {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.name);
        w.int64(2, self.order);
        w.int64(3, self.id);
        w.int64(100, self.flags);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let mut name = None;
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => name = Some(r.string(f, wt)?),
                2 => m.order = r.int64(f, wt)?,
                3 => m.id = r.int64(f, wt)?,
                100 => m.flags = r.int64(f, wt)?,
                _ => r.skip(wt)?,
            }
        }
        m.name = required(name, "name")?;
        Ok(m)
    }
}

/// Manga source reference (`Backup.sources`, field 101).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupSource {
    pub name: String,
    pub source_id: i64,
}

impl Message for BackupSource {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.name);
        w.int64(2, self.source_id);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let mut source_id = None;
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => m.name = r.string(f, wt)?,
                2 => source_id = Some(r.int64(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        m.source_id = required(source_id, "sourceId")?;
        Ok(m)
    }
}

/// Anime source reference (`Backup.anime_sources`, field 503).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupAnimeSource {
    pub name: String,
    pub source_id: i64,
}

impl Message for BackupAnimeSource {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.name);
        w.int64(2, self.source_id);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let mut source_id = None;
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => m.name = r.string(f, wt)?,
                2 => source_id = Some(r.int64(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        m.source_id = required(source_id, "sourceId")?;
        Ok(m)
    }
}

/// Manga extension repo (`Backup.manga_extension_repos`, field 106).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupExtensionRepos {
    pub base_url: String,
    pub name: String,
    pub short_name: Option<String>,
    pub website: String,
    pub signing_key_fingerprint: String,
}

impl Message for BackupExtensionRepos {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.base_url);
        w.string(2, &self.name);
        opt_str(w, 3, &self.short_name);
        w.string(4, &self.website);
        w.string(5, &self.signing_key_fingerprint);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut base_url, mut name, mut website, mut key) = (None, None, None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => base_url = Some(r.string(f, wt)?),
                2 => name = Some(r.string(f, wt)?),
                3 => m.short_name = Some(r.string(f, wt)?),
                4 => website = Some(r.string(f, wt)?),
                5 => key = Some(r.string(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        m.base_url = required(base_url, "baseUrl")?;
        m.name = required(name, "name")?;
        m.website = required(website, "website")?;
        m.signing_key_fingerprint = required(key, "signingKeyFingerprint")?;
        Ok(m)
    }
}

/// Anime extension store (`Backup.anime_extension_stores`, field 505).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupExtensionStore {
    pub index_url: String,
    pub name: String,
    pub badge_label: Option<String>,
    pub contact_website: String,
    pub signing_key: String,
    pub contact_discord: Option<String>,
    /// `None` is absent on the wire.
    pub is_legacy: Option<bool>,
    pub extension_list_url: Option<String>,
}

impl Message for BackupExtensionStore {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.index_url);
        w.string(2, &self.name);
        opt_str(w, 3, &self.badge_label);
        w.string(4, &self.contact_website);
        w.string(5, &self.signing_key);
        opt_str(w, 6, &self.contact_discord);
        if let Some(v) = self.is_legacy {
            w.boolean(7, v);
        }
        opt_str(w, 8, &self.extension_list_url);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let (mut index_url, mut name, mut website, mut key) = (None, None, None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => index_url = Some(r.string(f, wt)?),
                2 => name = Some(r.string(f, wt)?),
                3 => m.badge_label = Some(r.string(f, wt)?),
                4 => website = Some(r.string(f, wt)?),
                5 => key = Some(r.string(f, wt)?),
                6 => m.contact_discord = Some(r.string(f, wt)?),
                7 => m.is_legacy = Some(r.boolean(f, wt)?),
                8 => m.extension_list_url = Some(r.string(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        m.index_url = required(index_url, "indexUrl")?;
        m.name = required(name, "name")?;
        m.contact_website = required(website, "contactWebsite")?;
        m.signing_key = required(key, "signingKey")?;
        Ok(m)
    }
}

/// Installed extension package (`Backup.extensions`, field 504). Equality compares package id and bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupExtension {
    pub pkg_name: String,
    pub apk: Vec<u8>,
}

impl Message for BackupExtension {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.pkg_name);
        w.bytes(2, &self.apk);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let (mut pkg, mut apk) = (None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => pkg = Some(r.string(f, wt)?),
                2 => apk = Some(r.bytes(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        Ok(Self {
            pkg_name: required(pkg, "pkgName")?,
            apk: required(apk, "apk")?,
        })
    }
}

/// Custom button (`Backup.custom_buttons`, field 506). Every field is required on decode.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupCustomButtons {
    pub name: String,
    pub is_favorite: bool,
    pub sort_index: i64,
    pub content: String,
    pub long_press_content: String,
    pub on_startup: String,
}

impl Message for BackupCustomButtons {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.name);
        w.boolean(2, self.is_favorite);
        w.int64(3, self.sort_index);
        w.string(4, &self.content);
        w.string(5, &self.long_press_content);
        w.string(6, &self.on_startup);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let (mut name, mut fav, mut sort, mut content, mut long, mut startup) = (None, None, None, None, None, None);
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => name = Some(r.string(f, wt)?),
                2 => fav = Some(r.boolean(f, wt)?),
                3 => sort = Some(r.int64(f, wt)?),
                4 => content = Some(r.string(f, wt)?),
                5 => long = Some(r.string(f, wt)?),
                6 => startup = Some(r.string(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        Ok(Self {
            name: required(name, "name")?,
            is_favorite: required(fav, "isFavorite")?,
            sort_index: required(sort, "sortIndex")?,
            content: required(content, "content")?,
            long_press_content: required(long, "longPressContent")?,
            on_startup: required(startup, "onStartup")?,
        })
    }
}

/// Preference value. The six variants of the sealed hierarchy (data doc 4.3).
#[derive(Debug, Clone, PartialEq)]
pub enum PrefValue {
    Int(i32),
    Long(i64),
    Float(f32),
    String(String),
    Boolean(bool),
    StringSet(std::collections::BTreeSet<String>),
}

/// One app preference (`Backup.preferences`, field 104).
#[derive(Debug, Clone, PartialEq)]
pub struct BackupPreference {
    pub key: String,
    pub value: PrefValue,
}

impl Message for PrefValue {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        let mut inner = Writer::new();
        let tag = match self {
            Self::Int(v) => {
                inner.int32(1, *v);
                pref_tag::INT
            }
            Self::Long(v) => {
                inner.int64(1, *v);
                pref_tag::LONG
            }
            Self::Float(v) => {
                inner.float32(1, *v);
                pref_tag::FLOAT
            }
            Self::String(v) => {
                inner.string(1, v);
                pref_tag::STRING
            }
            Self::Boolean(v) => {
                inner.boolean(1, *v);
                pref_tag::BOOLEAN
            }
            Self::StringSet(set) => {
                for v in set {
                    inner.string(1, v);
                }
                pref_tag::STRING_SET
            }
        };
        w.string(1, tag);
        w.bytes(2, &inner.finish());
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut tag = None;
        let mut body: &[u8] = &[];
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => tag = Some(r.string(f, wt)?),
                2 => {
                    // Kept raw: the subclass is only known once field 1 has been read.
                    if wt != crate::wire::WireType::Len {
                        return Err(WireError::BadWireType(wt as u8));
                    }
                    body = r.len_delimited()?;
                }
                _ => r.skip(wt)?,
            }
        }
        let mut r = Reader::new(body);
        let mut int = None;
        let mut long = None;
        let mut float = None;
        let mut text = None;
        let mut flag = None;
        let mut set = std::collections::BTreeSet::new();
        let tag = required(tag, "type")?;
        // The class name is matched by its simple name, so `...backup.models.` and `...backup.full.models.` both decode.
        let kind = tag.rsplit('.').next().unwrap_or_default();
        while let Some((f, wt)) = r.next_tag()? {
            if f != 1 {
                r.skip(wt)?;
                continue;
            }
            match kind {
                "IntPreferenceValue" => int = Some(r.int32(f, wt)?),
                "LongPreferenceValue" => long = Some(r.int64(f, wt)?),
                "FloatPreferenceValue" => float = Some(r.float32(f, wt)?),
                "StringPreferenceValue" => text = Some(r.string(f, wt)?),
                "BooleanPreferenceValue" => flag = Some(r.boolean(f, wt)?),
                "StringSetPreferenceValue" => {
                    set.insert(r.string(f, wt)?);
                }
                _ => return Err(WireError::Unsupported(PREF_VALUE_UNKNOWN)),
            }
        }
        // An absent `value` is the Kotlin default of the subclass.
        match kind {
            "IntPreferenceValue" => Ok(Self::Int(int.unwrap_or(0))),
            "LongPreferenceValue" => Ok(Self::Long(long.unwrap_or(0))),
            "FloatPreferenceValue" => Ok(Self::Float(float.unwrap_or(0.0))),
            "StringPreferenceValue" => Ok(Self::String(text.unwrap_or_default())),
            "BooleanPreferenceValue" => Ok(Self::Boolean(flag.unwrap_or(false))),
            "StringSetPreferenceValue" => Ok(Self::StringSet(set)),
            _ => Err(WireError::Unsupported(PREF_VALUE_UNKNOWN)),
        }
    }
}

impl Message for BackupPreference {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.key);
        w.message(2, &self.value)
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut key = None;
        let mut value = None;
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => key = Some(r.string(f, wt)?),
                2 => value = Some(r.message(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        Ok(Self { key: required(key, "key")?, value: required(value, "value")? })
    }
}

/// Source preferences (`Backup.source_preferences`, field 105).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackupSourcePreferences {
    pub source_key: String,
    pub prefs: Vec<BackupPreference>,
}

impl Message for BackupSourcePreferences {
    fn encode(&self, w: &mut Writer) -> Result<(), WireError> {
        w.string(1, &self.source_key);
        for p in &self.prefs {
            w.message(2, p)?;
        }
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut m = Self::default();
        let mut key = None;
        while let Some((f, wt)) = r.next_tag()? {
            match f {
                1 => key = Some(r.string(f, wt)?),
                2 => m.prefs.push(r.message(f, wt)?),
                _ => r.skip(wt)?,
            }
        }
        m.source_key = required(key, "sourceKey")?;
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields_of(bytes: &[u8]) -> Vec<u32> {
        let mut out = Vec::new();
        let mut r = Reader::new(bytes);
        while let Some((f, wt)) = r.next_tag().unwrap() {
            out.push(f);
            r.skip(wt).unwrap();
        }
        out
    }

    #[test]
    fn backup_field_table_matches_data_doc() {
        // Every list populated; the encoder must emit exactly the field numbers of data doc section 5.
        let manga = BackupManga { source: 1, url: "u".into(), ..Default::default() };
        let cat = BackupCategory { name: "c".into(), ..Default::default() };
        let src = BackupSource { name: "s".into(), source_id: 1 };
        let asrc = BackupAnimeSource { name: "s".into(), source_id: 1 };
        let repo = BackupExtensionRepos { base_url: "b".into(), name: "n".into(), website: "w".into(), signing_key_fingerprint: "k".into(), ..Default::default() };
        let anime = BackupAnime { source: 1, url: "u".into(), ..Default::default() };
        let ext = BackupExtension { pkg_name: "p".into(), apk: vec![1] };
        let store = BackupExtensionStore { index_url: "i".into(), name: "n".into(), contact_website: "w".into(), signing_key: "k".into(), ..Default::default() };
        let btn = BackupCustomButtons { name: "n".into(), content: "c".into(), long_press_content: "l".into(), on_startup: "".into(), ..Default::default() };
        let pref = BackupSourcePreferences { source_key: "source_1".into(), prefs: vec![] };
        let b = Backup {
            manga: vec![manga],
            categories: vec![cat.clone()],
            sources: vec![src],
            source_preferences: vec![pref],
            manga_extension_repos: vec![repo],
            anime: vec![anime],
            anime_categories: vec![cat],
            anime_sources: vec![asrc],
            extensions: vec![ext],
            anime_extension_stores: vec![store],
            custom_buttons: vec![btn],
            ..Default::default()
        };
        let mut f = fields_of(&b.to_bytes().unwrap());
        f.sort_unstable();
        f.dedup();
        // 104 (preferences) is not populated here: its encoding is blocked on T-21a-034.
        assert_eq!(f, vec![1, 2, 101, 105, 106, 500, 501, 502, 503, 504, 505, 506]);
    }

    #[test]
    fn default_backup_writes_is_legacy_false() {
        let bytes = Backup::default().to_bytes().unwrap();
        assert_eq!(bytes, [0xA0, 0x1F, 0x00]);
        assert!(!Backup::from_bytes(&bytes).unwrap().is_legacy);
    }

    #[test]
    fn absent_is_legacy_decodes_as_true() {
        let b = Backup::from_bytes(&[]).unwrap();
        assert!(b.is_legacy);
    }

    #[test]
    fn manga_golden_bytes_with_defaults() {
        let m = BackupManga { source: 1, url: "a".into(), ..Default::default() };
        let expected: Vec<u8> = vec![
            0x08, 0x01, // 1 source = 1
            0x12, 0x01, 0x61, // 2 url = "a"
            0x1A, 0x00, // 3 title = ""
            0x40, 0x00, // 8 status = 0
            0x68, 0x00, // 13 dateAdded = 0
            0x70, 0x00, // 14 viewer = 0
            0xA0, 0x06, 0x01, // 100 favorite = true
            0xA8, 0x06, 0x00, // 101 chapterFlags = 0
            0xC8, 0x06, 0x00, // 105 updateStrategy = 0
            0xD0, 0x06, 0x00, // 106 lastModifiedAt = 0
            0xE8, 0x06, 0x00, // 109 version = 0
        ];
        assert_eq!(m.to_bytes().unwrap(), expected);
    }

    #[test]
    fn manga_viewer_flags_precedence_and_viewer_mask() {
        let m = BackupManga { source: 1, url: "a".into(), viewer_flags: Some(0x1F), ..Default::default() };
        let back = BackupManga::from_bytes(&m.to_bytes().unwrap()).unwrap();
        assert_eq!(back.viewer, 0x7, "viewer (14) is written as viewer_flags & 0x7");
        assert_eq!(back.viewer_flags, Some(0x1F));
        assert_eq!(back.effective_viewer_flags(), 0x1F);

        // A Mihon-shaped entry without 103 reads viewer (14).
        let mut w = Writer::new();
        w.int64(1, 1);
        w.string(2, "a");
        w.int32(14, 2);
        let old = BackupManga::from_bytes(&w.finish()).unwrap();
        assert_eq!(old.viewer_flags, None);
        assert_eq!(old.effective_viewer_flags(), 2);
    }

    #[test]
    fn tracking_remote_id_prefers_media_id_int_when_non_zero() {
        let mut t = BackupTracking { sync_id: 2, library_id: 0, media_id: 99, ..Default::default() };
        assert_eq!(t.remote_id(), 99);
        t.media_id_int = 7;
        assert_eq!(t.remote_id(), 7);
        // Legacy-only payload: field 3 set, field 100 absent.
        let mut w = Writer::new();
        w.int32(1, 2);
        w.int64(2, 0);
        w.int32(3, 42);
        let legacy = BackupTracking::from_bytes(&w.finish()).unwrap();
        assert_eq!(legacy.remote_id(), 42);
    }

    #[test]
    fn tracking_writes_media_id_only() {
        let t = BackupTracking { sync_id: 2, library_id: 5, media_id: 77, media_id_int: 3, ..Default::default() };
        let f = fields_of(&t.to_bytes().unwrap());
        assert!(!f.contains(&3), "media_id_int (3) is never written");
        assert!(f.contains(&100));
        let back = BackupTracking::from_bytes(&t.to_bytes().unwrap()).unwrap();
        assert_eq!(back.media_id, 77);
    }

    #[test]
    fn float_fields_are_fixed32_on_the_wire() {
        let c = BackupChapter { url: "u".into(), name: "n".into(), chapter_number: 1.5, ..Default::default() };
        let back = BackupChapter::from_bytes(&c.to_bytes().unwrap()).unwrap();
        assert_eq!(back.chapter_number, 1.5f32);
        assert!(c.to_bytes().unwrap().windows(5).any(|w| w == [0x4D, 0x00, 0x00, 0xC0, 0x3F]));
    }

    #[test]
    fn anime_defaults_and_memo_and_background() {
        let a = BackupAnime { source: 1, url: "u".into(), ..Default::default() };
        assert_eq!(a.season_number, -1.0);
        let back = BackupAnime::from_bytes(&a.to_bytes().unwrap()).unwrap();
        assert_eq!(back.memo, b"{}");
        assert_eq!(back.season_number, -1.0);

        let with_bg = BackupAnime { background_url: Some("https://x/bg.png".into()), ..a };
        let round = BackupAnime::from_bytes(&with_bg.to_bytes().unwrap()).unwrap();
        assert_eq!(round.background_url.as_deref(), Some("https://x/bg.png"));
        let no_bg = BackupAnime { source: 1, url: "u".into(), ..Default::default() };
        assert!(!fields_of(&no_bg.to_bytes().unwrap()).contains(&500));
    }

    #[test]
    fn anime_season_number_is_fixed64() {
        let a = BackupAnime { source: 1, url: "u".into(), season_number: 2.0, ..Default::default() };
        let bytes = a.to_bytes().unwrap();
        // tag 505 wire 1 = (505 << 3) | 1 = 4041 -> varint [0xC9, 0x1F].
        assert!(bytes.windows(2).any(|w| w == [0xC9, 0x1F]));
    }

    #[test]
    fn episode_fields_roundtrip() {
        let e = BackupEpisode { url: "e".into(), name: "n".into(), episode_number: 12.5, total_seconds: 1400, summary: Some("s".into()), ..Default::default() };
        let back = BackupEpisode::from_bytes(&e.to_bytes().unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn anime_tracking_and_history_roundtrip() {
        let t = BackupAnimeTracking { sync_id: 3, library_id: 1, last_episode_seen: 4.0, total_episodes: 12, media_id: 8, ..Default::default() };
        assert_eq!(BackupAnimeTracking::from_bytes(&t.to_bytes().unwrap()).unwrap(), t);
        let h = BackupAnimeHistory { url: "e".into(), last_read: 1_700_000_000_000, read_duration: 0 };
        assert_eq!(BackupAnimeHistory::from_bytes(&h.to_bytes().unwrap()).unwrap(), h);
    }

    #[test]
    fn required_field_missing_is_error() {
        // BackupManga without field 2 (url).
        let mut w = Writer::new();
        w.int64(1, 1);
        assert_eq!(BackupManga::from_bytes(&w.finish()).err(), Some(WireError::MissingField("url")));
    }

    #[test]
    fn bad_enum_ordinal_is_error() {
        let mut w = Writer::new();
        w.int64(1, 1);
        w.string(2, "a");
        w.enum_ordinal(105, 9);
        assert_eq!(
            BackupManga::from_bytes(&w.finish()).err(),
            Some(WireError::BadEnum { name: "UpdateStrategy", ordinal: 9 })
        );
    }

    #[test]
    fn custom_button_missing_field_fails() {
        let mut w = Writer::new();
        w.string(1, "n");
        w.boolean(2, true);
        w.int64(3, 0);
        w.string(4, "c");
        w.string(5, "l");
        // on_startup (6) absent.
        assert_eq!(BackupCustomButtons::from_bytes(&w.finish()).err(), Some(WireError::MissingField("onStartup")));
    }

    #[test]
    fn custom_button_empty_on_startup_roundtrips() {
        let b = BackupCustomButtons { name: "n".into(), is_favorite: true, sort_index: -3, content: "c".into(), long_press_content: "l".into(), on_startup: String::new() };
        let back = BackupCustomButtons::from_bytes(&b.to_bytes().unwrap()).unwrap();
        assert_eq!(back.on_startup, "");
        assert_eq!(back.sort_index, -3);
    }

    #[test]
    fn extension_store_is_legacy_absent_when_none() {
        let s = BackupExtensionStore { index_url: "i".into(), name: "n".into(), contact_website: "w".into(), signing_key: "k".into(), ..Default::default() };
        assert!(!fields_of(&s.to_bytes().unwrap()).contains(&7));
        let t = BackupExtensionStore { is_legacy: Some(false), ..s };
        assert!(fields_of(&t.to_bytes().unwrap()).contains(&7));
    }

    #[test]
    fn extension_equality_compares_package_and_bytes() {
        let a = BackupExtension { pkg_name: "p".into(), apk: vec![1, 2] };
        assert_eq!(a, BackupExtension { pkg_name: "p".into(), apk: vec![1, 2] });
        assert_ne!(a, BackupExtension { pkg_name: "p".into(), apk: vec![1, 3] });
    }

    #[test]
    fn legacy_into_current_maps_fields() {
        let legacy = LegacyBackup {
            backup_manga: vec![BackupManga { source: 1, url: "m".into(), ..Default::default() }],
            backup_anime: vec![BackupAnime { source: 2, url: "a".into(), ..Default::default() }],
            backup_extensions: vec![BackupExtension { pkg_name: "ext".into(), apk: vec![9] }],
            backup_manga_extension_repo: vec![BackupExtensionRepos { base_url: "repo".into(), ..Default::default() }],
            backup_custom_button: vec![BackupCustomButtons { name: "b".into(), ..Default::default() }],
            backup_anime_extension_stores: vec![BackupExtensionStore { index_url: "s".into(), ..Default::default() }],
            ..Default::default()
        };
        let cur = legacy.clone().into_current();
        assert!(!cur.is_legacy);
        assert_eq!(cur.manga.len(), 1);
        assert_eq!(cur.anime.len(), 1);
        assert_eq!(cur.extensions.len(), 1, "legacy 106 is extensions");
        assert!(cur.manga_extension_repos.iter().all(|r| r.base_url == "repo"), "legacy 108 is manga repos");
        assert_eq!(cur.manga_extension_repos.len(), 1);
        assert_eq!(cur.custom_buttons.len(), 1);
        assert_eq!(cur.anime_extension_stores.len(), 1);
    }

    #[test]
    fn legacy_field_106_decodes_into_extensions() {
        let legacy = LegacyBackup {
            backup_extensions: vec![BackupExtension { pkg_name: "e".into(), apk: vec![1] }],
            ..Default::default()
        };
        let back = LegacyBackup::from_bytes(&legacy.to_bytes().unwrap()).unwrap();
        let cur = back.into_current();
        assert_eq!(cur.extensions.len(), 1);
        assert!(cur.manga_extension_repos.is_empty());
    }

    fn assert_roundtrip<M: Message + PartialEq + std::fmt::Debug>(m: M) {
        assert_eq!(M::from_bytes(&m.to_bytes().unwrap()).unwrap(), m);
    }

    /// 4.1 `wire_round_trip_all_structs`: every message type with non-default values in every field.
    #[test]
    fn wire_round_trip_all_structs() {
        let chapter = BackupChapter {
            url: "/c/1".into(), name: "Ch 1".into(), scanlator: Some("grp".into()), read: true, bookmark: true, last_page_read: 9,
            date_fetch: 10, date_upload: 11, chapter_number: 1.5, source_order: 3, last_modified_at: 12, version: 13,
        };
        let history = BackupHistory { url: "/c/1".into(), last_read: 5, read_duration: 6 };
        let tracking = BackupTracking {
            sync_id: 2, library_id: 3, media_id_int: 0, tracking_url: "u".into(), title: "t".into(), last_chapter_read: 4.5,
            total_chapters: 20, score: 7.5, status: 1, started_reading_date: 8, finished_reading_date: 9, private: true, media_id: 77,
        };
        let manga = BackupManga {
            source: 7, url: "/m".into(), title: "T".into(), artist: Some("a".into()), author: Some("b".into()), description: Some("d".into()),
            genre: vec!["x".into(), "y".into()], status: 2, thumbnail_url: Some("th".into()), date_added: 99, viewer: 3, chapters: vec![chapter.clone()],
            categories: vec![1, 2, 3], tracking: vec![tracking.clone()], favorite: false, chapter_flags: 5, viewer_flags: Some(3),
            history: vec![history.clone()], update_strategy: UpdateStrategy::OnlyFetchOnce, last_modified_at: 4, favorite_modified_at: Some(6),
            excluded_scanlators: vec!["e".into()], version: 8,
        };
        let episode = BackupEpisode {
            url: "/e/1".into(), name: "E1".into(), scanlator: Some("s".into()), seen: true, bookmark: true, last_second_seen: 1, total_seconds: 2,
            date_fetch: 3, date_upload: 4, episode_number: 2.5, source_order: 5, last_modified_at: 6, version: 7, memo: b"{\"a\":1}".to_vec(),
            fillermark: true, summary: Some("sum".into()), preview_url: Some("pv".into()),
        };
        let atracking = BackupAnimeTracking {
            sync_id: 2, library_id: 3, media_id_int: 0, tracking_url: "u".into(), title: "t".into(), last_episode_seen: 4.5,
            total_episodes: 20, score: 7.5, status: 1, started_watching_date: 8, finished_watching_date: 9, private: true, media_id: 77,
        };
        let ahistory = BackupAnimeHistory { url: "/e/1".into(), last_read: 5, read_duration: 6 };
        let anime = BackupAnime {
            source: 8, url: "/a".into(), title: "A".into(), artist: Some("a".into()), author: Some("b".into()), description: Some("d".into()),
            genre: vec!["g".into()], status: 1, thumbnail_url: Some("th".into()), date_added: 5, episodes: vec![episode.clone()], categories: vec![4],
            tracking: vec![atracking.clone()], favorite: false, episode_flags: 6, viewer_flags: 7, history: vec![ahistory.clone()],
            update_strategy: UpdateStrategy::OnlyFetchOnce, last_modified_at: 8, favorite_modified_at: Some(9), version: 10, memo: b"{\"k\":2}".to_vec(),
            background_url: Some("bg".into()), parent_id: Some(11), id: Some(12), season_flags: 13, season_number: 2.5, season_source_order: 14,
            fetch_type: FetchType::Episodes,
        };
        let category = BackupCategory { name: "c".into(), order: 2, id: 4, flags: 8 };
        let source = BackupSource { name: "s".into(), source_id: 7 };
        let asource = BackupAnimeSource { name: "s".into(), source_id: 8 };
        let repo = BackupExtensionRepos { base_url: "b".into(), name: "n".into(), short_name: Some("sn".into()), website: "w".into(), signing_key_fingerprint: "f".into() };
        let store = BackupExtensionStore {
            index_url: "i".into(), name: "n".into(), badge_label: Some("bl".into()), contact_website: "w".into(), signing_key: "k".into(),
            contact_discord: Some("dc".into()), is_legacy: Some(true), extension_list_url: Some("l".into()),
        };
        let ext = BackupExtension { pkg_name: "p".into(), apk: vec![0, 1, 255] };
        let button = BackupCustomButtons { name: "n".into(), is_favorite: true, sort_index: 3, content: "c".into(), long_press_content: "l".into(), on_startup: "o".into() };
        let pref = BackupPreference { key: "k".into(), value: PrefValue::Float(2.5) };
        let spref = BackupSourcePreferences { source_key: "source_7".into(), prefs: vec![pref.clone()] };

        assert_roundtrip(chapter);
        assert_roundtrip(history);
        assert_roundtrip(tracking);
        assert_roundtrip(manga.clone());
        assert_roundtrip(episode);
        assert_roundtrip(atracking);
        assert_roundtrip(ahistory);
        assert_roundtrip(anime.clone());
        assert_roundtrip(category.clone());
        assert_roundtrip(source.clone());
        assert_roundtrip(asource.clone());
        assert_roundtrip(repo.clone());
        assert_roundtrip(store.clone());
        assert_roundtrip(ext.clone());
        assert_roundtrip(button.clone());
        assert_roundtrip(pref.clone());
        assert_roundtrip(spref.clone());
        assert_roundtrip(Backup {
            manga: vec![manga.clone()], categories: vec![category.clone()], sources: vec![source.clone()], preferences: vec![pref.clone()],
            source_preferences: vec![spref.clone()], manga_extension_repos: vec![repo.clone()], is_legacy: false, anime: vec![anime.clone()],
            anime_categories: vec![category.clone()], anime_sources: vec![asource.clone()], extensions: vec![ext.clone()],
            anime_extension_stores: vec![store.clone()], custom_buttons: vec![button.clone()],
        });
        assert_roundtrip(LegacyBackup {
            backup_manga: vec![manga], backup_categories: vec![category.clone()], backup_anime: vec![anime], backup_anime_categories: vec![category],
            backup_sources: vec![source], backup_anime_sources: vec![asource], backup_preferences: vec![pref], backup_source_preferences: vec![spref],
            backup_extensions: vec![ext], backup_anime_extension_stores: vec![store], backup_manga_extension_repo: vec![repo], backup_custom_button: vec![button],
        });
    }

    fn pref_roundtrip(v: PrefValue) {
        let p = BackupPreference { key: "k".into(), value: v };
        assert_eq!(BackupPreference::from_bytes(&p.to_bytes().unwrap()).unwrap(), p);
    }

    #[test]
    fn preference_values_roundtrip() {
        pref_roundtrip(PrefValue::Int(-4));
        pref_roundtrip(PrefValue::Int(0));
        pref_roundtrip(PrefValue::Long(1779815489026));
        pref_roundtrip(PrefValue::Float(1.5));
        pref_roundtrip(PrefValue::String("SYSTEM".into()));
        pref_roundtrip(PrefValue::Boolean(false));
        pref_roundtrip(PrefValue::Boolean(true));
        pref_roundtrip(PrefValue::StringSet(["a".to_string(), "b".to_string()].into_iter().collect()));
        pref_roundtrip(PrefValue::StringSet(Default::default()));
    }

    #[test]
    fn preference_bytes_match_real_aniyomi_file() {
        // Bytes seen in a real Aniyomi backup: key `library_unseen_updates_count`, IntPreferenceValue(4).
        let mut real = vec![0x0a, 28];
        real.extend_from_slice(b"library_unseen_updates_count");
        real.extend_from_slice(&[0x12, 63, 0x0a, 57]);
        real.extend_from_slice(pref_tag::INT.as_bytes());
        real.extend_from_slice(&[0x12, 2, 0x08, 0x04]);
        let p = BackupPreference::from_bytes(&real).unwrap();
        assert_eq!(p, BackupPreference { key: "library_unseen_updates_count".into(), value: PrefValue::Int(4) });
        assert_eq!(p.to_bytes().unwrap(), real);
    }

    #[test]
    fn the_full_models_serial_names_decode_like_the_plain_ones() {
        for (suffix, value) in [("Int", PrefValue::Int(4)), ("Long", PrefValue::Long(5)), ("Float", PrefValue::Float(1.5)),
            ("String", PrefValue::String("s".into())), ("Boolean", PrefValue::Boolean(true)),
            ("StringSet", PrefValue::StringSet(["a".to_string()].into_iter().collect()))]
        {
            let plain = value.to_bytes().unwrap();
            // The plain bytes with field 1 (the serial name) swapped for the full-models one.
            let mut full = Writer::new();
            full.string(1, &format!("eu.kanade.tachiyomi.data.backup.full.models.{suffix}PreferenceValue"));
            let mut r = Reader::new(&plain);
            while let Some((f, wt)) = r.next_tag().unwrap() {
                if f == 2 {
                    full.bytes(2, r.len_delimited().unwrap());
                } else {
                    r.skip(wt).unwrap();
                }
            }
            assert_eq!(PrefValue::from_bytes(&full.finish()).unwrap(), value, "{suffix}");
        }
    }

    #[test]
    fn unknown_preference_type_is_an_error() {
        let mut w = Writer::new();
        w.string(1, "k");
        let mut v = Writer::new();
        v.string(1, "x.Other");
        v.bytes(2, &[0x08, 1]);
        w.bytes(2, &v.finish());
        assert_eq!(BackupPreference::from_bytes(&w.finish()), Err(WireError::Unsupported(PREF_VALUE_UNKNOWN)));
    }
}
