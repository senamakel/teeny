//! One slot of the day's calendar, on one cron: which job posts in it is the
//! rota's to say.
//!
//! The slot fires, the day's rota slot picks the owner, the owner runs, and
//! the fallback takes every day nobody claimed, so switching a job off hands
//! its days to the fallback rather than leaving them quiet. Each owner keeps
//! its own guards (its interval, the cadence), so a restart near the cron
//! minute is still safe. A port of `legacy/src/jobs/slot.ts`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::clock::Clock;
use crate::rota::Rota;

/// Why an owner's run failed.
pub type SlotRunError = Box<dyn std::error::Error + Send + Sync>;

/// A job that can own days of a slot.
#[async_trait]
pub trait SlotRun: Send + Sync {
    async fn run(&self) -> Result<(), SlotRunError>;
}

/// What a slot did when it fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOutcome {
    /// Nobody has today, and there is no fallback.
    Quiet {
        day: u32,
    },
    Ran {
        day: u32,
        job: String,
    },
    /// The owner failed; the slot logged it and carried on.
    Failed {
        day: u32,
        job: String,
        error: String,
    },
}

struct Owner {
    name: String,
    slots: Vec<u32>,
    run: Arc<dyn SlotRun>,
}

/// One slot of the day, e.g. the evening slot.
pub struct SlotJob {
    label: String,
    rota: Rota,
    owners: Vec<Owner>,
    fallback: Option<Owner>,
    clock: Arc<dyn Clock>,
}

impl SlotJob {
    pub fn new(label: &str, rota: Rota, clock: Arc<dyn Clock>) -> Self {
        Self {
            label: label.to_owned(),
            rota,
            owners: Vec::new(),
            fallback: None,
            clock,
        }
    }

    /// A job that owns these rota slots (days). The first owner listed for a
    /// day wins.
    pub fn owner(
        mut self,
        name: &str,
        slots: impl IntoIterator<Item = u32>,
        run: Arc<dyn SlotRun>,
    ) -> Self {
        self.owners.push(Owner {
            name: name.to_owned(),
            slots: slots.into_iter().collect(),
            run,
        });
        self
    }

    /// The job that runs on every day no owner has.
    pub fn fallback(mut self, name: &str, run: Arc<dyn SlotRun>) -> Self {
        self.fallback = Some(Owner {
            name: name.to_owned(),
            slots: Vec::new(),
            run,
        });
        self
    }

    fn owner_of(&self, slot: u32) -> Option<&Owner> {
        self.owners
            .iter()
            .find(|owner| owner.slots.contains(&slot))
            .or(self.fallback.as_ref())
    }

    /// Who owns each slot of the rota, for the startup log: `0:moon 1:lore ...`.
    pub fn describe(&self) -> String {
        (0..self.rota.days())
            .map(|slot| {
                format!(
                    "{slot}:{}",
                    self.owner_of(slot).map_or("-", |owner| owner.name.as_str())
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Fires the slot: runs the day's owner, or nobody.
    pub async fn run(&self) -> SlotOutcome {
        let day = self.rota.slot(self.clock.now());
        let Some(owner) = self.owner_of(day) else {
            tracing::info!(slot = %self.label, day, "slot: nobody has today; quiet");
            return SlotOutcome::Quiet { day };
        };
        tracing::info!(slot = %self.label, day, job = %owner.name, "slot: running the day's job");
        match owner.run.run().await {
            Ok(()) => SlotOutcome::Ran {
                day,
                job: owner.name.clone(),
            },
            Err(error) => {
                tracing::error!(%error, slot = %self.label, job = %owner.name, "slot: the day's job failed");
                SlotOutcome::Failed {
                    day,
                    job: owner.name.clone(),
                    error: error.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
