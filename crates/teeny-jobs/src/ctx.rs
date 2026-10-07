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

use teeny_agent::deps::QueuedXPost;
use teeny_store::{
    ChatTranscript, InMemoryChatTranscript, InMemoryLedger, InMemoryMentionQueue, InMemoryPeople,
    InMemoryPostLedger, InMemoryStateStore, InferenceLedger, MentionQueue, People, PostLedger,
    StateStore,
};
use teeny_x::intake::AccountId;

use crate::brain_fold::BrainFold;
use crate::ingest::{FAKE_ACCOUNT_ID, FixedAccount, MentionSource, ScriptedMentions};
use crate::reply::{ArticleLookup, Pacing, RecordingOutlet, ReplyOutlet};
use crate::runner::AgentRunner;

/// Everything a job reaches.
pub struct JobCtx {
    pub deps: Arc<Deps>,
    pub runner: Arc<dyn AgentRunner>,
    pub config: Arc<Config>,
    pub journal: Arc<dyn Journal>,
    pub clock: Arc<dyn Clock>,
    /// The conversation side (mentions, replies, chat, heat, the brain
    /// fold): absent until [`with_conversation`](Self::with_conversation).
    pub conversation: Option<Arc<Conversation>>,
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
            conversation: None,
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
        .with_conversation(Conversation::fake())
    }

    /// Wires the conversation side.
    pub fn with_conversation(mut self, conversation: Conversation) -> Self {
        self.conversation = Some(Arc::new(conversation));
        self
    }

    /// The conversation side, or why a conversation job cannot run.
    pub fn conversation(&self) -> anyhow::Result<&Conversation> {
        self.conversation
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("the conversation side is not wired"))
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

/// The stores and X seams of the conversation side: the mention queue and
/// its cursor, the people record, the Telegram transcript, the post and
/// inference ledgers, and the X calls a public turn's host code makes for
/// it (identity, the mention poll, the reply outlet). Built once at boot;
/// the pacing and the brain cursor live across runs.
pub struct Conversation {
    pub queue: Arc<dyn MentionQueue<QueuedXPost>>,
    /// Who interacts most, so a batch answers the regulars first.
    pub people: Option<Arc<dyn People>>,
    /// The mention poll's `since_id` / `lastPolledAt` cursor.
    pub cursor: Arc<dyn StateStore>,
    pub transcript: Arc<dyn ChatTranscript>,
    /// The account's own posts, for the heat job.
    pub posts: Arc<dyn PostLedger>,
    /// Where the brain's spend is folded.
    pub inferences: Option<Arc<dyn InferenceLedger>>,
    /// The account's own id: nothing is answered without it.
    pub account: Arc<dyn AccountId>,
    pub mentions: Arc<dyn MentionSource>,
    /// Where host code posts a drafted reply, after the safety gate.
    pub outlet: Arc<dyn ReplyOutlet>,
    /// The account's own articles, by the post that carried them.
    pub articles: Option<Arc<dyn ArticleLookup>>,
    /// The brain's checkout, when there is one (`BRAIN_DATA_DIR`).
    pub brain: Option<Arc<BrainFold>>,
    /// The gap between published replies, kept across runs.
    pub pacing: Pacing,
}

impl Conversation {
    /// Every part in process: in-memory stores, an account of
    /// [`FAKE_ACCOUNT_ID`], no mentions to poll, and an outlet that records.
    pub fn fake() -> Self {
        Self {
            queue: Arc::new(InMemoryMentionQueue::<QueuedXPost>::default()),
            people: Some(Arc::new(InMemoryPeople::default())),
            cursor: Arc::new(InMemoryStateStore::default()),
            transcript: Arc::new(InMemoryChatTranscript::default()),
            posts: Arc::new(InMemoryPostLedger::default()),
            inferences: Some(Arc::new(InMemoryLedger::default())),
            account: Arc::new(FixedAccount::new(FAKE_ACCOUNT_ID)),
            mentions: Arc::new(ScriptedMentions::default()),
            outlet: Arc::new(RecordingOutlet::default()),
            articles: None,
            brain: None,
            pacing: Pacing::default(),
        }
    }
}

#[cfg(test)]
#[path = "ctx_tests.rs"]
mod tests;
