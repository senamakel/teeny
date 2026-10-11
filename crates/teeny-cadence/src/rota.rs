//! A rota for the posting jobs (`POST_ROTA_DAYS` from `POST_ROTA_ANCHOR`).
//!
//! Cron cannot say "every third day": a step in the day-of-month field
//! restarts every month and lands two posts back to back on the 31st and the
//! 1st. So each slot of the day keeps one daily cron and asks the rota whose
//! turn today is ([`crate::slot`]). Days are counted in UTC from a fixed anchor, so the answer is the
//! same in every process and after every restart.
//! A port of `legacy/src/jobs/cycle.ts`.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use teeny_core::config::RotaConfig;

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// A rota that cannot be counted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RotaError(String);

/// How many days one turn takes, and the day that is slot 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rota {
    days: u32,
    anchor: NaiveDate,
}

impl Rota {
    pub fn new(days: u32, anchor: NaiveDate) -> Result<Self, RotaError> {
        if days == 0 {
            return Err(RotaError("a rota needs at least one day".into()));
        }
        Ok(Self { days, anchor })
    }

    /// A rota anchored on an ISO date, `YYYY-MM-DD`.
    pub fn parse(days: u32, anchor: &str) -> Result<Self, RotaError> {
        let anchor = NaiveDate::parse_from_str(anchor, "%Y-%m-%d")
            .map_err(|_| RotaError(format!("rota anchor \"{anchor}\" is not an ISO date")))?;
        Self::new(days, anchor)
    }

    /// `POST_ROTA_DAYS` from `POST_ROTA_ANCHOR`.
    pub fn from_config(config: &RotaConfig) -> Result<Self, RotaError> {
        Self::new(config.days, config.anchor)
    }

    pub fn days(&self) -> u32 {
        self.days
    }

    /// Which slot of the rota a moment falls in, 0-based. Days before the
    /// anchor wrap backwards onto real slots.
    pub fn slot(&self, now: DateTime<Utc>) -> u32 {
        let start = self.anchor.and_time(NaiveTime::MIN).and_utc();
        let elapsed = (now - start).num_milliseconds().div_euclid(DAY_MS);
        u32::try_from(elapsed.rem_euclid(i64::from(self.days))).expect("below the rota's length")
    }

    /// Whether a job with this slot is on today. A slot past the rota's length
    /// is never on.
    pub fn is_turn(&self, slot: u32, now: DateTime<Utc>) -> bool {
        self.slot(now) == slot
    }
}

#[cfg(test)]
#[path = "rota_tests.rs"]
mod tests;
