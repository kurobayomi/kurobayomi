//! Background work owned by one screen: a tokio task group that is cancelled when the screen
//! closes. The screen keeps the value; dropping it cancels and aborts every task still running.

use std::future::Future;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct ScreenTasks {
    cancel: CancellationToken,
    tasks: JoinSet<()>,
}

impl ScreenTasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts `work`. It stops at its next await point once the screen is cancelled or dropped.
    pub fn spawn<F>(&mut self, work: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let cancel = self.cancel.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {}
                () = work => {}
            }
        });
    }

    /// Cancels every task. Tasks that already finished are not affected.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for ScreenTasks {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.tasks.abort_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn a_task_runs_to_completion_when_not_cancelled() {
        let done = Arc::new(AtomicBool::new(false));
        let mut tasks = ScreenTasks::new();
        let flag = done.clone();
        tasks.spawn(async move { flag.store(true, Ordering::SeqCst) });
        while tasks.tasks.join_next().await.is_some() {}
        assert!(done.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancelling_the_screen_stops_a_pending_task() {
        let finished = Arc::new(AtomicBool::new(false));
        let mut tasks = ScreenTasks::new();
        let flag = finished.clone();
        tasks.spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            flag.store(true, Ordering::SeqCst);
        });
        tasks.cancel();
        while tasks.tasks.join_next().await.is_some() {}
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropping_the_screen_aborts_its_tasks() {
        let marker = Arc::new(());
        let held = marker.clone();
        {
            let mut tasks = ScreenTasks::new();
            tasks.spawn(async move {
                let _held = held;
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        // The task future owned the clone; aborting the task drops it.
        assert_eq!(Arc::strong_count(&marker), 1);
    }
}
