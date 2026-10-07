//! The post ledger: what the account put on its own timeline, and when.
//!
//! The posting jobs each guard themselves against running twice, but none
//! of them knew about the others. This is the one place they all write, so
//! the cadence can read a single answer to "when did we last post", and the
//! heat job has the ids to watch.

use std::sync::Mutex;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::mongo::{Indexes, index, unique};

/// The default collection.
pub const COLLECTION: &str = "own_posts";
/// How many own posts [`PostLedger::since`] returns when asked for "all".
pub const SINCE_LIMIT: usize = 50;
/// The readings kept per post: plenty for a velocity, since a post is
/// watched for a day at most and the heat job runs a few times an hour.
pub const SAMPLES_KEPT: usize = 100;

/// One reading of a post's counters, taken by the heat job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricSample {
    pub at: String,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub impressions: i64,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub likes: i64,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub replies: i64,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub reposts: i64,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub quotes: i64,
}

/// A post the heat job saw take off. `hold_until` is how long the timeline
/// stays quiet for it; the reply budget reads the same field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hot {
    pub since: String,
    pub hold_until: String,
}

/// A post the account published to its own timeline: a morning post, a
/// chart, a moon chart, a calendar item, something the agent was asked to
/// post. Not a reply, which is somebody else's conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnPost {
    pub post_id: String,
    /// Which job published it: morning, chart, moon, calendar:<name>, agent, post.
    pub kind: String,
    pub posted_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub samples: Vec<MetricSample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hot: Option<Hot>,
}

/// A post to record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewOwnPost {
    pub post_id: String,
    pub kind: String,
    pub posted_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// The ledger.
#[async_trait]
pub trait PostLedger: Send + Sync {
    /// Records a post, once.
    async fn record(&self, post: &NewOwnPost) -> Result<()>;
    /// The most recent own post, by when it was posted.
    async fn latest(&self) -> Result<Option<OwnPost>>;
    /// Own posts published at or after `since`, newest first.
    async fn since(&self, since: &str, limit: usize) -> Result<Vec<OwnPost>>;
    async fn get(&self, post_id: &str) -> Result<Option<OwnPost>>;
    /// Adds a reading, keeping the last [`SAMPLES_KEPT`].
    async fn sample(&self, post_id: &str, sample: &MetricSample) -> Result<()>;
    async fn set_hot(&self, post_id: &str, hot: &Hot) -> Result<()>;
    /// The hot post whose hold is still running at `now`, if any.
    async fn hold(&self, now: &str) -> Result<Option<OwnPost>>;
}

/// The ledger in process, for tests and dry runs.
#[derive(Debug, Default)]
pub struct InMemoryPostLedger {
    posts: Mutex<Vec<OwnPost>>,
}

impl InMemoryPostLedger {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<OwnPost>> {
        self.posts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Newest first, by when it was posted.
    fn sorted(&self) -> Vec<OwnPost> {
        let mut posts = self.lock().clone();
        posts.sort_by(|a, b| b.posted_at.cmp(&a.posted_at));
        posts
    }

    fn update(&self, post_id: &str, change: impl FnOnce(&mut OwnPost)) {
        if let Some(post) = self.lock().iter_mut().find(|post| post.post_id == post_id) {
            change(post);
        }
    }
}

#[async_trait]
impl PostLedger for InMemoryPostLedger {
    async fn record(&self, post: &NewOwnPost) -> Result<()> {
        let mut posts = self.lock();
        if !posts
            .iter()
            .any(|existing| existing.post_id == post.post_id)
        {
            posts.push(OwnPost {
                post_id: post.post_id.clone(),
                kind: post.kind.clone(),
                posted_at: post.posted_at.clone(),
                text: post.text.clone(),
                samples: Vec::new(),
                hot: None,
            });
        }
        Ok(())
    }

    async fn latest(&self) -> Result<Option<OwnPost>> {
        Ok(self.sorted().into_iter().next())
    }

    async fn since(&self, since: &str, limit: usize) -> Result<Vec<OwnPost>> {
        Ok(self
            .sorted()
            .into_iter()
            .filter(|post| post.posted_at.as_str() >= since)
            .take(limit)
            .collect())
    }

    async fn get(&self, post_id: &str) -> Result<Option<OwnPost>> {
        Ok(self
            .lock()
            .iter()
            .find(|post| post.post_id == post_id)
            .cloned())
    }

    async fn sample(&self, post_id: &str, sample: &MetricSample) -> Result<()> {
        self.update(post_id, |post| {
            post.samples.push(sample.clone());
            let excess = post.samples.len().saturating_sub(SAMPLES_KEPT);
            post.samples.drain(..excess);
        });
        Ok(())
    }

    async fn set_hot(&self, post_id: &str, hot: &Hot) -> Result<()> {
        self.update(post_id, |post| post.hot = Some(hot.clone()));
        Ok(())
    }

    async fn hold(&self, now: &str) -> Result<Option<OwnPost>> {
        let held = self
            .lock()
            .iter()
            .filter(|post| {
                post.hot
                    .as_ref()
                    .is_some_and(|hot| hot.hold_until.as_str() > now)
            })
            .cloned()
            .max_by(|a, b| {
                let until = |post: &OwnPost| post.hot.as_ref().map(|hot| hot.hold_until.clone());
                until(a).cmp(&until(b))
            });
        Ok(held)
    }
}

/// The ledger in Mongo.
pub struct MongoPostLedger {
    posts: Collection<Document>,
    indexes: Indexes,
}

impl MongoPostLedger {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            posts: db.collection(name),
            indexes: Indexes::default(),
        }
    }
}

impl MongoPostLedger {
    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.posts
                    .create_indexes([
                        unique(doc! { "postId": 1 }),
                        index(doc! { "postedAt": -1 }),
                        index(doc! { "hot.holdUntil": -1 }),
                    ])
                    .await?;
                Ok(())
            })
            .await
    }

    fn typed(&self) -> Collection<OwnPost> {
        self.posts.clone_with_type()
    }
}

#[async_trait]
impl PostLedger for MongoPostLedger {
    async fn record(&self, post: &NewOwnPost) -> Result<()> {
        self.ready().await?;
        let mut row = bson::to_document(post)?;
        row.insert("samples", bson::Array::new());
        self.posts
            .update_one(
                doc! { "postId": &post.post_id },
                doc! { "$setOnInsert": row },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    async fn latest(&self) -> Result<Option<OwnPost>> {
        self.ready().await?;
        Ok(self
            .typed()
            .find_one(doc! {})
            .sort(doc! { "postedAt": -1 })
            .await?)
    }

    async fn since(&self, since: &str, limit: usize) -> Result<Vec<OwnPost>> {
        self.ready().await?;
        let posts = self
            .typed()
            .find(doc! { "postedAt": { "$gte": since } })
            .sort(doc! { "postedAt": -1 })
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .await?
            .try_collect()
            .await?;
        Ok(posts)
    }

    async fn get(&self, post_id: &str) -> Result<Option<OwnPost>> {
        self.ready().await?;
        Ok(self.typed().find_one(doc! { "postId": post_id }).await?)
    }

    async fn sample(&self, post_id: &str, sample: &MetricSample) -> Result<()> {
        self.ready().await?;
        let kept = -i64::try_from(SAMPLES_KEPT).unwrap_or(i64::MAX);
        self.posts
            .update_one(
                doc! { "postId": post_id },
                doc! { "$push": { "samples": { "$each": [bson::to_document(sample)?], "$slice": kept } } },
            )
            .await?;
        Ok(())
    }

    async fn set_hot(&self, post_id: &str, hot: &Hot) -> Result<()> {
        self.ready().await?;
        self.posts
            .update_one(
                doc! { "postId": post_id },
                doc! { "$set": { "hot": bson::to_document(hot)? } },
            )
            .await?;
        Ok(())
    }

    async fn hold(&self, now: &str) -> Result<Option<OwnPost>> {
        self.ready().await?;
        Ok(self
            .typed()
            .find_one(doc! { "hot.holdUntil": { "$gt": now } })
            .sort(doc! { "hot.holdUntil": -1 })
            .await?)
    }
}

#[cfg(test)]
#[path = "posts_tests.rs"]
mod tests;
