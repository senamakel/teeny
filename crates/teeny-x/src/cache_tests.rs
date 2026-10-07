//! Port of legacy/test/x-cache.test.ts. X bills every post a read returns:
//! nothing is bought twice, a list asked for twice in one window is one
//! request, and the store failing costs a hit, never a read.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use teeny_core::types::{PostId, UserId};

use super::*;
use crate::model::{OwnPostMetrics, SearchOptions, Trend, TrendOptions, XPost, XUser};
use crate::{XError, XResult};

fn post(id: &str) -> XPost {
    XPost {
        author_id: Some("u1".into()),
        conversation_id: Some(id.into()),
        ..XPost::new(id, format!("post {id}"))
    }
}

#[derive(Default)]
struct Calls {
    search: AtomicUsize,
    get_post: AtomicUsize,
    conversation: AtomicUsize,
    trends: AtomicUsize,
    metrics: AtomicUsize,
    follows: AtomicUsize,
    users: AtomicUsize,
}

/// A fake X that counts what it is asked, and takes a moment to answer so
/// concurrent callers overlap.
#[derive(Default)]
struct FakeX {
    calls: Calls,
}

fn count(counter: &AtomicUsize) -> usize {
    counter.load(Ordering::SeqCst)
}

#[async_trait]
impl XReads for FakeX {
    async fn search_posts(&self, _query: &str, _options: &SearchOptions) -> XResult<Vec<XPost>> {
        self.calls.search.fetch_add(1, Ordering::SeqCst);
        Ok(vec![post("1"), post("2")])
    }
    async fn get_post(&self, id: &PostId) -> XResult<Option<XPost>> {
        self.calls.get_post.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok((id.as_str() != "404").then(|| post(id.as_str())))
    }
    async fn get_conversation(&self, id: &PostId, _max: Option<u32>) -> XResult<Vec<XPost>> {
        self.calls.conversation.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(vec![post(&format!("{id}-a")), post(&format!("{id}-b"))])
    }
    async fn trends(&self, _options: TrendOptions) -> XResult<Vec<Trend>> {
        self.calls.trends.fetch_add(1, Ordering::SeqCst);
        Ok(vec![Trend {
            name: "tiny".into(),
            posts: None,
        }])
    }
    async fn post_metrics(&self, ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        self.calls.metrics.fetch_add(1, Ordering::SeqCst);
        Ok(ids
            .iter()
            .map(|id| OwnPostMetrics {
                id: id.clone(),
                impressions: 1,
                likes: 0,
                replies: 0,
                reposts: 0,
                quotes: 0,
            })
            .collect())
    }
    async fn follows_account(&self, id: &UserId) -> Option<bool> {
        self.calls.follows.fetch_add(1, Ordering::SeqCst);
        (id.as_str() != "unknown").then_some(true)
    }
    async fn user_by_username(&self, username: &str) -> XResult<Option<XUser>> {
        self.calls.users.fetch_add(1, Ordering::SeqCst);
        Ok(Some(user(username)))
    }
}

fn user(username: &str) -> XUser {
    XUser {
        id: "9".into(),
        username: username.to_owned(),
        name: None,
        description: None,
        created_at: None,
        verified: None,
        followers: None,
        following: None,
        posts: None,
    }
}

/// A clock the test moves by hand.
fn clock(start: DateTime<Utc>) -> (Arc<Mutex<DateTime<Utc>>>, Clock) {
    let now = Arc::new(Mutex::new(start));
    let read = now.clone();
    (now, Arc::new(move || *read.lock().unwrap()))
}

fn at(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).unwrap()
}

fn cached(x: &Arc<FakeX>, options: CacheOptions) -> CachedX<FakeX> {
    CachedX::new(x.clone(), Arc::new(InMemoryReadCache::default()), options)
}

#[tokio::test]
async fn serves_a_post_from_the_store_after_the_first_read() {
    let x = Arc::new(FakeX::default());
    let c = cached(&x, CacheOptions::default());
    assert_eq!(c.get_post(&"7".into()).await.unwrap(), Some(post("7")));
    assert_eq!(c.get_post(&"7".into()).await.unwrap(), Some(post("7")));
    assert_eq!(count(&x.calls.get_post), 1);
}

#[tokio::test]
async fn remembers_every_post_a_search_or_a_conversation_brought_back() {
    let x = Arc::new(FakeX::default());
    let c = cached(&x, CacheOptions::default());
    c.search_posts("$TINY", &SearchOptions::default())
        .await
        .unwrap();
    c.get_conversation(&"9".into(), Some(25)).await.unwrap();
    assert_eq!(c.get_post(&"1".into()).await.unwrap(), Some(post("1")));
    assert_eq!(c.get_post(&"9-b".into()).await.unwrap(), Some(post("9-b")));
    assert_eq!(count(&x.calls.get_post), 0);
}

#[tokio::test]
async fn keeps_a_search_only_for_its_window_keyed_on_query_and_options() {
    let x = Arc::new(FakeX::default());
    let (now, clock) = clock(at(1_000_000));
    let ttl = CacheTtl {
        search: Duration::from_secs(60),
        ..CacheTtl::default()
    };
    let c = cached(
        &x,
        CacheOptions {
            ttl,
            now: Some(clock),
        },
    );
    let ten = SearchOptions {
        max_results: Some(10),
        ..SearchOptions::default()
    };
    c.search_posts("$TINY", &ten).await.unwrap();
    c.search_posts("$TINY", &ten).await.unwrap();
    assert_eq!(count(&x.calls.search), 1);
    let twenty = SearchOptions {
        max_results: Some(20),
        ..SearchOptions::default()
    };
    c.search_posts("$TINY", &twenty).await.unwrap();
    assert_eq!(count(&x.calls.search), 2);
    *now.lock().unwrap() = at(1_000_000 + 60_001);
    c.search_posts("$TINY", &ten).await.unwrap();
    assert_eq!(count(&x.calls.search), 3);
}

#[tokio::test]
async fn rounds_a_moving_time_window_to_the_search_ttl() {
    let x = Arc::new(FakeX::default());
    let start = 10 * 60_000;
    let (now, clock) = clock(at(start));
    let c = cached(
        &x,
        CacheOptions {
            ttl: CacheTtl::default(),
            now: Some(clock),
        },
    );
    let window = |ms: i64| SearchOptions {
        start_time: Some(at(ms - 3_600_000).to_rfc3339()),
        end_time: Some(at(ms - 60_000).to_rfc3339()),
        ..SearchOptions::default()
    };
    c.search_posts("$TINY", &window(start)).await.unwrap();
    *now.lock().unwrap() = at(start + 30_000);
    c.search_posts("$TINY", &window(start + 30_000))
        .await
        .unwrap();
    assert_eq!(count(&x.calls.search), 1);
}

#[tokio::test]
async fn remembers_that_a_post_was_missing_but_only_briefly() {
    let x = Arc::new(FakeX::default());
    let (now, clock) = clock(at(0));
    let c = cached(
        &x,
        CacheOptions {
            ttl: CacheTtl::default(),
            now: Some(clock),
        },
    );
    assert_eq!(c.get_post(&"404".into()).await.unwrap(), None);
    assert_eq!(c.get_post(&"404".into()).await.unwrap(), None);
    assert_eq!(count(&x.calls.get_post), 1);
    *now.lock().unwrap() = at(CacheTtl::default().missing.as_millis() as i64 + 1);
    c.get_post(&"404".into()).await.unwrap();
    assert_eq!(count(&x.calls.get_post), 2);
}

#[tokio::test]
async fn shares_one_flight_between_callers_asking_the_same_thing_at_once() {
    let x = Arc::new(FakeX::default());
    let c = cached(&x, CacheOptions::default());
    let id: PostId = "3".into();
    let (a, b, d, e) = tokio::join!(
        c.get_post(&id),
        c.get_post(&id),
        c.get_conversation(&id, None),
        c.get_conversation(&id, None)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(d.unwrap(), e.unwrap());
    assert_eq!(count(&x.calls.get_post), 1);
    assert_eq!(count(&x.calls.conversation), 1);
}

#[tokio::test]
async fn caches_trends_own_metrics_order_insensitively_and_a_known_follow_only() {
    let x = Arc::new(FakeX::default());
    let c = cached(&x, CacheOptions::default());
    c.trends(TrendOptions::default()).await.unwrap();
    c.trends(TrendOptions {
        woeid: Some(1),
        max: None,
    })
    .await
    .unwrap();
    assert_eq!(count(&x.calls.trends), 1);
    c.post_metrics(&["a".into(), "b".into()]).await.unwrap();
    c.post_metrics(&["b".into(), "a".into()]).await.unwrap();
    assert_eq!(count(&x.calls.metrics), 1);
    assert!(c.post_metrics(&[]).await.unwrap().is_empty());
    assert_eq!(count(&x.calls.metrics), 1);
    c.follows_account(&"u2".into()).await;
    c.follows_account(&"u2".into()).await;
    assert_eq!(count(&x.calls.follows), 1);
    c.follows_account(&"unknown".into()).await;
    c.follows_account(&"unknown".into()).await;
    assert_eq!(count(&x.calls.follows), 3);
}

struct BrokenStore;

#[async_trait]
impl ReadCache for BrokenStore {
    async fn get(&self, _key: &str) -> Result<Option<CacheEntry>, CacheError> {
        Err("mongo down".into())
    }
    async fn put(&self, _entry: CacheEntry) -> Result<(), CacheError> {
        Err("mongo down".into())
    }
}

#[tokio::test]
async fn goes_to_x_when_the_store_is_broken() {
    let x = Arc::new(FakeX::default());
    let c = CachedX::new(x.clone(), Arc::new(BrokenStore), CacheOptions::default());
    assert_eq!(c.get_post(&"5".into()).await.unwrap(), Some(post("5")));
    assert_eq!(c.get_post(&"5".into()).await.unwrap(), Some(post("5")));
    assert_eq!(count(&x.calls.get_post), 2);
}

#[tokio::test]
async fn a_failed_read_is_not_cached_and_reaches_every_waiter() {
    struct Failing(AtomicUsize);
    #[async_trait]
    impl XReads for Failing {
        async fn search_posts(&self, _q: &str, _o: &SearchOptions) -> XResult<Vec<XPost>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Err(XError::Http {
                status: 503,
                body: "down".into(),
            })
        }
        async fn get_post(&self, _id: &PostId) -> XResult<Option<XPost>> {
            unreachable!()
        }
        async fn get_conversation(&self, _id: &PostId, _m: Option<u32>) -> XResult<Vec<XPost>> {
            unreachable!()
        }
        async fn trends(&self, _o: TrendOptions) -> XResult<Vec<Trend>> {
            unreachable!()
        }
        async fn post_metrics(&self, _ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
            unreachable!()
        }
        async fn follows_account(&self, _id: &UserId) -> Option<bool> {
            unreachable!()
        }
        async fn user_by_username(&self, _username: &str) -> XResult<Option<XUser>> {
            unreachable!()
        }
    }
    let x = Arc::new(Failing(AtomicUsize::new(0)));
    let c = CachedX::new(
        x.clone(),
        Arc::new(InMemoryReadCache::default()),
        CacheOptions::default(),
    );
    let options = SearchOptions::default();
    let (a, b) = tokio::join!(c.search_posts("q", &options), c.search_posts("q", &options));
    assert!(a.unwrap_err().to_string().contains("503"));
    assert!(b.unwrap_err().to_string().contains("503"));
    assert_eq!(x.0.load(Ordering::SeqCst), 1);
    assert!(c.search_posts("q", &options).await.is_err());
    assert_eq!(x.0.load(Ordering::SeqCst), 2);
}

#[test]
fn keys_match_the_legacy_cache_so_existing_rows_are_hits() {
    // sha256 of the JSON the TypeScript keyed on, first 24 hex characters.
    assert_eq!(
        search_key(
            "$TINY",
            &SearchOptions {
                max_results: Some(10),
                ..SearchOptions::default()
            },
            Duration::from_secs(600)
        ),
        format!("search:{}", digest(r#"{"query":"$TINY","maxResults":10}"#))
    );
    assert_eq!(
        metrics_key(&["b".into(), "a".into()]),
        format!("metrics:{}", digest(r#"["a","b"]"#))
    );
    assert_eq!(digest("x"), "2d711642b726b04401627ca9");
}

#[test]
fn ttls_follow_the_configuration() {
    let ttl = CacheTtl::from_post_days_and_search_minutes(30.0, 5.0);
    assert_eq!(ttl.post, Duration::from_secs(30 * 24 * 3600));
    assert_eq!(ttl.search, Duration::from_secs(300));
    assert_eq!(ttl.conversation, Duration::from_secs(300));
    assert_eq!(ttl.trends, CacheTtl::default().trends);
}

fn x_config(cache_enabled: bool) -> XConfig {
    use teeny_core::Secret;
    XConfig {
        bot_username: "teenydroid".into(),
        api_key: Secret::new("k"),
        api_secret: Secret::new("s"),
        access_token: Secret::new("t"),
        access_token_secret: Secret::new("ts"),
        bearer_token: None,
        cache_enabled,
        cache_post_ttl_days: 30.0,
        cache_search_ttl_minutes: 10.0,
        mention_lookback_minutes: 15,
        search_max_results: 10,
        activity_stream_enabled: true,
        thread_context_max_posts: 10,
    }
}

#[tokio::test]
async fn x_cache_enabled_decides_whether_reads_go_through_the_store() {
    for (enabled, expected) in [(true, 1), (false, 2)] {
        let x = Arc::new(FakeX::default());
        let store: Arc<dyn ReadCache> = Arc::new(InMemoryReadCache::default());
        let reads = read_through(x.clone(), Some(store), &x_config(enabled));
        reads.get_post(&"7".into()).await.unwrap();
        reads.get_post(&"7".into()).await.unwrap();
        assert_eq!(
            count(&x.calls.get_post),
            expected,
            "X_CACHE_ENABLED={enabled}"
        );
    }
    let x = Arc::new(FakeX::default());
    let reads = read_through(x.clone(), None, &x_config(true));
    reads.get_post(&"7".into()).await.unwrap();
    reads.get_post(&"7".into()).await.unwrap();
    assert_eq!(count(&x.calls.get_post), 2, "no store, no cache");
}

#[tokio::test]
async fn looks_an_account_up_on_x_every_time() {
    let x = Arc::new(FakeX::default());
    let c = cached(&x, CacheOptions::default());
    assert_eq!(
        c.user_by_username("stonks").await.unwrap(),
        Some(user("stonks"))
    );
    assert_eq!(
        c.user_by_username("stonks").await.unwrap(),
        Some(user("stonks"))
    );
    assert_eq!(count(&x.calls.users), 2);
}
