//! The inference ledger: one row per model call, and a running count of
//! every token ever spent.
//!
//! The journal records turns; this records calls, the proof underneath every
//! turn that a model actually ran. The system prompt is never stored: a row
//! carries a digest of the whole request, prompt included, so it commits to
//! the exact prompt without revealing it, and a digest of the prompt alone.

use std::collections::BTreeMap;
use std::ops::Add;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::journal::redact_handles;
use crate::mongo::{Indexes, index, unique};
use crate::support::{iso_now, new_id};

/// The default collection; the totals live in `<collection>_totals`.
pub const COLLECTION: &str = "inferences";

/// Longer than this and a message on the page is a wall, not a record.
pub const MAX_MESSAGE_CHARS: usize = 2_000;
/// A run's tool trail can be long; the page shows the tail.
pub const MAX_MESSAGES: usize = 12;

/// Who spent the tokens: the mascot's own surfaces, or the brain's agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Mascot,
    Brain,
}

impl Lane {
    pub const ALL: [Lane; 2] = [Lane::Mascot, Lane::Brain];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mascot => "mascot",
            Self::Brain => "brain",
        }
    }
}

/// A token count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default, deserialize_with = "crate::support::lenient::i64")]
    pub input: i64,
    #[serde(default, deserialize_with = "crate::support::lenient::i64")]
    pub output: i64,
    /// Output tokens the provider reported as reasoning. Part of `output`.
    #[serde(default, deserialize_with = "crate::support::lenient::i64")]
    pub reasoning: i64,
    /// Input tokens served from the provider's cache. Part of `input`.
    #[serde(default, deserialize_with = "crate::support::lenient::i64")]
    pub cached: i64,
    #[serde(default, deserialize_with = "crate::support::lenient::i64")]
    pub total: i64,
}

impl Add for Tokens {
    type Output = Tokens;

    fn add(self, other: Tokens) -> Tokens {
        Tokens {
            input: self.input + other.input,
            output: self.output + other.output,
            reasoning: self.reasoning + other.reasoning,
            cached: self.cached + other.cached,
            total: self.total + other.total,
        }
    }
}

/// Who said a message the model was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

/// One message the model was shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerMessage {
    pub role: Role,
    /// The tool a `tool` message answers, or the tools an `assistant` message called.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub text: String,
}

/// One model call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inference {
    pub id: String,
    pub at: String,
    pub lane: Lane,
    /// What the call was for: a surface on the mascot's lane (`reply`,
    /// `chart`, `judge`...), the agent's id on the brain's.
    pub source: String,
    /// The brain's own word for the call: `inference`, `planningCall`...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub model: String,
    /// Everything the model was shown except the system prompt. Absent on a
    /// brain row, which is an accounting sample, not a transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<LedgerMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// What came back: text, or the tool calls it asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub tokens: Tokens,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// sha256 of the exact request, system prompt included, and the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// sha256 of the system prompt alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_digest: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::support::lenient::opt_i64"
    )]
    pub duration_ms: Option<i64>,
}

/// A call to record; the id and the time are filled in when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferenceInput {
    pub id: Option<String>,
    pub at: Option<String>,
    pub lane: Lane,
    pub source: String,
    pub kind: Option<String>,
    pub model: String,
    pub input: Option<Vec<LedgerMessage>>,
    pub reasoning: Option<String>,
    pub output: Option<String>,
    pub tokens: Tokens,
    pub response_id: Option<String>,
    pub digest: Option<String>,
    pub prompt_digest: Option<String>,
    pub duration_ms: Option<i64>,
}

impl InferenceInput {
    /// A call with nothing but what every row needs.
    pub fn new(
        lane: Lane,
        source: impl Into<String>,
        model: impl Into<String>,
        tokens: Tokens,
    ) -> Self {
        Self {
            id: None,
            at: None,
            lane,
            source: source.into(),
            kind: None,
            model: model.into(),
            input: None,
            reasoning: None,
            output: None,
            tokens,
            response_id: None,
            digest: None,
            prompt_digest: None,
            duration_ms: None,
        }
    }

    fn complete(self) -> Inference {
        Inference {
            id: self.id.unwrap_or_else(|| new_id("i")),
            at: self.at.unwrap_or_else(iso_now),
            lane: self.lane,
            source: self.source,
            kind: self.kind,
            model: self.model,
            input: self.input,
            reasoning: self.reasoning,
            output: self.output,
            tokens: self.tokens,
            response_id: self.response_id,
            digest: self.digest,
            prompt_digest: self.prompt_digest,
            duration_ms: self.duration_ms,
        }
    }
}

/// What a lane, or a source, has spent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneTotals {
    #[serde(default, deserialize_with = "crate::support::lenient::i64")]
    pub calls: i64,
    #[serde(default)]
    pub tokens: Tokens,
    /// The oldest and newest call counted, ISO. Absent until one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

impl LaneTotals {
    fn fold(&mut self, entry: &Inference) {
        self.calls += 1;
        self.tokens = self.tokens + entry.tokens;
        if self.first.as_ref().is_none_or(|first| entry.at < *first) {
            self.first = Some(entry.at.clone());
        }
        if self.last.as_ref().is_none_or(|last| entry.at > *last) {
            self.last = Some(entry.at.clone());
        }
    }
}

/// Both lanes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneTotalsByLane {
    #[serde(default)]
    pub mascot: LaneTotals,
    #[serde(default)]
    pub brain: LaneTotals,
}

impl LaneTotalsByLane {
    pub fn lane_mut(&mut self, lane: Lane) -> &mut LaneTotals {
        match lane {
            Lane::Mascot => &mut self.mascot,
            Lane::Brain => &mut self.brain,
        }
    }
}

/// Everything ever spent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub lanes: LaneTotalsByLane,
    /// By lane and source, `mascot/reply`, `brain/scout`.
    pub sources: BTreeMap<String, LaneTotals>,
}

/// Which rows [`InferenceLedger::recent`] lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentFilter {
    pub lane: Option<Lane>,
    /// Only rows strictly before this time, ISO.
    pub before: Option<String>,
}

/// The ledger.
#[async_trait]
pub trait InferenceLedger: Send + Sync {
    /// Records a call, once, and counts it into the totals. Best-effort by
    /// contract: see [`record_quietly`].
    async fn record(&self, entry: InferenceInput) -> Result<()>;
    /// Newest first.
    async fn recent(&self, limit: usize, filter: &RecentFilter) -> Result<Vec<Inference>>;
    async fn totals(&self) -> Result<Totals>;
}

/// `mascot/reply`.
pub fn source_key(lane: Lane, source: &str) -> String {
    format!("{}/{source}", lane.as_str())
}

/// For tests and for running without Mongo. Keeps the last few thousand
/// rows; the totals keep everything.
#[derive(Debug)]
pub struct InMemoryLedger {
    state: Mutex<(Vec<Inference>, Totals)>,
    capacity: usize,
}

impl Default for InMemoryLedger {
    fn default() -> Self {
        Self::with_capacity(5_000)
    }
}

impl InMemoryLedger {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            state: Mutex::new((Vec::new(), Totals::default())),
            capacity,
        }
    }
}

#[async_trait]
impl InferenceLedger for InMemoryLedger {
    async fn record(&self, entry: InferenceInput) -> Result<()> {
        let inference = entry.complete();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (entries, sums) = &mut *state;
        if entries.iter().any(|existing| existing.id == inference.id) {
            return Ok(());
        }
        sums.lanes.lane_mut(inference.lane).fold(&inference);
        sums.sources
            .entry(source_key(inference.lane, &inference.source))
            .or_default()
            .fold(&inference);
        entries.insert(0, inference);
        entries.truncate(self.capacity);
        Ok(())
    }

    async fn recent(&self, limit: usize, filter: &RecentFilter) -> Result<Vec<Inference>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut rows: Vec<_> = state
            .0
            .iter()
            .filter(|row| filter.lane.is_none_or(|lane| row.lane == lane))
            .filter(|row| filter.before.as_ref().is_none_or(|before| row.at < *before))
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.at.cmp(&a.at));
        rows.truncate(limit);
        Ok(rows)
    }

    async fn totals(&self) -> Result<Totals> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .1
            .clone())
    }
}

/// The ledger in Mongo. Rows go in one collection; the totals are one
/// document updated with `$inc` on every row, so the counter on the page is
/// one read whatever the row count, and it keeps counting after old rows are
/// gone. A source key carries a slash, which Mongo allows in a field name.
pub struct MongoLedger {
    rows: Collection<Document>,
    sums: Collection<Document>,
    indexes: Indexes,
}

impl MongoLedger {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            rows: db.collection(name),
            sums: db.collection(&format!("{name}_totals")),
            indexes: Indexes::default(),
        }
    }

    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.rows
                    .create_indexes([
                        unique(doc! { "id": 1 }),
                        index(doc! { "at": -1 }),
                        index(doc! { "lane": 1, "at": -1 }),
                    ])
                    .await?;
                Ok(())
            })
            .await
    }
}

/// The totals document as Mongo holds it.
#[derive(Deserialize)]
struct TotalsDoc {
    #[serde(default)]
    lanes: BTreeMap<String, LaneTotals>,
    #[serde(default)]
    sources: BTreeMap<String, LaneTotals>,
}

#[async_trait]
impl InferenceLedger for MongoLedger {
    async fn record(&self, entry: InferenceInput) -> Result<()> {
        self.ready().await?;
        let inference = entry.complete();
        // Insert-if-absent, and count only what was inserted: a retried fold
        // of the brain's samples must not count a sample twice.
        let result = self
            .rows
            .update_one(
                doc! { "id": &inference.id },
                doc! { "$setOnInsert": bson::to_document(&inference)? },
            )
            .upsert(true)
            .await?;
        if result.upserted_id.is_none() {
            return Ok(());
        }
        let (mut inc, mut min, mut max) = (Document::new(), Document::new(), Document::new());
        let tokens = inference.tokens;
        for prefix in [
            format!("lanes.{}", inference.lane.as_str()),
            format!("sources.{}", source_key(inference.lane, &inference.source)),
        ] {
            inc.insert(format!("{prefix}.calls"), 1_i64);
            for (field, value) in [
                ("input", tokens.input),
                ("output", tokens.output),
                ("reasoning", tokens.reasoning),
                ("cached", tokens.cached),
                ("total", tokens.total),
            ] {
                inc.insert(format!("{prefix}.tokens.{field}"), value);
            }
            min.insert(format!("{prefix}.first"), &inference.at);
            max.insert(format!("{prefix}.last"), &inference.at);
        }
        self.sums
            .update_one(
                doc! { "_id": "totals" },
                doc! { "$inc": inc, "$min": min, "$max": max },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    async fn recent(&self, limit: usize, filter: &RecentFilter) -> Result<Vec<Inference>> {
        self.ready().await?;
        let mut query = Document::new();
        if let Some(lane) = filter.lane {
            query.insert("lane", lane.as_str());
        }
        if let Some(before) = &filter.before {
            query.insert("at", doc! { "$lt": before });
        }
        let rows = self
            .rows
            .clone_with_type::<Inference>()
            .find(query)
            .sort(doc! { "at": -1 })
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .await?
            .try_collect()
            .await?;
        Ok(rows)
    }

    async fn totals(&self) -> Result<Totals> {
        self.ready().await?;
        let mut totals = Totals::default();
        let Some(doc) = self
            .sums
            .clone_with_type::<TotalsDoc>()
            .find_one(doc! { "_id": "totals" })
            .await?
        else {
            return Ok(totals);
        };
        for lane in Lane::ALL {
            if let Some(stored) = doc.lanes.get(lane.as_str()) {
                *totals.lanes.lane_mut(lane) = stored.clone();
            }
        }
        totals.sources = doc.sources;
        Ok(totals)
    }
}

/// Records a row and swallows the failure, logging it. A ledger that is
/// down costs a line in the log, never a reply.
pub async fn record_quietly(ledger: Option<&dyn InferenceLedger>, entry: InferenceInput) {
    let Some(ledger) = ledger else { return };
    if let Err(error) = ledger.record(entry).await {
        tracing::warn!(%error, "inference ledger write failed");
    }
}

/// `text` without carriage returns, trimmed, and cut to `max` characters,
/// the last an ellipsis.
pub fn cut(text: &str, max: usize) -> String {
    let flat = text.replace('\r', "");
    let flat = flat.trim();
    if flat.chars().count() <= max {
        return flat.to_owned();
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The hex sha256 of the parts, concatenated.
pub fn sha256(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A raw OpenAI-shaped `usage` block read into a [`Tokens`]. The details
/// blocks are optional on the wire and absent from most compatible
/// endpoints, and count as zero then. LangChain's own spelling
/// (`input_tokens`, `output_token_details.reasoning`) is read as a fallback.
pub fn tokens_from_usage(usage: Option<&serde_json::Value>) -> Tokens {
    use serde_json::Value;

    // `a ?? b`: the first that is there at all, usable or not.
    fn first<'a>(candidates: impl IntoIterator<Item = Option<&'a Value>>) -> Option<&'a Value> {
        candidates
            .into_iter()
            .flatten()
            .find(|value| !value.is_null())
    }
    fn n(value: Option<&Value>) -> i64 {
        value
            .and_then(Value::as_f64)
            .filter(|number| number.is_finite())
            .map_or(0, |number| number as i64)
    }
    let field = |block: &str, key: &str| {
        usage
            .and_then(|u| u.get(block))
            .filter(|d| d.is_object())
            .and_then(|d| d.get(key))
    };
    let top = |key: &str| usage.and_then(|u| u.get(key));

    let input = n(first([top("prompt_tokens"), top("input_tokens")]));
    let output = n(first([top("completion_tokens"), top("output_tokens")]));
    let reasoning = n(first([
        field("completion_tokens_details", "reasoning_tokens"),
        field("output_tokens_details", "reasoning_tokens"),
        field("output_token_details", "reasoning"),
    ]));
    let cached = n(first([
        field("prompt_tokens_details", "cached_tokens"),
        field("input_tokens_details", "cached_tokens"),
        field("input_token_details", "cache_read"),
    ]));
    let total = match n(top("total_tokens")) {
        0 => input + output,
        total => total,
    };
    Tokens {
        input,
        output,
        reasoning,
        cached,
        total,
    }
}

/// The public shape of a row: handles blanked, nothing else changed.
pub fn redact_inference(entry: &Inference) -> Inference {
    Inference {
        input: entry.input.as_ref().map(|messages| {
            messages
                .iter()
                .map(|message| LedgerMessage {
                    text: redact_handles(&message.text),
                    ..message.clone()
                })
                .collect()
        }),
        reasoning: entry.reasoning.as_deref().map(redact_handles),
        output: entry.output.as_deref().map(redact_handles),
        ..entry.clone()
    }
}

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;
