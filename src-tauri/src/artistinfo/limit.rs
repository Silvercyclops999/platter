//! Per-source request pacing and backoff.
//!
//! Each source gets a [`Limiter`] that spaces requests at least `1/rate`
//! apart. A 429 or 5xx reply pauses the source: for `Retry-After` when the
//! reply has one, else 30 s doubling per failure up to 15 minutes. While
//! paused, requests fail at once (the caller falls back to the next source),
//! and the first success clears the pause.

use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

use super::http::{HttpResponse, InfoHttp};
use crate::error::{AppError, AppResult};

pub const BACKOFF_FIRST: Duration = Duration::from_secs(30);
pub const BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
struct State {
    next: Instant,
    paused_until: Option<Instant>,
    failures: u32,
}

#[derive(Debug)]
pub struct Limiter {
    gap: Duration,
    state: Mutex<State>,
}

impl Limiter {
    /// At most `per_sec` requests per second.
    pub fn per_second(per_sec: f64) -> Self {
        Self::with_gap(Duration::from_secs_f64(1.0 / per_sec.max(0.01)))
    }

    pub fn with_gap(gap: Duration) -> Self {
        Self { gap, state: Mutex::new(State { next: Instant::now(), paused_until: None, failures: 0 }) }
    }

    pub fn gap(&self) -> Duration {
        self.gap
    }

    /// Whether the source is paused after a 429 or 5xx.
    pub fn paused(&self) -> bool {
        matches!(self.state.lock().paused_until, Some(t) if Instant::now() < t)
    }

    /// Wait for the next free slot. Fails at once while paused.
    pub async fn acquire(&self) -> AppResult<()> {
        let at = {
            let mut s = self.state.lock();
            let now = Instant::now();
            if let Some(t) = s.paused_until
                && now < t
            {
                return Err(AppError::Server("source is backing off".into()));
            }
            let at = s.next.max(now);
            s.next = at + self.gap;
            at
        };
        tokio::time::sleep_until(at).await;
        Ok(())
    }

    pub fn succeeded(&self) {
        let mut s = self.state.lock();
        s.failures = 0;
        s.paused_until = None;
    }

    /// Pause after a 429 or 5xx. Returns how long.
    pub fn throttled(&self, retry_after: Option<u64>) -> Duration {
        let mut s = self.state.lock();
        s.failures = s.failures.saturating_add(1);
        let wait = match retry_after {
            Some(secs) => Duration::from_secs(secs).min(BACKOFF_MAX),
            None => BACKOFF_FIRST.saturating_mul(1 << (s.failures - 1).min(10)).min(BACKOFF_MAX),
        };
        s.paused_until = Some(Instant::now() + wait);
        wait
    }

    /// One paced GET. 429 and 5xx pause the source and come back as errors;
    /// other statuses are the caller's to read.
    pub async fn get(
        &self,
        http: &dyn InfoHttp,
        url: &str,
        headers: &[(&'static str, String)],
    ) -> AppResult<HttpResponse> {
        self.acquire().await?;
        let resp = http.get(url, headers).await?;
        if resp.status == 429 || resp.status >= 500 {
            let wait = self.throttled(resp.retry_after);
            tracing::debug!(status = resp.status, ?wait, "info source busy; backing off");
            return Err(AppError::Server(format!("source answered {}", resp.status)));
        }
        self.succeeded();
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::fake::FakeHttp;
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn spaces_requests_by_the_rate() {
        let l = Limiter::per_second(5.0);
        let t0 = Instant::now();
        for _ in 0..6 {
            l.acquire().await.unwrap();
        }
        // Six requests at 5/s: the last one starts a second after the first.
        assert_eq!(Instant::now() - t0, Duration::from_millis(1000));
    }

    #[tokio::test(start_paused = true)]
    async fn backs_off_on_429_and_5xx_then_recovers() {
        let http = FakeHttp::new();
        let l = Limiter::per_second(2.0);
        http.route("busy", 429, "");
        assert!(l.get(&http, "https://x/busy", &[]).await.is_err());
        assert!(l.paused());
        // Paused: fails without a request.
        assert!(l.get(&http, "https://x/ok", &[]).await.is_err());
        assert_eq!(http.calls("/ok"), 0);
        tokio::time::advance(BACKOFF_FIRST).await;
        http.route("ok", 200, "{}");
        assert!(l.get(&http, "https://x/ok", &[]).await.unwrap().ok());
        assert!(!l.paused());

        // Repeated failures double the pause, up to the cap.
        http.route("down", 503, "");
        for _ in 0..12 {
            tokio::time::advance(BACKOFF_MAX).await;
            let _ = l.get(&http, "https://x/down", &[]).await;
        }
        assert_eq!(l.throttled(None), BACKOFF_MAX);
    }

    #[tokio::test(start_paused = true)]
    async fn honours_retry_after() {
        let l = Limiter::per_second(2.0);
        assert_eq!(l.throttled(Some(7)), Duration::from_secs(7));
        tokio::time::advance(Duration::from_secs(6)).await;
        assert!(l.paused());
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(!l.paused());
    }
}
