use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[test]
fn asks_the_loopback_on_the_configured_port() {
    assert_eq!(health_url(None), "http://127.0.0.1:3001/health");
    assert_eq!(health_url(Some("8080")), "http://127.0.0.1:8080/health");
    assert_eq!(health_url(Some(" 8080 ")), "http://127.0.0.1:8080/health");
    assert_eq!(health_url(Some("nope")), "http://127.0.0.1:3001/health");
    assert_eq!(health_url(Some("0")), "http://127.0.0.1:3001/health");
}

async fn answering(status: u16, body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn passes_a_healthy_service() {
    let server = answering(200, json!({ "ok": true, "browser": null })).await;
    check(&format!("{}/health", server.uri())).await.unwrap();
}

#[tokio::test]
async fn fails_an_error_a_wrong_answer_or_silence() {
    let failing = answering(503, json!({ "ok": true })).await;
    assert!(check(&format!("{}/health", failing.uri())).await.is_err());
    let unwell = answering(200, json!({ "ok": false })).await;
    assert!(check(&format!("{}/health", unwell.uri())).await.is_err());
    let odd = answering(200, json!("ok")).await;
    assert!(check(&format!("{}/health", odd.uri())).await.is_err());
    assert!(check("http://127.0.0.1:9/health").await.is_err());
}
