//! Who a memory is about, and the inputs the capture calls read.
//!
//! The post and chat-message types here are the few fields memory reads, not
//! the X client's or the Telegram bot's full types, so this crate depends on
//! neither.

use serde::{Deserialize, Serialize};

/// The human on the other side of a conversation, as CortexDB knows them.
///
/// CortexDB reads the segment before the first colon as the actor type and
/// accepts only user, agent, service, or system, so the channel is
/// namespaced inside the identifier: `user:x-<id>`, `user:telegram-<id>`.
/// The numeric id is used because a handle can change and an id cannot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub username: Option<String>,
}

impl Person {
    /// An X account, by its numeric id.
    pub fn x(author_id: &str, username: Option<&str>) -> Self {
        Self::chat("x", author_id, username)
    }

    /// A chat channel's user: `user:<channel>-<id>`.
    pub fn chat(channel: &str, author_id: &str, username: Option<&str>) -> Self {
        Self {
            id: format!("user:{channel}-{author_id}"),
            username: username.map(str::to_owned),
        }
    }

    /// `@handle`, the label a memory about this person carries.
    pub(crate) fn handle_label(&self) -> Option<String> {
        self.username
            .as_ref()
            .map(|username| format!("@{username}"))
    }
}

/// An X post, as memory reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPost {
    pub id: String,
    pub text: String,
    pub author_id: Option<String>,
    pub author_username: Option<String>,
    pub conversation_id: Option<String>,
    /// ISO; when the post was published.
    pub created_at: Option<String>,
}

impl MemoryPost {
    /// The author, when the post says who that is.
    pub fn person(&self) -> Option<Person> {
        let author_id = self.author_id.as_deref()?;
        Some(Person::x(author_id, self.author_username.as_deref()))
    }
}

/// A chat is a DM with the account or a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatKind {
    Private,
    Group,
}

impl ChatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Group => "group",
        }
    }
}

/// A message in a chat channel, as memory reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryChatMessage {
    /// Unique within the chat.
    pub id: String,
    /// `telegram`.
    pub channel: String,
    pub chat_id: String,
    pub chat_kind: ChatKind,
    pub text: String,
    pub author_id: String,
    pub author_username: Option<String>,
    /// ISO; when the message was sent.
    pub created_at: Option<String>,
}

impl MemoryChatMessage {
    /// The author.
    pub fn person(&self) -> Person {
        Person::chat(
            &self.channel,
            &self.author_id,
            self.author_username.as_deref(),
        )
    }
}
