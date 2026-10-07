//! The time, as something a test can hold still.
//!
//! Every rule in this crate is a comparison against "now": the gap since the
//! last post, a hot post's hold, the day of the rota, the day of the media
//! budget. Each takes a [`Clock`], so the tests drive time by hand.

use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

/// Where "now" comes from.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// The system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that only moves when told to.
#[derive(Debug)]
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub fn new(at: DateTime<Utc>) -> Self {
        Self(Mutex::new(at))
    }

    pub fn set(&self, at: DateTime<Utc>) {
        *self.0.lock().expect("not poisoned") = at;
    }

    pub fn advance(&self, by: TimeDelta) {
        *self.0.lock().expect("not poisoned") += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().expect("not poisoned")
    }
}

/// A timestamp as JavaScript's `toISOString()` writes it, as the legacy
/// ledger stored them and the logs show them.
pub fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
pub(crate) fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 time")
        .to_utc()
}

#[cfg(test)]
#[path = "clock_tests.rs"]
mod tests;
