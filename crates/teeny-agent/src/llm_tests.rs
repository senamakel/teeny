//! The single-shot client against the ladder, and the ledger row each call
//! leaves. The row shape is legacy/test/ledger.test.ts' `recordCompletion`
//! case: no system prompt, a digest of it instead, usage read into tokens.

use std::sync::Arc;

use serde_json::{Value, json};
use teeny_media::reference::TextModel;
use teeny_safety::model::{JsonRequest, JudgeModel, Message};
use teeny_store::inference::{Lane, LedgerMessage, RecentFilter, Role, sha256};
use teeny_store::{InMemoryLedger, InferenceLedger};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn client(server: &MockServer, ledger: &Arc<InMemoryLedger>) -> LlmClient {
    LlmClient::new(format!("{}/v1/", server.uri()), Secret::new("k"), "eyes")
        .with_ledger(ledger.clone() as Arc<dyn InferenceLedger>)
}

async fn last_row(ledger: &InMemoryLedger) -> teeny_store::Inference {
    let rows = ledger.recent(10, &RecentFilter::default()).await.unwrap();
    assert_eq!(rows.len(), 1, "one row per call");
    rows.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_text_call_leaves_one_row_without_the_system_prompt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer k"))
        .and(body_partial_json(json!({
            "model": "eyes",
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": "describe pictures" },
                { "role": "user", "content": "what is this" }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp-1",
            "model": "eyes-v2",
            "choices": [{ "message": { "content": "  a smol robot " } }],
            "usage": { "prompt_tokens": 500, "completion_tokens": 12 }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    let text = client(&server, &ledger)
        .with_source("vision")
        .complete(Some("describe pictures"), "what is this")
        .await
        .unwrap();
    assert_eq!(text, "a smol robot");

    let row = last_row(&ledger).await;
    assert_eq!(row.lane, Lane::Mascot);
    assert_eq!(row.source, "vision");
    assert_eq!(row.model, "eyes-v2");
    assert_eq!(row.response_id.as_deref(), Some("resp-1"));
    assert_eq!(row.output.as_deref(), Some("a smol robot"));
    assert_eq!(
        (row.tokens.input, row.tokens.output, row.tokens.total),
        (500, 12, 512)
    );
    assert_eq!(
        row.input,
        Some(vec![LedgerMessage {
            role: Role::User,
            tool: None,
            text: "what is this".into()
        }])
    );
    assert_eq!(row.prompt_digest, Some(sha256(&["describe pictures"])));
    assert!(
        !serde_json::to_string(&row)
            .unwrap()
            .contains("describe pictures")
    );
    assert!(row.digest.is_some());
    assert!(row.duration_ms.is_some());
}

fn judge_request() -> JsonRequest {
    JsonRequest {
        source: "judge",
        name: "content_verdict",
        schema: json!({ "type": "object", "properties": { "verdict": { "type": "string" } } }),
        messages: vec![Message::system("judge it"), Message::user("gm")],
        temperature: 0.0,
        max_tokens: Some(300),
    }
}

#[tokio::test]
async fn a_json_call_is_a_forced_function_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "model": "eyes",
            "temperature": 0.0,
            "max_tokens": 300,
            "tools": [{ "type": "function", "function": { "name": "content_verdict" } }],
            "tool_choice": { "type": "function", "function": { "name": "content_verdict" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": null, "tool_calls": [{
                "id": "c1", "type": "function",
                "function": { "name": "content_verdict", "arguments": "{\"verdict\":\"clean\"}" }
            }] } }],
            "usage": { "prompt_tokens": 40, "completion_tokens": 5, "total_tokens": 45 }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    let answer = client(&server, &ledger)
        .complete_json(judge_request())
        .await
        .unwrap();
    assert_eq!(answer, json!({ "verdict": "clean" }));

    let row = last_row(&ledger).await;
    assert_eq!(row.source, "judge");
    assert_eq!(row.model, "eyes");
    assert_eq!(
        row.output.as_deref(),
        Some("→ content_verdict({\"verdict\":\"clean\"})")
    );
    assert_eq!(row.tokens.total, 45);
}

#[tokio::test]
async fn a_json_answer_written_as_text_is_still_read() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "```json\n{\"verdict\":\"prohibited\"}\n```" } }]
        })))
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    let answer = client(&server, &ledger)
        .complete_json(judge_request())
        .await
        .unwrap();
    assert_eq!(answer["verdict"], Value::from("prohibited"));
}

#[tokio::test]
async fn an_unreadable_answer_is_a_model_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "I think it is fine" } }]
        })))
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    assert!(
        client(&server, &ledger)
            .complete_json(judge_request())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_failed_call_is_an_error_and_leaves_no_row() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("rung down"))
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    let error = client(&server, &ledger)
        .complete(None, "hello")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("503"), "{error}");
    assert!(error.to_string().contains("rung down"), "{error}");
    assert!(
        ledger
            .recent(10, &RecentFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_empty_answer_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "   " } }]
        })))
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    assert!(
        client(&server, &ledger)
            .complete(None, "hello")
            .await
            .is_err()
    );
}

#[test]
fn another_alias_is_the_same_endpoint_with_another_model() {
    let llm = LlmClient::new("https://ladder/v1", Secret::new("k"), "chat");
    let critic = llm.with_model("flash");
    assert_eq!(critic.model(), "flash");
    assert_eq!(llm.model(), "chat");
}

#[test]
fn reads_the_ladder_from_config() {
    let config = teeny_core::config::LlmConfig {
        base_url: "https://ladder.example/v1".into(),
        api_key: Secret::new("k"),
        model: "chat".into(),
    };
    assert_eq!(LlmClient::from_config(&config).model(), "chat");
}

#[tokio::test]
async fn is_the_media_crate_s_text_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "messages": [{ "role": "system", "content": "sum up" }, { "role": "user", "content": "looks" }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "a sheet" } }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    let model: Arc<dyn TextModel> = Arc::new(client(&server, &ledger).with_source("reference"));
    assert_eq!(
        model.complete(Some("sum up"), "looks").await.unwrap(),
        "a sheet"
    );
    assert_eq!(last_row(&ledger).await.source, "reference");
}

/// e84e8d7: the article's editor and flow pass read and rewrite the whole
/// piece on a reasoning model; at a caption's deadline both were cut off.
#[tokio::test]
async fn a_client_waits_as_long_as_it_is_told_and_no_longer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "choices": [{ "message": { "content": "slow" } }] }))
                .set_delay(std::time::Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    let ledger = Arc::new(InMemoryLedger::default());
    let hurried = client(&server, &ledger).with_timeout(std::time::Duration::from_millis(100));
    assert!(matches!(
        hurried.complete(None, "go").await,
        Err(LlmError::Transport(error)) if error.is_timeout()
    ));
    let patient = client(&server, &ledger).with_timeout(std::time::Duration::from_secs(5));
    assert_eq!(patient.complete(None, "go").await.unwrap(), "slow");
    // The alias swap keeps the deadline.
    assert_eq!(
        patient.with_model("flash").timeout(),
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        client(&server, &ledger).timeout(),
        std::time::Duration::from_secs(60)
    );
}
