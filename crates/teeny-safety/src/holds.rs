//! The record of everything the content filter stopped, kept for a person to
//! look at afterwards.
//!
//! The filter is deliberately blunt, and a blunt filter is wrong sometimes.
//! The public journal says only that something was held; this log, which
//! never leaves the box, keeps the text, so an operator can say whether the
//! filter was right, and an approval acts: a held link's host joins the live
//! allowlist, a held mention is requeued, a withheld draft is released
//! (see [`crate::review`]).
//!
//! Every write here is best-effort from the gate's point of view: a log that
//! cannot be reached must never turn a hold into a publish.
//! A port of `legacy/src/safety/holds.ts`; the Mongo implementation of
//! [`HoldLog`] lives in `teeny-store`, writing the same document shape.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::links::RemovedLink;
use crate::screen::Direction;

/// Where a hold is in review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldStatus {
    Pending,
    Approved,
    Rejected,
}

/// What was held. `Message` is somebody else's text on the way in; `Draft`
/// the account's own on the way out; `Link` a draft that went out with links
/// cut, so the cut links are under review; `Prompt` a picture the account
/// declined to draw; `Task` an operator's ad-hoc task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldKind {
    Message,
    Draft,
    Link,
    Prompt,
    Task,
}

/// Where the held text was headed or came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldSurface {
    X,
    Telegram,
    Task,
    Chart,
    Calendar,
    Morning,
    Moon,
    Narrative,
    Article,
    Quote,
    Lore,
}

/// Which check held it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeldBy {
    Screen,
    Judge,
    Links,
}

/// An operator's answer to a hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Rejected,
}

impl Decision {
    fn status(self) -> HoldStatus {
        match self {
            Self::Approved => HoldStatus::Approved,
            Self::Rejected => HoldStatus::Rejected,
        }
    }
}

/// Where the held text came from, enough to act on an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldContext {
    pub surface: HoldSurface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<HoldKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

impl HoldContext {
    /// A context naming only the surface.
    pub fn new(surface: HoldSurface) -> Self {
        Self {
            surface,
            kind: None,
            post_id: None,
            conversation_id: None,
            author_username: None,
            chat_id: None,
            message_id: None,
            message_thread_id: None,
            author_name: None,
            tool: None,
        }
    }
}

/// A hold as the gate writes it, before the log stamps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldInput {
    pub direction: Direction,
    pub kind: HoldKind,
    pub context: HoldContext,
    /// The lexicon category, the judge's category, `links`, or `unknown`.
    pub category: String,
    pub reason: String,
    pub by: HeldBy,
    /// The held text itself. Private: never served publicly.
    pub text: String,
    /// For a `Link` hold, what was cut.
    pub links: Option<Vec<RemovedLink>>,
}

/// One entry of the hold log, in the legacy collection's document shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    pub id: String,
    #[serde(with = "js_time")]
    pub at: DateTime<Utc>,
    pub direction: Direction,
    pub kind: HoldKind,
    pub context: HoldContext,
    pub category: String,
    pub reason: String,
    pub by: HeldBy,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<RemovedLink>>,
    pub status: HoldStatus,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "js_time::option")]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// What the approval did: "requeued", "allowlisted x.example", ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}

/// `h-<ms in base 36>-<six random base-36 digits>`, as the legacy log spelled them.
fn hold_id(at: DateTime<Utc>) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    at.timestamp_nanos_opt().hash(&mut hasher);
    let noise = base36(hasher.finish());
    let ms = u64::try_from(at.timestamp_millis()).unwrap_or_default();
    format!("h-{}-{}", base36(ms), &noise[noise.len().saturating_sub(6)..])
}

impl Hold {
    /// A new pending hold, stamped `at`.
    pub fn new(input: HoldInput, at: DateTime<Utc>) -> Self {
        Self {
            id: hold_id(at),
            at,
            direction: input.direction,
            kind: input.kind,
            context: input.context,
            category: input.category,
            reason: input.reason,
            by: input.by,
            text: input.text,
            links: input.links,
            status: HoldStatus::Pending,
            decided_at: None,
            note: None,
            effect: None,
        }
    }
}

/// How many holds are in each status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HoldCounts {
    pub pending: u64,
    pub approved: u64,
    pub rejected: u64,
}

/// The log could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("hold log: {0}")]
pub struct HoldLogError(String);

impl HoldLogError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Where holds are written for review, and where approved links are read.
#[async_trait]
pub trait HoldLog: Send + Sync {
    async fn record(&self, entry: HoldInput) -> Result<Hold, HoldLogError>;
    /// Newest first; `limit` defaults to 100.
    async fn list(&self, status: Option<HoldStatus>, limit: Option<usize>) -> Result<Vec<Hold>, HoldLogError>;
    async fn get(&self, id: &str) -> Result<Option<Hold>, HoldLogError>;
    async fn decide(
        &self,
        id: &str,
        decision: Decision,
        note: Option<String>,
        effect: Option<String>,
    ) -> Result<Option<Hold>, HoldLogError>;
    async fn counts(&self) -> Result<HoldCounts, HoldLogError>;
    /// Hosts an operator approved, in allowlist spelling. The gate reads these
    /// alongside the built-in list, so an approval takes effect everywhere.
    async fn allowed_links(&self) -> Result<Vec<String>, HoldLogError>;
    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<(), HoldLogError>;
}

/// The log in memory, for tests and for running without Mongo.
#[derive(Default)]
pub struct MemoryHoldLog {
    entries: Mutex<Vec<Hold>>,
    /// `(entry, approving hold id)`, in the order they were approved.
    links: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl HoldLog for MemoryHoldLog {
    async fn record(&self, entry: HoldInput) -> Result<Hold, HoldLogError> {
        let hold = Hold::new(entry, Utc::now());
        self.entries.lock().expect("not poisoned").insert(0, hold.clone());
        Ok(hold)
    }

    async fn list(&self, status: Option<HoldStatus>, limit: Option<usize>) -> Result<Vec<Hold>, HoldLogError> {
        let entries = self.entries.lock().expect("not poisoned");
        Ok(entries
            .iter()
            .filter(|hold| status.is_none_or(|wanted| hold.status == wanted))
            .take(limit.unwrap_or(100))
            .cloned()
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<Hold>, HoldLogError> {
        Ok(self.entries.lock().expect("not poisoned").iter().find(|hold| hold.id == id).cloned())
    }

    async fn decide(
        &self,
        id: &str,
        decision: Decision,
        note: Option<String>,
        effect: Option<String>,
    ) -> Result<Option<Hold>, HoldLogError> {
        let mut entries = self.entries.lock().expect("not poisoned");
        let Some(hold) = entries.iter_mut().find(|hold| hold.id == id) else { return Ok(None) };
        hold.status = decision.status();
        hold.decided_at = Some(Utc::now());
        if note.is_some() {
            hold.note = note;
        }
        if effect.is_some() {
            hold.effect = effect;
        }
        Ok(Some(hold.clone()))
    }

    async fn counts(&self) -> Result<HoldCounts, HoldLogError> {
        let mut counts = HoldCounts::default();
        for hold in self.entries.lock().expect("not poisoned").iter() {
            match hold.status {
                HoldStatus::Pending => counts.pending += 1,
                HoldStatus::Approved => counts.approved += 1,
                HoldStatus::Rejected => counts.rejected += 1,
            }
        }
        Ok(counts)
    }

    async fn allowed_links(&self) -> Result<Vec<String>, HoldLogError> {
        Ok(self.links.lock().expect("not poisoned").iter().map(|(entry, _)| entry.clone()).collect())
    }

    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<(), HoldLogError> {
        let mut links = self.links.lock().expect("not poisoned");
        if !links.iter().any(|(existing, _)| existing == entry) {
            links.push((entry.to_owned(), hold_id.to_owned()));
        }
        Ok(())
    }
}

/// Timestamps as JavaScript's `toISOString()` writes them, millisecond
/// precision and a `Z`, so the documents sort and compare as the legacy ones.
pub mod js_time {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(at: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&at.to_rfc3339_opts(SecondsFormat::Millis, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&text).map(|at| at.to_utc()).map_err(serde::de::Error::custom)
    }

    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(at: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error> {
            match at {
                Some(at) => super::serialize(at, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .map(|text| DateTime::parse_from_rfc3339(&text).map(|at| at.to_utc()))
                .transpose()
                .map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(test)]
#[path = "holds_tests.rs"]
mod tests;
