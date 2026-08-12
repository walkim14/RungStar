//! Not hammering somebody else's server.
//!
//! A full catalog crawl is three hundred requests, and the reference sends them back to back
//! with no limit and no backoff. USDB is one PHP box run by volunteers; a client that treats
//! it as an API endpoint is the reason scrapers get blocked.
//!
//! A token bucket rather than a fixed sleep, so a handful of requests — opening a song page,
//! then its text — go straight through, and only a sustained crawl is paced. Retries back off
//! exponentially with jitter, because a synchronised retry from every client that saw the same
//! outage is the outage happening twice.

use std::time::{Duration, Instant};

/// How fast requests may be made.
#[derive(Debug, Clone, Copy)]
pub struct Rate {
    /// Sustained requests per second.
    pub per_second: f64,
    /// How many may be made at once after a quiet period.
    pub burst: f64,
}

impl Default for Rate {
    fn default() -> Self {
        // Two a second sustained, eight in hand. A page of a hundred songs per request makes
        // a full 30,000-song crawl about three hundred requests, so this paces a cold sync at
        // under three minutes while leaving an interactive click instant.
        Self {
            per_second: 2.0,
            burst: 8.0,
        }
    }
}

/// A token bucket.
pub struct Limiter {
    rate: Rate,
    tokens: f64,
    last: Instant,
}

impl Limiter {
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            tokens: rate.burst,
            last: Instant::now(),
        }
    }

    /// How long to wait before the next request may be made.
    ///
    /// Returns a duration rather than sleeping, so the caller decides whether to block a
    /// worker thread or to come back later — and so this is testable without a clock.
    pub fn take(&mut self, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate.per_second).min(self.rate.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return Duration::ZERO;
        }
        let short_by = 1.0 - self.tokens;
        self.tokens = 0.0;
        let wait = Duration::from_secs_f64(short_by / self.rate.per_second.max(f64::EPSILON));
        // The caller performs this request after sleeping, so reserve that future token now.
        // Otherwise the next call at the wake-up instant sees a freshly refilled token and a
        // paced crawl sends two requests together every interval.
        self.last += wait;
        wait
    }
}

/// How long to wait before retry number `attempt`, counting from zero.
///
/// Exponential from `base`, capped, and jittered by up to a quarter either way. The jitter is
/// the part that matters: without it every client that saw the same failure retries at the
/// same instant and the server gets the spike again.
pub fn backoff(attempt: u32, base: Duration, cap: Duration, noise: u64) -> Duration {
    let grown = base.saturating_mul(1u32 << attempt.min(10));
    let capped = grown.min(cap);
    // A deterministic jitter from the seed handed in, so a retry schedule can be tested.
    let spread = (noise % 1000) as f64 / 1000.0 - 0.5;
    let seconds = capped.as_secs_f64() * (1.0 + spread * 0.5);
    Duration::from_secs_f64(seconds.max(0.0))
}

/// How many times a failed request is worth repeating.
pub const RETRIES: u32 = 4;
