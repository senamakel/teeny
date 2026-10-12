//! Port of `legacy/test/cortex-health.test.ts`.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

async fn answering(body: serde_json::Value, status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/admin/ready"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn reads_an_embedding_outage_out_of_the_readiness_report() {
    // The shape robot1 reported on 2026-10-04, when the router had dropped
    // the embedding ladder: up, ready, and unable to embed anything.
    let server = answering(
        json!({
            "ready": true,
            "degraded": true,
            "embedding_provider": "openai-http:vectors:1024",
            "checks": { "embeddings": { "ok": false, "ok_count": 0, "fail_count": 28_517, "last_error": "unknown ladder vectors" } },
        }),
        200,
    )
    .await;
    // A path on the URL is replaced, as `new URL('/v1/admin/ready', url)` did.
    let health = check_cortex(&format!("{}/some/base", server.uri())).await;
    assert_eq!(
        health,
        CortexHealth {
            reachable: true,
            degraded: true,
            embeddings: Embeddings {
                ok: false,
                ok_count: Some(0),
                fail_count: Some(28_517),
                last_error: Some("unknown ladder vectors".into()),
            },
            provider: Some("openai-http:vectors:1024".into()),
            error: None,
        }
    );
}

#[tokio::test]
async fn warns_on_an_outage_and_is_quiet_about_a_healthy_store() {
    let outage = check_cortex(
        &answering(
            json!({ "degraded": true, "checks": { "embeddings": { "ok": false } } }),
            200,
        )
        .await
        .uri(),
    )
    .await;
    let (warn, message) = verdict(&outage);
    assert!(warn);
    assert!(message.contains("cannot embed"), "{message}");

    let healthy = check_cortex(
        &answering(json!({ "degraded": false, "checks": { "embeddings": { "ok": true, "ok_count": 149 } } }), 200)
            .await
            .uri(),
    )
    .await;
    assert_eq!(verdict(&healthy), (false, "memory: the store is healthy"));
    assert_eq!(healthy.embeddings.ok_count, Some(149));

    // No report body: the status decides.
    let down = check_cortex(&answering(json!({}), 503).await.uri()).await;
    assert!(down.reachable && down.degraded && !down.embeddings.ok);
    assert!(verdict(&down).0);
}

#[tokio::test]
async fn reports_an_unreachable_store_rather_than_failing() {
    let health = check_cortex("http://127.0.0.1:9").await;
    assert!(!health.reachable);
    assert!(health.degraded);
    assert!(health.error.is_some());
    let (warn, message) = verdict(&health);
    assert!(warn);
    assert!(message.contains("unreachable"), "{message}");
    // And the report logs it and hands it back.
    assert!(!report_cortex_health("http://127.0.0.1:9").await.reachable);
}
