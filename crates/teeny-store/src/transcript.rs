//! The verbatim recent history of every chat the account is in.
//!
//! Long-term memory is semantic and budgeted: it answers "what do I know
//! about this person and topic". It is the wrong tool for "what were the last
//! forty lines in this room", which is what makes a reply land in a
//! conversation rather than beside it. Telegram's Bot API has no way to read
//! history back, so the only transcript the account will ever have is the one
//! it keeps itself.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::mongo::{Indexes, index, unique};

/// The default collection, `TRANSCRIPT_COLLECTION`'s default.
pub const COLLECTION: &str = "telegram_transcript";

/// How many rows to read per query. Well above what a 5k-token budget can
/// hold, so the budget is the limit and this only guards against a chat with
/// a very long tail of one-word messages.
pub const MAX_ROWS: usize = 400;

/// Per-turn overhead for the "- who: " prefix and the line break.
const TURN_OVERHEAD_TOKENS: usize = 6;

/// Who said a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    /// The account itself.
    Bot,
    Person,
}

/// One remembered turn of a chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    pub from: Speaker,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub text: String,
}

/// One line of a chat as stored: a turn, plus where and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub chat_id: String,
    /// The channel's own message id, so a redelivered update is stored once.
    pub message_id: String,
    pub at: String,
    pub from: Speaker,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub text: String,
}

impl TranscriptEntry {
    /// The turn, without where and when.
    pub fn turn(&self) -> ChatTurn {
        ChatTurn {
            from: self.from,
            author_name: self.author_name.clone(),
            text: self.text.clone(),
        }
    }
}

/// A rough token count, good enough for a budget: four UTF-16 units per
/// token, which errs a little high for chat, the safe side of a budget.
pub fn estimate_tokens(text: &str) -> usize {
    text.encode_utf16().count().div_ceil(4)
}

/// Walks newest-first, keeping turns until the budget is spent, then returns
/// what was kept in reading order. A single turn larger than the whole budget
/// is still kept if it is the most recent one, so a long paste does not blank
/// the history entirely.
pub fn fit_to_budget(newest_first: Vec<ChatTurn>, max_tokens: usize) -> Vec<ChatTurn> {
    let mut kept = Vec::new();
    let mut spent = 0;
    for turn in newest_first {
        let cost = estimate_tokens(&turn.text) + TURN_OVERHEAD_TOKENS;
        if spent + cost > max_tokens && !kept.is_empty() {
            break;
        }
        spent += cost;
        kept.push(turn);
    }
    kept.reverse();
    kept
}

/// The transcript.
#[async_trait]
pub trait ChatTranscript: Send + Sync {
    /// Stores a line, once per chat and message id.
    async fn append(&self, entry: &TranscriptEntry) -> Result<()>;
    /// The most recent turns of a chat that fit the token budget, oldest
    /// first. `excluding` names the messages being answered, which are in
    /// the prompt in full already.
    async fn recent(
        &self,
        chat_id: &str,
        max_tokens: usize,
        excluding: &[String],
    ) -> Result<Vec<ChatTurn>>;
}

/// The in-process fallback: what the bot had before there was a store.
#[derive(Debug, Default)]
pub struct InMemoryChatTranscript {
    chats: Mutex<HashMap<String, Vec<TranscriptEntry>>>,
}

#[async_trait]
impl ChatTranscript for InMemoryChatTranscript {
    async fn append(&self, entry: &TranscriptEntry) -> Result<()> {
        let mut chats = self
            .chats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entries = chats.entry(entry.chat_id.clone()).or_default();
        if entries
            .iter()
            .any(|existing| existing.message_id == entry.message_id)
        {
            return Ok(());
        }
        entries.push(entry.clone());
        let excess = entries.len().saturating_sub(MAX_ROWS);
        entries.drain(..excess);
        Ok(())
    }

    async fn recent(
        &self,
        chat_id: &str,
        max_tokens: usize,
        excluding: &[String],
    ) -> Result<Vec<ChatTurn>> {
        let chats = self
            .chats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let newest_first = chats
            .get(chat_id)
            .into_iter()
            .flatten()
            .rev()
            .filter(|entry| !excluding.contains(&entry.message_id))
            .map(TranscriptEntry::turn)
            .collect();
        Ok(fit_to_budget(newest_first, max_tokens))
    }
}

/// The transcript in Mongo.
pub struct MongoChatTranscript {
    entries: Collection<Document>,
    indexes: Indexes,
}

impl MongoChatTranscript {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            entries: db.collection(name),
            indexes: Indexes::default(),
        }
    }

    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.entries
                    .create_indexes([
                        index(doc! { "chatId": 1, "at": -1 }),
                        unique(doc! { "chatId": 1, "messageId": 1 }),
                    ])
                    .await?;
                Ok(())
            })
            .await
    }
}

#[async_trait]
impl ChatTranscript for MongoChatTranscript {
    async fn append(&self, entry: &TranscriptEntry) -> Result<()> {
        self.ready().await?;
        // Telegram redelivers an update the bot did not acknowledge in time,
        // so a line already stored is left alone rather than doubled.
        self.entries
            .update_one(
                doc! { "chatId": &entry.chat_id, "messageId": &entry.message_id },
                doc! { "$setOnInsert": bson::to_document(entry)? },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    async fn recent(
        &self,
        chat_id: &str,
        max_tokens: usize,
        excluding: &[String],
    ) -> Result<Vec<ChatTurn>> {
        self.ready().await?;
        let mut query = doc! { "chatId": chat_id };
        if !excluding.is_empty() {
            query.insert("messageId", doc! { "$nin": excluding });
        }
        let rows: Vec<TranscriptEntry> = self
            .entries
            .clone_with_type()
            .find(query)
            .sort(doc! { "at": -1 })
            .limit(MAX_ROWS as i64)
            .await?
            .try_collect()
            .await?;
        Ok(fit_to_budget(
            rows.iter().map(TranscriptEntry::turn).collect(),
            max_tokens,
        ))
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
