use serde_json::json;
use teeny_core::Secret;
use wiremock::matchers::{body_json, body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::{Memory, MemoryPost};

fn client(server: &MockServer) -> CortexMemory {
    // A trailing slash on the URL is not doubled into the path.
    CortexMemory::new(
        &format!("{}/", server.uri()),
        "agent:teeny",
        Secret::new("tok"),
        "agent:teeny-scope",
    )
    .unwrap()
}

fn post() -> MemoryPost {
    MemoryPost {
        id: "10".into(),
        text: "gm".into(),
        author_id: Some("7".into()),
        author_username: Some("fren".into()),
        conversation_id: None,
        created_at: Some("2026-09-12T23:00:00.000Z".into()),
    }
}

#[tokio::test]
async fn captures_a_mention_with_the_actor_header_bearer_and_captured_wait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/experience"))
        .and(query_param("wait", "captured"))
        .and(header("x-cortex-actor", "agent:teeny"))
        .and(header("authorization", "Bearer tok"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "scope": "agent:teeny-scope",
            "modality": "conversation",
            "content": { "kind": "message", "role": "user", "text": "gm" },
            "context": { "observed_at": "2026-09-12T23:00:00.000Z", "labels": ["x", "mention", "@fren"] },
            "idempotency_key": "x:mention:2:10",
            "observed_actor": { "id": "user:x-7", "type": "person" },
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "event_id": "e1" })))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).capture_mention(&post()).await.unwrap();
}

#[tokio::test]
async fn a_reused_idempotency_key_is_already_stored_not_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/experience"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error_code": "IDEMPOTENCY_CONFLICT", "message": "key reused with a different body",
        })))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .capture_reply(&post(), "gm back", Some("99"))
        .await
        .unwrap();
}

#[tokio::test]
async fn any_other_refusal_surfaces_with_its_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/experience"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({ "error_code": "EXPIRED_TOKEN", "message": "expired" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
        .mount(&server)
        .await;

    let memory = client(&server);
    let error = memory
        .capture_activity("agent_note", "n1", json!({}), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, MemoryError::Api { status: 401, code, .. } if code == "EXPIRED_TOKEN"),
        "{error}"
    );
    assert_eq!(error.to_string(), "[401 EXPIRED_TOKEN] expired");
    let error = memory.recall("anything", None).await.unwrap_err();
    assert!(
        matches!(&error, MemoryError::Api { status: 502, code, message } if code == "UNKNOWN" && message == "bad gateway"),
        "{error}"
    );
}

#[tokio::test]
async fn recalls_the_person_and_the_topic_in_two_passes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(body_json(json!({
            "scope": "agent:teeny-scope",
            "filters": { "metadata": { "observed_actor": "user:x-7" } },
            "view": "holistic",
            "query": "Everything known about this person and past exchanges with them.",
            "include": ["events", "facts", "beliefs", "episodes"],
            "diagnostics": "none",
            "budgets": { "max_tokens": 600 },
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "pack_id": "p1", "context_block": "- asked about the chart" }),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(body_json(json!({
            "scope": "agent:teeny-scope",
            "view": "holistic",
            "query": "what is the chart",
            "include": ["events", "facts", "beliefs", "episodes"],
            "diagnostics": "none",
            "budgets": { "max_tokens": 1200 },
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "pack_id": "p2", "context_block": "- hourly candles" })),
        )
        .expect(2)
        .mount(&server)
        .await;

    let memory = client(&server);
    let fren = Person::x("7", Some("fren"));
    assert_eq!(
        memory
            .recall("what is the chart", Some(&fren))
            .await
            .unwrap(),
        "ABOUT @fren:\n- asked about the chart\n\nGENERAL CONTEXT:\n- hourly candles"
    );
    // Without a person, one pass.
    assert_eq!(
        memory.recall("what is the chart", None).await.unwrap(),
        "GENERAL CONTEXT:\n- hourly candles"
    );
}

#[tokio::test]
async fn an_empty_pack_recalls_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(body_partial_json(json!({ "query": "nothing" })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "pack_id": "p", "context_block": "" })),
        )
        .mount(&server)
        .await;
    assert_eq!(client(&server).recall("nothing", None).await.unwrap(), "");
}

#[test]
fn reads_its_settings_from_the_config_and_never_shows_the_token() {
    let config = teeny_core::config::CortexConfig {
        api_url: "https://api-v1.cortexdb.ai".into(),
        actor: "agent:teeny".into(),
        scope: "agent:teeny".into(),
        token: Secret::new("very-secret"),
    };
    let memory = CortexMemory::from_config(&config).unwrap();
    assert!(!format!("{memory:?}").contains("very-secret"));
}
