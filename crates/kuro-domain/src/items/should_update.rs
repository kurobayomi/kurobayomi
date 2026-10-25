//! Decides whether a stored chapter or episode must be rewritten after a source refresh
//! (`ShouldUpdateDbChapter.kt`, `ShouldUpdateDbEpisode.kt`).
//!
//! Deviation D4 / quirk Q1: the upload date is ignored when the source reports 0, so a source that never
//! dates its items no longer rewrites every row on each refresh.

use kuro_data::item::Item;

/// One item of either kind: scanlator, name, date upload (skipped when the source date is 0), number and source order,
/// plus the anime fields summary, fillermark, preview url and memo. A manga item has none of those (they are equal
/// `None`/false on both sides), so the one rule serves both kinds.
pub fn should_update_db_item(db: &Item, src: &Item) -> bool {
    db.scanlator != src.scanlator
        || db.name != src.name
        || (src.date_upload != 0 && db.date_upload != src.date_upload)
        || db.number != src.number
        || db.source_order != src.source_order
        || db.summary != src.summary
        || db.fillermark != src.fillermark
        || db.preview_url != src.preview_url
        || db.memo != src.memo
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;

    use super::*;

    fn chapter() -> Item {
        Item {
            scanlator: Some("Team".into()),
            name: "Ch 1".into(),
            date_upload: 1_700_000_000_000,
            number: 1.0,
            source_order: 2,
            ..Item::create(Kind::Manga)
        }
    }

    fn episode() -> Item {
        Item {
            scanlator: Some("Sub".into()),
            name: "Ep 1".into(),
            date_upload: 1_700_000_000_000,
            number: 1.0,
            source_order: 2,
            summary: Some("s".into()),
            fillermark: false,
            preview_url: Some("p".into()),
            ..Item::create(Kind::Anime)
        }
    }

    #[test]
    fn identical_items_do_not_update() {
        assert!(!should_update_db_item(&chapter(), &chapter()));
        assert!(!should_update_db_item(&episode(), &episode()));
    }

    #[test]
    fn each_chapter_field_flips_the_result() {
        let base = chapter();
        let flips: [(&str, Item); 5] = [
            ("scanlator", Item { scanlator: None, ..chapter() }),
            ("name", Item { name: "other".into(), ..chapter() }),
            ("date upload", Item { date_upload: 1, ..chapter() }),
            ("number", Item { number: 1.5, ..chapter() }),
            ("source order", Item { source_order: 9, ..chapter() }),
        ];
        for (field, changed) in flips {
            assert!(should_update_db_item(&base, &changed), "{field}");
        }
    }

    #[test]
    fn quirk_q1_zero_source_date_is_ignored() {
        assert!(!should_update_db_item(&chapter(), &Item { date_upload: 0, ..chapter() }));
        assert!(!should_update_db_item(&episode(), &Item { date_upload: 0, ..episode() }));
    }

    #[test]
    fn each_episode_field_flips_the_result() {
        let base = episode();
        let mut memo = serde_json::Map::new();
        memo.insert("k".into(), serde_json::Value::from(1));
        let flips: [(&str, Item); 9] = [
            ("scanlator", Item { scanlator: None, ..episode() }),
            ("name", Item { name: "x".into(), ..episode() }),
            ("date upload", Item { date_upload: 5, ..episode() }),
            ("number", Item { number: 2.0, ..episode() }),
            ("source order", Item { source_order: 0, ..episode() }),
            ("summary", Item { summary: None, ..episode() }),
            ("fillermark", Item { fillermark: true, ..episode() }),
            ("memo", Item { memo: Some(kuro_db::codec::Memo(memo)), ..episode() }),
            ("preview url", Item { preview_url: None, ..episode() }),
        ];
        for (field, changed) in flips {
            assert!(should_update_db_item(&base, &changed), "{field}");
        }
    }

    #[test]
    fn memo_key_order_does_not_matter() {
        let mut a = serde_json::Map::new();
        a.insert("x".into(), serde_json::Value::from(1));
        a.insert("y".into(), serde_json::Value::from(2));
        let mut b = serde_json::Map::new();
        b.insert("y".into(), serde_json::Value::from(2));
        b.insert("x".into(), serde_json::Value::from(1));
        let db = Item { memo: Some(kuro_db::codec::Memo(a)), ..episode() };
        let src = Item { memo: Some(kuro_db::codec::Memo(b)), ..episode() };
        assert!(!should_update_db_item(&db, &src));
    }
}
