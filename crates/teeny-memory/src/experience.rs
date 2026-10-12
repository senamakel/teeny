//! One memory to store, shaped exactly as the TypeScript service shaped it:
//! the same labels, the same idempotency keys, the same actor.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::person::{MemoryChatMessage, MemoryPost, Person};

/// The shape of a captured record is part of its identity. Bumping this
/// makes a new representation a new record rather than a 409 against the
/// old one, which is what CortexDB returns when a key is reused with a
/// different body.
pub const CAPTURE_VERSION: u32 = 2;

/// Who said a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// What is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Content {
    /// A chat turn.
    Message { role: Role, text: String },
    /// Opaque structured data.
    Json { data: Value },
}

/// One memory, without the scope the client adds.
#[derive(Debug, Clone, PartialEq)]
pub struct Experience {
    pub content: Content,
    pub labels: Vec<String>,
    /// RFC 3339.
    pub observed_at: String,
    /// The replay key: a retried capture is one record.
    pub idempotency_key: String,
    /// Who the memory is about, when it is about a person.
    pub observed_actor: Option<String>,
}

impl Experience {
    /// Something the account did: a post, a note. `x:<kind>:2:<id>`.
    pub fn activity(
        kind: &str,
        id: &str,
        payload: Value,
        person: Option<&Person>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut labels = labels(["x", "activity", kind]);
        labels.extend(person.and_then(Person::handle_label));
        Self {
            content: Content::Json {
                data: json!({ "kind": kind, "payload": payload }),
            },
            labels,
            observed_at: iso(now),
            idempotency_key: format!("x:{kind}:{CAPTURE_VERSION}:{id}"),
            observed_actor: person.map(|person| person.id.clone()),
        }
    }

    /// A mention is a message from a person, so it is stored as one: a chat
    /// turn whose actor is the author. Stored under the agent's own id
    /// instead, every conversation belongs to the bot and nobody can be told
    /// apart on recall.
    pub fn mention(post: &MemoryPost, now: DateTime<Utc>) -> Self {
        let person = post.person();
        Self {
            content: Content::Message {
                role: Role::User,
                text: post.text.clone(),
            },
            labels: post_labels(labels(["x", "mention"]), person.as_ref(), post),
            observed_at: post.created_at.clone().unwrap_or_else(|| iso(now)),
            idempotency_key: format!("x:mention:{CAPTURE_VERSION}:{}", post.id),
            observed_actor: person.map(|person| person.id),
        }
    }

    /// The account's own turn in the same conversation, kept about the same
    /// person.
    pub fn reply(
        input: &MemoryPost,
        reply: &str,
        posted_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Self {
        let person = input.person();
        let state = if posted_id.is_some() {
            "posted"
        } else {
            "dry-run"
        };
        Self {
            content: Content::Message {
                role: Role::Assistant,
                text: reply.to_owned(),
            },
            labels: post_labels(labels(["x", "reply", state]), person.as_ref(), input),
            observed_at: iso(now),
            idempotency_key: format!(
                "x:reply:{CAPTURE_VERSION}:{}:{}",
                input.id,
                posted_id.unwrap_or("dry-run")
            ),
            observed_actor: person.map(|person| person.id),
        }
    }

    /// A chat message, stored the way a mention is, in the same scope as
    /// everything from X. Somebody who asked about the roadmap on X last week
    /// and turns up in a Telegram group today is the same person; the
    /// channel is a label, not a scope.
    pub fn chat_message(message: &MemoryChatMessage, now: DateTime<Utc>) -> Self {
        let person = message.person();
        let channel = message.channel.as_str();
        Self {
            content: Content::Message {
                role: Role::User,
                text: message.text.clone(),
            },
            labels: chat_labels(
                labels([channel, "message", message.chat_kind.as_str()]),
                &person,
                message,
            ),
            observed_at: message.created_at.clone().unwrap_or_else(|| iso(now)),
            idempotency_key: format!(
                "{channel}:message:{CAPTURE_VERSION}:{}:{}",
                message.chat_id, message.id
            ),
            observed_actor: Some(person.id),
        }
    }

    /// The account's answer in a chat.
    pub fn chat_reply(
        message: &MemoryChatMessage,
        reply: &str,
        sent_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Self {
        let person = message.person();
        let channel = message.channel.as_str();
        let state = if sent_id.is_some() { "sent" } else { "dry-run" };
        Self {
            content: Content::Message {
                role: Role::Assistant,
                text: reply.to_owned(),
            },
            labels: chat_labels(labels([channel, "reply", state]), &person, message),
            observed_at: iso(now),
            idempotency_key: format!(
                "{channel}:reply:{CAPTURE_VERSION}:{}:{}:{}",
                message.chat_id,
                message.id,
                sent_id.unwrap_or("dry-run")
            ),
            observed_actor: Some(person.id),
        }
    }

    /// The `/v1/experience` envelope for `scope`, as the SDK builds it.
    pub fn envelope(&self, scope: &str) -> Value {
        let mut context = json!({ "observed_at": self.observed_at });
        if !self.labels.is_empty() {
            context["labels"] = json!(self.labels);
        }
        let mut envelope = json!({
            "scope": scope,
            "modality": "conversation",
            "content": self.content,
            "context": context,
            "idempotency_key": self.idempotency_key,
        });
        if let Some(actor) = &self.observed_actor {
            envelope["observed_actor"] = json!({ "id": actor, "type": "person" });
        }
        envelope
    }
}

fn labels<'a>(fixed: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    fixed.into_iter().map(str::to_owned).collect()
}

/// The author's handle and the thread, when known.
fn post_labels(mut labels: Vec<String>, person: Option<&Person>, post: &MemoryPost) -> Vec<String> {
    labels.extend(person.and_then(Person::handle_label));
    labels.extend(
        post.conversation_id
            .as_ref()
            .map(|id| format!("thread:{id}")),
    );
    labels
}

/// The author's handle and the chat.
fn chat_labels(
    mut labels: Vec<String>,
    person: &Person,
    message: &MemoryChatMessage,
) -> Vec<String> {
    labels.extend(person.handle_label());
    labels.push(format!("chat:{}-{}", message.channel, message.chat_id));
    labels
}

/// As `Date.prototype.toISOString` spells it.
pub(crate) fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
#[path = "experience_tests.rs"]
mod tests;
