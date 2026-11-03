//! Generator for the restore fixtures Q-01 to Q-03 (area 22). Pure Rust, no
//! Kotlin input. `write` puts each fixture and its `expected.json` into a folder.

use std::io::Write;
use std::path::Path;

use kuro_backup::schema::{
    Backup, BackupAnime, BackupAnimeSource, BackupCategory, BackupChapter, BackupCustomButtons, BackupExtension,
    BackupExtensionRepos, BackupExtensionStore, BackupHistory, BackupManga, BackupPreference, BackupSource, BackupTracking,
    LegacyBackup, PrefValue,
};
use kuro_backup::wire::{Message, Writer};
use serde_json::{Value, json};

pub struct Fixture {
    pub file_name: &'static str,
    pub bytes: Vec<u8>,
    pub expected: Value,
}

fn gzip(payload: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(payload).unwrap();
    enc.finish().unwrap()
}

fn manga(n: i64, chapters: bool) -> BackupManga {
    let chapters = if chapters {
        (0..2)
            .map(|i| BackupChapter { url: format!("/m/{n}/c/{i}"), name: format!("Chapter {i}"), chapter_number: i as f32 + 1.0, ..Default::default() })
            .collect()
    } else {
        Vec::new()
    };
    BackupManga { source: 7, url: format!("/m/{n}"), title: format!("Manga {n}"), chapters, ..Default::default() }
}

/// Q-01 `mihon_1x_small.tachibk`: Mihon-style (fields 1, 2, 101, 104; no 5xx), gzip.
pub fn q01() -> Fixture {
    let mut mangas: Vec<BackupManga> = (1..=3).map(|n| manga(n, true)).collect();
    mangas[0].history = vec![BackupHistory { url: "/m/1/c/0".into(), last_read: 1_700_000_000_000, read_duration: 0 }];
    mangas[0].tracking = vec![BackupTracking { sync_id: 1, library_id: 5, title: "Manga 1".into(), media_id: 99, ..Default::default() }];
    mangas[0].categories = vec![1];
    let mut w = Writer::new();
    for m in &mangas {
        w.message(1, m).unwrap();
    }
    w.message(2, &BackupCategory { name: "Reading".into(), order: 1, id: 4, flags: 0 }).unwrap();
    w.message(101, &BackupSource { name: "Mock".into(), source_id: 7 }).unwrap();
    w.message(104, &BackupPreference { key: "pref_theme_mode_key".into(), value: PrefValue::String("SYSTEM".into()) }).unwrap();
    w.message(104, &BackupPreference { key: "library_unseen_updates_count".into(), value: PrefValue::Int(4) }).unwrap();
    Fixture {
        file_name: "mihon_1x_small.tachibk",
        bytes: gzip(&w.finish()),
        expected: json!({ "decode": "ok", "legacy": false, "manga": 3, "categories": 1, "chapters": 6, "history": 1, "tracks": 1, "sources": 1, "preferences": 2 }),
    }
}

/// Q-02 `aniyomi_legacy_0x.tachibk`: 0.x legacy file, fields 1 to 4 and 103, gzip.
pub fn q02() -> Fixture {
    let legacy = LegacyBackup {
        backup_manga: vec![manga(1, false)],
        backup_anime: vec![
            BackupAnime { source: 8, url: "/a/1".into(), title: "Anime 1".into(), ..Default::default() },
            BackupAnime { source: 8, url: "/a/2".into(), title: "Anime 2".into(), ..Default::default() },
        ],
        backup_anime_categories: vec![BackupCategory { name: "Watching".into(), order: 1, id: 2, flags: 0 }],
        backup_anime_sources: vec![BackupAnimeSource { name: "AMock".into(), source_id: 8 }],
        ..Default::default()
    };
    Fixture {
        file_name: "aniyomi_legacy_0x.tachibk",
        bytes: gzip(&legacy.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "legacy": true, "anime": 2, "manga": 1, "anime_categories": 1, "anime_sources": 1 }),
    }
}

/// Q-03 `aniyomi_current.tachibk`: current Aniyomi format (5xx), gzip. Q-04 is the same payload, not gzipped.
pub fn q03() -> Fixture {
    let show = |id: i64| BackupAnime { source: 8, url: format!("/a/{id}"), title: format!("Show {id}"), id: Some(id), ..Default::default() };
    let season = |id: i64, parent: i64, number: f64| BackupAnime {
        parent_id: Some(parent),
        season_number: number,
        ..BackupAnime { url: format!("/a/{id}"), title: format!("Season {number}"), ..show(id) }
    };
    let backup = Backup {
        manga: (1..=3).map(|n| manga(n, false)).collect(),
        sources: vec![BackupSource { name: "Mock".into(), source_id: 7 }],
        is_legacy: false,
        anime: vec![show(1), show(2), season(3, 1, 1.0), season(4, 2, 1.0)],
        anime_sources: vec![BackupAnimeSource { name: "AMock".into(), source_id: 8 }],
        anime_extension_stores: vec![BackupExtensionStore { index_url: "https://example.org/index.min.json".into(), name: "Store".into(), ..Default::default() }],
        custom_buttons: vec![
            BackupCustomButtons { name: "Skip".into(), is_favorite: true, sort_index: 0, content: "seek 85".into(), long_press_content: "".into(), on_startup: "".into() },
            BackupCustomButtons { name: "Next".into(), is_favorite: false, sort_index: 1, content: "playlist-next".into(), long_press_content: "".into(), on_startup: "".into() },
        ],
        ..Default::default()
    };
    Fixture {
        file_name: "aniyomi_current.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "legacy": false, "anime": 4, "seasons": 2, "manga": 3, "extension_stores": 1, "custom_buttons": 2, "favourite_buttons": 1 }),
    }
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(bytes), &mut out).unwrap();
    out
}

fn show(id: i64) -> BackupAnime {
    BackupAnime { source: 8, url: format!("/a/{id}"), title: format!("Show {id}"), id: Some(id), ..Default::default() }
}

fn season(id: i64, parent: i64) -> BackupAnime {
    BackupAnime { parent_id: Some(parent), season_number: 1.0, ..BackupAnime { url: format!("/a/{id}"), ..show(id) } }
}

/// Q-04 `aniyomi_current_raw.tachibk`: the Q-03 payload without gzip.
pub fn q04() -> Fixture {
    let q = q03();
    Fixture {
        file_name: "aniyomi_current_raw.tachibk",
        bytes: gunzip(&q.bytes),
        expected: q.expected,
    }
}

/// Q-05 `truncated.tachibk`: the first 100 bytes of Q-03.
pub fn q05() -> Fixture {
    let mut bytes = q03().bytes;
    bytes.truncate(100);
    Fixture { file_name: "truncated.tachibk", bytes, expected: json!({ "decode": "error" }) }
}

/// Q-06 `empty.tachibk`: zero bytes (E-01).
pub fn q06() -> Fixture {
    Fixture { file_name: "empty.tachibk", bytes: Vec::new(), expected: json!({ "decode": "error" }) }
}

/// Q-07 `one_byte.tachibk`: one byte `0x0a` (E-02).
pub fn q07() -> Fixture {
    Fixture { file_name: "one_byte.tachibk", bytes: vec![0x0a], expected: json!({ "decode": "error" }) }
}

/// Q-08 `text.txt`: plain text, no gzip, no JSON signature (E-04). Rejected as corrupt.
pub fn q08() -> Fixture {
    Fixture { file_name: "text.txt", bytes: b"hello world".to_vec(), expected: json!({ "decode": "error" }) }
}

/// Q-09 `json_brace.json`: a JSON object starts with `{"` (E-05). Rejected before parsing.
pub fn q09() -> Fixture {
    Fixture {
        file_name: "json_brace.json",
        bytes: br#"{"a":1}"#.to_vec(),
        expected: json!({ "decode": "error", "message": "JSON backup not supported" }),
    }
}

/// Q-10 `dup_categories.tachibk`: two categories named "Reading"; the manga refers to both. One row is kept (E-16).
pub fn q10() -> Fixture {
    let reading = |order: i64, id: i64| BackupCategory { name: "Reading".into(), order, id, flags: 0 };
    let mut m = manga(1, false);
    m.categories = vec![1, 2];
    let backup = Backup { manga: vec![m], categories: vec![reading(1, 4), reading(2, 5)], ..Default::default() };
    Fixture {
        file_name: "dup_categories.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "categories": 2, "category_rows_after_restore": 1 }),
    }
}

/// Q-11 `seasons_parent_first.tachibk`: the root comes before its season in the file (E-11 order rule).
pub fn q11() -> Fixture {
    let backup = Backup { anime: vec![show(1), season(2, 1)], ..Default::default() };
    Fixture {
        file_name: "seasons_parent_first.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "anime": 2, "seasons": 1 }),
    }
}

/// Q-12 `seasons_season_first.tachibk`: the season comes before its root in the file.
pub fn q12() -> Fixture {
    let backup = Backup { anime: vec![season(2, 1), show(1)], ..Default::default() };
    Fixture {
        file_name: "seasons_season_first.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "anime": 2, "seasons": 1 }),
    }
}

/// Q-13 `float_numbers.tachibk`: chapter numbers 1.1, 2.5, 10.0 and track scores 7.3, 8.5.
pub fn q13() -> Fixture {
    let mut m = manga(1, false);
    m.chapters = [1.1_f32, 2.5, 10.0]
        .iter()
        .enumerate()
        .map(|(i, n)| BackupChapter { url: format!("/c/{i}"), name: format!("Chapter {n}"), chapter_number: *n, ..Default::default() })
        .collect();
    m.tracking = [7.3_f32, 8.5]
        .iter()
        .enumerate()
        .map(|(i, score)| BackupTracking { sync_id: i as i32 + 1, score: *score, ..Default::default() })
        .collect();
    let backup = Backup { manga: vec![m], ..Default::default() };
    Fixture {
        file_name: "float_numbers.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "chapters": 3, "tracks": 2 }),
    }
}

/// Q-14 `prefs_mixed.tachibk`: an int, a string set, a type mismatch and an app-state key.
pub fn q14() -> Fixture {
    let pref = |key: &str, value: PrefValue| BackupPreference { key: key.into(), value };
    let backup = Backup {
        preferences: vec![
            pref("default_category", PrefValue::Int(3)),
            pref("hidden_categories", PrefValue::StringSet(["1".to_string(), "x".to_string()].into_iter().collect())),
            pref("pref_theme_mode_key", PrefValue::Int(1)),
            pref("__APP_STATE_last_auto_backup_timestamp", PrefValue::Long(1_700_000_000_000)),
        ],
        ..Default::default()
    };
    Fixture {
        file_name: "prefs_mixed.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "preferences": 4 }),
    }
}

/// Q-15 `repos_dup.tachibk`: manga repos A, B (same URL as A, other key) and C (other URL, A's key). Restored in
/// order: A is added, B and C fail, and a second A succeeds without a write (D11).
pub fn q15() -> Fixture {
    let repo = |url: &str, fp: &str| BackupExtensionRepos {
        base_url: url.into(),
        name: "Repo".into(),
        short_name: None,
        website: "w".into(),
        signing_key_fingerprint: fp.into(),
    };
    let backup = Backup {
        manga_extension_repos: vec![repo("https://a.example", "fp-a"), repo("https://a.example", "fp-b"), repo("https://c.example", "fp-a")],
        ..Default::default()
    };
    Fixture {
        file_name: "repos_dup.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "repos": 3, "errors_in_order": 2, "rows_after_restore": 1 }),
    }
}

/// Q-16 `buttons_two_fav.tachibk`: two custom buttons marked favourite. The database has none, so one wins (D10).
pub fn q16() -> Fixture {
    let button = |name: &str, sort_index: i64| BackupCustomButtons {
        name: name.into(),
        is_favorite: true,
        sort_index,
        content: format!("{name} content"),
        long_press_content: String::new(),
        on_startup: String::new(),
    };
    let backup = Backup { custom_buttons: vec![button("One", 0), button("Two", 1)], ..Default::default() };
    Fixture {
        file_name: "buttons_two_fav.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "custom_buttons": 2, "favourites_after_restore": 1 }),
    }
}

/// Q-17 `history_collision.tachibk`: manga A and B both have chapter `/c/1`, and only B has a history row (E-39).
pub fn q17() -> Fixture {
    let chapter = |n: i64| BackupChapter { url: "/c/1".into(), name: format!("Chapter {n}"), chapter_number: 1.0, ..Default::default() };
    let mut a = manga(1, false);
    a.url = "/a".into();
    a.chapters = vec![chapter(1)];
    let mut b = manga(2, false);
    b.url = "/b".into();
    b.chapters = vec![chapter(2)];
    b.history = vec![BackupHistory { url: "/c/1".into(), last_read: 1_700_000_000_000, read_duration: 5_000 }];
    let backup = Backup { manga: vec![a, b], ..Default::default() };
    Fixture {
        file_name: "history_collision.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "manga": 2, "history": 1, "history_on": "/b" }),
    }
}

/// Q-18 `no_favorite_field.tachibk`: a manga whose field 100 (favorite) is absent. The default is true (D22).
pub fn q18() -> Fixture {
    let mut inner = Writer::new();
    inner.int64(1, 7);
    inner.string(2, "/m/9");
    inner.string(3, "Nine");
    let mut outer = Writer::new();
    outer.bytes(1, &inner.finish());
    Fixture {
        file_name: "no_favorite_field.tachibk",
        bytes: gzip(&outer.finish()),
        expected: json!({ "decode": "ok", "manga": 1, "favorite": true }),
    }
}

/// Q-19 `apk_ext.tachibk`: two APK extension payloads. They are skipped on Linux and never count as errors.
pub fn q19() -> Fixture {
    let backup = Backup {
        extensions: vec![
            BackupExtension { pkg_name: "eu.example.one".into(), apk: vec![1, 2, 3] },
            BackupExtension { pkg_name: "eu.example.two".into(), apk: vec![4, 5] },
        ],
        ..Default::default()
    };
    Fixture {
        file_name: "apk_ext.tachibk",
        bytes: gzip(&backup.to_bytes().unwrap()),
        expected: json!({ "decode": "ok", "extensions": 2, "skipped": 2, "errors": 0 }),
    }
}

/// Q-20 (`large_10k.tachibk`) is a benchmark input and is not written here. Q-21 (`export_roundtrip.tachibk`) is
/// produced by the creator inside the end-to-end test, so both are absent from this list.
pub fn all() -> Vec<Fixture> {
    vec![
        q01(), q02(), q03(), q04(), q05(), q06(), q07(), q08(), q09(), q10(), q11(), q12(), q13(), q14(), q15(), q16(),
        q17(), q18(), q19(),
    ]
}

/// Writes every fixture and `<name>.expected.json` into `dir`.
pub fn write(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    for f in all() {
        std::fs::write(dir.join(f.file_name), &f.bytes).unwrap();
        std::fs::write(dir.join(format!("{}.expected.json", f.file_name.trim_end_matches(".tachibk"))), serde_json::to_vec_pretty(&f.expected).unwrap()).unwrap();
    }
}
