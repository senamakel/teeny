//! CortexDB over HTTP: the `/v1/experience` and `/v1/recall` calls the
//! TypeScript service made through the `cortexdbai` SDK, with the same
//! headers and bodies.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use teeny_core::Secret;
use teeny_core::config::CortexConfig;

use crate::breaker::RecallBreaker;
use crate::memory::{ABOUT_QUERY, format_recall};
use crate::text::{cut_at, units};
use crate::{Experience, Memory, MemoryError, Person, Result};

#[path = "cortex_admin.rs"]
mod admin;
pub use admin::ForgetSelector;

/// The SDK's request timeout, for the operator's calls.
const TIMEOUT: Duration = Duration::from_secs(60);
/// What a recall pass reads.
const INCLUDE: [&str; 4] = ["events", "facts", "beliefs", "episodes"];
/// Token budgets: the person pass is half the general one.
const ABOUT_TOKENS: usize = 600;
const GENERAL_TOKENS: usize = 1_200;
/// Generous for prose, so the cap only bites when a budget was ignored: a
/// pack is mostly English and JSON, which run three to four characters a token.
const CHARS_PER_TOKEN: usize = 6;
/// What separates two memories in a pack.
const PACK_SEPARATOR: &str = "\n\n---\n\n";

/// What a recall may draw on: conversation, notes the account left itself,
/// and the account's own posts. Everything else in the scope is webhook
/// activity (raw like, repost and follow payloads, two thirds of it on
/// 2026-10-05), and a pack that fills with JSON about who liked what has no
/// room left for what anybody said.
pub const RECALL_LABELS: [&str; 12] = [
    "mention",
    "reply",
    "message",
    "agent_note",
    "chart_post",
    "morning_post",
    "moon_post",
    "quote_post",
    "lore_post",
    "article",
    "narrative_episode",
    "calendar_post",
];

/// How the client reads and writes: separate deadlines, the person pass, and
/// the recall breaker's settings. See `legacy/src/memory/cortex.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CortexOptions {
    /// Recall is on the turn's critical path, so it gets seconds: a reply
    /// that waits a minute on a recall which then times out is a reply
    /// drafted a minute late with no memory anyway.
    pub recall_timeout: Duration,
    /// Captures write after the turn and can wait.
    pub capture_timeout: Duration,
    /// Whether to run the second pack, about the person being answered, by
    /// their `@handle` label (under two seconds on the shared store, where
    /// the `observed_actor` filter it replaced never came back).
    pub person_pass: bool,
    /// How long recall stays off after one times out.
    pub cooldown: Duration,
    /// Recalls allowed in flight at once; past it a recall fails fast.
    pub max_in_flight: usize,
}

impl Default for CortexOptions {
    /// The SDK's minute for both calls, the person pass on, five minutes off
    /// after a timeout, two recalls at once.
    fn default() -> Self {
        Self {
            recall_timeout: TIMEOUT,
            capture_timeout: TIMEOUT,
            person_pass: true,
            cooldown: Duration::from_secs(5 * 60),
            max_in_flight: 2,
        }
    }
}

impl CortexOptions {
    /// `CORTEX_RECALL_TIMEOUT_MS`, `CORTEX_CAPTURE_TIMEOUT_MS`,
    /// `CORTEX_PERSON_RECALL`, `CORTEX_RECALL_COOLDOWN_MS`.
    pub fn from_config(config: &CortexConfig) -> Self {
        Self {
            recall_timeout: Duration::from_millis(config.recall_timeout_ms),
            capture_timeout: Duration::from_millis(config.capture_timeout_ms),
            person_pass: config.person_recall,
            cooldown: Duration::from_millis(config.recall_cooldown_ms),
            ..Self::default()
        }
    }
}

/// The store's token budget is a request, not a guarantee: a scope whose
/// derived layers were still being built answered a 1,200-token recall with
/// its entire event log, 3.9 million characters, which as a prompt is no
/// reply at all. Over `max_chars` (UTF-16 units), the pack is cut at a
/// memory boundary, newest-first order kept, so what survives is whole
/// memories; a single giant one is cut hard.
pub fn cap_pack(block: &str, max_chars: usize) -> String {
    if units(block) <= max_chars {
        return block.to_owned();
    }
    let separator = units(PACK_SEPARATOR);
    let mut kept: Vec<&str> = Vec::new();
    let mut length = 0;
    for item in block.split(PACK_SEPARATOR) {
        let added = units(item) + if kept.is_empty() { 0 } else { separator };
        if length + added > max_chars {
            break;
        }
        kept.push(item);
        length += added;
    }
    if kept.is_empty() {
        cut_at(block, max_chars).to_owned()
    } else {
        kept.join(PACK_SEPARATOR)
    }
}

/// The account's memory in CortexDB.
#[derive(Debug, Clone)]
pub struct CortexMemory {
    http: reqwest::Client,
    api_url: String,
    actor: String,
    bearer: Secret,
    scope: String,
    options: CortexOptions,
    breaker: RecallBreaker,
}

impl CortexMemory {
    /// A client for `api_url`, acting as `actor`, storing under `scope`, on
    /// [`CortexOptions::default`].
    pub fn new(api_url: &str, actor: &str, bearer: Secret, scope: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("teeny/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let options = CortexOptions::default();
        Ok(Self {
            http,
            api_url: api_url.trim_end_matches('/').to_owned(),
            actor: actor.to_owned(),
            bearer,
            scope: scope.to_owned(),
            breaker: RecallBreaker::new(options.cooldown, options.max_in_flight),
            options,
        })
    }

    /// The same client on `options`, with a breaker of its own.
    pub fn with_options(mut self, options: CortexOptions) -> Self {
        self.breaker = RecallBreaker::new(options.cooldown, options.max_in_flight);
        self.options = options;
        self
    }

    /// The client the configuration describes.
    pub fn from_config(config: &CortexConfig) -> Result<Self> {
        Ok(Self::new(
            &config.api_url,
            &config.actor,
            config.token.clone(),
            &config.scope,
        )?
        .with_options(CortexOptions::from_config(config)))
    }

    /// POSTs `body` within `deadline` and returns the answer's text, or the
    /// error envelope.
    async fn post(&self, path: &str, body: &Value, deadline: Duration) -> Result<String> {
        let sent = self
            .http
            .post(format!("{}{path}", self.api_url))
            .header("X-Cortex-Actor", &self.actor)
            .bearer_auth(self.bearer.expose())
            .timeout(deadline)
            .json(body)
            .send()
            .await;
        let response = sent.map_err(|error| timed(error, deadline))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| timed(error, deadline))?;
        if status.is_success() {
            Ok(text)
        } else {
            Err(api_error(status.as_u16(), &text))
        }
    }

    /// One recall pass. The person pass is filtered to the person's handle
    /// label, on half the general pass's budget; the general pass reads only
    /// [`RECALL_LABELS`]. Either way the pack is capped at its budget.
    async fn recall_pack(&self, query: &str, handle: Option<&str>) -> Result<String> {
        let tokens = if handle.is_some() {
            ABOUT_TOKENS
        } else {
            GENERAL_TOKENS
        };
        let filters = match handle {
            Some(handle) => json!({ "metadata": { "labels": handle } }),
            None => json!({ "metadata": { "labels": RECALL_LABELS } }),
        };
        let body = json!({
            "scope": self.scope,
            "view": "holistic",
            "query": query,
            "include": INCLUDE,
            "diagnostics": "none",
            "budgets": { "max_tokens": tokens },
            "filters": filters,
        });
        let answer = self
            .post("/v1/recall", &body, self.options.recall_timeout)
            .await;
        if answer.as_ref().is_err_and(MemoryError::is_timeout) {
            // A pass that timed out left a query running on the store, even
            // when the other pass answered: the breaker counts it.
            self.breaker.trip();
        }
        let pack: Pack = serde_json::from_str(&answer?)?;
        Ok(cap_pack(&pack.context_block, tokens * CHARS_PER_TOKEN))
    }

    async fn recall_once(&self, query: &str, person: Option<&Person>) -> Result<String> {
        // Only the handle can find a person: it is the label every exchange
        // with them carries. A person known by id alone is recalled about
        // generally.
        let person = person.filter(|person| person.username.is_some());
        let handle = person.and_then(Person::handle_label);
        if let Some(handle) = &handle
            && !self.options.person_pass
        {
            // The handle leads the query bare. Phrased as a subject ("About
            // @ana:", "Everything known about this person") the store treats
            // it as a request for that entity's dossier, a path that does
            // not come back.
            let general = self.recall_pack(&format!("{handle} {query}"), None).await?;
            return Ok(format_recall(None, "", &general));
        }
        // Two passes, settled rather than all-or-nothing: losing the slower
        // person pass costs the turn its history with this person, not its
        // general context too. Only when every pass that ran fails does the
        // recall fail, so the caller can say memory was unavailable instead
        // of reading the silence as "nothing is known".
        let about = async {
            match &handle {
                Some(handle) => self.recall_pack(ABOUT_QUERY, Some(handle)).await,
                None => Ok(String::new()),
            }
        };
        let (about, general) = futures::join!(about, self.recall_pack(query, None));
        let general = match (general, &about) {
            (Err(error), Err(_)) => return Err(error),
            (Err(error), Ok(_)) if handle.is_none() => return Err(error),
            (general, _) => general.unwrap_or_default(),
        };
        Ok(format_recall(person, &about.unwrap_or_default(), &general))
    }
}

/// A transport error, as the deadline it ran into when it was one.
fn timed(error: reqwest::Error, deadline: Duration) -> MemoryError {
    if error.is_timeout() {
        MemoryError::Timeout {
            after_ms: deadline.as_millis(),
        }
    } else {
        MemoryError::Transport(error)
    }
}

/// The part of a recall's stratified pack the account reads.
#[derive(Deserialize)]
struct Pack {
    #[serde(default)]
    context_block: String,
}

/// CortexDB's error body.
#[derive(Deserialize, Default)]
struct Envelope {
    error_code: Option<String>,
    message: Option<String>,
}

/// The error a refused call reads as, as the SDK builds it.
fn api_error(status: u16, body: &str) -> MemoryError {
    let envelope = if body.is_empty() {
        Envelope::default()
    } else {
        serde_json::from_str(body).unwrap_or_else(|_| Envelope {
            error_code: Some("UNKNOWN".to_owned()),
            message: Some(body.chars().take(500).collect()),
        })
    };
    MemoryError::Api {
        status,
        code: envelope.error_code.unwrap_or_else(|| "UNKNOWN".to_owned()),
        message: envelope
            .message
            .unwrap_or_else(|| "(no message)".to_owned()),
    }
}

#[async_trait]
impl Memory for CortexMemory {
    async fn capture(&self, experience: Experience) -> Result<()> {
        // `captured` returns at the durable accept point, ~10ms, with
        // indexing continuing in the background.
        match self
            .post(
                "/v1/experience?wait=captured",
                &experience.envelope(&self.scope),
                self.options.capture_timeout,
            )
            .await
        {
            Err(error) if error.is_idempotency_conflict() => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// Behind the breaker: refused without a call while recall is paused
    /// or too many are in flight, and paused for the cooldown after a
    /// timeout.
    async fn recall(&self, query: &str, person: Option<&Person>) -> Result<String> {
        let _permit = self.breaker.enter()?;
        let recalled = self.recall_once(query, person).await;
        if recalled.as_ref().is_err_and(MemoryError::is_timeout) {
            self.breaker.trip();
        }
        recalled
    }
}

#[cfg(test)]
#[path = "cortex_tests.rs"]
mod tests;
