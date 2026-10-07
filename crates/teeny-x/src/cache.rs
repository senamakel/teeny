//! A read-through cache in front of the X API, ported from
//! `legacy/src/clients/x-cache.ts` with the same keys, kinds and TTLs, so the
//! rows the TypeScript service wrote are hits here.
//!
//! X bills every post a read returns, and the same reads recur: a thread's
//! root once per mention under it, the quote job's two searches every hour,
//! a post the agent was just shown in a search. A post's content never
//! changes once published, so a post seen through any read is kept a long
//! while. Lists (searches, a conversation, trends) and counters do change and
//! are kept briefly: long enough to absorb a job asking the same thing several
//! times in one run, short enough that nothing reads as stale.
//!
//! Every read is also a single flight: callers asking the same thing at once
//! share one request. A store that cannot be read or written is a miss, not a
//! failed read. Writes and the mention poll (`XApi::find_mentions`) are not on
//! [`XReads`] at all, so they cannot come near this layer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::{BoxFuture, FutureExt, Shared, join_all};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use teeny_core::config::XConfig;
use teeny_core::types::{PostId, UserId};

use crate::client::XApi;
use crate::error::{XError, XResult};
use crate::model::{OwnPostMetrics, SearchOptions, SortOrder, Trend, TrendOptions, XPost, XUser};

/// The reads worth keeping. [`XApi`] answers them from X; [`CachedX`] from
/// the store first. A read-only tool takes a `&dyn XReads` and cannot tell.
#[async_trait]
pub trait XReads: Send + Sync {
    async fn search_posts(&self, query: &str, options: &SearchOptions) -> XResult<Vec<XPost>>;
    async fn get_post(&self, id: &PostId) -> XResult<Option<XPost>>;
    async fn get_conversation(
        &self,
        conversation_id: &PostId,
        max_results: Option<u32>,
    ) -> XResult<Vec<XPost>>;
    async fn trends(&self, options: TrendOptions) -> XResult<Vec<Trend>>;
    async fn post_metrics(&self, ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>>;
    async fn follows_account(&self, user_id: &UserId) -> Option<bool>;
    /// An account by its handle. Not kept: a profile changes, and a lookup
    /// is one user, not a bill per post.
    async fn user_by_username(&self, username: &str) -> XResult<Option<XUser>>;
}

#[async_trait]
impl XReads for XApi {
    async fn search_posts(&self, query: &str, options: &SearchOptions) -> XResult<Vec<XPost>> {
        XApi::search_posts(self, query, options).await
    }
    async fn get_post(&self, id: &PostId) -> XResult<Option<XPost>> {
        XApi::get_post(self, id).await
    }
    async fn get_conversation(
        &self,
        conversation_id: &PostId,
        max_results: Option<u32>,
    ) -> XResult<Vec<XPost>> {
        XApi::get_conversation(self, conversation_id, max_results).await
    }
    async fn trends(&self, options: TrendOptions) -> XResult<Vec<Trend>> {
        XApi::trends(self, options).await
    }
    async fn post_metrics(&self, ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        XApi::post_metrics(self, ids).await
    }
    async fn follows_account(&self, user_id: &UserId) -> Option<bool> {
        XApi::follows_account(self, user_id).await
    }
    async fn user_by_username(&self, username: &str) -> XResult<Option<XUser>> {
        XApi::user_by_username(self, username).await
    }
}

/// What kind of answer a row holds, which decides how long it is trusted.
/// Spelled on the wire as the legacy cache spelled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheKind {
    Post,
    /// A post X said was not there: deleted, protected, or not yet visible.
    Missing,
    Search,
    Conversation,
    Trends,
    /// The account's own counters.
    #[serde(rename = "metrics")]
    OwnCounters,
    /// Whether somebody follows the account.
    #[serde(rename = "follows")]
    Follow,
}

/// How long each kind of answer is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTtl {
    /// A post's content cannot change; the counters on it are a snapshot.
    pub post: Duration,
    pub missing: Duration,
    pub search: Duration,
    pub conversation: Duration,
    pub trends: Duration,
    /// The heat job reads them every 20 minutes.
    pub metrics: Duration,
    pub follows: Duration,
}

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

impl Default for CacheTtl {
    fn default() -> Self {
        Self {
            post: HOUR * 24 * 30,
            missing: HOUR,
            search: MINUTE * 10,
            conversation: MINUTE * 10,
            trends: MINUTE * 30,
            metrics: MINUTE * 10,
            follows: HOUR * 24,
        }
    }
}

impl CacheTtl {
    /// `X_CACHE_POST_TTL_DAYS` for posts, `X_CACHE_SEARCH_TTL_MINUTES` for
    /// searches and conversations, the defaults for the rest.
    pub fn from_post_days_and_search_minutes(post_days: f64, search_minutes: f64) -> Self {
        let search = Duration::from_secs_f64((search_minutes * 60.0).max(0.0));
        Self {
            post: Duration::from_secs_f64((post_days * 86_400.0).max(0.0)),
            search,
            conversation: search,
            ..Self::default()
        }
    }

    /// The TTLs the configuration asks for.
    pub fn from_config(config: &XConfig) -> Self {
        Self::from_post_days_and_search_minutes(
            config.cache_post_ttl_days,
            config.cache_search_ttl_minutes,
        )
    }

    pub fn of(&self, kind: CacheKind) -> Duration {
        match kind {
            CacheKind::Post => self.post,
            CacheKind::Missing => self.missing,
            CacheKind::Search => self.search,
            CacheKind::Conversation => self.conversation,
            CacheKind::Trends => self.trends,
            CacheKind::OwnCounters => self.metrics,
            CacheKind::Follow => self.follows,
        }
    }
}

/// One stored answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub key: String,
    pub kind: CacheKind,
    pub value: Value,
    pub stored_at: DateTime<Utc>,
    /// A store may expire rows itself (Mongo's TTL index); a row it has not
    /// swept yet is still checked against the clock on the way out.
    pub expires_at: DateTime<Utc>,
}

/// Whatever a store fails with; it is logged and read as a miss.
pub type CacheError = Box<dyn std::error::Error + Send + Sync>;

/// Where answers are kept. teeny-store's Mongo collection implements this;
/// [`InMemoryReadCache`] is for tests and for running without Mongo.
#[async_trait]
pub trait ReadCache: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CacheError>;
    /// Replaces any row under the same key.
    async fn put(&self, entry: CacheEntry) -> Result<(), CacheError>;
}

/// A process-local store.
#[derive(Debug, Default)]
pub struct InMemoryReadCache {
    rows: Mutex<HashMap<String, CacheEntry>>,
}

#[async_trait]
impl ReadCache for InMemoryReadCache {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CacheError> {
        Ok(self
            .rows
            .lock()
            .map_err(|_| "cache lock poisoned")?
            .get(key)
            .cloned())
    }

    async fn put(&self, entry: CacheEntry) -> Result<(), CacheError> {
        self.rows
            .lock()
            .map_err(|_| "cache lock poisoned")?
            .insert(entry.key.clone(), entry);
        Ok(())
    }
}

/// The time, injectable so a test can move it.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// How a [`CachedX`] behaves.
#[derive(Clone, Default)]
pub struct CacheOptions {
    pub ttl: CacheTtl,
    /// Defaults to the system clock.
    pub now: Option<Clock>,
}

/// The reads the configuration asks for: through `store` when
/// `X_CACHE_ENABLED` is on and there is one, straight to X otherwise.
pub fn read_through<R: XReads + 'static>(
    x: Arc<R>,
    store: Option<Arc<dyn ReadCache>>,
    config: &XConfig,
) -> Arc<dyn XReads> {
    match store {
        Some(store) if config.cache_enabled => Arc::new(CachedX::new(
            x,
            store,
            CacheOptions {
                ttl: CacheTtl::from_config(config),
                now: None,
            },
        )),
        _ => x,
    }
}

type Flight = Shared<BoxFuture<'static, Result<Value, Arc<XError>>>>;

/// [`XReads`] with every read going through a [`ReadCache`] first.
pub struct CachedX<R> {
    inner: Arc<Inner<R>>,
}

struct Inner<R> {
    x: Arc<R>,
    store: Arc<dyn ReadCache>,
    ttl: CacheTtl,
    now: Clock,
    inflight: Mutex<HashMap<String, Flight>>,
}

impl<R: XReads + 'static> CachedX<R> {
    pub fn new(x: Arc<R>, store: Arc<dyn ReadCache>, options: CacheOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                x,
                store,
                ttl: options.ttl,
                now: options.now.unwrap_or_else(|| Arc::new(Utc::now)),
                inflight: Mutex::new(HashMap::new()),
            }),
        }
    }
}

impl<R: XReads + 'static> Inner<R> {
    async fn read<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let entry = match self.store.get(key).await {
            Ok(entry) => entry?,
            Err(error) => {
                tracing::warn!(key, %error, "x cache: could not read");
                return None;
            }
        };
        if entry.expires_at <= (self.now)() {
            return None;
        }
        serde_json::from_value(entry.value).ok()
    }

    async fn write(&self, key: String, kind: CacheKind, value: Value) {
        let stored_at = (self.now)();
        let ttl = chrono::Duration::from_std(self.ttl.of(kind)).unwrap_or(chrono::Duration::MAX);
        let entry = CacheEntry {
            key,
            kind,
            value,
            stored_at,
            expires_at: stored_at
                .checked_add_signed(ttl)
                .unwrap_or(DateTime::<Utc>::MAX_UTC),
        };
        let key = entry.key.clone();
        if let Err(error) = self.store.put(entry).await {
            tracing::warn!(key, %error, "x cache: could not write");
        }
    }

    /// Every post a list brings back is a post already paid for; keeping each
    /// under its own key is what makes the next `get_post` free.
    async fn remember(&self, posts: &[XPost]) {
        join_all(posts.iter().map(|post| {
            let value = serde_json::to_value(post).unwrap_or(Value::Null);
            self.write(format!("post:{}", post.id), CacheKind::Post, value)
        }))
        .await;
    }

    /// Joins the flight already asking for `key`, or starts one. The flight
    /// leaves the table when it lands, whoever is still waiting on it.
    fn flight<F>(self: &Arc<Self>, key: &str, fetch: F) -> Flight
    where
        F: FnOnce(Arc<Self>) -> BoxFuture<'static, XResult<Value>> + Send + 'static,
    {
        let mut inflight = self
            .inflight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(pending) = inflight.get(key) {
            return pending.clone();
        }
        let inner = self.clone();
        let owned_key = key.to_owned();
        let flight = async move {
            let result = fetch(inner.clone()).await.map_err(Arc::new);
            inner
                .inflight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&owned_key);
            result
        }
        .boxed()
        .shared();
        inflight.insert(key.to_owned(), flight.clone());
        flight
    }

    async fn through<T, F>(self: &Arc<Self>, key: String, kind: CacheKind, fetch: F) -> XResult<T>
    where
        T: DeserializeOwned,
        F: FnOnce(Arc<Self>) -> BoxFuture<'static, XResult<Value>> + Send + 'static,
    {
        if let Some(hit) = self.read::<T>(&key).await {
            tracing::debug!(key, ?kind, "x cache: hit");
            return Ok(hit);
        }
        let stored_key = key.clone();
        let value = self
            .flight(&key, move |inner| {
                async move {
                    let value = fetch(inner.clone()).await?;
                    inner.write(stored_key, kind, value.clone()).await;
                    Ok(value)
                }
                .boxed()
            })
            .await
            .map_err(unshare)?;
        serde_json::from_value(value).map_err(|error| XError::Decode(error.to_string()))
    }
}

/// A flight's error, for one of the callers that shared it.
fn unshare(error: Arc<XError>) -> XError {
    Arc::try_unwrap(error).unwrap_or_else(XError::Shared)
}

fn to_value<T: Serialize>(value: &T) -> XResult<Value> {
    serde_json::to_value(value).map_err(|error| XError::Decode(error.to_string()))
}

#[async_trait]
impl<R: XReads + 'static> XReads for CachedX<R> {
    async fn search_posts(&self, query: &str, options: &SearchOptions) -> XResult<Vec<XPost>> {
        let key = search_key(query, options, self.inner.ttl.search);
        let (query, options) = (query.to_owned(), options.clone());
        self.inner
            .through(key, CacheKind::Search, move |inner| {
                async move {
                    let posts = inner.x.search_posts(&query, &options).await?;
                    inner.remember(&posts).await;
                    to_value(&posts)
                }
                .boxed()
            })
            .await
    }

    async fn get_post(&self, id: &PostId) -> XResult<Option<XPost>> {
        let key = format!("post:{id}");
        if let Some(hit) = self.inner.read::<XPost>(&key).await {
            return Ok(Some(hit));
        }
        if self
            .inner
            .read::<Value>(&format!("missing:{id}"))
            .await
            .is_some()
        {
            return Ok(None);
        }
        let id = id.clone();
        let flight = self.inner.flight(&key, move |inner| {
            async move {
                let post = inner.x.get_post(&id).await?;
                match &post {
                    Some(found) => {
                        inner
                            .write(format!("post:{id}"), CacheKind::Post, to_value(found)?)
                            .await
                    }
                    None => {
                        inner
                            .write(format!("missing:{id}"), CacheKind::Missing, Value::Null)
                            .await
                    }
                }
                to_value(&post)
            }
            .boxed()
        });
        let value = flight.await.map_err(unshare)?;
        serde_json::from_value(value).map_err(|error| XError::Decode(error.to_string()))
    }

    async fn get_conversation(
        &self,
        conversation_id: &PostId,
        max_results: Option<u32>,
    ) -> XResult<Vec<XPost>> {
        let key = format!(
            "conversation:{conversation_id}:{}",
            max_results.map(|max| max.to_string()).unwrap_or_default()
        );
        let conversation_id = conversation_id.clone();
        self.inner
            .through(key, CacheKind::Conversation, move |inner| {
                async move {
                    let posts = inner
                        .x
                        .get_conversation(&conversation_id, max_results)
                        .await?;
                    inner.remember(&posts).await;
                    to_value(&posts)
                }
                .boxed()
            })
            .await
    }

    async fn trends(&self, options: TrendOptions) -> XResult<Vec<Trend>> {
        let key = format!(
            "trends:{}:{}",
            options.woeid.unwrap_or(1),
            options.max.map(|max| max.to_string()).unwrap_or_default()
        );
        self.inner
            .through(key, CacheKind::Trends, move |inner| {
                async move { to_value(&inner.x.trends(options).await?) }.boxed()
            })
            .await
    }

    async fn post_metrics(&self, ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids.to_vec();
        self.inner
            .through(metrics_key(&ids), CacheKind::OwnCounters, move |inner| {
                async move { to_value(&inner.x.post_metrics(&ids).await?) }.boxed()
            })
            .await
    }

    async fn follows_account(&self, user_id: &UserId) -> Option<bool> {
        let key = format!("follows:{user_id}");
        if let Some(hit) = self.inner.read::<bool>(&key).await {
            return Some(hit);
        }
        let answer = self.inner.x.follows_account(user_id).await;
        // An unknown answer is a failed lookup, not a fact; it is asked again.
        if let Some(follows) = answer {
            self.inner
                .write(key, CacheKind::Follow, Value::Bool(follows))
                .await;
        }
        answer
    }

    async fn user_by_username(&self, username: &str) -> XResult<Option<XUser>> {
        self.inner.x.user_by_username(username).await
    }
}

/// The search key: a digest of the query and the options that change the
/// answer, in the legacy key's JSON shape. A moving `start_time`/`end_time`
/// ("now minus an hour") is rounded to the search TTL, so the same query
/// asked twice inside one window is one request.
pub(crate) fn search_key(query: &str, options: &SearchOptions, search_ttl: Duration) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Key<'a> {
        query: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_results: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        since_id: Option<&'a PostId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sort_order: Option<SortOrder>,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_time: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_time: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pagination_token: Option<&'a str>,
    }
    let bucket = |time: &Option<String>| -> Option<String> {
        let time = time.as_ref()?;
        let window = search_ttl.as_millis() as i64;
        match DateTime::parse_from_rfc3339(time) {
            Ok(parsed) if window > 0 => {
                Some((parsed.timestamp_millis().div_euclid(window)).to_string())
            }
            _ => Some(time.clone()),
        }
    };
    let key = Key {
        query,
        max_results: options.max_results,
        since_id: options.since_id.as_ref(),
        sort_order: options.sort_order,
        start_time: bucket(&options.start_time),
        end_time: bucket(&options.end_time),
        pagination_token: options.pagination_token.as_deref(),
    };
    format!(
        "search:{}",
        digest(&serde_json::to_string(&key).unwrap_or_default())
    )
}

/// Own counters, keyed on the ids in any order.
pub(crate) fn metrics_key(ids: &[PostId]) -> String {
    let mut sorted: Vec<&str> = ids.iter().map(PostId::as_str).collect();
    sorted.sort_unstable();
    format!(
        "metrics:{}",
        digest(&serde_json::to_string(&sorted).unwrap_or_default())
    )
}

/// The first 24 hex characters of the SHA-256 of `json`.
pub(crate) fn digest(json: &str) -> String {
    Sha256::digest(json.as_bytes())
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
