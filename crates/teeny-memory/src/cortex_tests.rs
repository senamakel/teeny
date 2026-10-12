use std::time::{Duration, Instant};

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

fn pack(block: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "pack_id": "p", "context_block": block }))
}

fn person_pass() -> impl wiremock::Match {
    body_partial_json(json!({ "filters": { "metadata": { "labels": "@ana" } } }))
}

fn general_pass() -> impl wiremock::Match {
    body_partial_json(json!({ "filters": { "metadata": { "labels": RECALL_LABELS } } }))
}

fn ana() -> Person {
    Person::x("1", Some("ana"))
}

fn with(server: &MockServer, options: CortexOptions) -> CortexMemory {
    client(server).with_options(options)
}

/// Port of `legacy/test/cortex-memory.test.ts`, `CortexMemory.recall`.
#[tokio::test]
async fn asks_a_pack_about_the_person_by_their_handle_label_and_one_over_conversation_only() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(body_json(json!({
            "scope": "agent:teeny-scope",
            "view": "holistic",
            "query": "What has this person said, and what did you say back?",
            "include": ["events", "facts", "beliefs", "episodes"],
            "diagnostics": "none",
            "budgets": { "max_tokens": 600 },
            // The handle label is what every exchange with a person carries;
            // the observed_actor filter it replaced never came back.
            "filters": { "metadata": { "labels": "@ana" } },
        })))
        .respond_with(pack("they asked about the roadmap"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(body_json(json!({
            "scope": "agent:teeny-scope",
            "view": "holistic",
            "query": "what about the roadmap?",
            "include": ["events", "facts", "beliefs", "episodes"],
            "diagnostics": "none",
            "budgets": { "max_tokens": 1200 },
            // Conversation, notes and own posts; never raw webhook activity.
            "filters": { "metadata": { "labels": RECALL_LABELS } },
        })))
        .respond_with(pack("the project ships weekly"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(!RECALL_LABELS.contains(&"activity"));

    let text = client(&server)
        .recall("what about the roadmap?", Some(&ana()))
        .await
        .unwrap();
    assert_eq!(
        text,
        "ABOUT @ana:\nthey asked about the roadmap\n\nGENERAL CONTEXT:\nthe project ships weekly"
    );
}

#[tokio::test]
async fn recalls_generally_about_someone_known_only_by_id_since_only_a_handle_can_find_them() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(general_pass())
        .respond_with(pack("context"))
        .expect(1)
        .mount(&server)
        .await;
    let nameless = Person::x("1", None);
    assert_eq!(
        client(&server).recall("hi", Some(&nameless)).await.unwrap(),
        "GENERAL CONTEXT:\ncontext"
    );
}

#[tokio::test]
async fn keeps_the_general_context_when_only_the_person_pass_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(person_pass())
        .respond_with(
            ResponseTemplate::new(503).set_body_json(json!({ "error_code": "DB_UNAVAILABLE" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(general_pass())
        .respond_with(pack("the project ships weekly"))
        .mount(&server)
        .await;
    assert_eq!(
        client(&server).recall("q", Some(&ana())).await.unwrap(),
        "GENERAL CONTEXT:\nthe project ships weekly"
    );
}

#[tokio::test]
async fn fails_when_both_passes_fail_so_the_caller_can_say_memory_was_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .respond_with(
            ResponseTemplate::new(503).set_body_json(json!({ "error_code": "DB_UNAVAILABLE" })),
        )
        .mount(&server)
        .await;
    let error = client(&server).recall("q", Some(&ana())).await.unwrap_err();
    assert!(
        matches!(error, MemoryError::Api { status: 503, .. }),
        "{error}"
    );
}

#[tokio::test]
async fn gives_up_on_a_recall_at_its_own_deadline_rather_than_the_sdk_minute() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .respond_with(pack("late").set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let memory = with(
        &server,
        CortexOptions {
            recall_timeout: Duration::from_millis(200),
            ..CortexOptions::default()
        },
    );
    let started = Instant::now();
    let error = memory.recall("q", None).await.unwrap_err();
    assert_eq!(error.to_string(), "request timed out after 200ms");
    assert!(error.is_timeout());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn without_the_person_pass_asks_one_pack_naming_the_person_never_as_a_subject() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(general_pass())
        .and(body_partial_json(
            json!({ "query": "@ana is the door open?" }),
        ))
        .respond_with(pack("the lab door is closed"))
        .expect(1)
        .mount(&server)
        .await;
    let memory = with(
        &server,
        CortexOptions {
            person_pass: false,
            ..CortexOptions::default()
        },
    );
    assert_eq!(
        memory
            .recall("is the door open?", Some(&ana()))
            .await
            .unwrap(),
        "GENERAL CONTEXT:\nthe lab door is closed"
    );
    // Never shaped as a subject: "About @ana" sends the store down its
    // entity-dossier path, which did not return.
    let sent: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.body_json().unwrap())
        .collect();
    for body in sent {
        let query = body["query"].as_str().unwrap().to_lowercase();
        assert!(
            !["about", "regarding", "everything known"]
                .iter()
                .any(|subject| query.starts_with(subject)),
            "{query}"
        );
    }
}

/// `capPack`.
#[test]
fn cuts_an_over_budget_pack_at_a_memory_boundary() {
    let items: Vec<String> = (0..1_000)
        .map(|index| format!("memory {index} ").repeat(20))
        .collect();
    let capped = cap_pack(&items.join("\n\n---\n\n"), 1_000);
    assert!(capped.len() <= 1_000);
    assert!(capped.starts_with("memory 0 "));
    // Whole items only: the last one kept ends where an item ends.
    for item in capped.split("\n\n---\n\n") {
        let index: usize = item.split(' ').nth(1).unwrap().parse().unwrap();
        assert_eq!(item, items[index]);
    }
}

#[test]
fn leaves_a_pack_inside_its_budget_alone_and_hard_cuts_a_single_giant_item() {
    assert_eq!(cap_pack("short", 100), "short");
    assert_eq!(cap_pack(&"x".repeat(500), 100).len(), 100);
}

#[tokio::test]
async fn the_cap_applies_to_what_recall_returns() {
    let server = MockServer::start().await;
    let flood = vec!["a like event"; 5_000].join("\n\n---\n\n");
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .respond_with(pack(&flood))
        .mount(&server)
        .await;
    let text = client(&server).recall("q", None).await.unwrap();
    assert!(text.len() < 1_200 * 6 + 100, "{}", text.len());
}

/// The recall breaker.
#[tokio::test]
async fn stops_recalling_for_the_cooldown_after_a_timeout_so_a_stalled_store_is_not_handed_more() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .respond_with(pack("late").set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let memory = with(
        &server,
        CortexOptions {
            recall_timeout: Duration::from_millis(100),
            cooldown: Duration::from_secs(60),
            ..CortexOptions::default()
        },
    );
    assert!(memory.recall("q", None).await.unwrap_err().is_timeout());
    let sent = server.received_requests().await.unwrap().len();
    let paused = memory.recall("q", None).await.unwrap_err();
    assert!(matches!(paused, MemoryError::Paused(_)), "{paused}");
    assert_eq!(server.received_requests().await.unwrap().len(), sent);
}

#[tokio::test]
async fn trips_on_a_timeout_in_either_pass_even_when_the_other_one_answered() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(person_pass())
        .respond_with(pack("they asked about the roadmap"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .and(general_pass())
        .respond_with(pack("late").set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let memory = with(
        &server,
        CortexOptions {
            recall_timeout: Duration::from_millis(100),
            cooldown: Duration::from_secs(60),
            ..CortexOptions::default()
        },
    );
    assert!(
        memory
            .recall("q", Some(&ana()))
            .await
            .unwrap()
            .contains("roadmap")
    );
    let sent = server.received_requests().await.unwrap().len();
    assert!(matches!(
        memory.recall("q", None).await.unwrap_err(),
        MemoryError::Paused(_)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), sent);
}

#[tokio::test]
async fn fails_fast_past_the_in_flight_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/recall"))
        .respond_with(pack("slow").set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    let memory = with(
        &server,
        CortexOptions {
            recall_timeout: Duration::from_secs(5),
            max_in_flight: 1,
            ..CortexOptions::default()
        },
    );
    let first = {
        let memory = memory.clone();
        tokio::spawn(async move { memory.recall("q", None).await })
    };
    // Let the first one take its place.
    while server.received_requests().await.unwrap().is_empty() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let refused = memory.recall("q", None).await.unwrap_err();
    assert_eq!(refused.to_string(), "too many recalls in flight");
    first.abort();
}

#[tokio::test]
async fn captures_wait_on_their_own_deadline_not_the_recalls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/experience"))
        .respond_with(ResponseTemplate::new(202).set_delay(Duration::from_millis(400)))
        .mount(&server)
        .await;
    let memory = with(
        &server,
        CortexOptions {
            recall_timeout: Duration::from_millis(100),
            capture_timeout: Duration::from_secs(5),
            ..CortexOptions::default()
        },
    );
    memory.capture_mention(&post()).await.unwrap();
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
        recall_timeout_ms: 15_000,
        capture_timeout_ms: 30_000,
        person_recall: true,
        recall_cooldown_ms: 300_000,
        health_cron: "17 * * * *".into(),
    };
    let memory = CortexMemory::from_config(&config).unwrap();
    assert!(!format!("{memory:?}").contains("very-secret"));
}
