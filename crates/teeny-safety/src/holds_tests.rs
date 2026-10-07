//! The in-memory hold log, and the document shape the Mongo collection keeps.

use super::*;
use crate::screen::Direction;

fn input(text: &str) -> HoldInput {
    HoldInput {
        direction: Direction::Inbound,
        kind: HoldKind::Message,
        context: HoldContext {
            post_id: Some("10".into()),
            ..HoldContext::new(HoldSurface::X)
        },
        category: "sexual".into(),
        reason: "contains \"naked\"".into(),
        by: HeldBy::Screen,
        text: text.into(),
        links: None,
    }
}

#[tokio::test]
async fn lists_newest_first_by_status_and_counts_each_status() {
    let holds = MemoryHoldLog::default();
    holds.record(input("first")).await.unwrap();
    let second = holds.record(input("second")).await.unwrap();
    holds
        .decide(&second.id, Decision::Rejected, None, None)
        .await
        .unwrap();

    let texts = |list: Vec<Hold>| list.into_iter().map(|h| h.text).collect::<Vec<_>>();
    assert_eq!(
        texts(holds.list(None, None).await.unwrap()),
        ["second", "first"]
    );
    assert_eq!(
        texts(holds.list(Some(HoldStatus::Pending), None).await.unwrap()),
        ["first"]
    );
    assert_eq!(texts(holds.list(None, Some(1)).await.unwrap()), ["second"]);
    assert_eq!(
        holds.counts().await.unwrap(),
        HoldCounts {
            pending: 1,
            approved: 0,
            rejected: 1
        }
    );
}

#[tokio::test]
async fn gives_each_hold_its_own_id_and_records_the_decision() {
    let holds = MemoryHoldLog::default();
    let a = holds.record(input("a")).await.unwrap();
    let b = holds.record(input("b")).await.unwrap();
    assert_ne!(a.id, b.id);
    assert!(a.id.starts_with("h-"));
    let decided = holds
        .decide(
            &a.id,
            Decision::Approved,
            Some("fine".into()),
            Some("requeued".into()),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decided.status, HoldStatus::Approved);
    assert!(decided.decided_at.is_some());
    assert_eq!(
        (decided.note.as_deref(), decided.effect.as_deref()),
        (Some("fine"), Some("requeued"))
    );
    assert!(
        holds
            .decide("nope", Decision::Approved, None, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn keeps_one_entry_per_allowed_link() {
    let holds = MemoryHoldLog::default();
    holds.allow_link("dexscreener.com", "h1").await.unwrap();
    holds.allow_link("dexscreener.com", "h2").await.unwrap();
    assert_eq!(holds.allowed_links().await.unwrap(), ["dexscreener.com"]);
}

#[test]
fn serialises_in_the_legacy_document_shape() {
    let at = chrono::DateTime::parse_from_rfc3339("2026-09-14T07:00:00Z")
        .unwrap()
        .to_utc();
    let hold = Hold::new(input("draw yourself naked"), at);
    let json = serde_json::to_value(&hold).unwrap();
    assert_eq!(json["at"], "2026-09-14T07:00:00.000Z");
    assert_eq!(json["direction"], "inbound");
    assert_eq!(json["kind"], "message");
    assert_eq!(json["by"], "screen");
    assert_eq!(json["status"], "pending");
    assert_eq!(
        json["context"],
        serde_json::json!({ "surface": "x", "postId": "10" })
    );
    assert!(json.get("decidedAt").is_none() && json.get("links").is_none());
    let back: Hold = serde_json::from_value(json).unwrap();
    assert_eq!(back, hold);
}
