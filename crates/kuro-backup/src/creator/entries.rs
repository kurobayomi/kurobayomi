//! Manga and anime entry creators (T-21a-013, T-21a-014) and the row conversions of B-08a, B-08b and B-09. Both read
//! the shared [`EntryData`]; only the wire shapes differ.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::creator::EntryData;
use crate::flags::chapter_flags_to_wire;
use crate::error::BackupError;
use crate::options::BackupOptions;
use crate::schema::{BackupAnime, BackupAnimeHistory, BackupAnimeTracking, BackupChapter, BackupEpisode, BackupHistory, BackupManga, BackupTracking};

/// Reader viewer mask (`ReadingMode.MASK`, B-08a).
const VIEWER_MASK: i32 = 0x7;

/// E-14: an integer that does not fit its format field fails the backup; the row id is logged and named in the error.
pub fn i32_of(entry: &'static str, id: i64, field: &'static str, v: i64) -> Result<i32, BackupError> {
    i32::try_from(v).map_err(|_| {
        tracing::warn!(entry, id, field, value = v, "backup value does not fit the format");
        BackupError::Overflow { entry, id, field }
    })
}

/// Category links are category `order` values; the system category (id 0) is never linked (B-08a step 4).
fn category_orders(cats: &[kuro_data::category::Category]) -> Vec<i64> {
    cats.iter().filter(|c| c.id != 0).map(|c| c.order).collect()
}

fn f32_of(v: f64) -> f32 {
    v as f32
}

/// Parses an episode or manga memo. Invalid JSON or a non-object yields `{}` and a warning, never a failure (E-28).
pub fn parse_memo(text: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => m,
        _ => {
            tracing::warn!("memo is not a JSON object; written as {{}}");
            Map::new()
        }
    }
}

/// UTF-8 JSON text of a memo. An empty object is `{}` (`7B 7D`).
pub fn memo_bytes(memo: &Map<String, Value>) -> Vec<u8> {
    serde_json::to_vec(memo).unwrap_or_else(|_| b"{}".to_vec())
}

/// History rows named by the url of their item: `(url, last_at, duration_ms)`. A row whose item is gone is skipped.
fn history_rows<'a>(e: &'a EntryData, what: &'static str) -> Vec<(&'a str, i64, i64)> {
    let urls: BTreeMap<i64, &str> = e.items.iter().map(|i| (i.id, i.url.as_str())).collect();
    e.history
        .iter()
        .filter_map(|h| match urls.get(&h.item_id) {
            Some(url) => Some((*url, h.last_at, h.duration_ms)),
            None => {
                tracing::warn!(entry = e.entry.id, item = h.item_id, "history row without {what}; skipped");
                None
            }
        })
        .collect()
}

pub fn manga_entry(e: &EntryData, opts: &BackupOptions) -> Result<BackupManga, BackupError> {
    let m = &e.entry;
    let id = m.id;
    let viewer_flags = i32_of("manga", id, "viewer_flags", m.viewer_flags)?;
    let mut out = BackupManga {
        source: m.source,
        url: m.url.clone(),
        title: m.title.clone(),
        artist: m.artist.clone(),
        author: m.author.clone(),
        description: m.description.clone(),
        genre: m.genre.clone().unwrap_or_default(),
        status: i32_of("manga", id, "status", m.status)?,
        thumbnail_url: m.thumbnail_url.clone(),
        date_added: m.date_added,
        viewer: viewer_flags & VIEWER_MASK,
        viewer_flags: Some(viewer_flags),
        chapter_flags: i32_of("manga", id, "chapter_flags", chapter_flags_to_wire(m.item_flags))?,
        favorite: m.favorite,
        update_strategy: m.update_strategy,
        last_modified_at: m.last_modified_at,
        favorite_modified_at: m.favorite_modified_at,
        version: m.version,
        // E-25: exclusions are written regardless of the chapters option.
        excluded_scanlators: e.excluded_scanlators.iter().cloned().collect(),
        ..Default::default()
    };

    if opts.chapters && !e.items.is_empty() {
        out.chapters = e
            .items
            .iter()
            .map(|c| BackupChapter {
                url: c.url.clone(),
                name: c.name.clone(),
                scanlator: c.scanlator.clone(),
                read: c.done,
                bookmark: c.bookmark,
                last_page_read: c.progress,
                date_fetch: c.date_fetch,
                date_upload: c.date_upload,
                chapter_number: f32_of(c.number),
                source_order: c.source_order,
                last_modified_at: c.last_modified_at,
                version: c.version,
                page_count: c.page_count,
            })
            .collect();
    }

    if opts.categories {
        out.categories = category_orders(&e.categories);
    }

    if opts.tracking && !e.tracks.is_empty() {
        out.tracking = e
            .tracks
            .iter()
            .map(|t| {
                Ok(BackupTracking {
                    sync_id: i32_of("manga", id, "tracker_id", t.tracker_id)?,
                    library_id: t.library_id.unwrap_or(0),
                    media_id_int: 0,
                    tracking_url: t.remote_url.clone(),
                    title: t.title.clone(),
                    last_chapter_read: f32_of(t.last_item),
                    total_chapters: i32_of("manga", id, "total_chapters", t.total_items)?,
                    score: f32_of(t.score),
                    status: i32_of("manga", id, "tracking_status", t.status)?,
                    started_reading_date: t.start_date,
                    finished_reading_date: t.finish_date,
                    private: t.private,
                    media_id: t.remote_id,
                })
            })
            .collect::<Result<_, BackupError>>()?;
    }

    if opts.history && !e.history.is_empty() {
        out.history = history_rows(e, "a chapter")
            .into_iter()
            .map(|(url, last_read, read_duration)| BackupHistory { url: url.to_owned(), last_read, read_duration })
            .collect();
    }

    Ok(out)
}

pub fn anime_entry(e: &EntryData, opts: &BackupOptions) -> Result<BackupAnime, BackupError> {
    let a = &e.entry;
    let id = a.id;
    let mut out = BackupAnime {
        source: a.source,
        url: a.url.clone(),
        title: a.title.clone(),
        artist: a.artist.clone(),
        author: a.author.clone(),
        description: a.description.clone(),
        genre: a.genre.clone().unwrap_or_default(),
        status: i32_of("anime", id, "status", a.status)?,
        thumbnail_url: a.thumbnail_url.clone(),
        date_added: a.date_added,
        favorite: a.favorite,
        // Anime `episodeFlags` is the stored word as it is.
        episode_flags: i32_of("anime", id, "episode_flags", a.item_flags)?,
        viewer_flags: i32_of("anime", id, "viewer_flags", a.viewer_flags)?,
        update_strategy: a.update_strategy,
        last_modified_at: a.last_modified_at,
        favorite_modified_at: a.favorite_modified_at,
        version: a.version,
        memo: memo_bytes(&a.memo()),
        // E-26: written when the anime has a background.
        background_url: a.background_url.clone(),
        parent_id: a.parent_id,
        id: Some(id),
        season_flags: a.season_flags(),
        season_number: a.season_number(),
        season_source_order: a.season_source_order(),
        fetch_type: a.fetch_type(),
        ..Default::default()
    };

    if opts.chapters && !e.items.is_empty() {
        out.episodes = e
            .items
            .iter()
            .map(|ep| BackupEpisode {
                url: ep.url.clone(),
                name: ep.name.clone(),
                scanlator: ep.scanlator.clone(),
                seen: ep.done,
                bookmark: ep.bookmark,
                last_second_seen: ep.progress,
                total_seconds: ep.total_seconds(),
                date_fetch: ep.date_fetch,
                date_upload: ep.date_upload,
                episode_number: f32_of(ep.number),
                source_order: ep.source_order,
                last_modified_at: ep.last_modified_at,
                version: ep.version,
                memo: memo_bytes(&ep.memo()),
                fillermark: ep.fillermark,
                summary: ep.summary.clone(),
                preview_url: ep.preview_url.clone(),
            })
            .collect();
    }

    if opts.categories {
        out.categories = category_orders(&e.categories);
    }

    if opts.tracking && !e.tracks.is_empty() {
        out.tracking = e
            .tracks
            .iter()
            .map(|t| {
                Ok(BackupAnimeTracking {
                    sync_id: i32_of("anime", id, "tracker_id", t.tracker_id)?,
                    library_id: t.library_id.unwrap_or(0),
                    media_id_int: 0,
                    tracking_url: t.remote_url.clone(),
                    title: t.title.clone(),
                    last_episode_seen: f32_of(t.last_item),
                    total_episodes: i32_of("anime", id, "total_episodes", t.total_items)?,
                    score: f32_of(t.score),
                    status: i32_of("anime", id, "tracking_status", t.status)?,
                    started_watching_date: t.start_date,
                    finished_watching_date: t.finish_date,
                    private: t.private,
                    media_id: t.remote_id,
                })
            })
            .collect::<Result<_, BackupError>>()?;
    }

    if opts.history && !e.history.is_empty() {
        // The duration is written for both kinds (E-42 fixed).
        out.history = history_rows(e, "an episode")
            .into_iter()
            .map(|(url, last_read, read_duration)| BackupAnimeHistory { url: url.to_owned(), last_read, read_duration })
            .collect();
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_invalid_writes_empty_object() {
        assert_eq!(memo_bytes(&parse_memo("not json")), b"{}");
        assert_eq!(memo_bytes(&parse_memo("[1,2]")), b"{}");
        assert_eq!(memo_bytes(&Map::new()), [0x7B, 0x7D]);
        let parsed = parse_memo("{\"a\":1}");
        assert_eq!(parsed.get("a"), Some(&Value::from(1)));
    }

    #[test]
    fn overflow_names_the_row() {
        let err = i32_of("manga", 42, "status", 1i64 << 40).unwrap_err();
        assert!(err.to_string().contains("manga 42"));
        assert_eq!(i32_of("manga", 1, "status", -5).unwrap(), -5);
    }

    #[test]
    fn category_orders_skip_system_category() {
        use kuro_data::category::Category;
        let cats = vec![
            Category { id: 0, name: "Default".into(), order: 0, flags: 0, hidden: false },
            Category { id: 5, name: "Read".into(), order: 2, flags: 0, hidden: false },
        ];
        assert_eq!(category_orders(&cats), vec![2]);
    }
}
