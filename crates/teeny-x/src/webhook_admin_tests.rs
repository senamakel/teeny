use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::test_support::{api, api_without_bearer, is_oauth};

#[tokio::test]
async fn lists_the_registered_webhooks_on_the_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .and(header("authorization", "Bearer app-bearer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": "w1", "url": "https://teeny.example/webhooks/x", "valid": true, "created_at": "2026-09-01T00:00:00.000Z" }]
        })))
        .mount(&server)
        .await;

    let hooks = api(&server).list_webhooks().await.unwrap();

    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].id.as_deref(), Some("w1"));
    assert_eq!(hooks[0].valid, Some(true));
}

#[tokio::test]
async fn no_data_is_no_webhooks() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "meta": {} })))
        .mount(&server)
        .await;

    assert!(api(&server).list_webhooks().await.unwrap().is_empty());
}

#[tokio::test]
async fn retries_as_the_account_when_x_asks_for_user_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/activity/subscriptions"))
        .and(header("authorization", "Bearer app-bearer"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errors": [{ "message": "OauthAccessTokenRequired" }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/activity/subscriptions"))
        .and(body_json(json!({
            "event_type": "post.mention.create",
            "filter": { "user_id": "42" },
            "webhook_id": "w1",
            "tag": "teeny post.mention.create"
        })))
        .and(|request: &wiremock::Request| is_oauth(request))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "subscription_id": "s1" } })))
        .expect(1)
        .mount(&server)
        .await;

    api(&server)
        .create_subscription("w1", "post.mention.create", "42")
        .await
        .unwrap();
}

#[tokio::test]
async fn retries_as_the_account_on_a_403_naming_oauth_1() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/2/webhooks/w1"))
        .and(header("authorization", "Bearer app-bearer"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "detail": "Authenticating with OAuth 2.0 Application-Only is forbidden for this endpoint. Supported authentication types are [OAuth 1.0a User Context]."
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/webhooks/w1"))
        .and(|request: &wiremock::Request| is_oauth(request))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })))
        .expect(1)
        .mount(&server)
        .await;

    api(&server).delete_webhook("w1").await.unwrap();
}

#[tokio::test]
async fn any_other_refusal_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/webhooks"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errors": [{ "message": "CrcValidationFailed" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = api(&server)
        .create_webhook("https://teeny.example/webhooks/x")
        .await
        .unwrap_err();

    assert!(error.to_string().contains("CrcValidationFailed"), "{error}");
}

#[tokio::test]
async fn creating_returns_the_new_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/webhooks"))
        .and(body_json(json!({ "url": "https://teeny.example/webhooks/x" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "w9", "url": "https://teeny.example/webhooks/x" } })))
        .mount(&server)
        .await;

    let id = api(&server)
        .create_webhook("https://teeny.example/webhooks/x")
        .await
        .unwrap();

    assert_eq!(id, "w9");
}

#[tokio::test]
async fn reads_and_deletes_subscriptions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/activity/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "subscription_id": "s1", "event_type": "like.create", "webhook_id": "w1", "filter": { "user_id": "42" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/s1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .expect(1)
        .mount(&server)
        .await;
    let x = api(&server);

    let subscriptions = x.list_subscriptions().await.unwrap();
    x.delete_subscription("s1").await.unwrap();

    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].describe(), "like.create for 42");
}

#[tokio::test]
async fn refuses_without_an_app_bearer() {
    let server = MockServer::start().await;

    let error = api_without_bearer(&server).list_webhooks().await.unwrap_err();

    assert!(error.to_string().contains("X_BEARER_TOKEN"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}
