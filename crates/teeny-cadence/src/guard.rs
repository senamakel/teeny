//! The re-run guard each daily posting job keeps.
//!
//! Each legacy job kept `lastRunAt` and refused to run again inside
//! `minIntervalMs`, twenty hours for the daily ones; a forced run skips the
//! check. The time is marked once the job has decided to run, so a run that
//! then fails still counts.
//!
//! What this covers is narrow: a second run of the same job inside one
//! process, such as a retry or a manual trigger on top of the cron. The last
//! run lives in memory, so a restart forgets it, and a second process never
//! sees it. Restarts are covered elsewhere: OpenHuman's cron advances a
//! job's `next_run` when it dispatches the job, so a restart does not fire
//! the same slot again, and the persisted post ledger behind
//! [`Cadence`](crate::cadence::Cadence) refuses an own post inside the gap
//! whichever process tries it.

use std::sync::Mutex;

use chrono::{DateTime, TimeDelta, Utc};

/// The daily jobs' interval: a day, less enough slack for a late cron.
pub const DEFAULT_RERUN_INTERVAL: TimeDelta = TimeDelta::hours(20);

/// The job already ran, this long ago.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RanRecently {
    pub since: TimeDelta,
    pub minimum: TimeDelta,
}

/// One job's last run.
#[derive(Debug)]
pub struct RerunGuard {
    minimum: TimeDelta,
    last_run: Mutex<Option<DateTime<Utc>>>,
}

impl RerunGuard {
    pub fn new(minimum: TimeDelta) -> Self {
        Self {
            minimum,
            last_run: Mutex::new(None),
        }
    }

    /// Whether the job may run at `now`.
    pub fn check(&self, now: DateTime<Utc>) -> Result<(), RanRecently> {
        match *self.last_run.lock().expect("not poisoned") {
            Some(last) if now - last < self.minimum => Err(RanRecently {
                since: now - last,
                minimum: self.minimum,
            }),
            _ => Ok(()),
        }
    }

    /// Records that the job is running at `now`.
    pub fn mark(&self, now: DateTime<Utc>) {
        *self.last_run.lock().expect("not poisoned") = Some(now);
    }
}

impl Default for RerunGuard {
    fn default() -> Self {
        Self::new(DEFAULT_RERUN_INTERVAL)
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
