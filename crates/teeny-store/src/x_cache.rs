//! The store behind the read-through cache in front of the X API.
//!
//! X bills every post a read returns, on top of the request itself, and the
//! same reads recur. A post's content never changes, so a post once seen is
//! kept for a long while; lists (searches, a conversation, trends) and
//! counters do change, and are kept only briefly. The caching client itself
//! lives with the X client; this is where its rows go, and how long each kind
//! is trusted.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mongodb::bson::{self, Bson, Document, doc};
use mongodb::options::IndexOptions;
use mongodb::{Collection, Database, IndexModel};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::mongo::{Indexes, unique};
use crate::support::iso;

/// The default collection.
pub const COLLECTION: &str = "x-cache";

/// What a cached answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum XCacheKind {
    /// A post, by id. Its content cannot change.
    Post,
    /// A post X said was not there: deleted, protected, or not yet visible.
    Missing,
    Search,
    Conversation,
    Trends,
    /// The account's own counters.
    Metrics,
    /// Whether somebody follows the account.
    Follows,
}

/// How long each kind of answer is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XCacheTtl {
    pub post: Duration,
    pub missing: Duration,
    pub search: Duration,
    pub conversation: Duration,
    pub trends: Duration,
    pub metrics: Duration,
    pub follows: Duration,
}

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3_600);

impl Default for XCacheTtl {
    fn default() -> Self {
        Self {
            // A post's content cannot change. Metrics carried on it are a snapshot.
            post: 30 * 24 * HOUR,
            missing: HOUR,
            search: 10 * MINUTE,
            conversation: 10 * MINUTE,
            trends: 30 * MINUTE,
            // The heat job reads the account's own counters every 20 minutes.
            metrics: 10 * MINUTE,
            follows: 24 * HOUR,
        }
    }
}

impl XCacheTtl {
    /// How long `kind` is trusted.
    pub fn of(&self, kind: XCacheKind) -> Duration {
        match kind {
            XCacheKind::Post => self.post,
            XCacheKind::Missing => self.missing,
            XCacheKind::Search => self.search,
            XCacheKind::Conversation => self.conversation,
            XCacheKind::Trends => self.trends,
            XCacheKind::Metrics => self.metrics,
            XCacheKind::Follows => self.follows,
        }
    }
}

/// One cached answer.
#[derive(Debug, Clone, PartialEq)]
pub struct XCacheEntry {
    pub key: String,
    pub kind: XCacheKind,
    /// The answer as JSON: a post, a list of posts, `null` for a missing post.
    pub value: serde_json::Value,
    /// When it was stored, ISO.
    pub stored_at: String,
    /// When it stops being trusted. Mongo sweeps the row after this.
    pub expires_at: DateTime<Utc>,
}

impl XCacheEntry {
    /// An answer stored at `now`, trusted for as long as `ttl` says its kind is.
    pub fn new(
        key: impl Into<String>,
        kind: XCacheKind,
        value: serde_json::Value,
        now: DateTime<Utc>,
        ttl: &XCacheTtl,
    ) -> Self {
        let expires_at = now + ttl.of(kind);
        Self {
            key: key.into(),
            kind,
            value,
            stored_at: iso(now),
            // Mongo keeps milliseconds; so does the entry, so it reads back equal.
            expires_at: DateTime::from_timestamp_millis(expires_at.timestamp_millis())
                .unwrap_or(expires_at),
        }
    }

    /// Whether the answer can still be served at `now`. A row Mongo has not
    /// swept yet is still checked against the clock on the way out.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now
    }
}

/// Where cached answers are kept. A store that cannot be read or written is
/// a cache miss to the caller, never a failed read.
#[async_trait]
pub trait XCacheStore: Send + Sync {
    /// The row under `key`, fresh or not.
    async fn get(&self, key: &str) -> Result<Option<XCacheEntry>>;
    /// Stores `entry`, replacing any row under its key.
    async fn set(&self, entry: &XCacheEntry) -> Result<()>;
}

/// The cache in process, for tests.
#[derive(Debug, Default)]
pub struct InMemoryXCache {
    rows: Mutex<HashMap<String, XCacheEntry>>,
}

#[async_trait]
impl XCacheStore for InMemoryXCache {
    async fn get(&self, key: &str) -> Result<Option<XCacheEntry>> {
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(key)
            .cloned())
    }

    async fn set(&self, entry: &XCacheEntry) -> Result<()> {
        self.rows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(entry.key.clone(), entry.clone());
        Ok(())
    }
}

/// The cache in Mongo, so a restart does not re-buy what the last process
/// already paid for. Mongo expires rows itself through the TTL index on
/// `expiresAt`.
pub struct MongoXCache {
    rows: Collection<Document>,
    indexes: Indexes,
}

impl MongoXCache {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            rows: db.collection(name),
            indexes: Indexes::default(),
        }
    }

    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                let expire = IndexOptions::builder().expire_after(Duration::ZERO).build();
                self.rows
                    .create_indexes([
                        unique(doc! { "key": 1 }),
                        IndexModel::builder()
                            .keys(doc! { "expiresAt": 1 })
                            .options(expire)
                            .build(),
                    ])
                    .await?;
                Ok(())
            })
            .await
    }
}

/// A row as Mongo holds it: `expiresAt` a BSON date, for the TTL index.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    key: String,
    kind: XCacheKind,
    value: Bson,
    stored_at: String,
    expires_at: bson::DateTime,
}

#[async_trait]
impl XCacheStore for MongoXCache {
    async fn get(&self, key: &str) -> Result<Option<XCacheEntry>> {
        self.ready().await?;
        let Some(row) = self
            .rows
            .clone_with_type::<Row>()
            .find_one(doc! { "key": key })
            .await?
        else {
            return Ok(None);
        };
        let millis = row.expires_at.timestamp_millis();
        Ok(Some(XCacheEntry {
            key: row.key,
            kind: row.kind,
            value: row.value.into_relaxed_extjson(),
            stored_at: row.stored_at,
            expires_at: DateTime::from_timestamp_millis(millis).unwrap_or_default(),
        }))
    }

    async fn set(&self, entry: &XCacheEntry) -> Result<()> {
        self.ready().await?;
        let row = Row {
            key: entry.key.clone(),
            kind: entry.kind,
            value: bson::to_bson(&entry.value)?,
            stored_at: entry.stored_at.clone(),
            expires_at: bson::DateTime::from_millis(entry.expires_at.timestamp_millis()),
        };
        self.rows
            .replace_one(doc! { "key": &entry.key }, bson::to_document(&row)?)
            .upsert(true)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "x_cache_tests.rs"]
mod tests;
