use base64::Engine as _;
use serde_json::json;
use wiremock::matchers::{body_json, body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::XError;
use crate::model::{Media, MediaType};
use crate::test_support::{api, is_oauth};

async fn initialize(server: &MockServer, mime: &str, category: &str, total: usize) {
    Mock::given(method("POST"))
        .and(path("/2/media/upload/initialize"))
        .and(body_json(
            json!({ "media_type": mime, "media_category": category, "total_bytes": total }),
        ))
        .and(|request: &Request| is_oauth(request))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": { "id": "m1", "media_key": "3_m1" } })),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn uploads_a_still_in_one_segment_into_the_image_category() {
    let server = MockServer::start().await;
    let bytes = vec![7u8; 1000];
    initialize(&server, "image/png", "tweet_image", 1000).await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/append"))
        .and(body_json(json!({
            "media": base64::engine::general_purpose::STANDARD.encode(&bytes),
            "segment_index": 0
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "expires_at": 1 } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "m1" } })))
        .expect(1)
        .mount(&server)
        .await;

    let uploaded = api(&server)
        .upload_media(&Media {
            data: bytes,
            mime: MediaType::Png,
        })
        .await
        .unwrap();
    assert_eq!(uploaded.media_id, "m1");
}

#[tokio::test]
async fn uploads_a_video_in_four_megabyte_chunks_and_waits_for_processing() {
    let server = MockServer::start().await;
    let total = UPLOAD_CHUNK_BYTES * 2 + 10;
    initialize(&server, "video/mp4", "tweet_video", total).await;
    for segment in 0..3 {
        Mock::given(method("POST"))
            .and(path("/2/media/upload/m1/append"))
            .and(body_partial_json(json!({ "segment_index": segment })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "m1", "processing_info": { "state": "pending", "check_after_secs": 0 } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/media/upload"))
        .and(query_param("media_id", "m1"))
        .and(query_param("command", "STATUS"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "m1", "processing_info": { "state": "succeeded" } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let media = Media {
        data: vec![1u8; total],
        mime: MediaType::Mp4,
    };
    assert_eq!(
        api(&server).upload_media(&media).await.unwrap().media_id,
        "m1"
    );
}

#[tokio::test]
async fn a_gif_goes_through_the_gif_pipeline_and_a_failed_one_is_an_error() {
    let server = MockServer::start().await;
    initialize(&server, "image/gif", "tweet_gif", 3).await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/append"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "m1", "processing_info": { "state": "failed" } }
        })))
        .mount(&server)
        .await;

    let error = api(&server)
        .upload_media(&Media {
            data: vec![1, 2, 3],
            mime: MediaType::Gif,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(error, XError::MediaFailed(ref id) if id == "m1"),
        "{error:?}"
    );
}
