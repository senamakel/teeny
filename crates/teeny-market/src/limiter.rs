//! Serialises calls to one provider and keeps a minimum gap between them.
//!
//! GeckoTerminal's free tier is roughly thirty calls a minute, shared across
//! everything on the box. A chart run reads four timeframes, holders, flow and
//! the raw candles; fired in parallel they reliably produced a burst of 429s,
//! and the agent then wrote a confident post about a chart it had not been
//! able to read. A queue rather than a token bucket, because nothing here is
//! latency-sensitive and a queue cannot burst by construction.

use std::future::Future;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// A FIFO queue with a minimum gap between the end of one call and the start
/// of the next.
#[derive(Debug)]
pub struct Limiter {
    gap: Duration,
    /// When the last call finished. Held for the length of a call, which is
    /// what serialises them; tokio's mutex is fair, so callers go in order.
    last: Mutex<Option<Instant>>,
}

impl Limiter {
    /// A limiter keeping `gap` between calls.
    pub fn new(gap: Duration) -> Self {
        Self {
            gap,
            last: Mutex::new(None),
        }
    }

    /// Runs `work` when its turn comes. A failed call does not wedge the
    /// queue: the result, error or not, goes back to its own caller.
    pub async fn run<T>(&self, work: impl Future<Output = T>) -> T {
        let mut last = self.last.lock().await;
        if let Some(finished) = *last {
            tokio::time::sleep_until(finished + self.gap).await;
        }
        let result = work.await;
        // Stamped after the call, not before, so a slow request does not have
        // its own duration counted as part of the gap that follows it.
        *last = Some(Instant::now());
        result
    }
}

#[cfg(test)]
#[path = "limiter_tests.rs"]
mod tests;
