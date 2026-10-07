use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
const WEBP: &[u8] = b"RIFF....WEBPVP8 ";

async fn first_body(server: &MockServer) -> Value {
    server.received_requests().await.unwrap()[0].body_json().unwrap()
}

fn b64_answer() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "b64_json": BASE64.encode(PNG) }] }))
}

#[tokio::test]
async fn asks_gpt_image_for_png_bytes_in_the_models_own_size_vocabulary() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/images/generations")).respond_with(b64_answer()).mount(&server).await;
    let images = OpenAiImageGenerator::new(format!("{}/v1", server.uri()), "k", "gpt-image-1").with_quality("medium");

    let result = images.generate("a tiny robot", ImageAspect::Landscape).await.unwrap();
    assert_eq!(result, GeneratedImage { data: PNG.to_vec(), mime_type: MimeType::Png });
    let body = first_body(&server).await;
    assert_eq!(
        body,
        json!({ "model": "gpt-image-1", "prompt": "a tiny robot", "n": 1, "size": "1536x1024", "quality": "medium", "output_format": "png" })
    );
    let request = &server.received_requests().await.unwrap()[0];
    assert_eq!(request.headers.get("authorization").unwrap(), "Bearer k");
}

#[tokio::test]
async fn sends_the_ladder_only_what_every_rung_understands_and_reads_whatever_type_comes_back() {
    let server = MockServer::start().await;
    let cdn = format!("{}/cdn/out", server.uri());
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "url": cdn }] })))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/cdn/out")).respond_with(ResponseTemplate::new(200).set_body_bytes(WEBP)).mount(&server).await;
    let images = OpenAiImageGenerator::new(format!("{}/v1", server.uri()), "k", "image").with_quality("medium");

    let result = images.generate("Tiny on a beach", ImageAspect::Square).await.unwrap();
    assert_eq!(result, GeneratedImage { data: WEBP.to_vec(), mime_type: MimeType::Webp });
    // Square is the ladder's default and the ladder's business; quality and
    // format flags are OpenAI's and would be a 400 on another rung.
    assert_eq!(first_body(&server).await, json!({ "model": "image", "prompt": "Tiny on a beach", "n": 1 }));
}

#[tokio::test]
async fn speaks_dall_e_when_that_is_the_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(b64_answer()).mount(&server).await;
    let images = OpenAiImageGenerator::new(format!("{}/v1/", server.uri()), "k", "dall-e-3");
    images.generate("a tiny robot", ImageAspect::Portrait).await.unwrap();
    let body = first_body(&server).await;
    assert_eq!(body["size"], "1024x1792");
    assert_eq!(body["response_format"], "b64_json");
    assert!(body.get("output_format").is_none());
    assert!(body.get("quality").is_none());
}

#[tokio::test]
async fn reports_failures_and_answers_that_are_not_images() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": { "message": "safety system" } })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "b64_json": BASE64.encode(b"GIF89a......") }] })))
        .mount(&server)
        .await;
    let images = OpenAiImageGenerator::new(format!("{}/v1", server.uri()), "k", "gpt-image-1");
    let error = images.generate("a tiny robot", ImageAspect::Square).await.unwrap_err();
    assert_eq!(error.to_string(), "image generation failed (400): safety system");
    let error = images.generate("a tiny robot", ImageAspect::Square).await.unwrap_err();
    assert_eq!(error.to_string(), "image generation returned no image");
    let error = images.generate("a tiny robot", ImageAspect::Square).await.unwrap_err();
    assert_eq!(error.to_string(), "image generation returned image/gif");
}
