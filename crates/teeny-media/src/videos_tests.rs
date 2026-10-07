use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const MP4: [u8; 8] = [0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70];

fn answer(body: Value, status: u16) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

async fn once(server: &MockServer, verb: &str, route: &str, response: ResponseTemplate) {
    Mock::given(method(verb)).and(path(route)).respond_with(response).up_to_n_times(1).mount(server).await;
}

async fn urls(server: &MockServer) -> Vec<String> {
    server.received_requests().await.unwrap().iter().map(|request| request.url.path().to_owned()).collect()
}

async fn first_body(server: &MockServer) -> Value {
    server.received_requests().await.unwrap()[0].body_json().unwrap()
}

fn request(prompt: &str, orientation: Option<VideoOrientation>) -> VideoRequest {
    VideoRequest { prompt: prompt.into(), orientation, seconds: None }
}

#[tokio::test]
async fn sora_creates_the_job_polls_until_it_is_done_then_downloads_the_clip() {
    let server = MockServer::start().await;
    once(&server, "POST", "/v1/videos", answer(json!({ "id": "video_1", "status": "queued" }), 200)).await;
    once(&server, "GET", "/v1/videos/video_1", answer(json!({ "id": "video_1", "status": "in_progress", "progress": 40 }), 200)).await;
    once(&server, "GET", "/v1/videos/video_1", answer(json!({ "id": "video_1", "status": "completed" }), 200)).await;
    once(&server, "GET", "/v1/videos/video_1/content", ResponseTemplate::new(200).set_body_bytes(MP4.to_vec())).await;
    let videos = OpenAiVideoGenerator::new(format!("{}/v1", server.uri()), "k", "sora-2")
        .with_seconds(4)
        .with_poll_interval(Duration::ZERO);

    let clip = videos.generate(request("a tiny robot waves", Some(VideoOrientation::Portrait))).await.unwrap();
    assert_eq!(clip, GeneratedVideo { data: MP4.to_vec(), seconds: Some(4) });
    assert_eq!(urls(&server).await, ["/v1/videos", "/v1/videos/video_1", "/v1/videos/video_1", "/v1/videos/video_1/content"]);
    assert_eq!(
        first_body(&server).await,
        json!({ "model": "sora-2", "prompt": "a tiny robot waves", "seconds": "4", "size": "720x1280" })
    );
}

#[tokio::test]
async fn sora_rounds_the_length_up_to_a_step_it_accepts_and_has_no_square() {
    let server = MockServer::start().await;
    once(&server, "POST", "/v1/videos", answer(json!({ "id": "v", "status": "completed" }), 200)).await;
    once(&server, "GET", "/v1/videos/v/content", ResponseTemplate::new(200).set_body_bytes(MP4.to_vec())).await;
    let videos = OpenAiVideoGenerator::new(format!("{}/v1", server.uri()), "k", "sora-2");
    let clip = videos
        .generate(VideoRequest { prompt: "p".into(), orientation: Some(VideoOrientation::Square), seconds: Some(9) })
        .await
        .unwrap();
    assert_eq!(clip.seconds, Some(12));
    let body = first_body(&server).await;
    assert_eq!((body["seconds"].as_str(), body["size"].as_str()), (Some("12"), Some("1280x720")));
}

#[tokio::test]
async fn sora_gives_up_on_a_failed_render_and_on_one_that_outlives_the_deadline() {
    let server = MockServer::start().await;
    once(&server, "POST", "/v1/videos", answer(json!({ "id": "video_2", "status": "queued" }), 200)).await;
    once(&server, "GET", "/v1/videos/video_2", answer(json!({ "id": "video_2", "status": "failed", "error": { "message": "moderation" } }), 200)).await;
    let videos = OpenAiVideoGenerator::new(format!("{}/v1", server.uri()), "k", "sora-2").with_poll_interval(Duration::ZERO);
    let error = videos.generate(request("anything at all", None)).await.unwrap_err();
    assert_eq!(error.to_string(), "video video_2 failed: moderation");

    let slow = MockServer::start().await;
    Mock::given(method("POST")).respond_with(answer(json!({ "id": "video_3", "status": "in_progress" }), 200)).mount(&slow).await;
    let impatient = OpenAiVideoGenerator::new(format!("{}/v1", slow.uri()), "k", "sora-2").with_timeout(Duration::ZERO);
    let error = impatient.generate(request("anything at all", None)).await.unwrap_err();
    assert_eq!(error.to_string(), "video video_3 was still in_progress at the deadline");

    let refused = MockServer::start().await;
    Mock::given(method("POST")).respond_with(answer(json!({ "error": { "message": "no key" } }), 401)).mount(&refused).await;
    let error = OpenAiVideoGenerator::new(refused.uri(), "k", "sora-2").generate(request("p", None)).await.unwrap_err();
    assert_eq!(error.to_string(), "video generation failed (401): no key");
}

#[tokio::test]
async fn ladder_submits_a_job_with_an_aspect_ratio_polls_it_and_downloads_the_clip_it_names() {
    let server = MockServer::start().await;
    let poll = format!("{}/v1/video/generations/01JOB", server.uri());
    let clip_url = format!("{}/clips/01JOB.mp4", server.uri());
    once(&server, "POST", "/v1/video/generations", answer(json!({ "id": "01JOB", "object": "media.job", "status": "queued", "poll_url": poll, "cancel_url": poll }), 202)).await;
    once(&server, "GET", "/v1/video/generations/01JOB", answer(json!({ "id": "01JOB", "status": "submitted", "poll_url": poll, "cancel_url": poll }), 200)).await;
    once(
        &server,
        "GET",
        "/v1/video/generations/01JOB",
        answer(
            json!({
                "id": "01JOB", "status": "completed", "poll_url": poll, "cancel_url": poll,
                "output": { "video_url": clip_url, "thumbnail_url": "https://cdn.example/t.jpg" }
            }),
            200,
        ),
    )
    .await;
    once(&server, "GET", "/clips/01JOB.mp4", ResponseTemplate::new(200).set_body_bytes(MP4.to_vec())).await;
    let videos = LadderVideoGenerator::new(format!("{}/v1", server.uri()), "k", "video").with_poll_interval(Duration::ZERO);

    let clip = videos.generate(request("Tiny waves", Some(VideoOrientation::Portrait))).await.unwrap();
    assert_eq!(clip, GeneratedVideo { data: MP4.to_vec(), seconds: None });
    assert_eq!(
        urls(&server).await,
        ["/v1/video/generations", "/v1/video/generations/01JOB", "/v1/video/generations/01JOB", "/clips/01JOB.mp4"]
    );
    assert_eq!(first_body(&server).await, json!({ "model": "video", "prompt": "Tiny waves", "aspect_ratio": "9:16" }));
}

#[tokio::test]
async fn ladder_sends_every_spelling_of_the_length_and_reads_an_inline_clip() {
    let server = MockServer::start().await;
    once(&server, "POST", "/video/generations", answer(json!({ "id": "j", "status": "SUCCEEDED", "data": [{ "b64_json": "AAAAGGZ0eXA=" }] }), 202)).await;
    let videos = LadderVideoGenerator::new(server.uri(), "k", "video");
    let clip = videos
        .generate(VideoRequest { prompt: "p".into(), orientation: Some(VideoOrientation::Landscape), seconds: Some(10) })
        .await
        .unwrap();
    assert_eq!(clip.data, MP4.to_vec());
    assert_eq!(
        first_body(&server).await,
        json!({
            "model": "video", "prompt": "p", "aspect_ratio": "16:9",
            "seconds": "10", "duration": "10", "video_length": 10, "length": 10, "duration_seconds": 10
        })
    );
}

#[tokio::test]
async fn ladder_defaults_to_square_gives_up_on_a_dead_job_and_says_what_it_saw_when_nothing_looks_like_a_clip() {
    let dead = MockServer::start().await;
    once(&dead, "POST", "/video/generations", answer(json!({ "id": "j1", "status": "queued" }), 202)).await;
    once(&dead, "GET", "/video/generations/j1", answer(json!({ "id": "j1", "status": "canceled", "error": { "message": "no seller" } }), 200)).await;
    let videos = LadderVideoGenerator::new(dead.uri(), "k", "video").with_poll_interval(Duration::ZERO);
    let error = videos.generate(request("anything", None)).await.unwrap_err();
    assert_eq!(error.to_string(), "video j1 canceled: no seller");
    assert_eq!(first_body(&dead).await["aspect_ratio"], "1:1");

    let bare = MockServer::start().await;
    once(&bare, "POST", "/video/generations", answer(json!({ "id": "j2", "status": "queued" }), 202)).await;
    // Raw text: the key order is the point, and a JSON value would sort it.
    let finished = r#"{"id":"j2","status":"completed","served_by":"someone"}"#;
    once(&bare, "GET", "/video/generations/j2", ResponseTemplate::new(200).set_body_raw(finished, "application/json")).await;
    let other = LadderVideoGenerator::new(bare.uri(), "k", "video").with_poll_interval(Duration::ZERO);
    let error = other.generate(request("anything", None)).await.unwrap_err();
    assert!(error.to_string().ends_with("top-level keys: id, status, served_by"), "{error}");
}

#[test]
fn finds_the_clip_wherever_the_job_put_it_preferring_an_mp4_and_never_its_own_poll_url() {
    let poll = "https://api.example/v1/video/generations/x";
    let job = |value: Value| -> JobNode { serde_json::from_value(value).unwrap() };
    assert_eq!(video_url_of(&job(json!({ "poll_url": poll, "cancel_url": poll, "status": "completed" }))), None);
    assert_eq!(
        video_url_of(&job(json!({ "poll_url": poll, "data": [{ "url": "https://cdn.example/a.mp4?sig=1" }] }))).as_deref(),
        Some("https://cdn.example/a.mp4?sig=1")
    );
    assert_eq!(
        video_url_of(&job(json!({
            "poll_url": poll,
            "result": { "thumbnail_url": "https://cdn.example/t.jpg", "video": { "url": "https://cdn.example/v" } }
        })))
        .as_deref(),
        Some("https://cdn.example/v")
    );
    assert_eq!(
        video_url_of(&job(json!({ "a": { "cover_url": "https://cdn.example/c" }, "b": { "asset_url": "https://cdn.example/x" } }))).as_deref(),
        Some("https://cdn.example/x")
    );
}
