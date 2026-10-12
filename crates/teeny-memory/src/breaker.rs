//! A circuit breaker for recall: off for a cooldown after a timeout, and
//! failing fast past a number of calls in flight.
//!
//! A recall the client gives up on keeps running on the store, and on
//! 2026-10-05 a handful of them, one per reply, stalled every read until a
//! restart. Backing off is what stops the account from being the thing that
//! keeps the store down. The breaker knows nothing about CortexDB: it guards
//! any memory engine's reads, the caller deciding what counts as a timeout
//! (see `legacy/src/memory/cortex.ts`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Recall is off for now: after a timeout, or because too many are in flight.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RecallPaused(String);

#[derive(Debug)]
struct State {
    cooldown: Duration,
    max_in_flight: usize,
    paused_until: Mutex<Option<Instant>>,
    in_flight: AtomicUsize,
}

/// The breaker. Clones share one state, so every holder of a memory backs
/// off together.
#[derive(Debug, Clone)]
pub struct RecallBreaker(Arc<State>);

/// A place among the calls in flight, given back when dropped.
#[derive(Debug)]
pub struct Permit<'a>(&'a State);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RecallBreaker {
    /// Off for `cooldown` after a timeout (zero never pauses), and at most
    /// `max_in_flight` calls at once.
    pub fn new(cooldown: Duration, max_in_flight: usize) -> Self {
        Self(Arc::new(State {
            cooldown,
            max_in_flight,
            paused_until: Mutex::new(None),
            in_flight: AtomicUsize::new(0),
        }))
    }

    /// A place for one call, or why there is none. Nothing is sent when this
    /// refuses.
    pub fn enter(&self) -> Result<Permit<'_>, RecallPaused> {
        let state = &*self.0;
        let paused = *state
            .paused_until
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(remaining) =
            paused.and_then(|until| until.checked_duration_since(Instant::now()))
            && !remaining.is_zero()
        {
            return Err(RecallPaused(format!(
                "recall paused for {}s after a timeout",
                remaining.as_secs_f64().ceil() as u64
            )));
        }
        if state.in_flight.fetch_add(1, Ordering::SeqCst) >= state.max_in_flight {
            state.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err(RecallPaused("too many recalls in flight".into()));
        }
        Ok(Permit(state))
    }

    /// A call timed out: no more for the cooldown. A timeout in any part of
    /// a call counts, even when another part answered, since the query it
    /// left behind is still running on the store.
    pub fn trip(&self) {
        let state = &*self.0;
        if state.cooldown.is_zero() {
            return;
        }
        *state
            .paused_until
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Instant::now() + state.cooldown);
    }
}

#[cfg(test)]
#[path = "breaker_tests.rs"]
mod tests;
