//! Sliding-window rate limiter, globally or for one host (T-12a-010, T-12a-011).
//! Waiters are served in FIFO order: a fair async mutex is the turn, and only the turn holder sleeps.
//! The window itself sits behind a short std mutex that is never held across an await, so a cache-hit refund can
//! always get in and wake the sleeper.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{Chain, Interceptor};
use crate::error::NetError;
use crate::request::Request;
use crate::response::Response;

pub struct RateLimit {
    host: Option<String>,
    permits: usize,
    period: Duration,
    window: Mutex<VecDeque<Instant>>,
    turn: AsyncMutex<()>,
    refunded: Notify,
}

impl RateLimit {
    /// `host` limits only requests to that host name; `None` limits every request. `permits` must be at least 1.
    pub fn new(host: Option<String>, permits: usize, period: Duration) -> Result<Self, NetError> {
        if permits == 0 {
            return Err(NetError::Other("rate limit needs at least one permit".to_owned()));
        }
        Ok(Self {
            host,
            permits,
            period,
            window: Mutex::new(VecDeque::new()),
            turn: AsyncMutex::new(()),
            refunded: Notify::new(),
        })
    }

    /// Limits the host of `url`. An unparsable URL is an error, not a limit on every host.
    pub fn for_host_url(url: &str, permits: usize, period: Duration) -> Result<Self, NetError> {
        let parsed = url::Url::parse(url).map_err(|e| NetError::InvalidUrl(e.to_string()))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| NetError::InvalidUrl(format!("no host in {url}")))?;
        Self::new(Some(host.to_owned()), permits, period)
    }

    /// Whether this limiter counts a request to `url`.
    pub fn applies_to(&self, url: &url::Url) -> bool {
        match &self.host {
            None => true,
            Some(host) => url.host_str() == Some(host.as_str()),
        }
    }

    fn window(&self) -> MutexGuard<'_, VecDeque<Instant>> {
        self.window.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for a permit and records it. Returns the timestamp that identifies the permit.
    pub async fn acquire(&self, cancel: &CancellationToken) -> Result<Instant, NetError> {
        let _turn = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(NetError::Canceled),
            guard = self.turn.lock() => guard,
        };
        loop {
            // Register for a refund before looking, so a refund between the look and the sleep is not lost.
            let refunded = self.refunded.notified();
            tokio::pin!(refunded);
            refunded.as_mut().enable();

            let wait = {
                let mut window = self.window();
                let now = Instant::now();
                while window.front().is_some_and(|&first| first + self.period <= now) {
                    window.pop_front();
                }
                if window.len() < self.permits {
                    window.push_back(now);
                    return Ok(now);
                }
                let first = window.front().copied().unwrap_or(now);
                (first + self.period).saturating_duration_since(now)
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = &mut refunded => {}
                _ = cancel.cancelled() => return Err(NetError::Canceled),
            }
        }
    }

    /// Returns the permit taken at `stamp`, when that entry is still in the window. Wakes the waiters.
    pub fn refund(&self, stamp: Instant) {
        let removed = {
            let mut window = self.window();
            match window.iter().position(|t| *t == stamp) {
                Some(pos) => {
                    window.remove(pos);
                    true
                }
                None => false,
            }
        };
        if removed {
            self.refunded.notify_waiters();
        }
    }

    /// The number of permits in use (test helper and diagnostics).
    pub fn in_use(&self) -> usize {
        self.window().len()
    }
}

#[async_trait]
impl Interceptor for RateLimit {
    async fn intercept(&self, chain: Chain<'_>, req: Request) -> Result<Response, NetError> {
        if req.opts.skip_rate_limit || !self.applies_to(&req.url) {
            return chain.proceed(req).await;
        }
        let cancel = req.opts.cancel.clone();
        let stamp = self.acquire(&cancel).await?;
        let resp = chain.proceed(req).await?;
        if resp.from_cache {
            self.refund(stamp);
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(permits: usize, secs: u64) -> RateLimit {
        RateLimit::new(None, permits, Duration::from_secs(secs)).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn the_permit_after_the_limit_waits_until_the_oldest_is_a_period_old() {
        let rl = limiter(85, 60);
        let cancel = CancellationToken::new();
        let start = Instant::now();
        for _ in 0..85 {
            rl.acquire(&cancel).await.unwrap();
        }
        assert_eq!(Instant::now(), start);
        rl.acquire(&cancel).await.unwrap();
        assert_eq!(Instant::now() - start, Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn waiters_are_served_in_arrival_order() {
        let rl = std::sync::Arc::new(limiter(1, 10));
        let cancel = CancellationToken::new();
        rl.acquire(&cancel).await.unwrap();
        let served = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for i in 0..3u32 {
            let rl = rl.clone();
            let served = served.clone();
            handles.push(tokio::spawn(async move {
                let cancel = CancellationToken::new();
                rl.acquire(&cancel).await.unwrap();
                served.lock().unwrap().push(i);
            }));
            tokio::task::yield_now().await;
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(*served.lock().unwrap(), vec![0, 1, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_a_wait_returns_canceled_without_taking_a_permit() {
        let rl = limiter(1, 60);
        let first = CancellationToken::new();
        rl.acquire(&first).await.unwrap();
        let waiting = CancellationToken::new();
        let token = waiting.clone();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            token.cancel();
        });
        let err = rl.acquire(&waiting).await.err().unwrap();
        assert!(matches!(err, NetError::Canceled));
        canceller.await.unwrap();
        assert_eq!(rl.in_use(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cache_hit_refund_wakes_a_waiter_early() {
        let rl = std::sync::Arc::new(limiter(1, 60));
        let cancel = CancellationToken::new();
        let start = Instant::now();
        let stamp = rl.acquire(&cancel).await.unwrap();
        let waiter = {
            let rl = rl.clone();
            tokio::spawn(async move {
                let cancel = CancellationToken::new();
                rl.acquire(&cancel).await.unwrap();
                Instant::now()
            })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        rl.refund(stamp);
        let woke_at = waiter.await.unwrap();
        assert_eq!(woke_at - start, Duration::from_secs(1));
        assert_eq!(rl.in_use(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refund_for_an_expired_entry_removes_nothing() {
        let rl = limiter(2, 1);
        let cancel = CancellationToken::new();
        let stamp = rl.acquire(&cancel).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        rl.acquire(&cancel).await.unwrap();
        rl.refund(stamp);
        assert_eq!(rl.in_use(), 1);
    }

    #[tokio::test]
    async fn a_request_that_skips_the_limit_takes_no_permit_and_a_plain_one_takes_one() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let limit = std::sync::Arc::new(RateLimit::new(None, 1, Duration::from_secs(3600)).unwrap());
        let client = crate::client::NetworkClient::for_tests(vec![limit.clone() as std::sync::Arc<dyn Interceptor>]);
        let mut skipping = crate::request::get(&format!("{}/skip", server.uri())).unwrap();
        skipping.opts.skip_rate_limit = true;
        for _ in 0..3 {
            tokio::time::timeout(Duration::from_secs(5), client.execute(skipping.clone()))
                .await
                .expect("a call that skips the limit never waits")
                .unwrap();
        }
        assert_eq!(limit.in_use(), 0);
        client.execute(crate::request::get(&format!("{}/plain", server.uri())).unwrap()).await.unwrap();
        assert_eq!(limit.in_use(), 1);
    }

    #[test]
    fn zero_permits_are_rejected_at_construction() {
        assert!(RateLimit::new(None, 0, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn an_unparsable_host_url_is_an_error_not_a_global_limit() {
        assert!(RateLimit::for_host_url("not a url", 1, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn a_host_limiter_ignores_other_hosts() {
        let rl = RateLimit::for_host_url("https://api.test/v1", 1, Duration::from_secs(1)).unwrap();
        assert!(rl.applies_to(&url::Url::parse("https://api.test/x").unwrap()));
        assert!(!rl.applies_to(&url::Url::parse("https://other.test/x").unwrap()));
        assert!(limiter(1, 1).applies_to(&url::Url::parse("https://other.test/").unwrap()));
    }
}
