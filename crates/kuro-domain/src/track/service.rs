use std::collections::BTreeMap;

use kuro_data::track::Track;
use kuro_platform::kind::Kind;

use crate::error::DomainError;
use crate::source::model::Id;
use crate::track::repo::TrackPort;

/// Returns the value, or logs the error once and returns `T::default()`.
pub fn log_and_default<T: Default>(result: Result<T, DomainError>, what: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(e) => {
            tracing::error!(error = %e, "{what}");
            T::default()
        }
    }
}

/// Reads that return the error instead of swallowing it (new code; the `by_*` functions are the parity wrappers).
pub fn try_by_row_id(repo: &dyn TrackPort, row_id: Id) -> Result<Option<Track>, DomainError> {
    repo.by_row_id(row_id)
}

/// Reads that return the error instead of swallowing it, for the tracks of one entry.
pub fn try_by_entry(repo: &dyn TrackPort, entry_id: Id) -> Result<Vec<Track>, DomainError> {
    repo.by_entry(entry_id)
}

/// `GetMangaTracks.awaitOne(id)`: the binding with row id `row_id`, or `None` on error (Q-TRK-1).
pub fn by_row_id(repo: &dyn TrackPort, row_id: Id) -> Option<Track> {
    log_and_default(repo.by_row_id(row_id), "cannot read track by row id")
}

/// `GetMangaTracks.await(mangaId)`: the bindings of one entry, or an empty list on error.
pub fn by_entry(repo: &dyn TrackPort, entry_id: Id) -> Vec<Track> {
    log_and_default(repo.by_entry(entry_id), "cannot read tracks of an entry")
}

/// `InsertMangaTrack.await(track)`: the error is logged and not returned.
pub fn insert(repo: &dyn TrackPort, track: &Track) {
    log_and_default(repo.insert(track), "cannot insert track");
}

/// `InsertMangaTrack.awaitAll(tracks)`: the error is logged and not returned.
pub fn insert_all(repo: &dyn TrackPort, tracks: &[Track]) {
    log_and_default(repo.insert_all(tracks), "cannot insert tracks");
}

/// `DeleteMangaTrack.await(mangaId, trackerId)`: the error is logged and not returned.
pub fn delete(repo: &dyn TrackPort, entry_id: Id, tracker_id: Id) {
    log_and_default(repo.delete(entry_id, tracker_id), "cannot delete track");
}

/// Insert that returns the error to the caller.
pub fn try_insert(repo: &dyn TrackPort, track: &Track) -> Result<(), DomainError> {
    repo.insert(track)
}

/// Delete that returns the error to the caller.
pub fn try_delete(repo: &dyn TrackPort, entry_id: Id, tracker_id: Id) -> Result<(), DomainError> {
    repo.delete(entry_id, tracker_id)
}

/// `GetTracksPerManga` / `GetTracksPerAnime`: every track grouped by its entry. Entries without tracks are absent.
pub fn tracks_per_entry(repo: &dyn TrackPort, kind: Kind) -> BTreeMap<Id, Vec<Track>> {
    let mut grouped: BTreeMap<Id, Vec<Track>> = BTreeMap::new();
    for track in log_and_default(repo.all(kind), "cannot read tracks") {
        grouped.entry(track.entry_id).or_default().push(track);
    }
    grouped
}


#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    
    fn track(entry_id: Id, tracker: Id, title: &str) -> Track {
        Track {
            id: entry_id * 10 + tracker,
            entry_id,
            remote_id: 1,
            title: title.into(),
            ..Track::new(Kind::Manga, tracker)
        }
    }

    /// Keeps rows in memory; `broken` makes every call fail with a database error.
    #[derive(Default)]
    struct Fake {
        rows: Mutex<Vec<Track>>,
        broken: bool,
    }

    fn fail() -> DomainError {
        DomainError::Db(kuro_db::open::DbError::NotFound)
    }

    impl Fake {
        fn check(&self) -> Result<(), DomainError> {
            if self.broken { Err(fail()) } else { Ok(()) }
        }
    }

    impl TrackPort for Fake {
        fn by_row_id(&self, row_id: Id) -> Result<Option<Track>, DomainError> {
            self.check()?;
            Ok(self.rows.lock().unwrap().iter().find(|t| t.id == row_id).cloned())
        }
        fn by_entry(&self, entry_id: Id) -> Result<Vec<Track>, DomainError> {
            self.check()?;
            Ok(self.rows.lock().unwrap().iter().filter(|t| t.entry_id == entry_id).cloned().collect())
        }
        fn all(&self, kind: Kind) -> Result<Vec<Track>, DomainError> {
            self.check()?;
            Ok(self.rows.lock().unwrap().iter().filter(|t| t.kind == kind).cloned().collect())
        }
        fn insert(&self, t: &Track) -> Result<(), DomainError> {
            self.check()?;
            self.rows.lock().unwrap().push(t.clone());
            Ok(())
        }
        fn insert_all(&self, ts: &[Track]) -> Result<(), DomainError> {
            for t in ts {
                self.insert(t)?;
            }
            Ok(())
        }
        fn delete(&self, entry_id: Id, tracker_id: Id) -> Result<(), DomainError> {
            self.check()?;
            self.rows.lock().unwrap().retain(|t| !(t.entry_id == entry_id && t.tracker_id == tracker_id));
            Ok(())
        }
    }

    /// Captures what the subscriber logs, so a test can count the error lines.
    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
        type Writer = Logs;
        fn make_writer(&'a self) -> Logs {
            self.clone()
        }
    }

    fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::ERROR)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, || {
            // parallel tests can cache a log statement as unwatched before this thread's subscriber arrives: rebuild the cache
            tracing::callsite::rebuild_interest_cache();
            f()
        });
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (value, text)
    }

    #[test]
    fn a_failing_read_gives_none_or_empty_and_logs_once() {
        let repo = Fake { broken: true, ..Default::default() };
        let (value, text) = logged(|| (by_row_id(&repo, 1), by_entry(&repo, 1)));
        assert_eq!(value, (None, Vec::new()), "TC-TRK-05");
        assert_eq!(text.matches("cannot read track by row id").count(), 1);
        assert_eq!(text.matches("cannot read tracks of an entry").count(), 1);
    }

    #[test]
    fn the_try_reads_return_the_error_the_parity_reads_swallow() {
        let repo = Fake { broken: true, ..Default::default() };
        assert!(try_by_row_id(&repo, 1).is_err());
        assert!(try_by_entry(&repo, 1).is_err());
        let ok = Fake::default();
        assert_eq!(try_by_entry(&ok, 1).expect("read"), Vec::<Track>::new());
    }

    #[test]
    fn insert_and_delete_swallow_errors_while_try_variants_return_them() {
        let repo = Fake { broken: true, ..Default::default() };
        let (_, text) = logged(|| {
            insert(&repo, &track(1, 2, "x"));
            delete(&repo, 1, 2);
        });
        assert!(text.contains("cannot insert track") && text.contains("cannot delete track"));
        assert!(try_insert(&repo, &track(1, 2, "x")).is_err(), "E-TRK-03");
        assert!(try_delete(&repo, 1, 2).is_err());
    }

    #[test]
    fn delete_of_an_absent_pair_is_not_an_error() {
        let repo = Fake::default();
        assert!(try_delete(&repo, 9, 9).is_ok(), "TC-TRK-03");
    }

    #[test]
    fn tracks_per_entry_groups_by_entry_and_leaves_out_entries_without_tracks() {
        let repo = Fake::default();
        insert_all(&repo, &[track(1, 2, "a"), track(1, 3, "b"), track(4, 2, "c")]);
        let grouped = tracks_per_entry(&repo, Kind::Manga);
        assert_eq!(grouped.keys().copied().collect::<Vec<_>>(), vec![1, 4], "TC-TRK-07");
        assert_eq!(grouped[&1].len(), 2);
        assert!(!grouped.contains_key(&2));
    }

    #[test]
    fn by_row_id_takes_the_row_id_not_the_entry_id() {
        let repo = Fake::default();
        insert(&repo, &track(7, 2, "only"));
        assert_eq!(by_row_id(&repo, 72).map(|t| t.title), Some("only".to_owned()));
        assert!(by_row_id(&repo, 7).is_none(), "Q-TRK-1: 7 is the entry id");
    }

    #[test]
    fn the_grouping_is_scoped_to_one_kind() {
        let repo = Fake::default();
        let anime = Track { kind: Kind::Anime, ..track(5, 2, "a") };
        insert_all(&repo, &[track(1, 2, "m"), anime]);
        assert_eq!(tracks_per_entry(&repo, Kind::Anime).keys().copied().collect::<Vec<_>>(), vec![5]);
        assert_eq!(tracks_per_entry(&repo, Kind::Manga).keys().copied().collect::<Vec<_>>(), vec![1]);
    }
}
