use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use crate::XError;
use crate::model::CreateOptions;
use crate::test_support::{api, is_oauth};

#[tokio::test]
async fn creates_a_post_signed_as_the_account_even_with_a_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(json!({ "text": "gm" })))
        .and(|request: &Request| is_oauth(request))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "data": { "id": "100", "text": "gm" } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let created = api(&server)
        .create_post("gm", &CreateOptions::default())
        .await
        .unwrap();
    assert_eq!(created.id.as_str(), "100");
}

#[tokio::test]
async fn attaches_a_reply_target_a_quote_and_media() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(json!({
            "text": "look",
            "reply": { "in_reply_to_tweet_id": "7" },
            "quote_tweet_id": "8",
            "media": { "media_ids": ["m1", "m2"] }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "data": { "id": "101" } })))
        .expect(1)
        .mount(&server)
        .await;
    let options = CreateOptions {
        reply_to: Some("7".into()),
        quote: Some("8".into()),
        media_ids: vec!["m1".into(), "m2".into()],
    };
    api(&server).create_post("look", &options).await.unwrap();
}

#[tokio::test]
async fn a_post_without_an_id_back_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "errors": [{ "message": "duplicate" }] })),
        )
        .mount(&server)
        .await;
    let error = api(&server)
        .create_post("gm", &CreateOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, XError::Missing(ref message) if message.contains("duplicate")),
        "{error:?}"
    );
}

#[tokio::test]
async fn deletes_a_post_as_the_account() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/2/tweets/55"))
        .and(|request: &Request| is_oauth(request))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/tweets/56"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "errors": [] })))
        .mount(&server)
        .await;
    let x = api(&server);
    assert!(x.delete_post(&"55".into()).await.unwrap());
    assert!(!x.delete_post(&"56".into()).await.unwrap());
}

#[tokio::test]
async fn likes_a_post_as_the_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": { "id": "1", "username": "teenydroid" } })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/users/1/likes"))
        .and(body_json(json!({ "tweet_id": "77" })))
        .and(|request: &Request| is_oauth(request))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "liked": true } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    api(&server).like_post(&"77".into()).await.unwrap();
}
