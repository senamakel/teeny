//! The seam between a pipeline and the agent that drafts for it.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use teeny_agent::posture::Belt;

/// Who drafts: one of the fixed belts, or a named backrooms persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKey {
    /// A fixed agent with its belt's tools.
    Belt(Belt),
    /// A backrooms persona: the `Room` belt with its own system prompt.
    Room {
        /// Stable persona id (`[a-z0-9_-]`, at most 48 chars), e.g. `skeptic`.
        persona: String,
        /// The persona's system prompt.
        system_prompt: String,
    },
}

/// Where a turn's input came from. Decides the access tier the runner
/// applies: `Public` turns are untrusted (the read-only public tier, `specs::access`),
/// `Scheduled` and `Operator` are trusted automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Text from the public (an X mention, a Telegram message).
    Public,
    /// A cron-driven pipeline.
    Scheduled {
        /// Never retried by the runner: a turn that posts must run once.
        once: bool,
    },
    /// An operator behind the bearer token.
    Operator,
}

/// One agent turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRun {
    pub agent: AgentKey,
    pub origin: Origin,
    /// The user message for this turn.
    pub prompt: String,
    /// Continue an earlier session (e.g. a Telegram chat); `None` starts fresh.
    pub session: Option<String>,
    /// Override the agent's default model alias.
    pub model: Option<String>,
    /// Label for the ledger / journal (e.g. `reply`, `morning.caption`).
    pub label: String,
}

/// A tool call the agent made during the turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: String,
    pub is_error: bool,
}

/// Token usage for the turn, when the runtime reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cached: u64,
}

/// What a turn produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentOutcome {
    /// The final assistant text.
    pub reply: String,
    pub session_id: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub usage: Option<TurnUsage>,
    pub model: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("agent turn failed: {0}")]
    Failed(String),
    #[error("agent turn timed out")]
    Timeout,
}

/// Runs one agent turn. Implemented over OpenHuman in `teeny` (the bin), and by
/// [`ScriptedRunner`] in tests.
#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn run(&self, run: AgentRun) -> Result<AgentOutcome, RunError>;
}

/// A test runner that answers turns from a script, in order, and records
/// every request it saw.
#[derive(Default)]
pub struct ScriptedRunner {
    answers: Mutex<VecDeque<Result<AgentOutcome, String>>>,
    seen: Mutex<Vec<AgentRun>>,
}

impl ScriptedRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a plain text answer.
    pub fn reply(self, text: impl Into<String>) -> Self {
        self.outcome(AgentOutcome {
            reply: text.into(),
            session_id: "scripted".into(),
            tool_calls: Vec::new(),
            usage: None,
            model: None,
        })
    }

    /// Queue a full outcome.
    pub fn outcome(self, outcome: AgentOutcome) -> Self {
        self.answers
            .lock()
            .expect("script lock")
            .push_back(Ok(outcome));
        self
    }

    /// Queue a failure.
    pub fn fail(self, message: impl Into<String>) -> Self {
        self.answers
            .lock()
            .expect("script lock")
            .push_back(Err(message.into()));
        self
    }

    /// Every turn requested so far.
    pub fn seen(&self) -> Vec<AgentRun> {
        self.seen.lock().expect("seen lock").clone()
    }
}

#[async_trait]
impl AgentRunner for ScriptedRunner {
    async fn run(&self, run: AgentRun) -> Result<AgentOutcome, RunError> {
        self.seen.lock().expect("seen lock").push(run);
        match self.answers.lock().expect("script lock").pop_front() {
            Some(Ok(outcome)) => Ok(outcome),
            Some(Err(message)) => Err(RunError::Failed(message)),
            None => Err(RunError::Failed("script exhausted".into())),
        }
    }
}
