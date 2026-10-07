use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use serde_json::{Value, json};

use super::*;
use crate::http::test_support::call;

async fn echo(payload: Payload) -> axum::Json<Value> {
    axum::Json(json!({
        "raw": payload.raw.map(|raw| String::from_utf8_lossy(&raw).into_owned()),
        "body": payload.body,
    }))
}

fn app() -> Router {
    Router::new().route("/echo", post(echo))
}

fn request(content_type: Option<&str>, body: impl Into<Body>) -> Request<Body> {
    let mut builder = Request::post("/echo");
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder.body(body.into()).unwrap()
}

#[tokio::test]
async fn keeps_the_raw_bytes_of_a_json_body_beside_the_parse() {
    let raw = "{ \"a\" : 1 }";
    let answer = call(
        &app(),
        request(Some("application/json; charset=utf-8"), raw),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.json(), json!({ "raw": raw, "body": { "a": 1 } }));
}

#[tokio::test]
async fn reads_plain_text_as_a_string_with_no_raw_bytes() {
    let answer = call(&app(), request(Some("text/plain"), "hello")).await;
    assert_eq!(answer.json(), json!({ "raw": null, "body": "hello" }));
}

#[tokio::test]
async fn reads_no_body_as_nothing() {
    let answer = call(&app(), request(None, Body::empty())).await;
    assert_eq!(answer.json(), json!({ "raw": null, "body": null }));
}

#[tokio::test]
async fn refuses_a_body_over_a_mebibyte() {
    let big = format!("\"{}\"", "a".repeat(BODY_LIMIT));
    let answer = call(&app(), request(Some("application/json"), big)).await;
    assert_eq!(answer.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        answer.json(),
        json!({
            "statusCode": 413,
            "code": "FST_ERR_CTP_BODY_TOO_LARGE",
            "error": "Payload Too Large",
            "message": "Request body is too large",
        })
    );
}

#[tokio::test]
async fn refuses_json_that_does_not_parse_and_an_empty_json_body() {
    let broken = call(&app(), request(Some("application/json"), "{nope")).await;
    assert_eq!(broken.status, StatusCode::BAD_REQUEST);
    assert_eq!(broken.json()["code"], "FST_ERR_CTP_INVALID_JSON_BODY");

    let empty = call(&app(), request(Some("application/json"), Body::empty())).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.json()["code"], "FST_ERR_CTP_EMPTY_JSON_BODY");
}

#[tokio::test]
async fn refuses_a_media_type_it_has_no_parser_for() {
    for content_type in [Some("application/xml"), None] {
        let answer = call(&app(), request(content_type, "<a/>")).await;
        assert_eq!(
            answer.status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type:?}"
        );
        assert_eq!(answer.json()["code"], "FST_ERR_CTP_INVALID_MEDIA_TYPE");
    }
}
