use axum::http::StatusCode;
use serde_json::json;

use super::test_support::*;

#[tokio::test]
async fn answers_an_unknown_route_the_way_fastify_did() {
    let app = super::router(services());

    let missing = call(&app, get("/nope")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(
        missing.json(),
        json!({ "message": "Route GET:/nope not found", "error": "Not Found", "statusCode": 404 })
    );

    // Fastify has no 405: a known path under the wrong method is a 404 too.
    let wrong_method = call(&app, post_json("/health", "{}")).await;
    assert_eq!(wrong_method.status, StatusCode::NOT_FOUND);
    assert_eq!(wrong_method.json()["message"], "Route POST:/health not found");
}
