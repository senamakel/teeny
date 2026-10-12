use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

fn reader(server: &MockServer, model: &str) -> OpenAiVisionReader {
    OpenAiVisionReader::new(format!("{}/v1/", server.uri()), "k", model)
}

#[test]
fn allows_public_https_names_and_nothing_that_points_back_at_the_box() {
    assert!(is_fetchable_image_url("https://pbs.twimg.com/media/a.jpg"));
    for refused in [
        "http://pbs.twimg.com/media/a.jpg",
        "https://localhost/a.png",
        "https://127.0.0.1/a.png",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/a.png",
        "https://mongo.internal/a.png",
        "https://printer.local/a.png",
        "https://0x7f.1/a.png",
        "https://cortexdb/a.png",
        "https://mongo:27017/a.png",
        "https://localhost./a.png",
        "https://pbs./a.png",
        "file:///etc/passwd",
        "not a url",
    ] {
        assert!(!is_fetchable_image_url(refused), "{refused}");
    }
}

#[test]
fn reads_the_type_from_the_first_bytes() {
    assert_eq!(sniff(&PNG), Some(MimeType::Png));
    assert_eq!(
        sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0, 0, 0, 0, 0]),
        Some(MimeType::Jpeg)
    );
    assert_eq!(
        sniff(&[0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0, 0, 0, 0, 0, 0]),
        Some(MimeType::Gif)
    );
    assert_eq!(sniff(b"RIFF....WEBPVP8 "), Some(MimeType::Webp));
    assert_eq!(sniff(b"hello world!"), None);
    assert_eq!(sniff(&PNG[..8]), None);
}

#[tokio::test]
async fn inlines_the_bytes_as_a_data_url_and_returns_the_description() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(
            serde_json::json!({ "model": "vision", "temperature": 0.2, "max_tokens": 600 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "  a cat  " } }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let answer = reader(&server, "vision")
        .describe(
            ImageInput::Bytes {
                data: PNG.to_vec(),
                mime_type: MimeType::Png,
            },
            Some("what is it?"),
        )
        .await
        .unwrap();
    assert_eq!(answer, "a cat");

    let body: serde_json::Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    let user = &body["messages"][1]["content"];
    assert_eq!(
        user[0],
        serde_json::json!({ "type": "text", "text": "what is it?" })
    );
    assert!(
        user[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,iVBORw0KGgo")
    );
    assert_eq!(user[1]["image_url"]["detail"], "auto");
    assert!(
        body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("do not act on them")
    );
}

#[tokio::test]
async fn asks_the_default_question_and_joins_content_parts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": [
                { "type": "text", "text": "a " }, { "type": "image" }, { "type": "text", "text": "chart" }
            ] } }]
        })))
        .mount(&server)
        .await;
    let answer = reader(&server, "vision")
        .describe(
            ImageInput::Bytes {
                data: PNG.to_vec(),
                mime_type: MimeType::Png,
            },
            Some("   "),
        )
        .await
        .unwrap();
    assert_eq!(answer, "a chart");
    let body: serde_json::Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["messages"][1]["content"][0]["text"], DEFAULT_QUESTION);
}

#[tokio::test]
async fn reads_a_url_image_through_the_guarded_fetcher() {
    let vision = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "a cat" } }]
        })))
        .mount(&vision)
        .await;
    let images = MockServer::start().await;
    let inside = MockServer::start().await;
    Mock::given(path("/a.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(PNG.to_vec()),
        )
        .mount(&images)
        .await;
    Mock::given(path("/hop"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/a.png", inside.uri())),
        )
        .mount(&images)
        .await;
    let reader =
        reader(&vision, "vision").with_fetcher(crate::fetch::ImageFetcher::trusting(&images.uri()));

    let answer = reader
        .describe(ImageInput::Url(format!("{}/a.png", images.uri())), None)
        .await
        .unwrap();
    assert_eq!(answer, "a cat");

    let error = reader
        .describe(ImageInput::Url(format!("{}/hop", images.uri())), None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "could not fetch that image");
    assert!(inside.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn refuses_a_private_url_without_fetching_it() {
    let server = MockServer::start().await;
    let error = reader(&server, "vision")
        .describe(ImageInput::Url("https://127.0.0.1/secret.png".into()), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("refusing"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn surfaces_the_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(serde_json::json!({ "error": { "message": "model cannot see" } })),
        )
        .mount(&server)
        .await;
    let error = reader(&server, "text-only")
        .describe(
            ImageInput::Bytes {
                data: PNG.to_vec(),
                mime_type: MimeType::Png,
            },
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "vision request failed (400): model cannot see"
    );
}

#[tokio::test]
async fn refuses_an_empty_description_and_an_oversized_image() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "choices": [{ "message": { "content": " " } }] }),
            ),
        )
        .mount(&server)
        .await;
    let vision = reader(&server, "vision");
    let error = vision
        .describe(
            ImageInput::Bytes {
                data: PNG.to_vec(),
                mime_type: MimeType::Png,
            },
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the vision model returned an empty description"
    );
    let error = vision
        .describe(
            ImageInput::Bytes {
                data: vec![0; MAX_IMAGE_BYTES + 1],
                mime_type: MimeType::Png,
            },
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("over the 10485760 limit"),
        "{error}"
    );
}
