//! The mention queue: the cache that separates reading X from replying to it.
//!
//! Retrieval is the expensive, rate-limited half and it is the half that
//! cannot be retried later: a post outside the 7-day search window is simply
//! gone. So ingest writes here and returns, and replying drains this at
//! whatever pace the model and the posting limits allow. A crash between the
//! two loses nothing.

use std::marker::PhantomData;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::options::ReturnDocument;
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::Result;
use crate::mongo::{Indexes, group_counts, index, unique};
use crate::support::{clip, iso, iso_now};

/// The default collection, `MENTIONS_COLLECTION`'s default.
pub const COLLECTION: &str = "mentions";

/// How long a claim holds before a consumer that went quiet is presumed
/// dead and its mention, if nothing was posted, is pending again. Longer
/// than any one draft takes, pacing included.
pub const DEFAULT_CLAIM_LEASE: Duration = Duration::from_secs(15 * 60);

/// Where a mention is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MentionStatus {
    Pending,
    /// Claimed by a consumer drafting an answer; see [`MentionQueue::claim`].
    Replying,
    Replied,
    Failed,
    Skipped,
}

/// Which reply budget an answer came out of (the cadence's `Lane`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplyLane {
    /// A comment under one of the account's own posts.
    Auto,
    /// A mention the account chose to answer.
    Smart,
    /// Somebody typed the handle on purpose.
    Called,
}

/// The post a mention carries. The queue is generic over it so this crate
/// does not depend on the X client; the post is stored whole under `post`.
pub trait QueuedPost: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    /// The X id, the queue's key.
    fn post_id(&self) -> &str;
}

/// One row of the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "P: DeserializeOwned"))]
pub struct QueuedMention<P> {
    pub post_id: String,
    pub post: P,
    pub status: MentionStatus,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub attempts: i64,
    pub cached_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Set when an operator approved a held mention; the inbound content
    /// check is waived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_approved: Option<bool>,
    /// Which reply budget the answer came out of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<ReplyLane>,
    /// For the `auto` lane, the own post the answer sits under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_post_id: Option<String>,
    /// When the account liked the post; every mention gets a like.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liked_at: Option<String>,
    /// When a consumer claimed it to answer; the start of the lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
}

/// What went out in answer to a mention.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reply {
    /// The posted reply's id; absent on a dry run.
    pub id: Option<String>,
    pub text: String,
    pub lane: Option<ReplyLane>,
    pub own_post_id: Option<String>,
}

/// Which replies to count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepliedFilter {
    pub lane: ReplyLane,
    /// Only replies under this own post.
    pub own_post_id: Option<String>,
    /// Only replies at or after this time, ISO.
    pub since: Option<String>,
}

/// How many mentions are in each status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusCounts {
    pub pending: u64,
    pub replied: u64,
    pub failed: u64,
    pub skipped: u64,
    /// Left out when there are none, so the counts read as they always did.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub replying: u64,
}

fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// The queue.
#[async_trait]
pub trait MentionQueue<P: QueuedPost>: Send + Sync {
    /// Caches posts that are not already known. Returns how many were new.
    async fn cache(&self, posts: &[P]) -> Result<u64>;
    /// Oldest pending posts first, so a backlog drains in the order it arrived.
    /// A claim whose lease ran out before any reply went out is pending
    /// again, and is read with them. Reading claims nothing: see
    /// [`claim`](Self::claim).
    async fn take_pending(&self, limit: usize) -> Result<Vec<QueuedMention<P>>>;
    /// Takes one mention to answer, atomically: pending (or a stale claim
    /// with no reply out) becomes `replying` with `claimedAt` now, and the
    /// row is returned. `None` when another consumer holds a live claim, a
    /// reply already went out, or the mention is done or unknown. Only the
    /// consumer that gets the row back drafts an answer.
    async fn claim(&self, post_id: &str) -> Result<Option<QueuedMention<P>>>;
    /// Records the posted reply's id the moment the post returns, before
    /// anything else: a claim carrying a reply id is never offered again,
    /// so a crash after posting cannot answer the mention twice.
    async fn record_reply_id(&self, post_id: &str, reply_id: &str) -> Result<()>;
    async fn mark_replied(&self, post_id: &str, reply: &Reply) -> Result<()>;
    async fn mark_failed(&self, post_id: &str, error: &str) -> Result<()>;
    /// Set aside without an answer: over budget, or not worth one. Not a failure.
    async fn mark_skipped(&self, post_id: &str, reason: &str) -> Result<()>;
    async fn mark_liked(&self, post_id: &str) -> Result<()>;
    /// How many replies went out of a lane; under one own post, or since a time.
    async fn count_replied(&self, filter: &RepliedFilter) -> Result<u64>;
    /// Puts a held mention back to be answered, with the inbound content
    /// check waived. False when there is no such mention.
    async fn requeue_approved(&self, post_id: &str) -> Result<bool>;
    /// Puts a mention whose reply was deleted back to be answered afresh:
    /// pending, no attempts, no error, no reply. False when there is no
    /// such mention.
    async fn requeue_fresh(&self, post_id: &str) -> Result<bool>;
    /// Where one mention stands; `None` when it is not queued.
    async fn status_of(&self, post_id: &str) -> Result<Option<MentionStatus>>;
    async fn counts(&self) -> Result<StatusCounts>;
}

/// The queue in process, for tests and dry runs.
pub struct InMemoryMentionQueue<P> {
    rows: Mutex<Vec<QueuedMention<P>>>,
    lease: Duration,
}

impl<P> Default for InMemoryMentionQueue<P> {
    fn default() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
            lease: DEFAULT_CLAIM_LEASE,
        }
    }
}

impl<P> InMemoryMentionQueue<P> {
    /// With a claim lease other than [`DEFAULT_CLAIM_LEASE`].
    pub fn with_claim_lease(mut self, lease: Duration) -> Self {
        self.lease = lease;
        self
    }
}

impl<P: QueuedPost> InMemoryMentionQueue<P> {
    /// One row, by post id.
    pub fn get(&self, post_id: &str) -> Option<QueuedMention<P>> {
        self.lock()
            .iter()
            .find(|row| row.post_id == post_id)
            .cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<QueuedMention<P>>> {
        self.rows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Applies `change` to the row for `post_id`; false when there is none.
    fn update(&self, post_id: &str, change: impl FnOnce(&mut QueuedMention<P>)) -> bool {
        match self.lock().iter_mut().find(|row| row.post_id == post_id) {
            Some(row) => {
                change(row);
                true
            }
            None => false,
        }
    }
}

#[async_trait]
impl<P: QueuedPost> MentionQueue<P> for InMemoryMentionQueue<P> {
    async fn cache(&self, posts: &[P]) -> Result<u64> {
        let now = iso_now();
        let mut rows = self.lock();
        let mut added = 0;
        for post in posts {
            if rows.iter().any(|row| row.post_id == post.post_id()) {
                continue;
            }
            rows.push(fresh(post, &now));
            added += 1;
        }
        Ok(added)
    }

    async fn take_pending(&self, limit: usize) -> Result<Vec<QueuedMention<P>>> {
        let cutoff = stale_before(self.lease);
        let mut pending: Vec<_> = self
            .lock()
            .iter()
            .filter(|row| is_open(row, &cutoff))
            .cloned()
            .collect();
        pending.sort_by(|a, b| a.cached_at.cmp(&b.cached_at));
        pending.truncate(limit);
        Ok(pending)
    }

    async fn claim(&self, post_id: &str) -> Result<Option<QueuedMention<P>>> {
        let cutoff = stale_before(self.lease);
        let now = iso_now();
        let mut rows = self.lock();
        let Some(row) = rows
            .iter_mut()
            .find(|row| row.post_id == post_id && is_open(row, &cutoff))
        else {
            return Ok(None);
        };
        row.status = MentionStatus::Replying;
        row.claimed_at = Some(now.clone());
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    async fn record_reply_id(&self, post_id: &str, reply_id: &str) -> Result<()> {
        self.update(post_id, |row| {
            row.reply_id = Some(reply_id.to_owned());
            row.updated_at = iso_now();
        });
        Ok(())
    }

    async fn mark_replied(&self, post_id: &str, reply: &Reply) -> Result<()> {
        self.update(post_id, |row| {
            row.status = MentionStatus::Replied;
            row.updated_at = iso_now();
            row.reply_text = Some(reply.text.clone());
            if let Some(id) = &reply.id {
                row.reply_id = Some(id.clone());
            }
            if let Some(lane) = reply.lane {
                row.lane = Some(lane);
            }
            if let Some(own) = &reply.own_post_id {
                row.own_post_id = Some(own.clone());
            }
            row.attempts += 1;
        });
        Ok(())
    }

    async fn mark_failed(&self, post_id: &str, error: &str) -> Result<()> {
        self.update(post_id, |row| {
            row.status = MentionStatus::Failed;
            row.error = Some(clip(error, MAX_ERROR));
            row.updated_at = iso_now();
            row.attempts += 1;
        });
        Ok(())
    }

    async fn mark_skipped(&self, post_id: &str, reason: &str) -> Result<()> {
        self.update(post_id, |row| {
            row.status = MentionStatus::Skipped;
            row.error = Some(clip(reason, MAX_ERROR));
            row.updated_at = iso_now();
        });
        Ok(())
    }

    async fn mark_liked(&self, post_id: &str) -> Result<()> {
        self.update(post_id, |row| row.liked_at = Some(iso_now()));
        Ok(())
    }

    async fn count_replied(&self, filter: &RepliedFilter) -> Result<u64> {
        let rows = self.lock();
        let count = rows
            .iter()
            .filter(|row| row.status == MentionStatus::Replied && row.lane == Some(filter.lane))
            .filter(|row| filter.own_post_id.is_none() || row.own_post_id == filter.own_post_id)
            .filter(|row| {
                filter
                    .since
                    .as_ref()
                    .is_none_or(|since| row.updated_at >= *since)
            })
            .count();
        Ok(count as u64)
    }

    async fn requeue_approved(&self, post_id: &str) -> Result<bool> {
        Ok(self.update(post_id, |row| {
            row.status = MentionStatus::Pending;
            row.filter_approved = Some(true);
            row.updated_at = iso_now();
            row.error = None;
        }))
    }

    async fn requeue_fresh(&self, post_id: &str) -> Result<bool> {
        Ok(self.update(post_id, |row| {
            row.status = MentionStatus::Pending;
            row.attempts = 0;
            row.updated_at = iso_now();
            row.error = None;
            row.reply_id = None;
            row.reply_text = None;
            row.claimed_at = None;
        }))
    }

    async fn status_of(&self, post_id: &str) -> Result<Option<MentionStatus>> {
        Ok(self.get(post_id).map(|row| row.status))
    }

    async fn counts(&self) -> Result<StatusCounts> {
        let mut counts = StatusCounts::default();
        for row in self.lock().iter() {
            counts.add(row.status, 1);
        }
        Ok(counts)
    }
}

/// Errors and skip reasons keep this many characters.
const MAX_ERROR: usize = 500;

/// A claim made before this, ISO, has outlived its lease.
fn stale_before(lease: Duration) -> String {
    let lease = TimeDelta::from_std(lease).unwrap_or(TimeDelta::MAX);
    iso(Utc::now()
        .checked_sub_signed(lease)
        .unwrap_or(DateTime::<Utc>::MIN_UTC))
}

/// Whether a row is there to be answered: pending, or claimed by a consumer
/// that went quiet before any reply went out.
fn is_open<P>(row: &QueuedMention<P>, stale_before: &str) -> bool {
    match row.status {
        MentionStatus::Pending => true,
        MentionStatus::Replying => {
            row.reply_id.is_none()
                && row
                    .claimed_at
                    .as_deref()
                    .is_none_or(|claimed| claimed < stale_before)
        }
        _ => false,
    }
}

/// [`is_open`] as a Mongo filter.
fn open_query(stale_before: &str) -> Document {
    doc! { "$or": [
        { "status": "pending" },
        { "status": "replying", "replyId": null, "claimedAt": { "$not": { "$gte": stale_before } } },
    ] }
}

fn fresh<P: QueuedPost>(post: &P, now: &str) -> QueuedMention<P> {
    QueuedMention {
        post_id: post.post_id().to_owned(),
        post: post.clone(),
        status: MentionStatus::Pending,
        attempts: 0,
        cached_at: now.to_owned(),
        updated_at: now.to_owned(),
        reply_id: None,
        reply_text: None,
        error: None,
        filter_approved: None,
        lane: None,
        own_post_id: None,
        liked_at: None,
        claimed_at: None,
    }
}

impl StatusCounts {
    fn add(&mut self, status: MentionStatus, total: u64) {
        match status {
            MentionStatus::Pending => self.pending += total,
            MentionStatus::Replying => self.replying += total,
            MentionStatus::Replied => self.replied += total,
            MentionStatus::Failed => self.failed += total,
            MentionStatus::Skipped => self.skipped += total,
        }
    }
}

/// The queue in Mongo.
pub struct MongoMentionQueue<P> {
    rows: Collection<Document>,
    indexes: Indexes,
    lease: Duration,
    post: PhantomData<fn() -> P>,
}

impl<P: QueuedPost> MongoMentionQueue<P> {
    /// The queue in the default collection.
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            rows: db.collection(name),
            indexes: Indexes::default(),
            lease: DEFAULT_CLAIM_LEASE,
            post: PhantomData,
        }
    }

    /// With a claim lease other than [`DEFAULT_CLAIM_LEASE`].
    pub fn with_claim_lease(mut self, lease: Duration) -> Self {
        self.lease = lease;
        self
    }

    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.rows
                    .create_indexes([
                        unique(doc! { "postId": 1 }),
                        index(doc! { "status": 1, "cachedAt": 1 }),
                        index(doc! { "status": 1, "lane": 1, "ownPostId": 1, "updatedAt": 1 }),
                    ])
                    .await?;
                Ok(())
            })
            .await
    }
}

#[async_trait]
impl<P: QueuedPost> MentionQueue<P> for MongoMentionQueue<P> {
    async fn cache(&self, posts: &[P]) -> Result<u64> {
        if posts.is_empty() {
            return Ok(0);
        }
        self.ready().await?;
        let now = iso_now();
        // Upsert-if-absent: a post already in the queue keeps its status,
        // so a re-read from the overlapping search window never resets work.
        let mut added = 0;
        for post in posts {
            let result = self
                .rows
                .update_one(
                    doc! { "postId": post.post_id() },
                    doc! { "$setOnInsert": bson::to_document(&fresh(post, &now))? },
                )
                .upsert(true)
                .await?;
            if result.upserted_id.is_some() {
                added += 1;
            }
        }
        Ok(added)
    }

    async fn take_pending(&self, limit: usize) -> Result<Vec<QueuedMention<P>>> {
        self.ready().await?;
        let rows = self
            .rows
            .clone_with_type::<QueuedMention<P>>()
            .find(open_query(&stale_before(self.lease)))
            .sort(doc! { "cachedAt": 1 })
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .await?
            .try_collect()
            .await?;
        Ok(rows)
    }

    async fn claim(&self, post_id: &str) -> Result<Option<QueuedMention<P>>> {
        self.ready().await?;
        let mut query = open_query(&stale_before(self.lease));
        query.insert("postId", post_id);
        let now = iso_now();
        Ok(self
            .rows
            .clone_with_type::<QueuedMention<P>>()
            .find_one_and_update(
                query,
                doc! { "$set": { "status": "replying", "claimedAt": &now, "updatedAt": &now } },
            )
            .return_document(ReturnDocument::After)
            .await?)
    }

    async fn record_reply_id(&self, post_id: &str, reply_id: &str) -> Result<()> {
        self.ready().await?;
        self.rows
            .update_one(
                doc! { "postId": post_id },
                doc! { "$set": { "replyId": reply_id, "updatedAt": iso_now() } },
            )
            .await?;
        Ok(())
    }

    async fn mark_replied(&self, post_id: &str, reply: &Reply) -> Result<()> {
        self.ready().await?;
        let mut set =
            doc! { "status": "replied", "updatedAt": iso_now(), "replyText": &reply.text };
        if let Some(id) = &reply.id {
            set.insert("replyId", id);
        }
        if let Some(lane) = reply.lane {
            set.insert("lane", bson::to_bson(&lane)?);
        }
        if let Some(own) = &reply.own_post_id {
            set.insert("ownPostId", own);
        }
        self.rows
            .update_one(
                doc! { "postId": post_id },
                doc! { "$set": set, "$inc": { "attempts": 1 } },
            )
            .await?;
        Ok(())
    }

    async fn mark_failed(&self, post_id: &str, error: &str) -> Result<()> {
        self.ready().await?;
        self.rows
            .update_one(
                doc! { "postId": post_id },
                doc! {
                    "$set": { "status": "failed", "error": clip(error, MAX_ERROR), "updatedAt": iso_now() },
                    "$inc": { "attempts": 1 },
                },
            )
            .await?;
        Ok(())
    }

    async fn mark_skipped(&self, post_id: &str, reason: &str) -> Result<()> {
        self.ready().await?;
        self.rows
            .update_one(
                doc! { "postId": post_id },
                doc! { "$set": { "status": "skipped", "error": clip(reason, MAX_ERROR), "updatedAt": iso_now() } },
            )
            .await?;
        Ok(())
    }

    async fn mark_liked(&self, post_id: &str) -> Result<()> {
        self.ready().await?;
        self.rows
            .update_one(
                doc! { "postId": post_id },
                doc! { "$set": { "likedAt": iso_now() } },
            )
            .await?;
        Ok(())
    }

    async fn count_replied(&self, filter: &RepliedFilter) -> Result<u64> {
        self.ready().await?;
        let mut query = doc! { "status": "replied", "lane": bson::to_bson(&filter.lane)? };
        if let Some(own) = &filter.own_post_id {
            query.insert("ownPostId", own);
        }
        if let Some(since) = &filter.since {
            query.insert("updatedAt", doc! { "$gte": since });
        }
        Ok(self.rows.count_documents(query).await?)
    }

    async fn requeue_approved(&self, post_id: &str) -> Result<bool> {
        self.ready().await?;
        let result = self
            .rows
            .update_one(
                doc! { "postId": post_id },
                doc! {
                    "$set": { "status": "pending", "filterApproved": true, "updatedAt": iso_now() },
                    "$unset": { "error": "" },
                },
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    async fn requeue_fresh(&self, post_id: &str) -> Result<bool> {
        self.ready().await?;
        // Cleared, not just reset: the attempt count and the old error
        // belong to an answer that no longer exists.
        let result = self
            .rows
            .update_one(
                doc! { "postId": post_id },
                doc! {
                    "$set": { "status": "pending", "attempts": 0, "updatedAt": iso_now() },
                    "$unset": { "error": "", "reply": "", "replyId": "", "replyText": "", "claimedAt": "" },
                },
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    async fn status_of(&self, post_id: &str) -> Result<Option<MentionStatus>> {
        self.ready().await?;
        let row = self
            .rows
            .find_one(doc! { "postId": post_id })
            .projection(doc! { "status": 1 })
            .await?;
        Ok(match row.and_then(|row| row.get("status").cloned()) {
            Some(status) => Some(bson::from_bson(status)?),
            None => None,
        })
    }

    async fn counts(&self) -> Result<StatusCounts> {
        self.ready().await?;
        let mut counts = StatusCounts::default();
        for (status, total) in group_counts(&self.rows, "status").await? {
            if let Ok(status) = bson::from_bson::<MentionStatus>(status) {
                counts.add(status, total);
            }
        }
        Ok(counts)
    }
}

#[cfg(test)]
#[path = "mentions_tests.rs"]
mod tests;
