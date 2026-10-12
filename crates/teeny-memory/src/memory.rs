//! The seam every caller holds: store a memory, recall what is known.

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::{Experience, MemoryChatMessage, MemoryPost, Person, Result};

/// The query of the first recall pass, the one about the person. Never
/// phrased as a subject ("Everything known about this person"): the store
/// reads that as a request for the entity's dossier, a path that did not
/// come back on the shared store.
pub const ABOUT_QUERY: &str = "What has this person said, and what did you say back?";

/// Long-term memory.
///
/// Two calls are the whole contract; the rest shape a capture exactly as the
/// TypeScript service did, so every implementation stores the same labels
/// and keys.
#[async_trait]
pub trait Memory: Send + Sync {
    /// Stores one memory. A key already stored is not a failure.
    async fn capture(&self, experience: Experience) -> Result<()>;

    /// What is known about `query`, and about `person` when given, as one
    /// context block: `ABOUT @handle:` then `GENERAL CONTEXT:`. Empty when
    /// nothing is known.
    async fn recall(&self, query: &str, person: Option<&Person>) -> Result<String>;

    /// Something the account did: a post, a note.
    async fn capture_activity(
        &self,
        kind: &str,
        id: &str,
        payload: Value,
        person: Option<&Person>,
    ) -> Result<()> {
        self.capture(Experience::activity(kind, id, payload, person, Utc::now()))
            .await
    }

    /// A mention, as a chat turn from the person who wrote it.
    async fn capture_mention(&self, post: &MemoryPost) -> Result<()> {
        self.capture(Experience::mention(post, Utc::now())).await
    }

    /// The account's reply to `input`; `posted_id` absent on a dry run.
    async fn capture_reply(
        &self,
        input: &MemoryPost,
        reply: &str,
        posted_id: Option<&str>,
    ) -> Result<()> {
        self.capture(Experience::reply(input, reply, posted_id, Utc::now()))
            .await
    }

    /// A chat message, as a turn from the person who wrote it.
    async fn capture_chat_message(&self, message: &MemoryChatMessage) -> Result<()> {
        self.capture(Experience::chat_message(message, Utc::now()))
            .await
    }

    /// The account's answer in a chat; `sent_id` absent on a dry run.
    async fn capture_chat_reply(
        &self,
        message: &MemoryChatMessage,
        reply: &str,
        sent_id: Option<&str>,
    ) -> Result<()> {
        self.capture(Experience::chat_reply(message, reply, sent_id, Utc::now()))
            .await
    }
}

/// The two passes of a recall as one block. They answer different
/// questions: what the account knows about this person, and what it knows
/// about the topic. One query over everything buries a quiet person's
/// history under general context.
pub fn format_recall(person: Option<&Person>, about: &str, general: &str) -> String {
    let mut sections = Vec::new();
    if !about.trim().is_empty() {
        let who = person
            .and_then(|person| person.username.as_ref())
            .map_or_else(
                || "this person".to_owned(),
                |username| format!("@{username}"),
            );
        sections.push(format!("ABOUT {who}:\n{about}"));
    }
    if !general.trim().is_empty() {
        sections.push(format!("GENERAL CONTEXT:\n{general}"));
    }
    sections.join("\n\n")
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
