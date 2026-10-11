//! `recall_memory` and `remember`: long-term memory, as
//! `legacy/src/agent/tools/memory.ts` offered it. The bodies are
//! teeny-memory's; this is the tool around them.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use teeny_memory::Memory;
use teeny_memory::tools::{NOTE_MAX_CHARS, NOTE_MAX_LABELS};

use super::{Access, HostTool, Tool, ToolResult, failed, parse};

#[derive(Deserialize)]
struct RecallArgs {
    query: String,
    #[serde(default)]
    about: Option<String>,
}

#[derive(Deserialize)]
struct RememberArgs {
    note: String,
    #[serde(default)]
    labels: Vec<String>,
}

/// `recall_memory`.
pub fn recall_memory(memory: Arc<dyn Memory>) -> Arc<dyn Tool> {
    HostTool::build(
        "recall_memory",
        "Search long-term memory, the one every part of the account shares, for prior interactions, product facts, tone, and past decisions. \
         Call this before answering anything about the project or a user the bot has spoken to before. \
         Pass `about` with an @handle to search what is known about, and what was said with, that particular person.",
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "A natural-language question about what should be remembered."
                },
                "about": {
                    "type": "string",
                    "pattern": "^@?\\w{1,15}$",
                    "description": "An X handle, e.g. \"@someone\", to focus the search on that person."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let memory = Arc::clone(&memory);
            async move {
                let args = match parse::<RecallArgs>(args) {
                    Ok(args) => args,
                    Err(refused) => return refused,
                };
                match teeny_memory::tools::recall_memory(
                    memory.as_ref(),
                    &args.query,
                    args.about.as_deref(),
                )
                .await
                {
                    Ok(text) => ToolResult::success(text),
                    Err(error) => failed(error),
                }
            }
        },
    )
}

/// `remember`. A write: what it stores is carried into every future turn.
pub fn remember(memory: Arc<dyn Memory>) -> Arc<dyn Tool> {
    HostTool::build(
        "remember",
        "Write a durable note to long-term memory, shared by every part of the account. Use for facts worth recalling in future turns: decisions \
         taken, promises made, user preferences, correction of a mistake.",
        json!({
            "type": "object",
            "properties": {
                "note": { "type": "string", "minLength": 1, "maxLength": NOTE_MAX_CHARS },
                "labels": { "type": "array", "items": { "type": "string" }, "maxItems": NOTE_MAX_LABELS }
            },
            "required": ["note"],
            "additionalProperties": false
        }),
        Access::Write,
        move |args| {
            let memory = Arc::clone(&memory);
            async move {
                let args = match parse::<RememberArgs>(args) {
                    Ok(args) => args,
                    Err(refused) => return refused,
                };
                match teeny_memory::tools::remember(memory.as_ref(), &args.note, &args.labels).await
                {
                    Ok(text) => ToolResult::success(text),
                    Err(error) => failed(error),
                }
            }
        },
    )
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
