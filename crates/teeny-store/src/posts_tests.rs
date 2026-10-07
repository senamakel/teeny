use serde_json::json;

use super::*;

fn own(post_id: &str, posted_at: &str) -> NewOwnPost {
    NewOwnPost {
        post_id: post_id.into(),
        kind: "chart".into(),
        posted_at: posted_at.into(),
        text: None,
    }
}

fn sample(at: &str, likes: i64) -> MetricSample {
    MetricSample {
        at: at.into(),
        impressions: 10,
        likes,
        replies: 0,
        reposts: 0,
        quotes: 0,
    }
}

/// The ledger's contract, run against every implementation.
async fn contract(ledger: &dyn PostLedger) {
    assert_eq!(ledger.latest().await.unwrap(), None);
    ledger
        .record(&own("1", "2026-09-13T07:00:00.000Z"))
        .await
        .unwrap();
    ledger
        .record(&NewOwnPost {
            text: Some("gm".into()),
            kind: "morning".into(),
            ..own("2", "2026-09-13T08:00:00.000Z")
        })
        .await
        .unwrap();
    // Recording again changes nothing.
    ledger
        .record(&own("2", "2026-09-13T09:00:00.000Z"))
        .await
        .unwrap();

    let latest = ledger.latest().await.unwrap().unwrap();
    assert_eq!(latest.post_id, "2");
    assert_eq!(latest.kind, "morning");
    assert_eq!(latest.posted_at, "2026-09-13T08:00:00.000Z");
    assert_eq!(latest.text.as_deref(), Some("gm"));
    assert!(latest.samples.is_empty());

    let since = |at: &'static str, limit| async move {
        ledger
            .since(at, limit)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.post_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(since("2026-09-13T07:00:00.000Z", 50).await, ["2", "1"]);
    assert_eq!(since("2026-09-13T07:30:00.000Z", 50).await, ["2"]);
    assert_eq!(since("2026-09-13T00:00:00.000Z", 1).await, ["2"]);
    assert_eq!(ledger.get("404").await.unwrap(), None);

    // The last hundred readings are kept.
    for i in 0..105 {
        ledger
            .sample(
                "1",
                &sample(&format!("2026-09-13T07:{:02}:00.000Z", i % 60), i),
            )
            .await
            .unwrap();
    }
    ledger
        .sample("404", &sample("2026-09-13T07:00:00.000Z", 1))
        .await
        .unwrap();
    let watched = ledger.get("1").await.unwrap().unwrap();
    assert_eq!(watched.samples.len(), 100);
    assert_eq!(watched.samples[0].likes, 5);
    assert_eq!(watched.samples[99].likes, 104);

    // The hot post whose hold is still running.
    assert_eq!(ledger.hold("2026-09-13T09:00:00.000Z").await.unwrap(), None);
    let hot = |since: &str, until: &str| Hot {
        since: since.into(),
        hold_until: until.into(),
    };
    ledger
        .set_hot(
            "1",
            &hot("2026-09-13T08:00:00.000Z", "2026-09-13T10:00:00.000Z"),
        )
        .await
        .unwrap();
    ledger
        .set_hot(
            "2",
            &hot("2026-09-13T08:30:00.000Z", "2026-09-13T11:00:00.000Z"),
        )
        .await
        .unwrap();
    assert_eq!(
        ledger
            .hold("2026-09-13T09:00:00.000Z")
            .await
            .unwrap()
            .unwrap()
            .post_id,
        "2"
    );
    assert_eq!(ledger.hold("2026-09-13T11:00:00.000Z").await.unwrap(), None);
    assert_eq!(
        ledger.get("1").await.unwrap().unwrap().hot,
        Some(hot("2026-09-13T08:00:00.000Z", "2026-09-13T10:00:00.000Z"))
    );
}

#[tokio::test]
async fn the_in_memory_post_ledger_keeps_the_contract() {
    contract(&InMemoryPostLedger::default()).await;
}

#[test]
fn an_own_post_is_spelled_as_the_typescript_service_stored_it() {
    let post = OwnPost {
        post_id: "1".into(),
        kind: "calendar:gm".into(),
        posted_at: "2026-09-13T07:00:00.000Z".into(),
        text: None,
        samples: vec![sample("2026-09-13T07:20:00.000Z", 3)],
        hot: Some(Hot {
            since: "a".into(),
            hold_until: "b".into(),
        }),
    };
    assert_eq!(
        serde_json::to_value(&post).unwrap(),
        json!({
            "postId": "1", "kind": "calendar:gm", "postedAt": "2026-09-13T07:00:00.000Z",
            "samples": [{ "at": "2026-09-13T07:20:00.000Z", "impressions": 10, "likes": 3, "replies": 0, "reposts": 0, "quotes": 0 }],
            "hot": { "since": "a", "holdUntil": "b" },
        })
    );
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_post_ledger_keeps_the_contract() {
    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("own_posts");
    contract(&MongoPostLedger::with_collection(&db, &name)).await;
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
}
