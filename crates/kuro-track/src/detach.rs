//! Threading of the write paths. The source runs the bind,
//! the progress writes and the remove as non-cancellable work, so closing the page never stops them. The GUI
//! spawns them here and drops the handle. Results reach the GTK thread through a channel, in the GUI crate.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::task::JoinHandle;

/// Runs `fut` on the runtime. Dropping the returned handle does not cancel the task, and nothing aborts it.
pub fn detached<F>(fut: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(fut)
}

/// [`detached`], with the binding locks taken in the submitting call (T-27b-041). Each lock is polled once here, so
/// the caller joins the lock's FIFO queue now and the task runs after every earlier submission for the same binding,
/// whatever order the runtime starts the tasks in. `fut` must not take these locks itself. Locks go in a fixed
/// order, so two submissions never wait on each other.
pub fn detached_locked<F>(locks: Vec<Arc<Mutex<()>>>, fut: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    type Acquire = Pin<Box<dyn Future<Output = OwnedMutexGuard<()>> + Send>>;
    let mut cx = Context::from_waker(Waker::noop());
    let mut held = Vec::new();
    let mut waiting: Vec<Acquire> = Vec::new();
    for lock in locks {
        let mut acquire: Acquire = Box::pin(lock.lock_owned());
        match acquire.as_mut().poll(&mut cx) {
            Poll::Ready(guard) => held.push(guard),
            Poll::Pending => waiting.push(acquire),
        }
    }
    detached(async move {
        for acquire in waiting {
            held.push(acquire.await);
        }
        let out = fut.await;
        drop(held);
        out
    })
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::fakes::MemAnimeRepo;
    use crate::model::{AnimeTrack, TrackerId};
    use crate::repo::AnimeTrackRepo;

    fn binding() -> AnimeTrack {
        AnimeTrack {
            id: None,
            anime_id: 4,
            tracker_id: TrackerId::ANILIST,
            remote_id: 1,
            library_id: None,
            title: "Show".into(),
            last_episode_seen: 3.0,
            total_episodes: 12,
            status: 1,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    #[tokio::test]
    async fn tr_27_068_dropping_the_handle_does_not_cancel_the_write() {
        let repo = Arc::new(MemAnimeRepo::default());
        let writer = Arc::clone(&repo);
        let handle = detached(async move {
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            writer.upsert(&binding()).await
        });
        drop(handle);
        for _ in 0..200 {
            if !repo.list_for_entry(4).await.unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            repo.list_for_entry(4).await.unwrap().len(),
            1,
            "the write completed"
        );
    }

    #[tokio::test]
    async fn tr_27_068_a_detached_result_comes_back_through_the_handle() {
        let id = detached(async { 7_i64 }).await.unwrap();
        assert_eq!(id, 7);
    }
}

#[cfg(test)]
mod order_tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use tokio::sync::Mutex;

    use super::detached_locked;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tr_27b_041_tasks_for_one_lock_run_in_submission_order() {
        for _ in 0..50 {
            let lock = Arc::new(Mutex::new(()));
            let log = Arc::new(StdMutex::new(Vec::new()));
            let held = Arc::clone(&lock).lock_owned().await;
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let log = Arc::clone(&log);
                    detached_locked(vec![Arc::clone(&lock)], async move {
                        log.lock().unwrap().push(i);
                    })
                })
                .collect();
            drop(held);
            for h in handles {
                h.await.unwrap();
            }
            assert_eq!(*log.lock().unwrap(), (0..8).collect::<Vec<_>>());
        }
    }
}
