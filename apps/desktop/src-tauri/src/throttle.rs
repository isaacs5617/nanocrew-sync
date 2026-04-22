//! Simple async byte-rate limiter (token bucket) for S3 I/O.
//!
//! Used at chunk boundaries — one `acquire(n).await` per multipart part
//! upload, per `get_object` range read, and per small-file PUT. At those
//! granularities (typically 16 MiB uploads, a few MiB range reads) the timer
//! resolution is more than good enough to approximate a cap within a few
//! percent of the configured rate.
//!
//! The counter is allowed to go negative — if a caller asks for more bytes
//! than are currently available, it sleeps for the deficit / rate and all
//! subsequent callers wait behind it naturally. This is a classic
//! credit-debit bucket; it avoids the "chunk larger than burst" edge case
//! that a clamped bucket would deadlock on.
//!
//! `RateLimiter::new(None)` (or rate = 0) builds a pass-through limiter:
//! `acquire` returns immediately, no timer / atomic / lock overhead. This is
//! the default path when throttling is disabled.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// One-megabyte default burst floor — allows small transient bursts even at
/// low configured rates so a single range-read doesn't serialise into tiny
/// sub-chunks.
const MIN_BURST: u64 = 1 * 1024 * 1024;

pub struct RateLimiter {
    /// `None` means "unlimited" — `acquire` is a no-op in that case.
    inner: Option<Mutex<Inner>>,
}

struct Inner {
    rate_bytes_per_sec: f64,
    burst: f64,
    /// Current token count. May be negative after a large acquire; callers
    /// then wait until it recovers.
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// Build a new limiter. `bytes_per_sec = None | Some(0)` → unlimited.
    pub fn new(bytes_per_sec: Option<u64>) -> Self {
        let rate = bytes_per_sec.filter(|&r| r > 0);
        Self {
            inner: rate.map(|r| {
                let burst = r.max(MIN_BURST) as f64;
                Mutex::new(Inner {
                    rate_bytes_per_sec: r as f64,
                    burst,
                    tokens: burst,
                    last: Instant::now(),
                })
            }),
        }
    }

    /// Reserve `n` bytes of capacity, sleeping if necessary to stay under
    /// the configured rate. Returns immediately on an unlimited limiter.
    pub async fn acquire(&self, n: u64) {
        let Some(inner) = &self.inner else {
            return;
        };
        let wait = {
            let mut g = inner.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            let elapsed = now.duration_since(g.last).as_secs_f64();
            // Refill, capped at burst.
            g.tokens = (g.tokens + elapsed * g.rate_bytes_per_sec).min(g.burst);
            g.last = now;
            // Debit — allow negative so chunks larger than burst still work.
            g.tokens -= n as f64;
            if g.tokens < 0.0 {
                Duration::from_secs_f64(-g.tokens / g.rate_bytes_per_sec)
            } else {
                Duration::ZERO
            }
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn unlimited_is_instant() {
        let rl = RateLimiter::new(None);
        let t0 = std::time::Instant::now();
        rl.acquire(10_000_000).await;
        assert!(t0.elapsed() < Duration::from_millis(5));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn first_acquire_within_burst_is_instant() {
        // 10 MB/s, burst = 10 MiB → a 1 MiB ask is free on a cold bucket.
        let rl = RateLimiter::new(Some(10 * 1024 * 1024));
        let t0 = std::time::Instant::now();
        rl.acquire(1024 * 1024).await;
        assert!(t0.elapsed() < Duration::from_millis(5));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn second_acquire_waits_proportionally() {
        // 1 MB/s, burst = 1 MiB. First 1 MiB is free, second 1 MiB should
        // wait ~1 s (real time — we use current_thread with real clock).
        let rl = RateLimiter::new(Some(1024 * 1024));
        rl.acquire(1024 * 1024).await;
        let t0 = std::time::Instant::now();
        rl.acquire(1024 * 1024).await;
        let waited = t0.elapsed();
        assert!(
            waited >= Duration::from_millis(800) && waited <= Duration::from_millis(1_400),
            "waited {waited:?}"
        );
    }
}
