//! The bodies of the agent's two memory tools. The tool wrappers (names,
//! JSON schemas, permission levels) live with the agent; these are what the
//! tools do.

use chrono::Utc;
use serde_json::json;

use crate::{Memory, MemoryError, Person, Result};

/// `recall_memory`'s answer when memory had nothing.
pub const NOTHING_RECALLED: &str = "No relevant memory was recalled.";
/// `remember`'s schema: a note of at most this many characters...
pub const NOTE_MAX_CHARS: usize = 2_000;
/// ...and at most this many labels.
pub const NOTE_MAX_LABELS: usize = 8;

/// `recall_memory`: search long-term memory for prior interactions, product
/// facts, tone, and past decisions; with `about`, an X handle, what is known
/// about and was said with that person.
///
/// Without `about`, a recall is a search over everything, and a quiet
/// person's history sinks under general context. With it, memory is asked
/// about that person the way a turn's own recall asks about whoever is being
/// answered: by the handle label every exchange with them carries, so the
/// handle alone finds them.
pub async fn recall_memory(
    memory: &dyn Memory,
    query: &str,
    about: Option<&str>,
) -> Result<String> {
    if query.is_empty() {
        return Err(MemoryError::InvalidInput(
            "query must not be empty".to_owned(),
        ));
    }
    let person = about.map(Person::handle).transpose()?;
    let context = memory.recall(query, person.as_ref()).await?;
    let context = context.trim();
    Ok(if context.is_empty() {
        NOTHING_RECALLED.to_owned()
    } else {
        context.to_owned()
    })
}

/// `remember`: write a durable note, stored as an `agent_note` activity.
/// Answers with the note's id.
pub async fn remember(memory: &dyn Memory, note: &str, labels: &[String]) -> Result<String> {
    // The schema counts as JavaScript does, in UTF-16 units.
    let length = note.encode_utf16().count();
    if length == 0 || length > NOTE_MAX_CHARS {
        return Err(MemoryError::InvalidInput(format!(
            "note must be 1 to {NOTE_MAX_CHARS} characters"
        )));
    }
    if labels.len() > NOTE_MAX_LABELS {
        return Err(MemoryError::InvalidInput(format!(
            "at most {NOTE_MAX_LABELS} labels"
        )));
    }
    let id = format!("note:{}:{}", Utc::now().timestamp_millis(), random_suffix());
    memory
        .capture_activity(
            "agent_note",
            &id,
            json!({ "note": note, "labels": labels }),
            None,
        )
        .await?;
    Ok(format!("Stored as {id}."))
}

/// Eight base-36 characters, as `Math.random().toString(36).slice(2, 10)`.
/// Not for anything secret.
fn random_suffix() -> String {
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_i64(Utc::now().timestamp_nanos_opt().unwrap_or_default());
    let mut value = hasher.finish();
    (0..8)
        .map(|_| {
            let digit = (value % 36) as u32;
            value /= 36;
            char::from_digit(digit, 36).unwrap_or('0')
        })
        .collect()
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
