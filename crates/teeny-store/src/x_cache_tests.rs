use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::json;

use super::*;

const HOUR: Duration = Duration::from_secs(3_600);
const MINUTE: Duration = Duration::from_secs(60);

#[test]
fn trusts_each_kind_of_answer_as_long_as_the_typescript_service_did() {
    let ttl = XCacheTtl::default();
    assert_eq!(ttl.post, 30 * 24 * HOUR);
    assert_eq!(ttl.missing, HOUR);
    assert_eq!(ttl.search, 10 * MINUTE);
    assert_eq!(ttl.conversation, 10 * MINUTE);
    assert_eq!(ttl.trends, 30 * MINUTE);
    assert_eq!(ttl.metrics, 10 * MINUTE);
    assert_eq!(ttl.follows, 24 * HOUR);
    assert_eq!(ttl.of(XCacheKind::Missing), HOUR);
    assert_eq!(ttl.of(XCacheKind::Follows), 24 * HOUR);
}

#[test]
fn an_entry_is_fresh_until_the_moment_it_expires() {
    let now = Utc.with_ymd_and_hms(2026, 9, 13, 1, 0, 0).unwrap();
    let ttl = XCacheTtl {
        missing: MINUTE,
        ..XCacheTtl::default()
    };
    let entry = XCacheEntry::new("missing:404", XCacheKind::Missing, json!(null), now, &ttl);

    assert_eq!(entry.stored_at, "2026-09-13T01:00:00.000Z");
    assert_eq!(entry.expires_at, now + MINUTE);
    assert!(entry.is_fresh(now));
    assert!(entry.is_fresh(now + Duration::from_secs(59)));
    assert!(!entry.is_fresh(now + MINUTE));
}

fn entry(key: &str, value: serde_json::Value) -> XCacheEntry {
    let now = Utc.with_ymd_and_hms(2026, 9, 13, 1, 0, 0).unwrap();
    XCacheEntry::new(key, XCacheKind::Post, value, now, &XCacheTtl::default())
}

/// The store's contract, run against every implementation.
async fn contract(store: &dyn XCacheStore) {
    assert_eq!(store.get("post:1").await.unwrap(), None);

    let post = entry(
        "post:1",
        json!({ "id": "1", "text": "gm", "media": [{ "type": "photo" }], "metrics": { "likes": 3 } }),
    );
    store.set(&post).await.unwrap();
    assert_eq!(store.get("post:1").await.unwrap(), Some(post));

    // A key is one row: setting it again replaces it.
    let again = entry("post:1", json!({ "id": "1", "text": "gm again" }));
    store.set(&again).await.unwrap();
    assert_eq!(store.get("post:1").await.unwrap(), Some(again));

    let missing = XCacheEntry {
        kind: XCacheKind::Missing,
        ..entry("missing:2", json!(null))
    };
    store.set(&missing).await.unwrap();
    assert_eq!(store.get("missing:2").await.unwrap(), Some(missing));
}

#[tokio::test]
async fn the_in_memory_cache_keeps_the_contract() {
    contract(&InMemoryXCache::default()).await;
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_cache_keeps_the_contract_and_lets_mongo_expire_rows() {
    use futures::TryStreamExt;
    use mongodb::bson::{Bson, Document, doc};

    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("x_cache");
    contract(&MongoXCache::with_collection(&db, &name)).await;

    let raw = db.collection::<Document>(&name);
    let row = raw
        .find_one(doc! { "key": "post:1" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("kind").unwrap(), "post");
    assert_eq!(row.get_str("storedAt").unwrap(), "2026-09-13T01:00:00.000Z");
    // A BSON date, or the TTL index would never sweep the row.
    assert!(matches!(row.get("expiresAt"), Some(Bson::DateTime(_))));
    let indexes: Vec<_> = raw
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let ttl = indexes
        .iter()
        .find(|index| index.keys == doc! { "expiresAt": 1 })
        .expect("a TTL index");
    assert_eq!(
        ttl.options.as_ref().and_then(|o| o.expire_after),
        Some(Duration::ZERO)
    );
    raw.drop().await.unwrap();
}
