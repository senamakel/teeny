//! The transport half of `legacy/test/telegram-bot.test.ts`, against a
//! wiremock Bot API.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_jobs::chat::{ChatAnswer, ChatMessage};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::telegram::api::TelegramApi;
use crate::telegram::inbox::BotIdentity;

#[derive(Default)]
struct Handler {
    answer: Option<ChatAnswer>,
    asked: Mutex<Vec<ChatMessage>>,
    kept: Mutex<Vec<ChatMessage>>,
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
    for call in ["sendMessage", "setMessageReaction", "sendChatAction"] {
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
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
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
            text: Some("gm fren".into()),
            reaction: Some("🔥".into()),
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
        answer: Some(ChatAnswer {
            text: Some("a token".into()),
            reaction: None,
        }),
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
        answer: Some(ChatAnswer {
            text: Some(long),
            reaction: None,
        }),
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
