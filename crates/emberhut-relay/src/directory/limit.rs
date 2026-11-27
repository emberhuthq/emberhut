//! Request limits (429 `limit`, with `Retry-After`): a token bucket per key, held in memory.
//!
//! The directory counts an account's requests by the account, once its token has passed, and a
//! machine's own unlinks by the machine id in the path. The relay sits behind a TLS proxy for
//! now, so it has no client address of its own to count by.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// How many requests one key may make: `burst` at once, then one more every `every`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    /// Requests that may be made at once.
    pub burst: u32,
    /// How long until one more may be made.
    pub every: Duration,
}

/// The directory's request limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestLimits {
    /// Per account, on the routes that take an account token: 30 at once, then one every 2 s.
    pub per_account: Rate,
    /// Per machine, on its own unlink: 5 at once, then one every 12 s.
    pub per_machine: Rate,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            per_account: Rate {
                burst: 30,
                every: Duration::from_secs(2),
            },
            per_machine: Rate {
                burst: 5,
                every: Duration::from_secs(12),
            },
        }
    }
}

/// How the map of buckets is kept small: buckets that have filled up again are dropped once it
/// holds more than `prune_above` keys, at most once every `prune_every` (a prune walks the whole
/// map); and it is emptied if it still reaches `max_keys`, so a flood of keys costs memory and
/// time only up to a bound. Emptying it gives every key a full bucket again: under such a flood
/// the limits slacken rather than lock anyone out, since a machine must always be able to leave
/// its account.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    prune_above: usize,
    prune_every: Duration,
    max_keys: usize,
}

impl Bounds {
    const DEFAULT: Self = Self {
        prune_above: 10_000,
        prune_every: Duration::from_secs(10),
        max_keys: 100_000,
    };
}

/// A bucket per key.
#[derive(Debug)]
pub(super) struct Limiter {
    rate: Rate,
    bounds: Bounds,
    buckets: Mutex<Buckets>,
}

#[derive(Debug, Default)]
struct Buckets {
    by_key: HashMap<String, Bucket>,
    /// When the map was last pruned.
    pruned: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    /// Requests left, as of `at`, in units of `rate.every`.
    tokens: f64,
    at: Instant,
}

impl Limiter {
    pub(super) fn new(rate: Rate) -> Self {
        Self::with_bounds(rate, Bounds::DEFAULT)
    }

    fn with_bounds(rate: Rate, bounds: Bounds) -> Self {
        Self {
            rate,
            bounds,
            buckets: Mutex::default(),
        }
    }

    /// Takes one request for `key`, or says how long until one may be made.
    pub(super) fn take(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let burst = f64::from(self.rate.burst.max(1));
        let every = self.rate.every.as_secs_f64();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let Buckets { by_key, pruned } = &mut *buckets;
        if by_key.len() > self.bounds.prune_above
            && pruned.is_none_or(|pruned| now >= pruned + self.bounds.prune_every)
        {
            by_key.retain(|_, bucket| refilled(*bucket, now, every, burst) < burst);
            *pruned = Some(now);
        }
        if by_key.len() >= self.bounds.max_keys && !by_key.contains_key(key) {
            tracing::warn!(
                keys = by_key.len(),
                "too many keys are being limited at once; starting their counts again"
            );
            by_key.clear();
        }
        let bucket = by_key.entry(key.to_owned()).or_insert(Bucket {
            tokens: burst,
            at: now,
        });
        let tokens = refilled(*bucket, now, every, burst);
        if tokens >= 1.0 {
            *bucket = Bucket {
                tokens: tokens - 1.0,
                at: now,
            };
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - tokens) * every))
        }
    }
}

fn refilled(bucket: Bucket, now: Instant, every: f64, burst: f64) -> f64 {
    let gained = if every > 0.0 {
        now.saturating_duration_since(bucket.at).as_secs_f64() / every
    } else {
        burst
    };
    (bucket.tokens + gained).min(burst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_burst_then_one_per_interval() {
        let limiter = Limiter::new(Rate {
            burst: 2,
            every: Duration::from_secs(10),
        });
        assert!(limiter.take("a").is_ok());
        assert!(limiter.take("a").is_ok());
        let near = |wait: Duration, secs: u64| {
            wait.abs_diff(Duration::from_secs(secs)) < Duration::from_millis(1)
        };
        assert!(near(limiter.take("a").unwrap_err(), 10));
        assert!(limiter.take("b").is_ok(), "keys don't share a bucket");
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(near(limiter.take("a").unwrap_err(), 6));
        tokio::time::advance(Duration::from_secs(6)).await;
        assert!(limiter.take("a").is_ok());
        assert!(limiter.take("a").is_err());
    }

    fn keys(limiter: &Limiter) -> usize {
        limiter.buckets.lock().unwrap().by_key.len()
    }

    const ONE_A_SECOND: Rate = Rate {
        burst: 1,
        every: Duration::from_secs(1),
    };

    #[tokio::test(start_paused = true)]
    async fn past_ten_thousand_keys_full_buckets_are_dropped_at_most_once_an_interval() {
        let limiter = Limiter::new(ONE_A_SECOND);
        for key in 0..10_001 {
            assert!(limiter.take(&key.to_string()).is_ok());
        }
        // Over the threshold, but every bucket is still empty: a prune keeps them all.
        assert!(limiter.take("one more").is_ok());
        assert_eq!(keys(&limiter), 10_002);
        // Refilled, but within the interval since that prune: no walk of the map.
        tokio::time::advance(Duration::from_secs(2)).await;
        for key in 10_001..12_000 {
            assert!(limiter.take(&key.to_string()).is_ok());
        }
        assert_eq!(keys(&limiter), 12_001);
        // Once the interval has passed, the next request drops every full bucket.
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(limiter.take("after").is_ok());
        assert_eq!(keys(&limiter), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_map_is_never_larger_than_its_cap() {
        let bounds = Bounds {
            prune_above: 1_000,
            prune_every: Duration::from_secs(10),
            max_keys: 100,
        };
        let limiter = Limiter::with_bounds(ONE_A_SECOND, bounds);
        for key in 0..1_000 {
            assert!(limiter.take(&key.to_string()).is_ok());
            assert!(keys(&limiter) <= 100);
        }
        let limiter = Limiter::with_bounds(ONE_A_SECOND, bounds);
        for key in 0..100 {
            assert!(limiter.take(&key.to_string()).is_ok());
        }
        // Full: a key already counted keeps its count.
        assert!(limiter.take("0").is_err());
        assert_eq!(keys(&limiter), 100);
        // A new key starts the counts again.
        assert!(limiter.take("new").is_ok());
        assert_eq!(keys(&limiter), 1);
    }
}
