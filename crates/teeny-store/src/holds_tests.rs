use serde_json::json;

use super::*;

fn hold(text: &str) -> HoldInput {
    HoldInput {
        direction: Direction::Inbound,
        kind: HoldKind::Message,
        context: HoldContext {
            post_id: Some("10".into()),
            author_username: Some("alice".into()),
            ..HoldContext::new(HoldSurface::X)
        },
        category: "sexual".into(),
        reason: "contains a word".into(),
        by: HoldBy::Screen,
        text: text.into(),
        links: None,
    }
}

/// The hold log's contract, run against every implementation.
async fn contract(holds: &dyn HoldLog) {
    let first = holds.record(hold("one")).await.unwrap();
    assert!(first.id.starts_with("h-"), "{}", first.id);
    assert_eq!(first.status, HoldStatus::Pending);
    assert_eq!(first.decided_at, None);
    // Two holds in one millisecond would tie on `at`.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = holds
        .record(HoldInput {
            kind: HoldKind::Link,
            direction: Direction::Outbound,
            by: HoldBy::Links,
            links: Some(vec![HeldLink {
                link: "https://x.example".into(),
                reason: "not allowlisted".into(),
            }]),
            ..hold("two")
        })
        .await
        .unwrap();

    assert_eq!(holds.get(&first.id).await.unwrap(), Some(first.clone()));
    assert_eq!(holds.get("h-none").await.unwrap(), None);

    // Newest first, by status, and limited.
    let all = holds.list(None, None).await.unwrap();
    assert_eq!(
        all.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        ["two", "one"]
    );
    assert_eq!(all[0].links, second.links);
    assert_eq!(holds.list(None, Some(1)).await.unwrap().len(), 1);
    assert_eq!(
        holds.list(None, Some(0)).await.unwrap().len(),
        1,
        "a limit is at least one"
    );

    let decided = holds
        .decide(
            &first.id,
            HoldDecision::Approved,
            &DecisionDetail {
                note: Some("fine".into()),
                effect: Some("requeued".into()),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decided.status, HoldStatus::Approved);
    assert!(decided.decided_at.is_some());
    assert_eq!(decided.note.as_deref(), Some("fine"));
    assert_eq!(decided.effect.as_deref(), Some("requeued"));
    assert_eq!(
        holds
            .decide("h-none", HoldDecision::Rejected, &DecisionDetail::default())
            .await
            .unwrap(),
        None
    );

    let pending = holds.list(Some(HoldStatus::Pending), None).await.unwrap();
    assert_eq!(
        pending.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        ["two"]
    );
    assert_eq!(
        holds.counts().await.unwrap(),
        HoldCounts {
            pending: 1,
            approved: 1,
            rejected: 0,
            releasing: 0
        }
    );

    // An approved host is allowed everywhere the gate runs, once.
    assert!(holds.allowed_links().await.unwrap().is_empty());
    holds.allow_link("x.example", &second.id).await.unwrap();
    holds.allow_link("x.example", &first.id).await.unwrap();
    holds.allow_link("*.y.example", &second.id).await.unwrap();
    let mut allowed = holds.allowed_links().await.unwrap();
    allowed.sort();
    assert_eq!(allowed, ["*.y.example", "x.example"]);

    // A decision claims the hold first, once: two approvals at the same
    // moment get one claim between them.
    let third = holds.record(hold("three")).await.unwrap();
    let (a, b) = tokio::join!(holds.claim(&third.id), holds.claim(&third.id));
    let claimed: Vec<Hold> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(claimed.len(), 1, "{claimed:?}");
    assert_eq!(claimed[0].status, HoldStatus::Releasing);
    assert_eq!(claimed[0].text, "three");
    assert_eq!(holds.claim(&first.id).await.unwrap(), None, "decided");
    assert_eq!(holds.claim("h-none").await.unwrap(), None);
    assert_eq!(holds.counts().await.unwrap().releasing, 1);
    assert_eq!(
        holds.list(Some(HoldStatus::Releasing), None).await.unwrap()[0].id,
        third.id
    );

    // A release that failed puts it back, saying why.
    let back = holds
        .unclaim(&third.id, "X is down")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.status, HoldStatus::Pending);
    assert!(back.effect.unwrap().contains("X is down"));
    assert_eq!(
        holds.unclaim(&third.id, "again").await.unwrap(),
        None,
        "only a claimed hold goes back"
    );
    assert!(holds.claim(&third.id).await.unwrap().is_some());
}

#[tokio::test]
async fn the_in_memory_hold_log_keeps_the_contract() {
    contract(&InMemoryHoldLog::default()).await;
}

#[test]
fn a_hold_is_spelled_as_the_typescript_service_stored_it() {
    let hold = Hold {
        id: "h-1".into(),
        at: "2026-09-14T00:00:00.000Z".into(),
        direction: Direction::Outbound,
        kind: HoldKind::Draft,
        context: HoldContext {
            chat_id: Some("c".into()),
            message_thread_id: Some("t".into()),
            ..HoldContext::new(HoldSurface::Telegram)
        },
        category: "unknown".into(),
        reason: "r".into(),
        by: HoldBy::Judge,
        text: "t".into(),
        links: None,
        status: HoldStatus::Rejected,
        decided_at: Some("d".into()),
        note: None,
        effect: None,
    };
    assert_eq!(
        serde_json::to_value(&hold).unwrap(),
        json!({
            "id": "h-1", "at": "2026-09-14T00:00:00.000Z", "direction": "outbound", "kind": "draft",
            "context": { "surface": "telegram", "chatId": "c", "messageThreadId": "t" },
            "category": "unknown", "reason": "r", "by": "judge", "text": "t",
            "status": "rejected", "decidedAt": "d",
        })
    );
}

#[test]
fn a_hold_being_released_is_spelled_releasing_and_the_counts_keep_their_shape() {
    assert_eq!(
        serde_json::to_value(HoldStatus::Releasing).unwrap(),
        json!("releasing")
    );
    assert_eq!(
        serde_json::to_value(HoldCounts::default()).unwrap(),
        json!({ "pending": 0, "approved": 0, "rejected": 0 })
    );
    assert_eq!(
        serde_json::to_value(HoldCounts {
            releasing: 2,
            ..HoldCounts::default()
        })
        .unwrap(),
        json!({ "pending": 0, "approved": 0, "rejected": 0, "releasing": 2 })
    );
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_hold_log_keeps_the_contract() {
    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("holds");
    contract(&MongoHoldLog::with_collection(&db, &name)).await;
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
    db.collection::<mongodb::bson::Document>(&format!("{name}_links"))
        .drop()
        .await
        .unwrap();
}
