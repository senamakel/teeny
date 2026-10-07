//! The transport half of `legacy/test/telegram-bot.test.ts`, against a
//! wiremock Bot API.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_jobs::chat::{ChatAnswer, ChatMessage};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use teeny_media::store::StoredMedia;
use teeny_media::types::{MediaKind, MimeType};

use super::*;
use crate::telegram::api::TelegramApi;
use crate::telegram::inbox::BotIdentity;

#[derive(Default)]
struct Handler {
    answer: Option<ChatAnswer>,
    asked: Mutex<Vec<ChatMessage>>,
    kept: Mutex<Vec<ChatMessage>>,
    /// What the eyes say; `None` is no eyes.
    sees: Option<Result<String, String>>,
    looked_at: Mutex<Vec<(Vec<u8>, MimeType)>>,
}

#[async_trait]
impl ChatHandler for Handler {
    async fn keep_line(&self, message: &ChatMessage) {
        self.kept.lock().unwrap().push(message.clone());
    }

    async fn respond(&self, message: &ChatMessage) -> Option<ChatAnswer> {
        self.asked.lock().unwrap().push(message.clone());
        self.answer.clone()
    }

    fn can_see(&self) -> bool {
        self.sees.is_some()
    }

    async fn describe_image(&self, data: Vec<u8>, mime_type: MimeType) -> anyhow::Result<String> {
        self.looked_at.lock().unwrap().push((data, mime_type));
        match self.sees.clone().expect("only asked when it can see") {
            Ok(seen) => Ok(seen),
            Err(error) => Err(anyhow::anyhow!(error)),
        }
    }
}

fn text(text: &str) -> ChatAnswer {
    ChatAnswer {
        text: Some(text.into()),
        reaction: None,
        gif_url: None,
        media: None,
    }
}

fn update(id: i64, message_id: i64, chat: Value, text: &str) -> Value {
    json!({ "update_id": id, "message": {
        "message_id": message_id, "date": 1_790_000_000, "chat": chat, "text": text,
        "from": { "id": 7, "is_bot": false, "first_name": "Alice", "username": "alice" }
    }})
}

fn dm() -> Value {
    json!({ "id": 7, "type": "private" })
}

async fn bot_api(updates: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/botT/getUpdates"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": updates })),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/botT/getUpdates"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "ok": true, "result": [] }))
                .set_delay(Duration::from_millis(50)),
        )
        .mount(&server)
        .await;
    for call in [
        "sendMessage",
        "setMessageReaction",
        "sendChatAction",
        "sendAnimation",
        "sendPhoto",
        "sendVideo",
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/botT/{call}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "ok": true, "result": { "message_id": 500 } })),
            )
            .mount(&server)
            .await;
    }
    server
}

fn options(debounce_ms: u64) -> PollerOptions {
    PollerOptions {
        bot: BotIdentity {
            id: 99,
            username: "teenybot".into(),
        },
        debounce: Duration::from_millis(debounce_ms),
        poll_timeout_s: 0,
        dry_run: false,
    }
}

async fn calls(server: &MockServer, name: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request: &&Request| request.url.path().ends_with(name))
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

/// The multipart bodies of `name`'s calls, as text.
async fn uploads(server: &MockServer, name: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request: &&Request| request.url.path().ends_with(name))
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect()
}

/// The non-poll calls, in order.
async fn order(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().rsplit('/').next().unwrap().to_owned())
        .filter(|call| call != "getUpdates" && call != "sendChatAction")
        .collect()
}

async fn answering(update: Value, handler: Handler, wait_for: &str) -> (MockServer, Arc<Handler>) {
    let server = bot_api(vec![update]).await;
    let handler = Arc::new(handler);
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    let running = Poller::new(api, handler.clone(), options(0)).start();
    settle(&server, wait_for, 1).await;
    settle(&server, "getUpdates", 2).await;
    running.stop().await;
    (server, handler)
}

fn stored(kind: MediaKind) -> StoredMedia {
    StoredMedia {
        id: "img_x_1".into(),
        kind,
        data: vec![9],
        mime_type: match kind {
            MediaKind::Image => MimeType::Png,
            MediaKind::Video => MimeType::Mp4,
        },
        prompt: "a tiny robot".into(),
        created_at: 0,
        path: None,
    }
}

async fn settle(server: &MockServer, name: &str, count: usize) {
    for _ in 0..100 {
        if calls(server, name).await.len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn polls_answers_a_dm_with_the_reaction_first_then_the_text_and_moves_the_offset() {
    let server = bot_api(vec![update(41, 10, dm(), "gm")]).await;
    let handler = Arc::new(Handler {
        answer: Some(ChatAnswer {
            reaction: Some("🔥".into()),
            ..text("gm fren")
        }),
        ..Handler::default()
    });
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    let running = Poller::new(api, handler.clone(), options(0)).start();

    settle(&server, "sendMessage", 1).await;
    settle(&server, "getUpdates", 2).await;
    running.stop().await;

    assert_eq!(handler.asked.lock().unwrap()[0].text, "gm");
    let reaction = &calls(&server, "setMessageReaction").await[0];
    assert_eq!(reaction["chat_id"], 7);
    assert_eq!(reaction["message_id"], 10);
    assert_eq!(
        reaction["reaction"],
        json!([{ "type": "emoji", "emoji": "🔥" }])
    );
    let sent = &calls(&server, "sendMessage").await[0];
    assert_eq!(sent["chat_id"], 7);
    assert_eq!(sent["text"], "gm fren");
    assert!(
        sent.get("reply_parameters").is_none(),
        "a DM is not threaded"
    );
    let polls = calls(&server, "getUpdates").await;
    assert_eq!(polls[1]["offset"], 42);
    let order: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().rsplit('/').next().unwrap().to_owned())
        .filter(|call| call != "getUpdates")
        .collect();
    assert_eq!(
        order,
        ["sendChatAction", "setMessageReaction", "sendMessage"]
    );
}

#[tokio::test]
async fn joins_one_persons_burst_into_one_turn() {
    let group = json!({ "id": -100, "type": "supergroup", "title": "holders" });
    let mention = |id: i64, message_id: i64, text: &str| {
        let mut update = update(id, message_id, group.clone(), &format!("@teenybot {text}"));
        update["message"]["entities"] = json!([{ "type": "mention", "offset": 0, "length": 9 }]);
        update
    };
    let server = bot_api(vec![
        mention(1, 10, "hey"),
        mention(2, 11, "quick q"),
        mention(3, 12, "what is $TINY?"),
    ])
    .await;
    let handler = Arc::new(Handler {
        answer: Some(text("a token")),
        ..Handler::default()
    });
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    let running = Poller::new(api, handler.clone(), options(150)).start();
    settle(&server, "sendMessage", 1).await;
    running.stop().await;

    let asked = handler.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].text, "hey\nquick q\nwhat is $TINY?");
    assert_eq!(asked[0].id, "12");
    assert_eq!(asked[0].batch_ids, ["10", "11", "12"]);
    let sent = &calls(&server, "sendMessage").await[0];
    assert_eq!(
        sent["reply_parameters"]["message_id"], 12,
        "threaded in a room"
    );
}

#[tokio::test]
async fn keeps_a_group_line_not_for_it_and_answers_nothing() {
    let group = json!({ "id": -100, "type": "supergroup" });
    let server = bot_api(vec![update(1, 10, group, "hello room")]).await;
    let handler = Arc::new(Handler::default());
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    let running = Poller::new(api, handler.clone(), options(0)).start();
    settle(&server, "getUpdates", 2).await;
    running.stop().await;

    assert_eq!(handler.kept.lock().unwrap()[0].text, "hello room");
    assert!(handler.asked.lock().unwrap().is_empty());
    assert!(calls(&server, "sendMessage").await.is_empty());
}

#[tokio::test]
async fn splits_a_long_answer_and_flushes_a_waiting_burst_on_stop() {
    let server = bot_api(vec![update(1, 10, dm(), "tell me everything")]).await;
    let long = format!("{}\n\n{}", "a".repeat(4_000), "b".repeat(200));
    let handler = Arc::new(Handler {
        answer: Some(text(&long)),
        ..Handler::default()
    });
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    // A debounce longer than the test: only the stop's drain answers it.
    let running = Poller::new(api, handler.clone(), options(60_000)).start();
    settle(&server, "getUpdates", 2).await;
    running.stop().await;

    let sent = calls(&server, "sendMessage").await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["text"], "b".repeat(200));
}

// telegram-bot: "sends a GIF after the text when the drafter asked for one"
#[tokio::test]
async fn sends_the_gif_after_the_words() {
    let (server, _) = answering(
        update(1, 10, dm(), "hi"),
        Handler {
            answer: Some(ChatAnswer {
                gif_url: Some("https://g/wave.gif".into()),
                ..text("hi")
            }),
            ..Handler::default()
        },
        "sendAnimation",
    )
    .await;
    assert_eq!(order(&server).await, ["sendMessage", "sendAnimation"]);
    let gif = &calls(&server, "sendAnimation").await[0];
    assert_eq!(gif["chat_id"], 7);
    assert_eq!(gif["animation"], "https://g/wave.gif");
}

// telegram-bot: "sends a generated image with the reply as its caption"
#[tokio::test]
async fn sends_a_generated_image_with_the_words_as_its_caption() {
    let (server, _) = answering(
        update(1, 10, dm(), "draw me a tiny robot"),
        Handler {
            answer: Some(ChatAnswer {
                media: Some(stored(MediaKind::Image)),
                ..text("here you go")
            }),
            ..Handler::default()
        },
        "sendPhoto",
    )
    .await;
    assert_eq!(order(&server).await, ["sendPhoto"]);
    let upload = &uploads(&server, "sendPhoto").await[0];
    assert!(upload.contains("here you go"), "the caption");
}

// telegram-bot: "sends a generated video as a video, and a picture alone
// when there are no words"
#[tokio::test]
async fn sends_a_generated_video_as_a_video_alone_when_there_are_no_words() {
    let (server, _) = answering(
        update(1, 10, dm(), "make it dance"),
        Handler {
            answer: Some(ChatAnswer {
                text: None,
                reaction: None,
                gif_url: None,
                media: Some(stored(MediaKind::Video)),
            }),
            ..Handler::default()
        },
        "sendVideo",
    )
    .await;
    assert_eq!(order(&server).await, ["sendVideo"]);
}

// telegram-bot: "falls back to the words when the picture will not send"
#[tokio::test]
async fn falls_back_to_the_words_when_the_picture_will_not_send() {
    let server = bot_api(vec![update(1, 10, dm(), "draw")]).await;
    Mock::given(method("POST"))
        .and(path("/botT/sendPhoto"))
        .respond_with(ResponseTemplate::new(413).set_body_json(
            json!({ "ok": false, "error_code": 413, "description": "Request Entity Too Large" }),
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    let handler = Arc::new(Handler {
        answer: Some(ChatAnswer {
            media: Some(stored(MediaKind::Image)),
            ..text("here")
        }),
        ..Handler::default()
    });
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    let running = Poller::new(api, handler, options(0)).start();
    settle(&server, "sendMessage", 1).await;
    running.stop().await;
    assert_eq!(order(&server).await, ["sendPhoto", "sendMessage"]);
    assert_eq!(calls(&server, "sendMessage").await[0]["text"], "here");
}

// --- pictures sent to the account ---

const PNG: [u8; 16] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52,
];

fn photo(chat: Value, caption: Option<&str>) -> Value {
    let mut update = update(1, 10, chat, "");
    let message = update["message"].as_object_mut().unwrap();
    message.remove("text");
    if let Some(caption) = caption {
        message.insert("caption".into(), json!(caption));
    }
    message.insert(
        "photo".into(),
        json!([
            { "file_id": "small", "file_unique_id": "s", "width": 90, "height": 90 },
            { "file_id": "large", "file_unique_id": "l", "width": 800, "height": 800 }
        ]),
    );
    update
}

/// Telegram's file API: `getFile`, then the bytes under `/file/botT/`.
async fn file_api(server: &MockServer, size: u64, bytes: Vec<u8>) {
    Mock::given(method("POST"))
        .and(path("/botT/getFile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": {
            "file_id": "large", "file_unique_id": "l", "file_size": size, "file_path": "photos/file_1.jpg"
        }})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/file/botT/photos/file_1.jpg"))
        // Telegram calls everything octet-stream; the bytes say what it is.
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(bytes),
        )
        .mount(server)
        .await;
}

async fn photo_turn(
    update: Value,
    handler: Handler,
    size: u64,
    bytes: Vec<u8>,
) -> (MockServer, Arc<Handler>) {
    let server = bot_api(vec![update]).await;
    file_api(&server, size, bytes).await;
    let handler = Arc::new(handler);
    let api = Arc::new(TelegramApi::new("T").with_base_url(server.uri()));
    let running = Poller::new(api, handler.clone(), options(0)).start();
    settle(&server, "getUpdates", 2).await;
    for _ in 0..100 {
        if !handler.asked.lock().unwrap().is_empty() || !handler.kept.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    running.stop().await;
    (server, handler)
}

// telegram-bot: "looks at a photo sent to it and hands the drafter what it saw"
#[tokio::test]
async fn looks_at_the_largest_size_of_a_photo_and_hands_the_drafter_what_it_saw() {
    let (server, handler) = photo_turn(
        photo(dm(), Some("thoughts?")),
        Handler {
            sees: Some(Ok("a chart of $TINY,\n up and to the right".into())),
            ..Handler::default()
        },
        16,
        PNG.to_vec(),
    )
    .await;
    assert_eq!(calls(&server, "getFile").await[0]["file_id"], "large");
    assert_eq!(
        *handler.looked_at.lock().unwrap(),
        [(PNG.to_vec(), MimeType::Png)],
        "the type comes from the bytes"
    );
    assert_eq!(
        handler.asked.lock().unwrap()[0].text,
        "thoughts?\n[image: a chart of $TINY, up and to the right]"
    );
}

// telegram-bot: "answers a photo with no caption at all"
#[tokio::test]
async fn answers_a_photo_with_no_caption_at_all() {
    let (_, handler) = photo_turn(
        photo(dm(), None),
        Handler {
            sees: Some(Ok("a cat".into())),
            ..Handler::default()
        },
        16,
        PNG.to_vec(),
    )
    .await;
    assert_eq!(handler.asked.lock().unwrap()[0].text, "[image: a cat]");
}

// telegram-bot: "says a photo was sent when nothing can look at it, and
// keeps going when looking fails"
#[tokio::test]
async fn says_a_photo_was_sent_when_blind_and_could_not_be_read_when_looking_fails() {
    let (server, handler) =
        photo_turn(photo(dm(), None), Handler::default(), 16, PNG.to_vec()).await;
    assert!(
        calls(&server, "getFile").await.is_empty(),
        "no eyes, no download"
    );
    assert_eq!(handler.asked.lock().unwrap()[0].text, "[sent a photo]");

    let (_, handler) = photo_turn(
        photo(dm(), Some("look")),
        Handler {
            sees: Some(Err("400".into())),
            ..Handler::default()
        },
        16,
        PNG.to_vec(),
    )
    .await;
    assert_eq!(
        handler.asked.lock().unwrap()[0].text,
        "look\n[sent a photo that could not be read]"
    );
}

#[tokio::test]
async fn never_looks_at_a_file_that_is_too_big_or_not_a_picture() {
    let eyes = || Handler {
        sees: Some(Ok("a cat".into())),
        ..Handler::default()
    };
    let (server, handler) =
        photo_turn(photo(dm(), None), eyes(), 50 * 1024 * 1024, PNG.to_vec()).await;
    assert!(handler.looked_at.lock().unwrap().is_empty());
    assert_eq!(
        handler.asked.lock().unwrap()[0].text,
        "[sent a photo that could not be read]"
    );
    let downloads = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path().starts_with("/file/"))
        .count();
    assert_eq!(
        downloads, 0,
        "refused on getFile's size, before downloading"
    );

    let (_, handler) = photo_turn(
        photo(dm(), None),
        eyes(),
        16,
        b"<html>not an image</html>".to_vec(),
    )
    .await;
    assert!(handler.looked_at.lock().unwrap().is_empty());
    assert_eq!(
        handler.asked.lock().unwrap()[0].text,
        "[sent a photo that could not be read]"
    );
}

// telegram-bot: "does not spend a vision call on a group photo that was not for it"
#[tokio::test]
async fn does_not_look_at_a_group_photo_that_was_not_for_it() {
    let group = json!({ "id": -100, "type": "supergroup", "title": "holders" });
    let (server, handler) = photo_turn(
        photo(group, Some("look at this")),
        Handler {
            sees: Some(Ok("a cat".into())),
            ..Handler::default()
        },
        16,
        PNG.to_vec(),
    )
    .await;
    assert!(calls(&server, "getFile").await.is_empty());
    assert!(handler.asked.lock().unwrap().is_empty());
    // The room still knows a photo went by.
    assert_eq!(
        handler.kept.lock().unwrap()[0].text,
        "look at this\n[sent a photo]"
    );
}
