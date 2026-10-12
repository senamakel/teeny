//! The journal's helpers, shared by every job that keeps a line in it: a
//! port of `summariseRecall` and `summariseResult` from
//! `legacy/src/journal/journal.ts`, the turn's tool steps as the journal
//! keeps them (`toolSteps` in `legacy/src/agent/graph.ts`), and a journal
//! write that never takes the turn down.

use teeny_store::Journal;
use teeny_store::journal::{ReflectionInput, ToolCall};

use crate::runner::ToolCallRecord;

/// The longest line a summary keeps, in UTF-16 units as the legacy counted.
const MAX_LINE: usize = 180;
/// The longest tool result a journal line keeps.
pub const MAX_RESULT: usize = 160;

/// `text` cut to `max` UTF-16 units, the last an ellipsis, as
/// `${text.slice(0, max - 1)}…` did.
fn ellipsis(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.to_owned();
    }
    let mut kept = String::new();
    let mut units = 0;
    for ch in text.chars() {
        if units + ch.len_utf16() > max - 1 {
            break;
        }
        units += ch.len_utf16();
        kept.push(ch);
    }
    kept.push('…');
    kept
}

/// Runs of whitespace as one space, trimmed.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `^[\s\-*•>#]+` and then `^\[[^\]]{0,40}\]\s*` removed.
fn strip_markers(line: &str) -> &str {
    let line = line.trim_start_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '-' | '*' | '•' | '>' | '#')
    });
    if let Some(rest) = line.strip_prefix('[')
        && let Some(close) = rest.find(']')
        && rest[..close].encode_utf16().count() <= 40
    {
        return rest[close + 1..].trim_start();
    }
    line
}

/// `^[A-Z][A-Z\s]*(?:@\S+)?:$`: a section header, structure not memory.
fn is_header(line: &str) -> bool {
    let Some(body) = line.strip_suffix(':') else {
        return false;
    };
    let (words, handle) = match body.find('@') {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let mut chars = words.chars();
    let starts = chars.next().is_some_and(|ch| ch.is_ascii_uppercase());
    let rest = chars.all(|ch| ch.is_ascii_uppercase() || ch.is_whitespace());
    let handle_ok =
        handle.is_none_or(|handle| !handle.is_empty() && !handle.chars().any(char::is_whitespace));
    starts && rest && handle_ok
}

/// A memory context block, cut down to the few lines a reader can take in.
///
/// CortexDB's block mixes prose with raw event records; only lines that read
/// as sentences are kept, so a block that is all plumbing summarises to
/// nothing.
pub fn summarise_recall(block: &str, max_lines: usize) -> Vec<String> {
    block
        .split('\n')
        .map(|line| flatten(strip_markers(line)))
        .filter(|line| !line.is_empty() && !is_header(line) && !line.starts_with('{'))
        .take(max_lines)
        .map(|line| ellipsis(&line, MAX_LINE))
        .collect()
}

/// A tool's output, as one line: the prose lines of a multi-line block
/// joined, or the flattened original when that leaves nothing.
pub fn summarise_result(text: &str, max: usize) -> String {
    let prose = if text.contains('\n') {
        summarise_recall(text, 3).join(" · ")
    } else {
        String::new()
    };
    let flat = flatten(if prose.is_empty() { text } else { &prose });
    ellipsis(&flat, max)
}

/// The turn's tool calls as the journal keeps them: the name, and one line
/// on what came back.
pub fn tool_steps(calls: &[ToolCallRecord]) -> Vec<ToolCall> {
    calls
        .iter()
        .map(|call| ToolCall {
            name: call.name.clone(),
            result: summarise_result(&call.result, MAX_RESULT),
        })
        .collect()
}

/// Whether a tool in this turn actually returned this media id. A model that
/// has seen the id format can produce one that fits it, and the store is
/// shared across turns, so "it exists" is not "it is yours".
pub fn produced_media_id(calls: &[ToolCallRecord], media_id: &str) -> bool {
    let quoted = format!("\"{media_id}\"");
    calls.iter().any(|call| call.result.contains(&quoted))
}

/// Writes a line; a failure is logged and nothing else. The journal is the
/// record, not the act.
pub async fn record_quietly(journal: &dyn Journal, entry: ReflectionInput) {
    let id = entry.id.clone();
    if let Err(error) = journal.record(entry).await {
        tracing::warn!(%error, ?id, "could not journal the turn");
    }
}

#[cfg(test)]
#[path = "journal_glue_tests.rs"]
mod tests;
