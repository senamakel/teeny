//! A port of the release hook's cases in `legacy/src/index.ts`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use teeny_agent::deps::fake_vars;
use teeny_cadence::cadence::{Cadence, CadenceRules};
use teeny_cadence::clock::{SystemClock, iso};
use teeny_core::Config;
use teeny_store::holds::{Direction, HoldBy, HoldContext, HoldKind, HoldSurface};
use teeny_store::{Hold, HoldStatus, InMemoryPostLedger, PostLedger};
use teeny_store::posts::NewOwnPost;
use teeny_x::XApi;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn config() -> Config {
    Config::from_vars(fake_vars()).unwrap()
}

fn hold(context: HoldContext) -> Hold {
    Hold {
        id: "h1".into(),
        at: "2026-09-14T12:00:00.000Z".into(),
        direction: Direction::Outbound,
        kind: HoldKind::Draft,
        context,
        category: "judge".into(),
        reason: "unsure".into(),
        by: HoldBy::Judge,
        text: "gm fren".into(),
        links: None,
        status: HoldStatus::Pending,
        decided_at: None,
        note: None,
        effect: None,
    }
}

#[derive(Default)]
struct Sent(Mutex<Vec<(i64, String, Option<i64>, Option<i64>)>>);

#[async_trait]
impl ChatSend for Sent {
    async fn send_reply(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
        thread: Option<i64>,
    ) -> anyhow::Result<i64> {
        self.0
            .lock()
            .unwrap()
            .push((chat_id, text.to_owned(), reply_to, thread));
        Ok(5)
    }
}

fn release(x: XApi, posts: Arc<InMemoryPostLedger>, dry_run: bool) -> HoldRelease {
    let config = config();
    HoldRelease {
        x: Arc::new(x),
        cadence: Arc::new(Cadence::new(
            posts,
            CadenceRules::from_config(&config.cadence),
            Arc::new(SystemClock),
        )),
        dry_run,
        telegram: None,
        telegram_dry_run: false,
    }
}

#[tokio::test]
async fn publishes_an_approved_reply_on_x_as_written() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            serde_json::json!({ "data": { "id": "99", "text": "gm fren" } }),
        ))
        .mount(&server)
        .await;
    let x = XApi::from_config(&config().x).with_base_url(server.uri());
    let it = release(x, Arc::new(InMemoryPostLedger::default()), false);
    let context = HoldContext {
        post_id: Some("42".into()),
        ..HoldContext::new(HoldSurface::X)
    };
    assert_eq!(
        it.release(&hold(context)).await.unwrap(),
        "published on X as 99"
    );
    let body: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["text"], "gm fren");
    assert_eq!(body["reply"]["in_reply_to_tweet_id"], "42");
}

#[tokio::test]
async fn honours_dry_run_and_the_cadence_for_an_own_post() {
    let x = || XApi::from_config(&config().x).with_base_url("http://127.0.0.1:9");
    let rehearsal = release(x(), Arc::new(InMemoryPostLedger::default()), true);
    assert_eq!(
        rehearsal
            .release(&hold(HoldContext::new(HoldSurface::Chart)))
            .await
            .unwrap(),
        "dry run: would have published on X"
    );

    let posts = Arc::new(InMemoryPostLedger::default());
    posts
        .record(&NewOwnPost {
            post_id: "1".into(),
            kind: "morning".into(),
            posted_at: iso(chrono::Utc::now()),
            text: None,
        })
        .await
        .unwrap();
    let held = release(x(), posts, false)
        .release(&hold(HoldContext::new(HoldSurface::X)))
        .await
        .unwrap();
    assert!(held.starts_with("recorded, not published"), "{held}");
}

#[tokio::test]
async fn sends_an_approved_chat_draft_where_it_was_going() {
    let sent = Arc::new(Sent::default());
    let mut it = release(
        XApi::from_config(&config().x),
        Arc::new(InMemoryPostLedger::default()),
        false,
    );
    let context = HoldContext {
        chat_id: Some("-100".into()),
        message_id: Some("7".into()),
        message_thread_id: Some("3".into()),
        ..HoldContext::new(HoldSurface::Telegram)
    };
    assert_eq!(
        it.release(&hold(context.clone())).await.unwrap(),
        "recorded; this process has no Telegram client to send it with"
    );
    it.telegram = Some(sent.clone());
    assert_eq!(
        it.release(&hold(context.clone())).await.unwrap(),
        "sent on Telegram as 5"
    );
    assert_eq!(
        *sent.0.lock().unwrap(),
        [(-100, "gm fren".to_owned(), Some(7), Some(3))]
    );
    it.telegram_dry_run = true;
    assert_eq!(
        it.release(&hold(context)).await.unwrap(),
        "dry run: would have sent on Telegram"
    );
}

#[tokio::test]
async fn records_what_cannot_be_published_after_the_fact() {
    let it = release(
        XApi::from_config(&config().x),
        Arc::new(InMemoryPostLedger::default()),
        false,
    );
    let says = |surface| {
        let it = &it;
        async move { it.release(&hold(HoldContext::new(surface))).await.unwrap() }
    };
    assert_eq!(says(HoldSurface::Morning).await, "recorded; the morning post runs again tomorrow");
    assert_eq!(says(HoldSurface::Calendar).await, "recorded; the \"calendar\" item posts again on its next run");
    assert_eq!(says(HoldSurface::Narrative).await, "recorded; the serial writes its next episode on schedule");
    assert_eq!(says(HoldSurface::Article).await, "recorded; the next article is written on schedule");
    assert_eq!(says(HoldSurface::Moon).await, "recorded; the moon chart runs again tomorrow");
    assert_eq!(says(HoldSurface::Quote).await, "recorded; the quote post runs again tomorrow");
    assert_eq!(says(HoldSurface::Lore).await, "recorded; the lore post runs again on its next day");
    assert_eq!(says(HoldSurface::Task).await, "recorded; nothing here can publish it");
}
