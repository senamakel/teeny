//! Port of legacy/test/activity-stream.test.ts.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const BOT: &str = "2094854245468000256";

fn event(event_type: &str, id: &str) -> String {
    json!({ "data": { "event_uuid": id, "event_type": event_type, "filter": { "user_id": BOT }, "payload": { "id": id } } })
        .to_string()
}

/// A sleeper that records what it was asked to wait, and does not.
fn recording() -> (Arc<Mutex<Vec<Duration>>>, Sleeper) {
    let delays = Arc::new(Mutex::new(Vec::new()));
    let log = delays.clone();
    let sleeper: Sleeper = Arc::new(move |delay| {
        log.lock().unwrap().push(delay);
        Box::pin(async {})
    });
    (delays, sleeper)
}

fn stream_at(server: &MockServer, sleeper: Sleeper) -> ActivityStream {
    ActivityStream::new(teeny_core::Secret::new("t"))
        .with_url(format!("{}/2/activity/stream", server.uri()))
        .with_sleeper(sleeper)
}

#[test]
fn splits_lines_across_chunks_and_drops_keep_alives() {
    let first = event("post.mention.create", "900");
    let second = event("like.create", "901");
    let mut buffer = LineBuffer::default();
    let mut lines = Vec::new();
    for chunk in [
        "\r\n".to_owned(),
        first[..20].to_owned(),
        format!("{}\r\n\r\n", &first[20..]),
        format!("{second}\r\n"),
    ] {
        lines.extend(buffer.push(chunk.as_bytes()));
    }
    assert_eq!(lines, vec![first, second]);
}

#[test]
fn keeps_only_event_envelopes() {
    assert_eq!(
        parse_line(&event("like.create", "1")).unwrap()["data"]["event_uuid"],
        "1"
    );
    assert!(parse_line("not json").is_none());
    assert!(parse_line(r#"{"errors":[{"title":"ConnectionException"}]}"#).is_none());
    assert!(parse_line(r#"{"meta":{}}"#).is_none());
    assert!(parse_line("42").is_none());
}

#[test]
fn backs_off_linearly_on_a_drop_exponentially_on_http_and_longest_on_429() {
    assert_eq!(backoff(Failure::Network, 1), Duration::from_millis(250));
    assert_eq!(backoff(Failure::Network, 3), Duration::from_millis(750));
    assert_eq!(backoff(Failure::Network, 1000), Duration::from_secs(16));
    assert_eq!(backoff(Failure::Http(503), 1), Duration::from_secs(5));
    assert_eq!(backoff(Failure::Http(503), 2), Duration::from_secs(10));
    assert_eq!(backoff(Failure::Http(503), 30), Duration::from_secs(300));
    assert_eq!(backoff(Failure::Http(429), 1), Duration::from_secs(60));
    assert_eq!(backoff(Failure::Http(429), 2), Duration::from_secs(120));
    assert_eq!(backoff(Failure::Http(429), 30), Duration::from_secs(900));
}

#[tokio::test]
async fn yields_each_event_with_the_bearer_and_skips_what_is_not_one() {
    let server = MockServer::start().await;
    let body = format!(
        "\r\n{}\r\nnot json\r\n{{\"errors\":[{{\"title\":\"x\"}}]}}\r\n{}\r\n",
        event("post.mention.create", "900"),
        event("like.create", "901")
    );
    Mock::given(method("GET"))
        .and(path("/2/activity/stream"))
        .and(header("authorization", "Bearer t"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    let (_, sleeper) = recording();
    let events: Vec<Value> = stream_at(&server, sleeper).events().take(2).collect().await;
    assert_eq!(
        events[0],
        serde_json::from_str::<Value>(&event("post.mention.create", "900")).unwrap()
    );
    assert_eq!(events[1]["data"]["event_uuid"], "901");
}

#[tokio::test]
async fn reconnects_after_the_stream_ends_and_backs_off_on_http_errors() {
    let server = MockServer::start().await;
    for response in [
        ResponseTemplate::new(200).set_body_string(format!("{}\n", event("like.create", "1"))),
        ResponseTemplate::new(429).set_body_string(r#"{"title":"Too Many Requests"}"#),
        ResponseTemplate::new(503).set_body_string(r#"{"title":"nope"}"#),
        ResponseTemplate::new(200).set_body_string(format!("{}\n", event("like.create", "2"))),
    ] {
        Mock::given(method("GET"))
            .and(path("/2/activity/stream"))
            .respond_with(response)
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    let (delays, sleeper) = recording();
    let events: Vec<Value> = stream_at(&server, sleeper).events().take(2).collect().await;
    assert_eq!(events[1]["data"]["event_uuid"], "2");
    // A 429 waits a minute; the 503 after it is the second consecutive
    // failure, so the HTTP backoff has doubled once.
    assert_eq!(
        *delays.lock().unwrap(),
        vec![Duration::from_secs(60), Duration::from_secs(10)]
    );
}

#[tokio::test]
async fn gives_up_after_the_configured_number_of_consecutive_failures() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/activity/stream"))
        .respond_with(ResponseTemplate::new(500))
        .expect(3)
        .mount(&server)
        .await;
    let (delays, sleeper) = recording();
    let events: Vec<Value> = stream_at(&server, sleeper)
        .with_max_attempts(3)
        .events()
        .collect()
        .await;
    assert!(events.is_empty());
    assert_eq!(delays.lock().unwrap().len(), 2);
}
