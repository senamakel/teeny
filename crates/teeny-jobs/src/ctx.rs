//! What every job runs with: the clients ([`Deps`]), the agent runner, the
//! configuration, the journal, a clock and the jobs' re-run guards.
//!
//! The scheduler builds one [`JobCtx`] at boot and hands the same one to
//! every run, so a job's [`RerunGuard`] outlives the run that marked it, as
//! the legacy jobs' `lastRunAt` field did.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use teeny_agent::deps::{Deps, fake_vars};
use teeny_cadence::clock::{Clock, SystemClock};
use teeny_cadence::guard::RerunGuard;
use teeny_core::Config;
use teeny_store::journal::Outcome;
use teeny_store::{InMemoryJournal, Journal};

use crate::runner::AgentRunner;

/// Everything a job reaches.
pub struct JobCtx {
    pub deps: Arc<Deps>,
    pub runner: Arc<dyn AgentRunner>,
    pub config: Arc<Config>,
    pub journal: Arc<dyn Journal>,
    pub clock: Arc<dyn Clock>,
    guards: Mutex<HashMap<String, Arc<RerunGuard>>>,
}

impl JobCtx {
    pub fn new(
        deps: Arc<Deps>,
        runner: Arc<dyn AgentRunner>,
        config: Arc<Config>,
        journal: Arc<dyn Journal>,
    ) -> Self {
        Self {
            deps,
            runner,
            config,
            journal,
            clock: Arc::new(SystemClock),
            guards: Mutex::new(HashMap::new()),
        }
    }

    /// [`Deps::fake`], the fake configuration (`DRY_RUN` on), an in-memory
    /// journal and the system clock: nothing on the network.
    pub fn fake(runner: Arc<dyn AgentRunner>) -> Self {
        let config = Config::from_vars(fake_vars()).expect("the fake configuration loads");
        Self::new(
            Arc::new(Deps::fake()),
            runner,
            Arc::new(config),
            Arc::new(InMemoryJournal::default()),
        )
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The named job's re-run guard (the default twenty hours), the same one
    /// on every call.
    pub fn guard(&self, job: &str) -> Arc<RerunGuard> {
        let mut guards = self
            .guards
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Arc::clone(guards.entry(job.to_owned()).or_default())
    }
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobReport {
    /// The job's name, e.g. `morning`.
    pub job: String,
    /// Whether it got past its guards (the rota, the re-run guard, the
    /// cadence) and did any work.
    pub ran: bool,
    pub outcome: Outcome,
    /// Why it declined, or what held it.
    pub note: Option<String>,
    /// X's id, or a `dry-run-` placeholder, when something went out.
    pub posted_id: Option<String>,
    /// What was written to be posted: in a rehearsal, what would have gone out.
    pub drafts: Vec<String>,
    /// Every stage that went wrong and was worked around.
    pub problems: Vec<String>,
}

impl JobReport {
    /// The job did not run at all, which is not an error.
    pub fn declined(job: &str, why: impl Into<String>) -> Self {
        Self {
            job: job.to_owned(),
            ran: false,
            outcome: Outcome::Skipped,
            note: Some(why.into()),
            posted_id: None,
            drafts: Vec::new(),
            problems: Vec::new(),
        }
    }

    /// The job ran to this outcome.
    pub fn ran(job: &str, outcome: Outcome) -> Self {
        Self {
            job: job.to_owned(),
            ran: true,
            outcome,
            note: None,
            posted_id: None,
            drafts: Vec::new(),
            problems: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "ctx_tests.rs"]
mod tests;
