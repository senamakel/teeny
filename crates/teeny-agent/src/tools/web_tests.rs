//! Port of legacy/test/agent-tools.test.ts, "Exa tools".

use std::sync::Arc;

use serde_json::json;
use teeny_core::Secret;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::exa::ExaClient;
use crate::tools::test_support::{by_name, call};
use crate::tools::text_of;

async fn exa() -> (MockServer, Arc<ExaClient>) {
    let server = MockServer::start().await;
    let client = Arc::new(ExaClient::new(Secret::new("exa")).with_base_url(server.uri()));
    (server, client)
}

#[tokio::test]
async fn formats_search_hits_with_urls_and_excerpts() {
    let (server, client) = exa().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "title": "Launch notes", "url": "https://example.com/a",
                          "publishedDate": "2026-01-02", "text": "the dapp shipped" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/answer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answer": "42", "citations": [{ "url": "https://example.com/b", "title": "B" }]
        })))
        .mount(&server)
        .await;
    let tools = by_name(web_tools(client));
    let search = text_of(&call(&tools["web_search"], json!({ "query": "stonks.rocks" })).await);
    assert!(search.contains("https://example.com/a"));
    assert!(search.contains("excerpt: the dapp shipped"));
    let answer = text_of(&call(&tools["web_answer"], json!({ "question": "what" })).await);
    assert!(answer.contains("42"));
    assert!(answer.contains("https://example.com/b"));
}

#[tokio::test]
async fn are_the_four_legacy_tools_in_order() {
    let (_server, client) = exa().await;
    let names: Vec<String> = web_tools(client)
        .iter()
        .map(|t| t.name().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "web_search",
            "read_web_pages",
            "find_similar_pages",
            "web_answer"
        ]
    );
}

#[tokio::test]
async fn refuses_arguments_outside_the_schema() {
    let (_server, client) = exa().await;
    let tools = by_name(web_tools(client));
    assert!(
        call(
            &tools["web_search"],
            json!({ "query": "q", "numResults": 11 })
        )
        .await
        .is_error
    );
    assert!(
        call(
            &tools["web_search"],
            json!({ "query": "q", "category": "memes" })
        )
        .await
        .is_error
    );
    assert!(
        call(&tools["read_web_pages"], json!({ "urls": [] }))
            .await
            .is_error
    );
    assert!(
        call(&tools["read_web_pages"], json!({ "urls": ["not a url"] }))
            .await
            .is_error
    );
    assert!(
        call(&tools["find_similar_pages"], json!({ "url": "ftp://x" }))
            .await
            .is_error
    );
    assert!(
        call(&tools["web_answer"], json!({ "question": "" }))
            .await
            .is_error
    );
}

#[tokio::test]
async fn an_exa_failure_reaches_the_model_as_an_error() {
    let (server, client) = exa().await;
    Mock::given(method("POST"))
        .and(path("/contents"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let tools = by_name(web_tools(client));
    let result = call(
        &tools["read_web_pages"],
        json!({ "urls": ["https://example.com"] }),
    )
    .await;
    assert!(result.is_error);
    assert!(text_of(&result).contains("boom"));
}
