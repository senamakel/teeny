//! The record of everything the content filter stopped, kept for a person
//! to look at afterwards.
//!
//! The filter is deliberately blunt, and a blunt filter is wrong sometimes.
//! The public journal says only that something was held; this log, which
//! never leaves the box, keeps the text, so an operator can say whether the
//! filter was right. Approving a held link puts its host on the live
//! allowlist, approving a held mention sends it back to the queue, approving
//! a withheld draft releases it. Rejecting confirms the filter.
//!
//! Every write here is best-effort from the gate's point of view: a log that
//! cannot be reached must never turn a hold into a publish.

use std::sync::Mutex;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::options::ReturnDocument;
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::mongo::{Indexes, group_counts, index, unique};
use crate::support::{iso_now, new_id};

/// The default collection, `HOLDS_COLLECTION`'s default; approved hosts
/// live in `<collection>_links`.
pub const COLLECTION: &str = "holds";
/// How many holds a list returns by default, and at most.
pub const LIST_DEFAULT: usize = 100;
pub const LIST_MAX: usize = 500;

/// Where a hold is in review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldStatus {
    Pending,
    Approved,
    Rejected,
}

/// An operator's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldDecision {
    Approved,
    Rejected,
}

/// What was held. `message` is somebody else's text on the way in; `draft`
/// the account's own on the way out; `link` a draft that went out with its
/// links cut; `prompt` a picture the account declined to draw; `task` an
/// operator's ad-hoc task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldKind {
    Message,
    Draft,
    Link,
    Prompt,
    Task,
}

/// Where the held text was going, or coming from.
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

/// Which way the text was travelling when the screen saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Inbound,
    Outbound,
    Narrative,
    Article,
}

/// Which check held it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoldBy {
    Screen,
    Judge,
    Links,
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

/// A link cut from a draft, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldLink {
    pub link: String,
    pub reason: String,
}

/// One held text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    pub id: String,
    pub at: String,
    pub direction: Direction,
    pub kind: HoldKind,
    pub context: HoldContext,
    /// The lexicon category, the judge's category, `links`, or `unknown`.
    pub category: String,
    pub reason: String,
    pub by: HoldBy,
    /// The held text itself. Private: this collection is never served publicly.
    pub text: String,
    /// For a `link` hold, what was cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<HeldLink>>,
    pub status: HoldStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// What the approval did: "requeued", "allowlisted x.example", "released as 123".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

/// A hold to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldInput {
    pub direction: Direction,
    pub kind: HoldKind,
    pub context: HoldContext,
    pub category: String,
    pub reason: String,
    pub by: HoldBy,
    pub text: String,
    pub links: Option<Vec<HeldLink>>,
}

impl HoldInput {
    fn complete(self) -> Hold {
        Hold {
            id: new_id("h"),
            at: iso_now(),
            direction: self.direction,
            kind: self.kind,
            context: self.context,
            category: self.category,
            reason: self.reason,
            by: self.by,
            text: self.text,
            links: self.links,
            status: HoldStatus::Pending,
            decided_at: None,
            note: None,
            effect: None,
        }
    }
}

/// What an operator added to a decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionDetail {
    pub note: Option<String>,
    pub effect: Option<String>,
}

/// How many holds are in each status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldCounts {
    pub pending: u64,
    pub approved: u64,
    pub rejected: u64,
}

impl HoldCounts {
    fn add(&mut self, status: HoldStatus, total: u64) {
        match status {
            HoldStatus::Pending => self.pending += total,
            HoldStatus::Approved => self.approved += total,
            HoldStatus::Rejected => self.rejected += total,
        }
    }
}

fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(LIST_DEFAULT).clamp(1, LIST_MAX)
}

impl From<HoldDecision> for HoldStatus {
    fn from(decision: HoldDecision) -> Self {
        match decision {
            HoldDecision::Approved => Self::Approved,
            HoldDecision::Rejected => Self::Rejected,
        }
    }
}

/// The hold log.
#[async_trait]
pub trait HoldLog: Send + Sync {
    async fn record(&self, entry: HoldInput) -> Result<Hold>;
    /// Newest first; `limit` defaults to [`LIST_DEFAULT`] and is held to
    /// `1..=`[`LIST_MAX`].
    async fn list(&self, status: Option<HoldStatus>, limit: Option<usize>) -> Result<Vec<Hold>>;
    async fn get(&self, id: &str) -> Result<Option<Hold>>;
    /// Closes a hold; `None` when there is no such hold.
    async fn decide(
        &self,
        id: &str,
        decision: HoldDecision,
        detail: &DecisionDetail,
    ) -> Result<Option<Hold>>;
    async fn counts(&self) -> Result<HoldCounts>;
    /// Hosts an operator approved, in allowlist spelling. The gate reads
    /// these alongside the built-in list, so an approval takes effect
    /// everywhere the gate runs.
    async fn allowed_links(&self) -> Result<Vec<String>>;
    /// Allows a host, once.
    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<()>;
}

/// The log in process, for tests and running without Mongo.
#[derive(Debug, Default)]
pub struct InMemoryHoldLog {
    entries: Mutex<Vec<Hold>>,
    links: Mutex<Vec<AllowedLink>>,
}

#[async_trait]
impl HoldLog for InMemoryHoldLog {
    async fn record(&self, entry: HoldInput) -> Result<Hold> {
        let hold = entry.complete();
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(0, hold.clone());
        Ok(hold)
    }

    async fn list(&self, status: Option<HoldStatus>, limit: Option<usize>) -> Result<Vec<Hold>> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Ok(entries
            .iter()
            .filter(|hold| status.is_none_or(|status| hold.status == status))
            .take(clamp_limit(limit))
            .cloned()
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<Hold>> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .find(|hold| hold.id == id)
            .cloned())
    }

    async fn decide(
        &self,
        id: &str,
        decision: HoldDecision,
        detail: &DecisionDetail,
    ) -> Result<Option<Hold>> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(hold) = entries.iter_mut().find(|hold| hold.id == id) else {
            return Ok(None);
        };
        hold.status = decision.into();
        hold.decided_at = Some(iso_now());
        if let Some(note) = detail.note.as_ref().filter(|note| !note.is_empty()) {
            hold.note = Some(note.clone());
        }
        if let Some(effect) = detail.effect.as_ref().filter(|effect| !effect.is_empty()) {
            hold.effect = Some(effect.clone());
        }
        Ok(Some(hold.clone()))
    }

    async fn counts(&self) -> Result<HoldCounts> {
        let mut counts = HoldCounts::default();
        for hold in self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
        {
            counts.add(hold.status, 1);
        }
        Ok(counts)
    }

    async fn allowed_links(&self) -> Result<Vec<String>> {
        Ok(self
            .links
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .map(|link| link.entry.clone())
            .collect())
    }

    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<()> {
        let mut links = self
            .links
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !links.iter().any(|link| link.entry == entry) {
            links.push(AllowedLink {
                entry: entry.to_owned(),
                hold_id: hold_id.to_owned(),
                at: iso_now(),
            });
        }
        Ok(())
    }
}

/// An approved host, as `<collection>_links` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AllowedLink {
    entry: String,
    hold_id: String,
    at: String,
}

/// The log in Mongo.
pub struct MongoHoldLog {
    holds: Collection<Document>,
    links: Collection<Document>,
    indexes: Indexes,
}

impl MongoHoldLog {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            holds: db.collection(name),
            links: db.collection(&format!("{name}_links")),
            indexes: Indexes::default(),
        }
    }
}

impl MongoHoldLog {
    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.holds
                    .create_indexes([
                        unique(doc! { "id": 1 }),
                        index(doc! { "status": 1, "at": -1 }),
                    ])
                    .await?;
                self.links.create_index(unique(doc! { "entry": 1 })).await?;
                Ok(())
            })
            .await
    }

    fn typed(&self) -> Collection<Hold> {
        self.holds.clone_with_type()
    }
}

#[async_trait]
impl HoldLog for MongoHoldLog {
    async fn record(&self, entry: HoldInput) -> Result<Hold> {
        self.ready().await?;
        let hold = entry.complete();
        self.holds.insert_one(bson::to_document(&hold)?).await?;
        Ok(hold)
    }

    async fn list(&self, status: Option<HoldStatus>, limit: Option<usize>) -> Result<Vec<Hold>> {
        self.ready().await?;
        let query = match status {
            Some(status) => doc! { "status": bson::to_bson(&status)? },
            None => doc! {},
        };
        let holds = self
            .typed()
            .find(query)
            .sort(doc! { "at": -1 })
            .limit(clamp_limit(limit) as i64)
            .await?
            .try_collect()
            .await?;
        Ok(holds)
    }

    async fn get(&self, id: &str) -> Result<Option<Hold>> {
        self.ready().await?;
        Ok(self.typed().find_one(doc! { "id": id }).await?)
    }

    async fn decide(
        &self,
        id: &str,
        decision: HoldDecision,
        detail: &DecisionDetail,
    ) -> Result<Option<Hold>> {
        self.ready().await?;
        let mut set = doc! { "status": bson::to_bson(&decision)?, "decidedAt": iso_now() };
        if let Some(note) = detail.note.as_ref().filter(|note| !note.is_empty()) {
            set.insert("note", note);
        }
        if let Some(effect) = detail.effect.as_ref().filter(|effect| !effect.is_empty()) {
            set.insert("effect", effect);
        }
        Ok(self
            .typed()
            .find_one_and_update(doc! { "id": id }, doc! { "$set": set })
            .return_document(ReturnDocument::After)
            .await?)
    }

    async fn counts(&self) -> Result<HoldCounts> {
        self.ready().await?;
        let mut counts = HoldCounts::default();
        for (status, total) in group_counts(&self.holds, "status").await? {
            if let Ok(status) = bson::from_bson::<HoldStatus>(status) {
                counts.add(status, total);
            }
        }
        Ok(counts)
    }

    async fn allowed_links(&self) -> Result<Vec<String>> {
        self.ready().await?;
        let links: Vec<AllowedLink> = self
            .links
            .clone_with_type()
            .find(doc! {})
            .await?
            .try_collect()
            .await?;
        Ok(links.into_iter().map(|link| link.entry).collect())
    }

    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<()> {
        self.ready().await?;
        let link = AllowedLink {
            entry: entry.to_owned(),
            hold_id: hold_id.to_owned(),
            at: iso_now(),
        };
        self.links
            .update_one(
                doc! { "entry": entry },
                doc! { "$setOnInsert": bson::to_document(&link)? },
            )
            .upsert(true)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "holds_tests.rs"]
mod tests;
