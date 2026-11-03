//! Fixtures Q-01 to Q-19 from `fixtures_gen`: decode outcomes, counts and the restore rules each one exercises
//! (area 22, T-22a-042; T-22a-003).

mod fixtures_gen;

use kuro_backup::format::{decode_file, is_legacy, read_backup_bytes};
use kuro_backup::restore::extensions::legacy_repo_urls;
use kuro_backup::schema::BackupExtension;
use serde_json::Value;

fn count(v: &Value, key: &str) -> usize {
    v[key].as_u64().unwrap_or_else(|| panic!("expected.json has no {key}")) as usize
}

fn generated() -> (tempfile::TempDir, Vec<fixtures_gen::Fixture>) {
    let dir = tempfile::tempdir().unwrap();
    fixtures_gen::write(dir.path());
    (dir, fixtures_gen::all())
}

fn expected_of(dir: &std::path::Path, f: &fixtures_gen::Fixture) -> Value {
    let name = format!("{}.expected.json", f.file_name.trim_end_matches(".tachibk"));
    serde_json::from_slice(&std::fs::read(dir.join(name)).unwrap()).unwrap()
}

#[test]
fn every_fixture_has_expected_json_and_matches_its_decode_outcome() {
    let (dir, fixtures) = generated();
    assert_eq!(fixtures.len(), 19);
    for f in &fixtures {
        let expected = expected_of(dir.path(), f);
        let decoded = decode_file(&dir.path().join(f.file_name));
        match expected["decode"].as_str() {
            Some("error") => assert!(decoded.is_err(), "{} must be rejected", f.file_name),
            Some("ok") => assert!(decoded.is_ok(), "{} must decode: {:?}", f.file_name, decoded.err()),
            other => panic!("{} has no decode expectation: {other:?}", f.file_name),
        }
    }
}

#[test]
fn json_fixture_is_rejected_with_the_json_message() {
    let (dir, _) = generated();
    let err = decode_file(&dir.path().join("json_brace.json")).unwrap_err();
    assert!(err.to_string().contains("JSON backup not supported"), "{err}");
}

#[test]
fn detection_matches_expected_json_for_legacy_fixtures() {
    let (dir, fixtures) = generated();
    for f in fixtures.iter().filter(|f| f.expected.get("legacy").is_some()) {
        let payload = read_backup_bytes(&dir.path().join(f.file_name)).unwrap();
        assert_eq!(is_legacy(&payload), f.expected["legacy"].as_bool().unwrap(), "{}", f.file_name);
    }
}

#[test]
fn q01_mihon_file_with_preferences_decodes_with_counts() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("mihon_1x_small.tachibk")).unwrap();
    assert_eq!((b.manga.len(), b.categories.len(), b.sources.len(), b.preferences.len()), (3, 1, 1, 2));
    assert_eq!(b.manga.iter().map(|m| m.chapters.len()).sum::<usize>(), 6);
    assert_eq!(b.manga.iter().map(|m| m.history.len()).sum::<usize>(), 1);
    assert_eq!(b.manga.iter().map(|m| m.tracking.len()).sum::<usize>(), 1);
    assert!(b.anime.is_empty() && b.anime_sources.is_empty());
}

#[test]
fn q02_legacy_file_converts_to_current_lists() {
    let (dir, fixtures) = generated();
    let expected = &fixtures[1].expected;
    let b = decode_file(&dir.path().join("aniyomi_legacy_0x.tachibk")).unwrap();
    assert!(!b.is_legacy);
    assert_eq!(b.anime.len(), count(expected, "anime"));
    assert_eq!(b.manga.len(), count(expected, "manga"));
    assert_eq!(b.anime_categories.len(), count(expected, "anime_categories"));
    assert_eq!(b.anime_sources.len(), count(expected, "anime_sources"));
    assert!(b.anime.iter().all(|a| a.parent_id.is_none()), "no season rows from a legacy file");
}

#[test]
fn q03_and_q04_current_file_counts_match_raw_and_gzipped() {
    let (dir, fixtures) = generated();
    let expected = &fixtures[2].expected;
    for name in ["aniyomi_current.tachibk", "aniyomi_current_raw.tachibk"] {
        let b = decode_file(&dir.path().join(name)).unwrap();
        assert_eq!(b.anime.len(), count(expected, "anime"), "{name}");
        assert_eq!(b.anime.iter().filter(|a| a.parent_id.is_some()).count(), count(expected, "seasons"), "{name}");
        assert_eq!(b.manga.len(), count(expected, "manga"), "{name}");
        assert_eq!(b.anime_extension_stores.len(), count(expected, "extension_stores"), "{name}");
        assert_eq!(b.custom_buttons.len(), count(expected, "custom_buttons"), "{name}");
        assert_eq!(b.custom_buttons.iter().filter(|c| c.is_favorite).count(), count(expected, "favourite_buttons"), "{name}");
    }
}

#[test]
fn q10_duplicate_category_names_decode_to_both_orders() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("dup_categories.tachibk")).unwrap();
    assert_eq!(b.categories.len(), 2);
    assert_eq!(b.categories.iter().filter(|c| c.name == "Reading").count(), 2);
    assert_eq!(b.manga[0].categories, vec![1, 2]);
}

#[test]
fn q11_q12_seasons_keep_their_parent_in_either_order() {
    let (dir, _) = generated();
    for name in ["seasons_parent_first.tachibk", "seasons_season_first.tachibk"] {
        let b = decode_file(&dir.path().join(name)).unwrap();
        assert_eq!(b.anime.len(), 2, "{name}");
        let season = b.anime.iter().find(|a| a.parent_id.is_some()).unwrap();
        assert_eq!(season.parent_id, Some(1), "{name}");
    }
}

#[test]
fn q13_chapter_numbers_and_scores_keep_their_values() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("float_numbers.tachibk")).unwrap();
    let numbers: Vec<f32> = b.manga[0].chapters.iter().map(|c| c.chapter_number).collect();
    assert_eq!(numbers, vec![1.1, 2.5, 10.0]);
    let scores: Vec<f32> = b.manga[0].tracking.iter().map(|t| t.score).collect();
    assert_eq!(scores, vec![7.3, 8.5]);
}

#[test]
fn q14_preferences_keep_every_key_for_the_restore_rules() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("prefs_mixed.tachibk")).unwrap();
    assert_eq!(b.preferences.len(), 4);
    assert!(b.preferences.iter().any(|p| p.key == "__APP_STATE_last_auto_backup_timestamp"));
}

#[test]
fn q15_repo_rows_become_legacy_urls_and_are_not_written_to_a_table() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("repos_dup.tachibk")).unwrap();
    let urls = legacy_repo_urls(&b);
    assert!(!urls.is_empty());
    for r in &b.manga_extension_repos {
        assert!(urls.contains(&r.base_url), "{}", r.base_url);
    }
}

#[test]
fn q17_history_stays_on_the_manga_it_belongs_to() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("history_collision.tachibk")).unwrap();
    assert_eq!(b.manga.len(), 2);
    assert!(b.manga[0].history.is_empty());
    assert_eq!(b.manga[1].history.len(), 1);
    assert_eq!(b.manga[1].history[0].url, "/c/1");
    assert_eq!(b.manga[0].chapters[0].url, b.manga[1].chapters[0].url);
}

#[test]
fn q18_missing_favorite_field_defaults_to_true() {
    let (dir, _) = generated();
    let b = decode_file(&dir.path().join("no_favorite_field.tachibk")).unwrap();
    assert_eq!(b.manga.len(), 1);
    assert!(b.manga[0].favorite);
}

#[test]
fn q19_apk_payloads_decode_and_are_not_written() {
    let (dir, fixtures) = generated();
    let b = decode_file(&dir.path().join("apk_ext.tachibk")).unwrap();
    let expected = &fixtures[18].expected;
    assert_eq!(b.extensions.len(), count(expected, "extensions"));
    assert_eq!(b.extensions[0], BackupExtension { pkg_name: "eu.example.one".into(), apk: vec![1, 2, 3] });
}
