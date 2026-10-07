use std::sync::Arc;

use serde_json::json;
use teeny_cadence::cadence::{Cadence, CadenceRules};
use teeny_cadence::clock::SystemClock;
use teeny_core::Secret;
use teeny_store::posts::{InMemoryPostLedger, PostLedger};
use teeny_x::oauth::OAuth1;
use teeny_x::{NoopGuard, XApi, XPublisher};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn x_at(server: &MockServer) -> Arc<XApi> {
    let oauth = OAuth1::new("k", Secret::new("s"), "t", Secret::new("ts"));
    Arc::new(XApi::new("teenydroid", oauth, None).with_base_url(server.uri()))
}

fn cadence(ledger: Arc<InMemoryPostLedger>) -> Arc<Cadence> {
    Arc::new(Cadence::new(
        ledger,
        CadenceRules {
            min_gap: chrono::TimeDelta::hours(4),
            tolerance: chrono::TimeDelta::zero(),
        },
        Arc::new(SystemClock),
    ))
}

fn post(kind: &str) -> OwnPost {
    OwnPost {
        kind: kind.into(),
        text: "gm builders".into(),
        media: Some(Attachment {
            data: vec![0x89, 0x50, 0x4e, 0x47],
            mime_type: MimeType::Png,
        }),
        quote: None,
    }
}

#[tokio::test]
async fn uploads_posts_and_records_the_own_post_under_its_kind() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/initialize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "m1" } })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/append"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "m1" } })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_partial_json(json!({ "text": "gm builders", "media": { "media_ids": ["m1"] } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "9001" } })))
        .expect(1)
        .mount(&server)
        .await;
    let x = x_at(&server);
    let ledger = Arc::new(InMemoryPostLedger::default());
    let poster = XOwnPoster::new(
        Arc::clone(&x),
        Arc::new(XPublisher::new(x, Arc::new(NoopGuard), false)),
        cadence(Arc::clone(&ledger)),
    );

    let posted = poster.post(post("morning")).await.unwrap();
    assert_eq!(posted.id, "9001");
    assert!(!posted.dry_run);
    let last = ledger.latest().await.unwrap().unwrap();
    assert_eq!((last.post_id.as_str(), last.kind.as_str()), ("9001", "morning"));

    // A second own post inside the gap is the cadence's to refuse.
    let refused = poster.post(post("moon")).await.unwrap_err();
    assert!(matches!(refused, PostFailure::Held(_)), "{refused:?}");
}

#[tokio::test]
async fn a_dry_run_touches_neither_x_nor_the_ledger() {
    let server = MockServer::start().await;
    let x = x_at(&server);
    let ledger = Arc::new(InMemoryPostLedger::default());
    let poster = XOwnPoster::new(
        Arc::clone(&x),
        Arc::new(XPublisher::new(x, Arc::new(NoopGuard), true)),
        cadence(Arc::clone(&ledger)),
    );
    let posted = poster.post(post("morning")).await.unwrap();
    assert!(posted.dry_run);
    assert!(posted.id.starts_with("dry-run-"));
    assert!(ledger.latest().await.unwrap().is_none());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_quote_carries_the_quoted_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_partial_json(json!({ "quote_tweet_id": "77" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "9002" } })))
        .expect(1)
        .mount(&server)
        .await;
    let x = x_at(&server);
    let poster = XOwnPoster::new(
        Arc::clone(&x),
        Arc::new(XPublisher::new(x, Arc::new(NoopGuard), false)),
        cadence(Arc::new(InMemoryPostLedger::default())),
    );
    let quote = OwnPost {
        media: None,
        quote: Some(PostId::from("77")),
        ..post("quote")
    };
    assert_eq!(poster.post(quote).await.unwrap().id, "9002");
}
