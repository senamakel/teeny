use serde_json::json;
use teeny_core::Secret;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::cmd::test_support::printed;

fn memory(server: &MockServer) -> CortexMemory {
    CortexMemory::new(&server.uri(), "agent:teeny", Secret::new("tok"), "s1").unwrap()
}

fn args(subject: Option<&str>, find: Option<&str>, ids: &[&str], confirm: bool) -> ForgetArgs {
    ForgetArgs {
        subject: subject.map(str::to_owned),
        find: find.map(str::to_owned),
        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        confirm,
    }
}

async fn preview(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/forget/preview"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "preview_id": "p1", "estimated_deleted": { "events": 2 }
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn find_lists_the_matching_events_with_their_ids() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "items": [
            { "id": "e1", "content": { "note": "the Droplet server" } },
            { "id": "e2", "content": { "note": "gm" } }
        ] })))
        .mount(&server)
        .await;
    let mut out = Vec::new();

    run(
        &memory(&server),
        &args(None, Some("droplet"), &[], false),
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(printed.starts_with("e1\t{"), "{printed}");
    assert!(!printed.contains("e2"), "{printed}");
}

#[tokio::test]
async fn without_confirm_it_only_previews() {
    let server = MockServer::start().await;
    preview(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/forget"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let mut out = Vec::new();

    run(
        &memory(&server),
        &args(None, None, &["a", "b"], false),
        &mut out,
    )
    .await
    .unwrap();

    let printed = printed(out);
    assert!(printed.contains("\"preview_id\": \"p1\""), "{printed}");
    assert!(printed.contains("Re-run with --confirm"), "{printed}");
}

#[tokio::test]
async fn with_confirm_it_forgets_from_the_preview() {
    let server = MockServer::start().await;
    preview(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/forget"))
        .and(body_partial_json(json!({
            "selector": { "about_subject": "the droplet" },
            "from_preview_id": "p1",
            "confirm_all": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "forgotten": 2 })))
        .expect(1)
        .mount(&server)
        .await;
    let mut out = Vec::new();

    run(
        &memory(&server),
        &args(Some("the droplet"), None, &[], true),
        &mut out,
    )
    .await
    .unwrap();

    assert!(printed(out).contains("\"forgotten\": 2"));
}

#[tokio::test]
async fn refuses_without_a_selector() {
    let server = MockServer::start().await;

    let error = run(
        &memory(&server),
        &args(None, None, &[], true),
        &mut Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("usage"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}
