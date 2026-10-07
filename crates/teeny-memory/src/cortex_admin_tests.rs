use serde_json::json;
use teeny_core::Secret;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::{CortexMemory, ForgetSelector, MemoryError};

fn client(server: &MockServer) -> CortexMemory {
    CortexMemory::new(&server.uri(), "agent:teeny", Secret::new("tok"), "s1").unwrap()
}

#[tokio::test]
async fn lists_the_scopes_events() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .and(query_param("scope", "s1"))
        .and(query_param("view", "local"))
        .and(query_param("limit", "500"))
        .and(header("X-Cortex-Actor", "agent:teeny"))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{ "id": "e1", "content": { "kind": "agent_note" } }, { "id": "e2" }]
        })))
        .mount(&server)
        .await;

    let events = client(&server).events(500).await.unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["id"], "e1");
}

#[tokio::test]
async fn previews_a_forget_by_ids_redacting_events_and_never_scope_wide() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/forget/preview"))
        .and(body_json(json!({
            "scope": "s1",
            "selector": { "memory_ids": ["a", "b"] },
            "cascade": "redact_events",
            "confirm_all": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "preview_id": "p1", "estimated_deleted": { "events": 2 }
        })))
        .mount(&server)
        .await;

    let preview = client(&server)
        .forget_preview(&ForgetSelector::MemoryIds(vec!["a".into(), "b".into()]))
        .await
        .unwrap();

    assert_eq!(preview["preview_id"], "p1");
}

#[tokio::test]
async fn forgets_by_subject_from_the_preview() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/forget"))
        .and(body_json(json!({
            "scope": "s1",
            "selector": { "about_subject": "droplet" },
            "cascade": "redact_events",
            "confirm_all": false,
            "reason": "operational note leaking into public replies",
            "from_preview_id": "p1"
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "deleted": { "events": 1 } })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let result = client(&server)
        .forget(
            &ForgetSelector::AboutSubject("droplet".into()),
            "operational note leaking into public replies",
            Some("p1"),
        )
        .await
        .unwrap();

    assert_eq!(result["deleted"]["events"], 1);
}

#[tokio::test]
async fn a_refusal_is_the_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/forget/preview"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error_code": "AMBIGUOUS_SELECTOR", "message": "nope"
        })))
        .mount(&server)
        .await;

    let error = client(&server)
        .forget_preview(&ForgetSelector::AboutSubject("x".into()))
        .await
        .unwrap_err();

    assert!(
        matches!(error, MemoryError::Api { status: 422, ref code, .. } if code == "AMBIGUOUS_SELECTOR"),
        "{error:?}"
    );
}
