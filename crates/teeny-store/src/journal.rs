//! The journal: one row per turn of the account's mind, as published on the
//! landing page.
//!
//! This is the record of what the account *did*, kept apart from what it
//! *remembers*: memory is what it carries into the next conversation and is
//! deliberately selective, while the journal is the audit trail and keeps
//! everything, including the turns where it decided to say nothing.

use std::sync::Mutex;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::mongo::{Indexes, index, unique};
use crate::support::{iso_now, new_id};

/// The default collection.
pub const COLLECTION: &str = "reflections";

/// What started the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Mention,
    Chart,
    Morning,
    Moon,
    Calendar,
    Task,
    Narrative,
    Article,
    Quote,
    Lore,
}

/// How the incoming text read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Friendly,
    Neutral,
    Negative,
    Hostile,
}

/// What became of the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Posted,
    DryRun,
    Skipped,
    Held,
    Failed,
}

/// A tool the turn called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    /// One line on what came back, never the raw payload.
    pub result: String,
}

/// One turn of the account's mind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reflection {
    pub id: String,
    pub at: String,
    pub trigger: Trigger,
    /// What arrived. Handles are redacted before this leaves the box.
    pub incoming: String,
    pub tone: Tone,
    /// What memory said, summarised to a few lines.
    pub recalled: Vec<String>,
    pub tools: Vec<ToolCall>,
    /// The reasoning, in the account's own words.
    pub thinking: String,
    /// The reply, chart caption, or task output. Empty when it stayed quiet.
    pub said: String,
    /// Things it decided against, and why.
    pub declined: Vec<String>,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_note: Option<String>,
}

/// A turn to record; the id and the time are filled in when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionInput {
    pub id: Option<String>,
    pub at: Option<String>,
    pub trigger: Trigger,
    pub incoming: String,
    pub tone: Tone,
    pub recalled: Vec<String>,
    pub tools: Vec<ToolCall>,
    pub thinking: String,
    pub said: String,
    pub declined: Vec<String>,
    pub outcome: Outcome,
    pub outcome_note: Option<String>,
}

impl ReflectionInput {
    fn complete(self) -> Reflection {
        Reflection {
            id: self.id.unwrap_or_else(|| new_id("r")),
            at: self.at.unwrap_or_else(iso_now),
            trigger: self.trigger,
            incoming: self.incoming,
            tone: self.tone,
            recalled: self.recalled,
            tools: self.tools,
            thinking: self.thinking,
            said: self.said,
            declined: self.declined,
            outcome: self.outcome,
            outcome_note: self.outcome_note,
        }
    }
}

/// The journal.
#[async_trait]
pub trait Journal: Send + Sync {
    /// Records a turn, once: a turn already recorded under the same id is
    /// left alone. Callers treat a failure as best-effort; a journal that
    /// cannot be written must never take a reply down.
    async fn record(&self, entry: ReflectionInput) -> Result<()>;
    /// Newest first.
    async fn recent(&self, limit: usize) -> Result<Vec<Reflection>>;
}

/// For tests and for running without Mongo. Keeps the last few hundred.
#[derive(Debug)]
pub struct InMemoryJournal {
    entries: Mutex<Vec<Reflection>>,
    capacity: usize,
}

impl Default for InMemoryJournal {
    fn default() -> Self {
        Self::with_capacity(500)
    }
}

impl InMemoryJournal {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            capacity,
        }
    }
}

#[async_trait]
impl Journal for InMemoryJournal {
    async fn record(&self, entry: ReflectionInput) -> Result<()> {
        let reflection = entry.complete();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if entries.iter().any(|existing| existing.id == reflection.id) {
            return Ok(());
        }
        entries.insert(0, reflection);
        entries.truncate(self.capacity);
        Ok(())
    }

    async fn recent(&self, limit: usize) -> Result<Vec<Reflection>> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        entries.sort_by(|a, b| b.at.cmp(&a.at));
        entries.truncate(limit);
        Ok(entries)
    }
}

/// The journal in the same Mongo the mention queue lives in.
pub struct MongoJournal {
    rows: Collection<Document>,
    indexes: Indexes,
}

impl MongoJournal {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            rows: db.collection(name),
            indexes: Indexes::default(),
        }
    }

    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.rows
                    .create_indexes([unique(doc! { "id": 1 }), index(doc! { "at": -1 })])
                    .await?;
                Ok(())
            })
            .await
    }
}

#[async_trait]
impl Journal for MongoJournal {
    async fn record(&self, entry: ReflectionInput) -> Result<()> {
        self.ready().await?;
        let reflection = entry.complete();
        // Insert-if-absent, so a retried turn cannot journal itself twice.
        self.rows
            .update_one(
                doc! { "id": &reflection.id },
                doc! { "$setOnInsert": bson::to_document(&reflection)? },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    async fn recent(&self, limit: usize) -> Result<Vec<Reflection>> {
        self.ready().await?;
        let rows = self
            .rows
            .clone_with_type::<Reflection>()
            .find(doc! {})
            .sort(doc! { "at": -1 })
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .await?
            .try_collect()
            .await?;
        Ok(rows)
    }
}

/// Blanks every `@handle`. The page shows what the account thought, not who
/// it was talking to: a handle is somebody else's name on a page they never
/// asked to be on. As `/@[A-Za-z0-9_]{1,15}\b/g` does, a run longer than X
/// allows is left alone.
pub fn redact_handles(text: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let run = after.find(|c: char| !is_word(c)).unwrap_or(after.len());
        if (1..=15).contains(&run) {
            out.push_str("@…");
            rest = &after[run..];
        } else {
            out.push('@');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The public shape of a reflection: every handle blanked.
pub fn redact_reflection(reflection: &Reflection) -> Reflection {
    Reflection {
        incoming: redact_handles(&reflection.incoming),
        said: redact_handles(&reflection.said),
        thinking: redact_handles(&reflection.thinking),
        recalled: reflection
            .recalled
            .iter()
            .map(|line| redact_handles(line))
            .collect(),
        declined: reflection
            .declined
            .iter()
            .map(|line| redact_handles(line))
            .collect(),
        tools: reflection
            .tools
            .iter()
            .map(|call| ToolCall {
                name: call.name.clone(),
                result: redact_handles(&call.result),
            })
            .collect(),
        ..reflection.clone()
    }
}

/// A recalled line on the page is cut at this many characters.
const MAX_LINE: usize = 180;

/// How many lines of a memory block the page shows by default.
pub const RECALL_LINES: usize = 4;

/// How long a tool result's one line runs by default.
pub const RESULT_CHARS: usize = 160;

/// A memory context block, cut down to the few prose lines a reader can
/// take in.
///
/// CortexDB's block mixes prose with raw event records --
/// `{"redacted":true}`, `{"RecordAsserted":{...}}` -- and the records are
/// memory's plumbing, not what it remembers. Only lines that read as
/// sentences are kept; a block that is all plumbing summarises to nothing.
pub fn summarise_recall(block: &str, max_lines: usize) -> Vec<String> {
    block
        .split('\n')
        .map(clean_line)
        // Section headers like "ABOUT @fren:" are structure, not memory, and
        // a line opening with `{` is a raw record. A leading "[assistant]"
        // is a role tag and stays.
        .filter(|line| !line.is_empty() && !is_header(line) && !line.starts_with('{'))
        .take(max_lines)
        .map(|line| truncate(&line, MAX_LINE))
        .collect()
}

/// A tool's output as one line: a string as itself, anything else as JSON,
/// nothing as nothing.
pub fn summarise_result(output: Option<&serde_json::Value>, max: usize) -> String {
    match output {
        None => String::new(),
        Some(serde_json::Value::String(text)) => summarise_text(text, max),
        Some(value) => summarise_text(&value.to_string(), max),
    }
}

/// A tool's text output as one line. Multi-line output is a context block or
/// a listing, and gets the same treatment as recall: the prose lines, joined,
/// with the raw records left out. Only when that leaves nothing does the
/// flattened original stand in.
pub fn summarise_text(text: &str, max: usize) -> String {
    let prose = if text.contains('\n') {
        summarise_recall(text, 3).join(" · ")
    } else {
        String::new()
    };
    let flat = collapse_whitespace(if prose.is_empty() { text } else { &prose });
    truncate(&flat, max)
}

/// Strips list markers and a leading `[2026-09-02 07:47 UTC]` stamp, and
/// collapses whitespace.
fn clean_line(line: &str) -> String {
    let line = line.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, '-' | '*' | '•' | '>' | '#')
    });
    collapse_whitespace(strip_stamp(line))
}

/// `^\[[^\]]{0,40}\]\s*`: a bracketed stamp of up to 40 characters.
fn strip_stamp(line: &str) -> &str {
    let Some(inside) = line.strip_prefix('[') else {
        return line;
    };
    let Some(close) = inside.find(']') else {
        return line;
    };
    if inside[..close].chars().count() > 40 {
        return line;
    }
    inside[close + 1..].trim_start()
}

/// `^[A-Z][A-Z\s]*(?:@\S+)?:$`, e.g. `ABOUT @fren:` or `GENERAL CONTEXT:`.
fn is_header(line: &str) -> bool {
    let Some(body) = line.strip_suffix(':') else {
        return false;
    };
    if !body.starts_with(|c: char| c.is_ascii_uppercase()) {
        return false;
    }
    let rest = body.trim_start_matches(|c: char| c.is_ascii_uppercase() || c.is_whitespace());
    match rest.strip_prefix('@') {
        _ if rest.is_empty() => true,
        Some(handle) => !handle.is_empty() && !handle.contains(char::is_whitespace),
        None => false,
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut to `max` characters, the last one an ellipsis.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
