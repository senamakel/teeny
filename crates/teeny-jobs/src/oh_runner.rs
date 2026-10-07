//! [`AgentRunner`] over the embedded OpenHuman agents.
//!
//! - **Who**: [`AgentKey::Belt`] is that belt's agent; [`AgentKey::Room`] is
//!   the persona's own agent, registered on first use
//!   ([`TeenyAgents::room`]).
//! - **With what authority** ([`turn_origin`]): a public turn runs as the
//!   `public` external channel (`Access::public`'s origin), and only on
//!   `teeny-reply` or `teeny-chat`, whose access refuses a write at once (a
//!   trusted agent keeps its writes under a public origin, so a public turn
//!   on one is refused before it starts); a scheduled turn runs as
//!   `TrustedAutomation{Cron}`, the origin the browser's unattended actions
//!   are allowed for; an operator turn (`/agent/run`, behind the bearer) runs
//!   as `TrustedAutomation{Workflow{require_approval: false}}`, the trust
//!   root `Access::full` itself states. `Cron` would mislabel an ad-hoc
//!   request, and `Background` promises no outside text reaches the prompt.
//!   `teeny-operator`, the one agent that posts and remembers, takes only
//!   operator turns: a scheduled or public turn on it is refused up front.
//! - **How often** ([`attempts`]): once, except a scheduled turn that is
//!   allowed to repeat, which gets one retry. A turn that may post
//!   (`once: true`, the operator) is never retried, whatever failed: a retry
//!   re-runs the whole turn and can post twice.
//! - **For how long**: every attempt is bounded by the runner's timeout;
//!   on expiry the turn's future is dropped, which cancels it.
//! - **What it reports**: the reply and session, and, from a bounded
//!   progress channel drained beside the turn, every tool call and every
//!   model call. Each model call is one inference-ledger row (the legacy
//!   row shape, lane `mascot`, source the run's label); a turn that reported
//!   no model call but did report usage gets one row for the turn.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openhuman_embed::agent_progress::AgentProgress;
use openhuman_embed::{Access, Agent, AgentTurnOrigin, TrustedAutomationSource};
use serde_json::Value;
use teeny_agent::posture::Belt;
use teeny_agent::specs::TeenyAgents;
use teeny_store::InferenceLedger;
use teeny_store::inference::{
    InferenceInput, Lane, LedgerMessage, MAX_MESSAGE_CHARS, MAX_MESSAGES, Role, Tokens, cut,
    record_quietly,
};

use crate::runner::{
    AgentKey, AgentOutcome, AgentRun, AgentRunner, Origin, RunError, ToolCallRecord, TurnUsage,
};

/// How long one attempt may run unless the runner is told otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Progress events buffered between the turn and the collector. The core
/// awaits its sends, so this is backpressure; the collector drains it
/// continuously, so it never stalls a turn.
const PROGRESS_CAPACITY: usize = 256;
/// Prefix of a trusted turn's job id, so the core's audit names Teeny.
const JOB_PREFIX: &str = "teeny:";

/// The runner.
pub struct OhRunner {
    agents: Arc<TeenyAgents>,
    ledger: Option<Arc<dyn InferenceLedger>>,
    timeout: Duration,
}

impl OhRunner {
    pub fn new(agents: Arc<TeenyAgents>) -> Self {
        Self {
            agents,
            ledger: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Writes every turn's model calls to `ledger`.
    pub fn with_ledger(mut self, ledger: Arc<dyn InferenceLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Bounds every attempt.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn agent(&self, key: &AgentKey) -> Result<Agent, RunError> {
        match key {
            AgentKey::Belt(belt) => self
                .agents
                .belt(*belt)
                .cloned()
                .ok_or_else(|| RunError::Failed(format!("no agent for the {belt:?} belt"))),
            AgentKey::Room {
                persona,
                system_prompt,
            } => self
                .agents
                .room(persona, system_prompt)
                .map_err(|error| RunError::Failed(error.to_string())),
        }
    }

    async fn attempt(&self, agent: &Agent, run: &AgentRun) -> Result<AgentOutcome, RunError> {
        let started = Instant::now();
        let mut turn = agent
            .turn(run.prompt.clone())
            .origin(turn_origin(run.origin, &run.label));
        if let Some(session) = &run.session {
            turn = turn.session(session.clone());
        }
        if let Some(model) = &run.model {
            turn = turn.model(model.clone());
        }
        let (sink, mut events) = tokio::sync::mpsc::channel(PROGRESS_CAPACITY);
        let collector = tokio::spawn(async move {
            let mut progress = Progress::default();
            while let Some(event) = events.recv().await {
                progress.observe(event);
            }
            progress
        });
        let sent = tokio::time::timeout(self.timeout, turn.on_progress(sink).send()).await;
        // The turn (and with it the sender) is gone either way, so this ends.
        let progress = collector.await.unwrap_or_default();
        let outcome = match sent {
            Err(_) => {
                tracing::warn!(label = %run.label, "[oh_runner] turn timed out");
                self.record(run, &progress, None, None, started).await;
                return Err(RunError::Timeout);
            }
            Ok(Err(error)) => {
                self.record(run, &progress, None, None, started).await;
                return Err(RunError::Failed(error.to_string()));
            }
            Ok(Ok(outcome)) => outcome,
        };
        let usage = progress.usage().or_else(|| {
            outcome.usage.as_ref().map(|usage| TurnUsage {
                input: usage.input_tokens,
                output: usage.output_tokens,
                reasoning: 0,
                cached: usage.cached_input_tokens,
            })
        });
        let model = progress.model().or_else(|| run.model.clone());
        self.record(run, &progress, Some(&outcome.reply), usage, started)
            .await;
        Ok(AgentOutcome {
            reply: outcome.reply,
            session_id: outcome.session_id,
            tool_calls: progress.tool_calls(),
            usage,
            model,
        })
    }

    /// One ledger row per model call; with none reported, one for the turn
    /// when it reported usage.
    async fn record(
        &self,
        run: &AgentRun,
        progress: &Progress,
        reply: Option<&str>,
        usage: Option<TurnUsage>,
        started: Instant,
    ) {
        let Some(ledger) = &self.ledger else {
            return;
        };
        let prompt = vec![LedgerMessage {
            role: Role::User,
            tool: None,
            text: cut(&run.prompt, MAX_MESSAGE_CHARS),
        }];
        let calls = progress.model_calls();
        if calls.is_empty() {
            let Some(usage) = usage else {
                return;
            };
            let mut row = InferenceInput::new(
                Lane::Mascot,
                &run.label,
                run.model.as_deref().unwrap_or("agent"),
                tokens(usage),
            );
            row.input = Some(prompt);
            row.output = reply.map(|reply| cut(reply, MAX_MESSAGE_CHARS));
            row.duration_ms = Some(millis(started.elapsed()));
            record_quietly(Some(ledger.as_ref()), row).await;
            return;
        }
        let last = calls.len() - 1;
        for (index, call) in calls.iter().enumerate() {
            let mut row = InferenceInput::new(Lane::Mascot, &run.label, &call.model, call.tokens());
            row.input = Some(
                call.input
                    .as_ref()
                    .map(ledger_messages)
                    .filter(|messages| !messages.is_empty())
                    .unwrap_or_else(|| prompt.clone()),
            );
            row.output = call
                .output
                .as_ref()
                .map(|output| cut(&value_text(output), MAX_MESSAGE_CHARS))
                .or_else(|| (index == last).then(|| reply.map(|r| cut(r, MAX_MESSAGE_CHARS)))?);
            record_quietly(Some(ledger.as_ref()), row).await;
        }
    }
}

#[async_trait]
impl AgentRunner for OhRunner {
    async fn run(&self, run: AgentRun) -> Result<AgentOutcome, RunError> {
        if run.origin == Origin::Public && !is_public(&run.agent) {
            return Err(RunError::Failed(format!(
                "a public turn runs only on teeny-reply or teeny-chat, not {:?}",
                run.agent
            )));
        }
        if run.origin != Origin::Operator && operator_only(&run.agent) {
            return Err(RunError::Failed(format!(
                "{:?} runs only the operator's turns, behind the bearer, not a {:?} one",
                run.agent, run.origin
            )));
        }
        let agent = self.agent(&run.agent)?;
        let tries = attempts(run.origin);
        let mut last = RunError::Failed("the turn was never attempted".into());
        for attempt in 1..=tries {
            match self.attempt(&agent, &run).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) => {
                    tracing::warn!(
                        label = %run.label,
                        attempt,
                        tries,
                        %error,
                        "[oh_runner] turn failed"
                    );
                    last = error;
                }
            }
        }
        Err(last)
    }
}

/// The authority a turn from `origin` runs with; `label` names a trusted
/// turn's job in the core's audit.
pub fn turn_origin(origin: Origin, label: &str) -> AgentTurnOrigin {
    let job_id = format!("{JOB_PREFIX}{label}");
    match origin {
        Origin::Public => Access::public()
            .turn_origin()
            .cloned()
            .expect("the public access states its origin"),
        Origin::Scheduled { .. } => AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::Cron,
        },
        Origin::Operator => AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::Workflow {
                require_approval: false,
            },
        },
    }
}

/// Whether `key` is an agent whose access is public. Only these may take a
/// public turn: an agent's tier is its own, not the turn's, so a trusted
/// agent under a public origin still holds its writes, and a write there
/// would park for an approval no one on a public channel can give.
fn is_public(key: &AgentKey) -> bool {
    matches!(key, AgentKey::Belt(Belt::Reply | Belt::Chat))
}

/// Whether `key` is a belt only the operator may start: it posts and
/// remembers, so no cron and no public turn may drive it.
fn operator_only(key: &AgentKey) -> bool {
    matches!(key, AgentKey::Belt(belt) if belt.operator_only())
}

/// How many times a turn from `origin` may be tried.
pub fn attempts(origin: Origin) -> u32 {
    match origin {
        Origin::Scheduled { once: false } => 2,
        Origin::Scheduled { once: true } | Origin::Public | Origin::Operator => 1,
    }
}

/// One model call, as reported.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCall {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
    /// The request messages, when the core captured them.
    pub input: Option<Value>,
    /// The completion, when the core captured it.
    pub output: Option<Value>,
}

impl ModelCall {
    fn tokens(&self) -> Tokens {
        tokens(TurnUsage {
            input: self.input_tokens,
            output: self.output_tokens,
            reasoning: self.reasoning_tokens,
            cached: self.cached_tokens,
        })
    }
}

/// What a turn's progress events add up to. Sub-agent events are ignored:
/// no Teeny agent can start one.
#[derive(Debug, Default)]
pub struct Progress {
    arguments: HashMap<String, Value>,
    calls: Vec<ToolCallRecord>,
    models: Vec<ModelCall>,
}

impl Progress {
    pub fn observe(&mut self, event: AgentProgress) {
        match event {
            AgentProgress::ToolCallStarted {
                call_id, arguments, ..
            } => {
                self.arguments.insert(call_id, arguments);
            }
            AgentProgress::ToolCallCompleted {
                call_id,
                tool_name,
                success,
                output,
                arguments,
                ..
            } => {
                let arguments = arguments
                    .or_else(|| self.arguments.remove(&call_id))
                    .unwrap_or(Value::Null);
                self.calls.push(ToolCallRecord {
                    name: tool_name,
                    arguments,
                    result: output,
                    is_error: !success,
                });
            }
            AgentProgress::ModelCallCompleted {
                model,
                subagent_task_id: None,
                input,
                output,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                reasoning_tokens,
                ..
            } => self.models.push(ModelCall {
                model,
                input_tokens,
                output_tokens,
                cached_tokens: cached_input_tokens,
                reasoning_tokens,
                input,
                output,
            }),
            _ => {}
        }
    }

    pub fn tool_calls(&self) -> Vec<ToolCallRecord> {
        self.calls.clone()
    }

    pub fn model_calls(&self) -> &[ModelCall] {
        &self.models
    }

    /// The model calls' tokens, summed; `None` when there were none.
    pub fn usage(&self) -> Option<TurnUsage> {
        (!self.models.is_empty()).then(|| {
            self.models
                .iter()
                .fold(TurnUsage::default(), |sum, call| TurnUsage {
                    input: sum.input + call.input_tokens,
                    output: sum.output + call.output_tokens,
                    reasoning: sum.reasoning + call.reasoning_tokens,
                    cached: sum.cached + call.cached_tokens,
                })
        })
    }

    /// The model that answered last.
    pub fn model(&self) -> Option<String> {
        self.models.last().map(|call| call.model.clone())
    }
}

fn tokens(usage: TurnUsage) -> Tokens {
    let count = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    Tokens {
        input: count(usage.input),
        output: count(usage.output),
        reasoning: count(usage.reasoning),
        cached: count(usage.cached),
        total: count(usage.input.saturating_add(usage.output)),
    }
}

fn millis(elapsed: Duration) -> i64 {
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

/// Captured request messages as ledger messages: no system prompt, the tail.
fn ledger_messages(input: &Value) -> Vec<LedgerMessage> {
    let messages: Vec<LedgerMessage> = input
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| {
            let role = match message["role"].as_str()? {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => return None,
            };
            Some(LedgerMessage {
                role,
                tool: message["name"].as_str().map(str::to_owned),
                text: cut(&value_text(&message["content"]), MAX_MESSAGE_CHARS),
            })
        })
        .collect();
    let skip = messages.len().saturating_sub(MAX_MESSAGES);
    messages.into_iter().skip(skip).collect()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "oh_runner_tests.rs"]
mod tests;
