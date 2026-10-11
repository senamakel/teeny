//! One slot of the day's calendar, on one cron: which job posts in it is the
//! rota's to say.
//!
//! The slot fires, the day's rota slot picks the owner, the owner runs, and
//! the fallback takes every day nobody claimed, so switching a job off hands
//! its days to the fallback rather than leaving them quiet. An owner that
//! runs and does not post (no seed, the filter held it, a rehearsal, an
//! error) hands the day over the same way, so a bad night for the article is
//! still a post. Each job keeps its own guards (its interval, the cadence),
//! so a restart near the cron minute is still safe. A port of
//! `legacy/src/jobs/slot.ts`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::clock::Clock;
use crate::rota::Rota;

/// Why a job's run failed.
pub type SlotRunError = Box<dyn std::error::Error + Send + Sync>;

/// A job that can own days of a slot.
#[async_trait]
pub trait SlotRun: Send + Sync {
    /// Runs the job: whether it put a post on the timeline.
    async fn run(&self) -> Result<bool, SlotRunError>;
}

/// How one job's run in the slot ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    Posted,
    /// It ran and did not post: it skipped, was held, or rehearsed.
    NotPosted,
    /// It failed; the slot logged it and carried on.
    Failed(String),
}

/// One job's run in the slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotAttempt {
    pub job: String,
    pub result: Attempt,
}

/// What a slot did when it fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOutcome {
    /// Nobody has today, and there is no fallback.
    Quiet { day: u32 },
    /// Every job that ran, in order: the day's owner, then the fallback when
    /// the owner did not post.
    Ran {
        day: u32,
        attempts: Vec<SlotAttempt>,
    },
}

impl SlotOutcome {
    /// Whether the slot put a post on the timeline.
    pub fn posted(&self) -> bool {
        matches!(self, Self::Ran { attempts, .. }
            if attempts.iter().any(|attempt| attempt.result == Attempt::Posted))
    }

    /// The job and error, when the slot's last word was a failure: an
    /// owner's failure the fallback answered is not one.
    pub fn failure(&self) -> Option<(&str, &str)> {
        match self {
            Self::Ran { attempts, .. } => match attempts.last() {
                Some(SlotAttempt {
                    job,
                    result: Attempt::Failed(error),
                }) => Some((job.as_str(), error.as_str())),
                _ => None,
            },
            Self::Quiet { .. } => None,
        }
    }
}

struct Owner<'a> {
    name: String,
    slots: Vec<u32>,
    run: Arc<dyn SlotRun + 'a>,
}

/// One slot of the day, e.g. the evening slot. Its jobs may borrow what
/// they run with (`'a`): the scheduler builds a slot per firing around the
/// run's context.
pub struct SlotJob<'a> {
    label: String,
    rota: Rota,
    owners: Vec<Owner<'a>>,
    fallback: Option<Owner<'a>>,
    clock: Arc<dyn Clock>,
}

impl<'a> SlotJob<'a> {
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
        run: Arc<dyn SlotRun + 'a>,
    ) -> Self {
        self.owners.push(Owner {
            name: name.to_owned(),
            slots: slots.into_iter().collect(),
            run,
        });
        self
    }

    /// The job that runs on every day no owner has, and on an owner's day
    /// when the owner did not post. Its own guards (the cadence first) still
    /// decide whether it posts.
    pub fn fallback(mut self, name: &str, run: Arc<dyn SlotRun + 'a>) -> Self {
        self.fallback = Some(Owner {
            name: name.to_owned(),
            slots: Vec::new(),
            run,
        });
        self
    }

    fn owner_of(&self, slot: u32) -> Option<&Owner<'a>> {
        self.owners.iter().find(|owner| owner.slots.contains(&slot))
    }

    /// Who owns each slot of the rota, for the startup log: `0:chart 1:moon 2:lore`.
    pub fn describe(&self) -> String {
        (0..self.rota.days())
            .map(|slot| {
                let owner = self.owner_of(slot).or(self.fallback.as_ref());
                format!("{slot}:{}", owner.map_or("-", |owner| owner.name.as_str()))
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Whether anything can ever run in this slot.
    pub fn is_live(&self) -> bool {
        !self.owners.is_empty() || self.fallback.is_some()
    }

    /// Fires the slot: runs the day's owner, then the fallback when the
    /// owner did not post or there is none. Never fails: a job's failure is
    /// logged and recorded in the outcome.
    pub async fn run(&self) -> SlotOutcome {
        let day = self.rota.slot(self.clock.now());
        let mut attempts = Vec::new();
        if let Some(owner) = self.owner_of(day) {
            tracing::info!(slot = %self.label, day, job = %owner.name, "slot: running the day's job");
            let attempt = self.attempt(owner).await;
            let posted = attempt.result == Attempt::Posted;
            attempts.push(attempt);
            match &self.fallback {
                _ if posted => return SlotOutcome::Ran { day, attempts },
                None => return SlotOutcome::Ran { day, attempts },
                Some(fallback) => {
                    tracing::info!(slot = %self.label, day, job = %owner.name, fallback = %fallback.name, "slot: the day's job did not post; handing the slot to the fallback");
                }
            }
        }
        let Some(fallback) = &self.fallback else {
            tracing::info!(slot = %self.label, day, "slot: nobody has today; quiet");
            return SlotOutcome::Quiet { day };
        };
        if attempts.is_empty() {
            tracing::info!(slot = %self.label, day, job = %fallback.name, "slot: running the day's job");
        }
        attempts.push(self.attempt(fallback).await);
        SlotOutcome::Ran { day, attempts }
    }

    async fn attempt(&self, job: &Owner<'a>) -> SlotAttempt {
        let result = match job.run.run().await {
            Ok(true) => Attempt::Posted,
            Ok(false) => Attempt::NotPosted,
            Err(error) => {
                tracing::error!(%error, slot = %self.label, job = %job.name, "slot: the day's job failed");
                Attempt::Failed(error.to_string())
            }
        };
        SlotAttempt {
            job: job.name.clone(),
            result,
        }
    }
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
