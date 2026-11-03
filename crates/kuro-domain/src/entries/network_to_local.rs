//! `NetworkToLocalManga` and `NetworkToLocalAnime`: turn an entry from a source listing
//! into the stored row. The lookup and the insert run in one writer transaction (in `kuro-data`), so two
//! callers with the same `(url, source)` leave one row. The title of a non-favourite is replaced in memory
//! only; the database keeps its title until a later update writes it. The cover address is written: a stored
//! non-favourite takes the listing's cover when it differs (a source that once listed a wrong picture, such as an
//! episode screenshot, is corrected the next time it lists the entry), and any row trades the poster-on-demand marker
//! for a real address. A real address is never traded for the marker.

use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_data::{DataResult, LookupOrInsert};
use kuro_source::anime::COVER_MARKER;

fn is_marker(url: Option<&str>) -> bool {
    url.is_some_and(|url| url.starts_with(COVER_MARKER))
}

/// The cover `stored` takes after a listing showed `listed`, or `None` to keep it.
fn newer_cover<'a>(stored: &Entry, listed: Option<&'a str>) -> Option<&'a str> {
    let listed = listed.filter(|url| !url.is_empty())?;
    let current = stored.thumbnail_url.as_deref().filter(|url| !url.is_empty());
    if current == Some(listed) || (is_marker(Some(listed)) && current.is_some_and(|url| !is_marker(Some(url)))) {
        return None;
    }
    (!stored.favorite || is_marker(current)).then_some(listed)
}

/// The entry a lookup-or-insert stands for: the new row with its id, the stored row, or, for a stored non-favourite, the
/// stored row with the listing's title (in memory only).
fn resolve(new: &Entry, found: LookupOrInsert<Entry>) -> Entry {
    match found {
        LookupOrInsert::Inserted(id) => Entry { id, ..new.clone() },
        LookupOrInsert::Found(local) if !local.favorite => Entry { title: new.title.clone(), ..local },
        LookupOrInsert::Found(local) => local,
    }
}

/// `NetworkToLocalManga.await` and `NetworkToLocalAnime.await`: the one path.
pub fn network_to_local(repo: &EntryRepo, e: &Entry) -> DataResult<Entry> {
    let mut local = resolve(e, repo.lookup_or_insert(e)?);
    if let Some(cover) = newer_cover(&local, e.thumbnail_url.as_deref()) {
        repo.update(&EntryUpdate { thumbnail_url: Some(cover.to_owned()), ..EntryUpdate::new(local.id) })?;
        local.thumbnail_url = Some(cover.to_owned());
    }
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdb::entry_repo;
    use kuro_data::entry::EntryUpdate;
    use kuro_platform::kind::Kind;
    use std::sync::Arc;
    use std::thread;

    fn entry(kind: Kind, title: &str, favorite: bool) -> Entry {
        Entry { url: "/series/9".into(), title: title.into(), favorite, source: 5, ..Entry::create(kind) }
    }

    #[test]
    fn absent_row_is_inserted_and_the_new_id_is_returned() {
        let (_dir, repo) = entry_repo();
        for kind in Kind::ALL {
            let out = network_to_local(&repo, &entry(kind, "Fresh", false)).expect("ok");
            assert!(out.id > 0);
            let stored = repo.by_url_and_source(kind, "/series/9", 5).expect("ok").expect("row");
            assert_eq!((stored.id, stored.title.as_str()), (out.id, "Fresh"));
        }
    }

    #[test]
    fn existing_non_favourite_returns_source_title_in_memory_only() {
        let (_dir, repo) = entry_repo();
        for kind in Kind::ALL {
            let id = repo.insert(&entry(kind, "Stored", false)).expect("insert");
            let out = network_to_local(&repo, &entry(kind, "From source", false)).expect("ok");
            assert_eq!((out.id, out.title.as_str()), (id, "From source"));
            assert_eq!(repo.by_id(id).expect("row").title, "Stored", "database title is unchanged");
        }
    }

    #[test]
    fn existing_favourite_returns_the_stored_row_unchanged() {
        let (_dir, repo) = entry_repo();
        for kind in Kind::ALL {
            let id = repo.insert(&entry(kind, "My edit", true)).expect("insert");
            let out = network_to_local(&repo, &entry(kind, "From source", false)).expect("ok");
            assert_eq!(out.title, "My edit");
            assert_eq!(out, repo.by_id(id).expect("row"));
        }
    }

    #[test]
    fn a_stored_non_favourite_that_becomes_a_favourite_keeps_its_stored_title() {
        let (_dir, repo) = entry_repo();
        let fresh = network_to_local(&repo, &entry(Kind::Anime, "Fresh", false)).expect("ok");
        repo.update(&EntryUpdate { favorite: Some(true), ..EntryUpdate::new(fresh.id) }).expect("update");
        let kept = network_to_local(&repo, &entry(Kind::Anime, "Source again", false)).expect("ok");
        assert_eq!(kept.title, "Fresh", "favourites keep the stored title");
    }

    #[test]
    fn a_listing_corrects_a_stored_cover_and_never_trades_a_real_one_for_the_marker() {
        let (_dir, repo) = entry_repo();
        let listed = |cover: &str, favorite: bool| Entry { thumbnail_url: Some(cover.into()), ..entry(Kind::Anime, "T", favorite) };
        let id = repo.insert(&listed("https://i/snapshots/ep.jpg", false)).expect("insert");
        let stored = |repo: &EntryRepo| repo.by_id(id).expect("row").thumbnail_url.unwrap();

        assert_eq!(network_to_local(&repo, &listed("kb-cover:/a/1", false)).expect("ok").thumbnail_url.as_deref(), Some("https://i/snapshots/ep.jpg"));
        assert_eq!(stored(&repo), "https://i/snapshots/ep.jpg", "a real address stays when the listing only has the marker");
        network_to_local(&repo, &listed("https://i/posters/p.jpg", false)).expect("ok");
        assert_eq!(stored(&repo), "https://i/posters/p.jpg", "the listing's poster replaces the wrong picture");

        repo.update(&EntryUpdate { favorite: Some(true), thumbnail_url: Some("kb-cover:/a/1".into()), ..EntryUpdate::new(id) }).expect("update");
        network_to_local(&repo, &listed("https://i/posters/p.jpg", false)).expect("ok");
        assert_eq!(stored(&repo), "https://i/posters/p.jpg", "a favourite trades the marker for the real poster");
        network_to_local(&repo, &listed("https://i/posters/other.jpg", false)).expect("ok");
        assert_eq!(stored(&repo), "https://i/posters/p.jpg", "a favourite's real cover is the details refresh's to change");
    }

    #[test]
    fn sixteen_concurrent_calls_leave_one_row_and_one_id() {
        let (_dir, repo) = entry_repo();
        let repo = Arc::new(repo);
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let repo = Arc::clone(&repo);
                thread::spawn(move || network_to_local(&repo, &entry(Kind::Manga, "Race", false)).expect("ok").id)
            })
            .collect();
        let ids: Vec<i64> = handles.into_iter().map(|h| h.join().expect("thread")).collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]), "all callers see one id: {ids:?}");
        let rows = repo.all_source_and_url(Kind::Manga).expect("ok").into_iter().filter(|(_, url)| url == "/series/9").count();
        assert_eq!(rows, 1);
    }
}
