use serde_json::json;
use teeny_agent::deps::QueuedXPost;
use teeny_store::mentions::{MentionStatus, Reply};
use teeny_store::{InMemoryMentionQueue, MentionQueue};
use teeny_x::model::XPost;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::cmd::test_support::{printed, publisher, x};

/// Reply 900 answered mention 800, which the queue has as replied.
async fn setup() -> (MockServer, InMemoryMentionQueue<QueuedXPost>) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/900"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "900", "text": "rude", "referenced_tweets": [{ "type": "replied_to", "id": "800" }] }
        })))
        .mount(&server)
        .await;
    let queue = InMemoryMentionQueue::default();
    queue
        .cache(&[QueuedXPost(XPost::new("800", "gm teeny"))])
        .await
        .unwrap();
    queue
        .mark_replied(
            "800",
            &Reply {
                id: Some("900".into()),
                text: "rude".into(),
                ..Reply::default()
            },
        )
        .await
        .unwrap();
    (server, queue)
}

fn args(ids: &[&str], confirm: bool) -> UnsayArgs {
    UnsayArgs {
        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        confirm,
    }
}

async fn deletes(server: &MockServer, times: u64) {
    Mock::given(method("DELETE"))
        .and(path("/2/tweets/900"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })),
        )
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn without_confirm_it_names_what_it_would_do_and_changes_nothing() {
    let (server, queue) = setup().await;
    deletes(&server, 0).await;
    let mut out = Vec::new();

    run(
        &x(&server),
        &publisher(&server, false),
        &queue,
        &args(&["900"], false),
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(
        printed.contains("would delete 900 and requeue 800 (replied)"),
        "{printed}"
    );
    assert!(printed.contains("Nothing was changed"), "{printed}");
    assert_eq!(
        queue.status_of("800").await.unwrap(),
        Some(MentionStatus::Replied)
    );
}

#[tokio::test]
async fn with_confirm_it_deletes_and_requeues_the_mention_afresh() {
    let (server, queue) = setup().await;
    deletes(&server, 1).await;
    let mut out = Vec::new();

    run(
        &x(&server),
        &publisher(&server, false),
        &queue,
        &args(&["900"], true),
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(printed.contains("deleted 900"), "{printed}");
    assert!(
        printed.contains("requeued 800 for a fresh reply"),
        "{printed}"
    );
    let row = queue.get("800").unwrap();
    assert_eq!(row.status, MentionStatus::Pending);
    assert_eq!(row.attempts, 0);
    assert_eq!(row.reply_text, None);
}

#[tokio::test]
async fn dry_run_holds_the_delete_and_the_requeue() {
    let (server, queue) = setup().await;
    deletes(&server, 0).await;
    let mut out = Vec::new();

    run(
        &x(&server),
        &publisher(&server, true),
        &queue,
        &args(&["900"], true),
        &mut out,
    )
    .await
    .unwrap();

    assert!(printed(out).contains("DRY_RUN"));
    assert_eq!(
        queue.status_of("800").await.unwrap(),
        Some(MentionStatus::Replied)
    );
}

#[tokio::test]
async fn refuses_anything_that_is_not_a_post_id() {
    let (server, queue) = setup().await;

    let error = run(
        &x(&server),
        &publisher(&server, false),
        &queue,
        &args(&["abc"], true),
        &mut Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("usage"), "{error}");
}
