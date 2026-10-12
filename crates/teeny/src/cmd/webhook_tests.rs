use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::cmd::test_support::{printed, x};

const OLD: &str = "https://old.example/webhooks/x";
const NEW: &str = "https://new.example/webhooks/x";

async fn webhooks(server: &MockServer, hooks: Value) {
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": hooks })))
        .mount(server)
        .await;
}

async fn subscriptions(server: &MockServer, subs: Value, first: Option<u64>) {
    let mock = Mock::given(method("GET"))
        .and(path("/2/activity/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": subs })));
    match first {
        Some(times) => mock.up_to_n_times(times).with_priority(1),
        None => mock.with_priority(5),
    }
    .mount(server)
    .await;
}

async fn me(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "42", "username": "teenydroid" }
        })))
        .mount(server)
        .await;
}

fn all_events(webhook: &str) -> Value {
    Value::Array(
        EVENTS
            .iter()
            .enumerate()
            .map(|(n, event)| {
                json!({ "subscription_id": format!("s{n}"), "event_type": event, "webhook_id": webhook, "filter": { "user_id": "42" } })
            })
            .collect(),
    )
}

async fn writes(server: &MockServer, verb: &str, times: u64) {
    Mock::given(method(verb))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(10)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_shows_each_webhook_and_its_subscriptions() {
    let server = MockServer::start().await;
    webhooks(&server, json!([{ "id": "w1", "url": OLD, "valid": true }])).await;
    subscriptions(&server, json!([{ "subscription_id": "s1", "event_type": "like.create", "webhook_id": "w1", "filter": { "user_id": "42" } }]), None).await;
    let mut out = Vec::new();

    run(&x(&server), "teenydroid", &WebhookCommand::List, &mut out)
        .await
        .unwrap();

    let printed = printed(out);
    assert!(printed.contains("Account: @teenydroid"), "{printed}");
    assert!(printed.contains(OLD), "{printed}");
    assert!(printed.contains("id w1   valid true"), "{printed}");
    assert!(
        printed.contains("subscriptions: like.create for 42"),
        "{printed}"
    );
}

#[tokio::test]
async fn list_says_when_nothing_is_registered() {
    let server = MockServer::start().await;
    webhooks(&server, json!([])).await;
    let mut out = Vec::new();

    run(&x(&server), "teenydroid", &WebhookCommand::List, &mut out)
        .await
        .unwrap();

    assert!(printed(out).contains("Nothing is registered"));
}

#[tokio::test]
async fn register_without_confirm_previews_the_move_and_writes_nothing() {
    let server = MockServer::start().await;
    webhooks(&server, json!([{ "id": "w-old", "url": OLD }])).await;
    writes(&server, "POST", 0).await;
    writes(&server, "DELETE", 0).await;
    let mut out = Vec::new();

    run(
        &x(&server),
        "teenydroid",
        &WebhookCommand::Register {
            url: NEW.into(),
            confirm: false,
        },
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(
        printed.contains(&format!("Would register: {NEW}")),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("Would first retire: {OLD} (w-old)")),
        "{printed}"
    );
    assert!(printed.contains("Re-run with --confirm"), "{printed}");
}

#[tokio::test]
async fn register_retires_the_old_one_registers_and_subscribes_every_event() {
    let server = MockServer::start().await;
    webhooks(&server, json!([{ "id": "w-old", "url": OLD }])).await;
    me(&server).await;
    // Before: one stale subscription on the old webhook, none on the new.
    subscriptions(
        &server,
        json!([{ "subscription_id": "old-1", "event_type": "like.create", "webhook_id": "w-old", "filter": { "user_id": "42" } }]),
        Some(2),
    )
    .await;
    // After: all six on the new one.
    subscriptions(&server, all_events("w-new"), None).await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/old-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/webhooks/w-old"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/webhooks"))
        .and(body_partial_json(json!({ "url": NEW })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": { "id": "w-new", "url": NEW } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/activity/subscriptions"))
        .and(body_partial_json(
            json!({ "webhook_id": "w-new", "filter": { "user_id": "42" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {} })))
        .expect(6)
        .mount(&server)
        .await;
    let mut out = Vec::new();

    run(
        &x(&server),
        "teenydroid",
        &WebhookCommand::Register {
            url: NEW.into(),
            confirm: true,
        },
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(
        printed.contains("unsubscribed like.create for 42"),
        "{printed}"
    );
    assert!(printed.contains("id w-new"), "{printed}");
    assert!(
        printed.contains("subscribed post.mention.create"),
        "{printed}"
    );
    assert!(printed.contains("Done."), "{printed}");
}

#[tokio::test]
async fn a_refused_registration_puts_the_previous_url_back() {
    let server = MockServer::start().await;
    webhooks(&server, json!([{ "id": "w-old", "url": OLD }])).await;
    subscriptions(&server, json!([]), None).await;
    Mock::given(method("DELETE"))
        .and(path("/2/webhooks/w-old"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/webhooks"))
        .and(body_partial_json(json!({ "url": NEW })))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({ "errors": [{ "message": "CrcValidationFailed" }] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/webhooks"))
        .and(body_partial_json(json!({ "url": OLD })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "w-back" } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut out = Vec::new();

    let error = run(
        &x(&server),
        "teenydroid",
        &WebhookCommand::Register {
            url: NEW.into(),
            confirm: true,
        },
        &mut out,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("CrcValidationFailed"), "{error}");
    assert!(printed(out).contains(&format!("refused; putting {OLD} back")));
}

#[tokio::test]
async fn subscribe_without_confirm_lists_the_missing_events() {
    let server = MockServer::start().await;
    webhooks(&server, json!([{ "id": "w1", "url": OLD }])).await;
    me(&server).await;
    subscriptions(&server, json!([{ "subscription_id": "s1", "event_type": "like.create", "webhook_id": "w1", "filter": { "user_id": "42" } }]), None).await;
    writes(&server, "POST", 0).await;
    let mut out = Vec::new();

    run(
        &x(&server),
        "teenydroid",
        &WebhookCommand::Subscribe {
            webhook_id: None,
            confirm: false,
        },
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(
        printed.contains("Would subscribe w1 to post.mention.create"),
        "{printed}"
    );
    assert!(!printed.contains("to like.create"), "{printed}");
}

#[tokio::test]
async fn delete_with_confirm_unsubscribes_then_deletes() {
    let server = MockServer::start().await;
    webhooks(&server, json!([{ "id": "w1", "url": OLD }])).await;
    subscriptions(&server, json!([{ "subscription_id": "s1", "event_type": "like.create", "webhook_id": "w1", "filter": { "user_id": "42" } }]), None).await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/s1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/webhooks/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let mut out = Vec::new();

    run(
        &x(&server),
        "teenydroid",
        &WebhookCommand::Delete {
            webhook_id: "w1".into(),
            confirm: true,
        },
        &mut out,
    )
    .await
    .unwrap();

    assert!(printed(out).contains("Deleted w1."));
}

#[tokio::test]
async fn delete_without_confirm_touches_nothing() {
    let server = MockServer::start().await;
    writes(&server, "DELETE", 0).await;
    let mut out = Vec::new();

    run(
        &x(&server),
        "teenydroid",
        &WebhookCommand::Delete {
            webhook_id: "w1".into(),
            confirm: false,
        },
        &mut out,
    )
    .await
    .unwrap();

    assert!(printed(out).contains("Would delete webhook w1. Re-run with --confirm."));
}
