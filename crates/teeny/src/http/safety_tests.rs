//! Ported from legacy/test/safety-review.test.ts, "the review endpoints".

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{StatusCode, header};
use base64::Engine as _;
use serde_json::json;
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_safety::review::{Release, ReviewError};
use teeny_store::holds::{
    Direction, HoldBy, HoldContext, HoldDecision, HoldInput, HoldKind, HoldSurface,
};
use teeny_store::{Hold, HoldLog, HoldStatus, InMemoryHoldLog, InMemoryMentionQueue, MentionQueue};
use teeny_x::model::XPost;

use super::*;
use crate::http::test_support::*;
use crate::http::{Services, router};
use crate::mentions::QueuedXPost;

fn hold(text: &str) -> HoldInput {
    let mut context = HoldContext::new(HoldSurface::X);
    context.post_id = Some("10".into());
    context.author_username = Some("alice".into());
    HoldInput {
        direction: Direction::Inbound,
        kind: HoldKind::Message,
        context,
        category: "sexual".into(),
        reason: "contains \"naked\"".into(),
        by: HoldBy::Screen,
        text: text.into(),
        links: None,
    }
}

struct FailingRelease;

#[async_trait]
impl Release for FailingRelease {
    async fn release(&self, _hold: &Hold) -> Result<String, ReviewError> {
        Err(ReviewError::Effect("X is down".into()))
    }
}

struct Rig {
    app: axum::Router,
    holds: Arc<InMemoryHoldLog>,
    queue: Arc<InMemoryMentionQueue<QueuedXPost>>,
    gate: Arc<SafetyGate>,
}

async fn rig(release: Option<Arc<dyn Release>>) -> Rig {
    let holds = Arc::new(InMemoryHoldLog::default());
    let queue = Arc::new(InMemoryMentionQueue::default());
    queue
        .cache(&[QueuedXPost(XPost::new("10", "draw yourself naked"))])
        .await
        .unwrap();
    queue.mark_skipped("10", "held").await.unwrap();
    let gate = Arc::new(SafetyGate::new(SafetyGateOptions {
        holds: Some(holds.clone()),
        ..SafetyGateOptions::default()
    }));
    let app = router(Services {
        agent_token: Some(TOKEN.into()),
        queue: Some(queue.clone()),
        safety: Some(gate.clone()),
        review: Some(Review {
            holds: holds.clone(),
            release,
        }),
        ..services()
    });
    Rig {
        app,
        holds,
        queue,
        gate,
    }
}

fn basic() -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("operator:{TOKEN}"))
    )
}

#[tokio::test]
async fn challenges_a_browser_with_basic_auth_and_accepts_the_token_as_the_password() {
    let rig = rig(None).await;
    let anonymous = call(&rig.app, get("/safety")).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(
        anonymous.headers[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .contains("Basic")
    );
    assert_eq!(anonymous.json(), json!({ "error": "unauthorized" }));

    let page = call(&rig.app, get_as("/safety", &basic())).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(
        page.headers[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert!(page.text().contains("Held for review"));
    assert_eq!(page.text(), DASHBOARD_HTML);

    assert_eq!(
        call(&rig.app, get_as("/safety", &bearer())).await.status,
        StatusCode::OK
    );
    assert_eq!(
        call(&rig.app, get_as("/safety", "Bearer nope"))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn lists_holds_by_status_with_counts() {
    let rig = rig(None).await;
    rig.holds.record(hold("draw yourself naked")).await.unwrap();
    let second = rig.holds.record(hold("second")).await.unwrap();
    rig.holds
        .decide(&second.id, HoldDecision::Rejected, &Default::default())
        .await
        .unwrap();

    let pending = call(&rig.app, get_as("/safety/holds?status=pending", &basic())).await;
    assert_eq!(pending.status, StatusCode::OK);
    let body = pending.json();
    assert_eq!(
        body["counts"],
        json!({ "pending": 1, "approved": 0, "rejected": 1 })
    );
    let texts: Vec<&str> = body["holds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["draw yourself naked"]);

    let all = call(&rig.app, get_as("/safety/holds", &basic()))
        .await
        .json();
    assert_eq!(all["holds"].as_array().unwrap().len(), 2);
    let one = call(
        &rig.app,
        get_as("/safety/holds?status=bogus&limit=1", &basic()),
    )
    .await
    .json();
    assert_eq!(one["holds"].as_array().unwrap().len(), 1);
    assert_eq!(
        call(&rig.app, get("/safety/holds")).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn lists_a_hold_being_released_under_its_own_status_and_counts_it() {
    let rig = rig(None).await;
    let held = rig.holds.record(hold("gm frens")).await.unwrap();
    rig.holds.claim(&held.id).await.unwrap();
    // Newer, and pending: first in the list unless the filter applies.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    rig.holds.record(hold("later")).await.unwrap();
    let body = call(&rig.app, get_as("/safety/holds?status=releasing", &basic()))
        .await
        .json();
    assert_eq!(body["holds"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["holds"][0]["id"], held.id);
    assert_eq!(body["holds"][0]["status"], "releasing");
    assert_eq!(body["counts"]["releasing"], 1);
    assert!(
        DASHBOARD_HTML.contains("data-status=\"releasing\""),
        "the page can show them"
    );
}

#[tokio::test]
async fn takes_a_decision_and_reports_its_effect() {
    let rig = rig(None).await;
    let held = rig.holds.record(hold("draw yourself naked")).await.unwrap();
    let response = call(
        &rig.app,
        post_json_as(
            &format!("/safety/holds/{}/decision", held.id),
            &basic(),
            r#"{"decision":"approved","note":"a joke, clearly"}"#,
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["status"], "approved");
    assert_eq!(body["effect"], "requeued to be answered");
    assert_eq!(body["note"], "a joke, clearly");
    assert_eq!(rig.queue.get("10").unwrap().filter_approved, Some(true));
}

#[tokio::test]
async fn approving_a_cut_link_reaches_the_gate_at_once() {
    let rig = rig(None).await;
    let draft = "look https://weird.example/a";
    let context = HoldContext::new(HoldSurface::X);
    assert!(
        !rig.gate
            .outbound(draft, Some(&context))
            .await
            .text
            .contains("weird.example")
    );
    let link = rig
        .holds
        .list(Some(HoldStatus::Pending), None)
        .await
        .unwrap()
        .remove(0);

    let response = call(
        &rig.app,
        post_json_as(
            &format!("/safety/holds/{}/decision", link.id),
            &bearer(),
            r#"{"decision":"approved"}"#,
        ),
    )
    .await;
    assert_eq!(response.json()["effect"], "allowlisted weird.example");
    // Without the refresh the gate would hold its cached list for a minute.
    assert!(
        rig.gate
            .outbound(draft, None)
            .await
            .text
            .contains("weird.example")
    );
}

#[tokio::test]
async fn refuses_a_malformed_decision_and_404s_an_unknown_hold() {
    let rig = rig(None).await;
    let bad = call(
        &rig.app,
        post_json_as(
            "/safety/holds/x/decision",
            &basic(),
            r#"{"decision":"maybe"}"#,
        ),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.json()["error"], "decision must be approved or rejected");
    assert_eq!(bad.json()["details"][0]["path"], json!(["decision"]));
    let long_note = format!(r#"{{"decision":"rejected","note":"{}"}}"#, "n".repeat(501));
    let too_long = call(
        &rig.app,
        post_json_as("/safety/holds/x/decision", &basic(), &long_note),
    )
    .await;
    assert_eq!(too_long.status, StatusCode::BAD_REQUEST);

    let missing = call(
        &rig.app,
        post_json_as(
            "/safety/holds/x/decision",
            &basic(),
            r#"{"decision":"rejected"}"#,
        ),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.json(), json!({ "error": "no such hold" }));
}

#[tokio::test]
async fn answers_502_and_leaves_the_hold_pending_when_a_release_fails() {
    let rig = rig(Some(Arc::new(FailingRelease))).await;
    let mut draft = hold("a draft");
    draft.direction = Direction::Outbound;
    draft.kind = HoldKind::Draft;
    let held = rig.holds.record(draft).await.unwrap();

    let response = call(
        &rig.app,
        post_json_as(
            &format!("/safety/holds/{}/decision", held.id),
            &basic(),
            r#"{"decision":"approved"}"#,
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_GATEWAY);
    assert_eq!(response.json(), json!({ "error": "X is down" }));
    assert_eq!(
        rig.holds.get(&held.id).await.unwrap().unwrap().status,
        HoldStatus::Pending
    );
}

#[tokio::test]
async fn is_503_without_a_hold_log_or_without_a_token() {
    let app = router(Services {
        agent_token: Some(TOKEN.into()),
        ..services()
    });
    for request in [
        get_as("/safety", &basic()),
        get_as("/safety/holds", &basic()),
    ] {
        let answer = call(&app, request).await;
        assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            answer.json(),
            json!({ "error": "no hold log is configured" })
        );
    }
    let decision = call(
        &app,
        post_json_as(
            "/safety/holds/x/decision",
            &basic(),
            r#"{"decision":"rejected"}"#,
        ),
    )
    .await;
    assert_eq!(decision.status, StatusCode::SERVICE_UNAVAILABLE);

    let holds = Arc::new(InMemoryHoldLog::default());
    let tokenless = router(Services {
        review: Some(Review {
            holds,
            release: None,
        }),
        ..services()
    });
    let closed = call(&tokenless, get_as("/safety", &basic())).await;
    assert_eq!(closed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        closed.json(),
        json!({ "error": "AGENT_API_TOKEN is not set" })
    );
}
