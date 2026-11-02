//! The per-season episode fetch of a season refresh (T-39b-022). Upstream: `AnimeScreenModel.fetchEpisodesFromSeasons`
//! (AnimeScreenModel.kt:670-702). Only an `Episodes` child that has never been updated or is not completed is fetched. A
//! source on the user's own machine or network (`UnmeteredSource`) fetches its seasons together, at most `MAX_PARALLEL`
//! at once; any other source fetches them one after the other. The page cancels a refresh by dropping the fetch, which
//! ends a one-by-one run before the next season and aborts the ones still running in a parallel run.

use std::future::Future;
use std::sync::Arc;

use kuro_data::entry::Entry;
use kuro_db::codec::FetchType;
use kuro_domain::entries::flags::STATUS_COMPLETED;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// The most seasons an unmetered source fetches at once.
pub const MAX_PARALLEL: usize = 8;

/// Whether a season's episodes are fetched by a season refresh.
pub fn needs_episodes(season: &Entry) -> bool {
    season.fetch_type() == FetchType::Episodes && (season.last_update == 0 || season.status != STATUS_COMPLETED)
}

/// Runs `fetch` for each season that `needs_episodes`. `fetch` reports its own failures, so one failing season does not
/// stop the others.
pub async fn fetch_episodes_of<F, Fut>(seasons: Vec<Entry>, unmetered: bool, fetch: F)
where
    F: Fn(Entry) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let seasons = seasons.into_iter().filter(needs_episodes);
    if !unmetered {
        for season in seasons {
            fetch(season).await;
        }
        return;
    }
    let gate = Arc::new(Semaphore::new(MAX_PARALLEL));
    let mut running = JoinSet::new();
    for season in seasons {
        let (gate, work) = (Arc::clone(&gate), fetch(season));
        running.spawn(async move {
            // the gate never closes, so a permit is always given
            let _permit = gate.acquire_owned().await;
            work.await;
        });
    }
    while running.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use kuro_platform::kind::Kind;

    use super::*;

    fn season(id: i64, fetch_type: FetchType, last_update: i64, status: i64) -> Entry {
        Entry { id, fetch_type: Some(fetch_type), last_update, status, ..Entry::create(Kind::Anime) }
    }

    #[test]
    fn only_unfinished_or_never_updated_episode_children_are_fetched() {
        let ongoing = STATUS_COMPLETED - 1;
        assert!(needs_episodes(&season(1, FetchType::Episodes, 5, ongoing)));
        assert!(needs_episodes(&season(2, FetchType::Episodes, 0, STATUS_COMPLETED)), "never updated");
        assert!(!needs_episodes(&season(3, FetchType::Episodes, 5, STATUS_COMPLETED)), "completed and updated");
        assert!(!needs_episodes(&season(4, FetchType::Seasons, 0, ongoing)), "a seasons child has no episodes");
    }

    #[tokio::test]
    async fn a_metered_source_fetches_one_season_at_a_time_in_order() {
        let (now, peak, order) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
        let seasons: Vec<Entry> = (1..=4).map(|id| season(id, FetchType::Episodes, 0, 0)).collect();
        fetch_episodes_of(seasons, false, |s| {
            let (now, peak, order) = (Arc::clone(&now), Arc::clone(&peak), Arc::clone(&order));
            async move {
                peak.fetch_max(now.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                order.lock().unwrap().push(s.id);
                now.fetch_sub(1, Ordering::SeqCst);
            }
        })
        .await;
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        assert_eq!(*order.lock().unwrap(), [1, 2, 3, 4]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_unmetered_source_fetches_in_parallel_but_never_more_than_eight_at_once() {
        let (now, peak, done) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let seasons: Vec<Entry> = (1..=20).map(|id| season(id, FetchType::Episodes, 0, 0)).collect();
        fetch_episodes_of(seasons, true, |_| {
            let (now, peak, done) = (Arc::clone(&now), Arc::clone(&peak), Arc::clone(&done));
            async move {
                peak.fetch_max(now.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                now.fetch_sub(1, Ordering::SeqCst);
                done.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await;
        assert_eq!(done.load(Ordering::SeqCst), 20);
        assert_eq!(peak.load(Ordering::SeqCst), MAX_PARALLEL);
    }

    // the clock is paused, so the 70 ms deadline falls between the second and the third 50 ms season on any machine
    #[tokio::test(start_paused = true)]
    async fn dropping_a_metered_run_stops_it_before_the_next_season() {
        let started = Arc::new(AtomicUsize::new(0));
        let seasons: Vec<Entry> = (1..=5).map(|id| season(id, FetchType::Episodes, 0, 0)).collect();
        let run = fetch_episodes_of(seasons, false, |_| {
            let started = Arc::clone(&started);
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        assert!(tokio::time::timeout(Duration::from_millis(70), run).await.is_err(), "the run was cut short");
        assert_eq!(started.load(Ordering::SeqCst), 2, "the third season never began");
    }

    #[tokio::test]
    async fn seasons_that_do_not_need_episodes_are_not_fetched() {
        let started = Arc::new(AtomicUsize::new(0));
        let seasons = vec![season(1, FetchType::Episodes, 9, STATUS_COMPLETED), season(2, FetchType::Seasons, 0, 0)];
        for unmetered in [false, true] {
            let started = Arc::clone(&started);
            fetch_episodes_of(seasons.clone(), unmetered, move |_| {
                let started = Arc::clone(&started);
                async move {
                    started.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await;
        }
        assert_eq!(started.load(Ordering::SeqCst), 0);
    }
}
