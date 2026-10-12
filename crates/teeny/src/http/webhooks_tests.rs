//! Ported from legacy/test/webhooks.test.ts: the routes end to end, the
//! forwarding, and the forwarder. The parsing cases live in teeny-x.

use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use teeny_core::types::UserId;
use teeny_store::InMemoryMentionQueue;
use teeny_x::activity::{ActivityInteraction, InteractionKind};
use teeny_x::intake::{AccountId, ActivityIntake, IntakeError, PeopleSink};
use wiremock::matchers::{body_string, header as has_header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::http::test_support::*;
use crate::http::{Services, router};
use crate::mentions::{QueueAdapter, QueuedXPost};

const BOT: &str = "2094854245468000256";

fn signed(payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(payload.as_bytes());
    format!(
        "sha256={}",
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    )
}

fn delivery_request(payload: &str, signature: Option<&str>) -> Request<Body> {
    let mut builder = Request::post("/webhooks/x").header(header::CONTENT_TYPE, "application/json");
    if let Some(signature) = signature {
        builder = builder.header(SIGNATURE_HEADER, signature);
    }
    builder.body(Body::from(payload.to_owned())).unwrap()
}

async fn eventually(mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("in time");
}

struct Account(Result<Option<&'static str>, &'static str>);

#[async_trait]
impl AccountId for Account {
    async fn bot_user_id(&self) -> Result<Option<UserId>, IntakeError> {
        match self.0 {
            Ok(id) => Ok(id.map(UserId::from)),
            Err(message) => Err(message.into()),
        }
    }
}

#[derive(Default)]
struct People(Mutex<Vec<ActivityInteraction>>);

#[async_trait]
impl PeopleSink for People {
    async fn observe(&self, interaction: ActivityInteraction) -> Result<(), IntakeError> {
        self.0.lock().unwrap().push(interaction);
        Ok(())
    }
}

struct Rig {
    app: axum::Router,
    queue: Arc<InMemoryMentionQueue<QueuedXPost>>,
    people: Arc<People>,
}

fn rig(account: Account) -> Rig {
    let queue = Arc::new(InMemoryMentionQueue::default());
    let people = Arc::new(People::default());
    let intake = ActivityIntake::new(Arc::new(RecordingMemory::default()))
        .with_queue(
            Arc::new(QueueAdapter(queue.clone())),
            Arc::new(account),
            "TinyMascot",
        )
        .with_people(people.clone());
    let app = router(Services {
        queue: Some(queue.clone()),
        ..Services::new(SECRET, Arc::new(intake))
    });
    Rig { app, queue, people }
}

#[tokio::test]
async fn answers_challenge_response_checks() {
    let app = router(services());
    let answer = call(&app, get("/webhooks/x?crc_token=challenge")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.json(),
        json!({ "response_token": signed("challenge") })
    );

    for uri in ["/webhooks/x", "/webhooks/x?crc_token="] {
        let refused = call(&app, get(uri)).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(refused.json(), json!({ "error": "crc_token is required" }));
    }
}

#[tokio::test]
async fn rejects_unsigned_events_and_captures_signed_activity() {
    let memory = Arc::new(RecordingMemory::default());
    let app = router(Services::new(
        SECRET,
        Arc::new(ActivityIntake::new(memory.clone())),
    ));
    let payload = json!({
        "tweet_create_events": [{ "id_str": "123", "text": "hello" }],
        "favorite_events": [{ "id": "456" }],
        "direct_message_events": [{ "id": "789" }]
    })
    .to_string();

    let unsigned = call(&app, delivery_request(&payload, None)).await;
    assert_eq!(unsigned.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unsigned.json(),
        json!({ "error": "invalid webhook signature" })
    );
    let forged = call(&app, delivery_request(&payload, Some("sha256=nope"))).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    let accepted = call(&app, delivery_request(&payload, Some(&signed(&payload)))).await;
    assert_eq!(accepted.status, StatusCode::OK);
    assert_eq!(accepted.json(), json!({ "ok": true }));
    // Only the message reaches memory: the post goes to the queue and is
    // captured later as a chat turn attributed to its author, and the like is
    // engagement, counted in the people store and not kept as a raw payload.
    eventually(|| memory.0.lock().unwrap().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let captured = memory.0.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        (captured[0].kind.as_str(), captured[0].id.as_str()),
        ("direct_message_events", "789")
    );
}

#[tokio::test]
async fn verifies_the_bytes_not_a_reparse_of_them() {
    let app = router(services());
    let payload = "{\"favorite_events\":[{\"id\":\"456\"}]}";
    let respaced = "{ \"favorite_events\": [ { \"id\": \"456\" } ] }";
    assert_eq!(
        call(&app, delivery_request(respaced, Some(&signed(payload))))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, delivery_request(respaced, Some(&signed(respaced))))
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn takes_no_signed_body_that_is_not_json() {
    let app = router(services());
    let request = Request::post("/webhooks/x")
        .header(header::CONTENT_TYPE, "text/plain")
        .header(SIGNATURE_HEADER, signed("hello"))
        .body(Body::from("hello"))
        .unwrap();
    assert_eq!(call(&app, request).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn queues_a_mention_using_the_full_text_of_a_truncated_post() {
    let rig = rig(Account(Ok(Some(BOT))));
    let payload = json!({
        "for_user_id": BOT,
        "tweet_create_events": [{
            "id_str": "900", "text": "hey @TinyMascot this is going to be cut off…", "truncated": true,
            "extended_tweet": { "full_text": "hey @TinyMascot the whole thing" },
            "created_at": "Wed Sep 02 13:30:16 +0000 2026",
            "user": { "id_str": "42", "screen_name": "someone" },
        }],
    })
    .to_string();

    assert_eq!(
        call(
            &rig.app,
            delivery_request(&payload, Some(&signed(&payload)))
        )
        .await
        .status,
        StatusCode::OK
    );
    let queued = rig.queue.get("900").expect("queued").post.0;
    assert_eq!(queued.text, "hey @TinyMascot the whole thing");
    assert_eq!(queued.author_username.as_deref(), Some("someone"));
    assert_eq!(
        queued.created_at.as_deref(),
        Some("2026-09-02T13:30:16.000Z")
    );
}

#[tokio::test]
async fn never_queues_the_accounts_own_posts_and_nothing_when_it_cannot_say_who_it_is() {
    let payload = json!({
        "for_user_id": BOT,
        "tweet_create_events": [{ "id_str": "901", "text": "gm from the bot", "user": { "id_str": BOT, "screen_name": "TinyMascot" } }],
    })
    .to_string();
    for account in [
        Account(Ok(Some(BOT))),
        Account(Ok(None)),
        Account(Err("Could not find user")),
    ] {
        let rig = rig(account);
        // Still 200: X retries anything else until it disables the webhook.
        assert_eq!(
            call(
                &rig.app,
                delivery_request(&payload, Some(&signed(&payload)))
            )
            .await
            .status,
            StatusCode::OK
        );
        assert!(rig.queue.get("901").is_none());
    }
}

#[tokio::test]
async fn queues_a_mention_from_the_activity_api_envelope_and_credits_the_people() {
    let rig = rig(Account(Ok(Some(BOT))));
    let mention = json!({ "data": {
        "event_type": "post.mention.create", "event_uuid": "777", "filter": { "user_id": BOT },
        "payload": { "id": "906", "text": "yo @TinyMascot", "author_id": "42" },
        "includes": { "users": [{ "id": "42", "username": "someone" }] },
    } })
    .to_string();
    let follow = json!({ "data": {
        "event_type": "follow.follow", "event_uuid": "778", "filter": { "user_id": BOT },
        "payload": { "source": { "id": "44" }, "target": { "id": BOT } },
    } })
    .to_string();

    assert_eq!(
        call(
            &rig.app,
            delivery_request(&mention, Some(&signed(&mention)))
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        call(&rig.app, delivery_request(&follow, Some(&signed(&follow))))
            .await
            .status,
        StatusCode::OK
    );

    let queued = rig.queue.get("906").expect("queued").post.0;
    assert_eq!(
        (queued.text.as_str(), queued.author_username.as_deref()),
        ("yo @TinyMascot", Some("someone"))
    );
    let people = rig.people.0.lock().unwrap().clone();
    assert!(people.contains(&ActivityInteraction {
        user_id: "44".into(),
        username: None,
        kind: InteractionKind::Follow
    }));
}

#[derive(Default)]
struct Copies(Mutex<Vec<(Bytes, Option<String>)>>);

#[async_trait]
impl Forwarder for Copies {
    async fn forward(&self, body: Bytes, signature: Option<String>) {
        self.0.lock().unwrap().push((body, signature));
    }
}

struct Stuck;

#[async_trait]
impl Forwarder for Stuck {
    async fn forward(&self, _body: Bytes, _signature: Option<String>) {
        pending::<()>().await;
    }
}

const FAVORITE: &str = "{\"favorite_events\":[{\"id\":\"456\"}]}";

#[tokio::test]
async fn sends_a_verified_delivery_on_body_and_signature_untouched() {
    let copies = Arc::new(Copies::default());
    let app = router(Services {
        forwarder: Some(copies.clone()),
        ..services()
    });

    let answer = call(&app, delivery_request(FAVORITE, Some(&signed(FAVORITE)))).await;
    assert_eq!(answer.status, StatusCode::OK);
    eventually(|| copies.0.lock().unwrap().len() == 1).await;
    let (body, signature) = copies.0.lock().unwrap()[0].clone();
    assert_eq!(body.as_ref(), FAVORITE.as_bytes());
    assert_eq!(signature, Some(signed(FAVORITE)));
}

#[tokio::test]
async fn forwards_nothing_x_did_not_sign() {
    let copies = Arc::new(Copies::default());
    let app = router(Services {
        forwarder: Some(copies.clone()),
        ..services()
    });
    let answer = call(&app, delivery_request(FAVORITE, Some("sha256=nope"))).await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(copies.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn answers_x_even_when_the_receiver_never_does() {
    let app = router(Services {
        forwarder: Some(Arc::new(Stuck)),
        ..services()
    });
    let answer = tokio::time::timeout(
        Duration::from_secs(2),
        call(&app, delivery_request(FAVORITE, Some(&signed(FAVORITE)))),
    )
    .await
    .expect("answered without waiting on the copy");
    assert_eq!(answer.status, StatusCode::OK);
}

#[tokio::test]
async fn the_forwarder_posts_the_bytes_with_the_signature_and_a_bearer_and_swallows_a_refusal() {
    let receiver = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/x"))
        .and(has_header("content-type", "application/json"))
        .and(has_header(SIGNATURE_HEADER, "sha256=abc"))
        .and(has_header(
            "authorization",
            "Bearer a-token-of-sixteen-chars",
        ))
        .and(body_string("{\"a\":1}"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&receiver)
        .await;
    let forwarder = HttpForwarder::new(
        format!("{}/x", receiver.uri()),
        Some("a-token-of-sixteen-chars".into()),
        Duration::from_secs(10),
    );

    forwarder
        .forward(Bytes::from_static(b"{\"a\":1}"), Some("sha256=abc".into()))
        .await;
}

#[tokio::test]
async fn the_forwarder_sends_no_signature_or_bearer_it_does_not_have() {
    let receiver = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&receiver)
        .await;
    HttpForwarder::new(receiver.uri(), None, Duration::from_secs(10))
        .forward(Bytes::from_static(b"{}"), None)
        .await;
    let request = &receiver.received_requests().await.unwrap()[0];
    assert!(request.headers.get(SIGNATURE_HEADER).is_none());
    assert!(request.headers.get("authorization").is_none());
}

#[tokio::test]
async fn the_forwarder_survives_a_receiver_that_is_down_or_slow() {
    // Nothing listens on port 9 of the loopback.
    HttpForwarder::new("http://127.0.0.1:9/x", None, Duration::from_secs(2))
        .forward(Bytes::from_static(b"{}"), None)
        .await;

    let slow = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&slow)
        .await;
    tokio::time::timeout(
        Duration::from_secs(2),
        HttpForwarder::new(slow.uri(), None, Duration::from_millis(100))
            .forward(Bytes::from_static(b"{}"), None),
    )
    .await
    .expect("gave up at its own timeout");
}
