//! The account's memory on a TinyMemory store, against the contract's own
//! in-memory reference engine.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{ItemKind, ListRequest, MemoryEngine, MetaFilter, Role as TurnRole};

use super::*;
use crate::{ChatKind, Memory, MemoryChatMessage, MemoryPost};

const ROOT: &str = "team:teeny";

fn engine() -> Arc<ReferenceEngine> {
    Arc::new(ReferenceEngine::new())
}

fn memory_on(engine: &Arc<ReferenceEngine>) -> TinyMemory {
    let store = EngineStore::new(engine.clone(), ROOT).unwrap();
    TinyMemory::new(Arc::new(store))
}

async fn everything(engine: &ReferenceEngine) -> Vec<Hit> {
    engine
        .list(ListRequest::new(MetaFilter::default(), 100))
        .await
        .unwrap()
        .items
}

fn post(id: &str, text: &str) -> MemoryPost {
    MemoryPost {
        id: id.into(),
        text: text.into(),
        author_id: Some("7".into()),
        author_username: Some("ana".into()),
        conversation_id: Some("55".into()),
        created_at: Some("2026-09-12T23:00:00.000Z".into()),
    }
}

#[tokio::test]
async fn a_mention_is_a_conversation_turn_at_the_root_with_every_legacy_label_and_key() {
    let engine = engine();
    let memory = memory_on(&engine);
    memory
        .capture_mention(&post("10", "gm teeny"))
        .await
        .unwrap();

    let items = everything(&engine).await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.kind, ItemKind::Conversation);
    assert_eq!(item.meta.namespace.to_string(), ROOT);
    assert_eq!(item.meta.tags, ["x", "mention", "@ana", "thread:55"]);
    assert_eq!(item.meta.source.id.as_deref(), Some("x:mention:2:10"));
    assert_eq!(
        item.meta.observed_at.map(|at| at.to_rfc3339()),
        Some("2026-09-12T23:00:00+00:00".to_owned())
    );
    let actor = item.meta.observed_actor.as_ref().unwrap();
    assert_eq!(actor.id, "user:x-7");
    assert_eq!(actor.name.as_deref(), Some("ana"));
    assert_eq!(item.text, "user: gm teeny");
}

#[tokio::test]
async fn capturing_the_same_thing_twice_stores_it_once() {
    let engine = engine();
    let memory = memory_on(&engine);
    memory.capture_mention(&post("10", "gm")).await.unwrap();
    memory.capture_mention(&post("10", "gm")).await.unwrap();
    // A reply's observed_at is the moment it was captured, so a retry is
    // a second later; it is still the same memory.
    memory
        .capture_reply(&post("10", "gm"), "gm fren", Some("11"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    memory
        .capture_reply(&post("10", "gm"), "gm fren", Some("11"))
        .await
        .unwrap();
    assert_eq!(everything(&engine).await.len(), 2);
}

#[tokio::test]
async fn activity_is_a_json_document_keyed_and_labelled_by_its_kind() {
    let engine = engine();
    let memory = memory_on(&engine);
    memory
        .capture_activity(
            "chart_post",
            "abc",
            json!({ "text": "hourly candles" }),
            None,
        )
        .await
        .unwrap();
    let items = everything(&engine).await;
    let item = &items[0];
    assert_eq!(item.kind, ItemKind::Document);
    assert_eq!(item.meta.tags, ["x", "activity", "chart_post"]);
    assert_eq!(item.meta.source.id.as_deref(), Some("x:chart_post:2:abc"));
    assert!(item.meta.observed_actor.is_none());
    let data: serde_json::Value = serde_json::from_str(&item.text).unwrap();
    assert_eq!(data["kind"], "chart_post");
    assert_eq!(data["payload"]["text"], "hourly candles");
}

#[tokio::test]
async fn a_chat_message_keeps_its_channel_chat_and_author() {
    let engine = engine();
    let memory = memory_on(&engine);
    let message = MemoryChatMessage {
        id: "3".into(),
        channel: "telegram".into(),
        chat_id: "-100".into(),
        chat_kind: ChatKind::Group,
        text: "wen chart".into(),
        author_id: "42".into(),
        author_username: Some("bob".into()),
        created_at: None,
    };
    memory.capture_chat_message(&message).await.unwrap();
    let items = everything(&engine).await;
    assert_eq!(
        items[0].meta.tags,
        ["telegram", "message", "group", "@bob", "chat:telegram--100"]
    );
    assert_eq!(
        items[0].meta.source.id.as_deref(),
        Some("telegram:message:2:-100:3")
    );
    assert_eq!(
        items[0]
            .meta
            .observed_actor
            .as_ref()
            .map(|actor| actor.id.as_str()),
        Some("user:telegram-42")
    );
}

#[tokio::test]
async fn a_message_with_no_text_is_not_stored() {
    let engine = engine();
    let memory = memory_on(&engine);
    memory.capture_mention(&post("10", "  ")).await.unwrap();
    assert!(everything(&engine).await.is_empty());
}

#[tokio::test]
async fn general_recall_reads_conversation_and_posts_but_never_webhook_noise() {
    let engine = engine();
    let memory = memory_on(&engine);
    memory
        .capture_activity(
            "chart_post",
            "1",
            json!({ "text": "the chart broke out" }),
            None,
        )
        .await
        .unwrap();
    memory
        .capture_activity(
            "like",
            "2",
            json!({ "text": "liked: the chart broke out" }),
            None,
        )
        .await
        .unwrap();
    let recalled = memory.recall("chart", None).await.unwrap();
    assert!(recalled.starts_with("GENERAL CONTEXT:\n"), "{recalled}");
    assert!(recalled.contains("the chart broke out"));
    assert!(!recalled.contains("liked:"), "{recalled}");
}

#[tokio::test]
async fn recall_about_a_person_reads_their_exchanges_by_handle_and_names_who_said_what() {
    let engine = engine();
    let memory = memory_on(&engine);
    memory
        .capture_mention(&post("10", "is the lab door open"))
        .await
        .unwrap();
    memory
        .capture_reply(
            &post("10", "is the lab door open"),
            "always ajar",
            Some("11"),
        )
        .await
        .unwrap();
    let mut other = post("12", "something else entirely");
    other.author_id = Some("8".into());
    other.author_username = Some("bob".into());
    memory.capture_mention(&other).await.unwrap();

    let ana = Person::x("7", Some("ana"));
    let recalled = memory.recall("door", Some(&ana)).await.unwrap();
    let about = recalled
        .strip_prefix("ABOUT @ana:\n")
        .unwrap_or_else(|| panic!("no person section: {recalled}"));
    let about = about.split("GENERAL CONTEXT:").next().unwrap();
    assert!(about.contains("@ana: is the lab door open"), "{about}");
    assert!(about.contains("you: always ajar"), "{about}");
    assert!(!about.contains("something else"), "{about}");
}

#[tokio::test]
async fn nothing_known_is_an_empty_recall() {
    let engine = engine();
    let memory = memory_on(&engine);
    assert_eq!(memory.recall("anything", None).await.unwrap(), "");
}

/// A store that answers after `delay`, counting the reads it is asked.
struct Slow {
    inner: EngineStore,
    delay: Duration,
    reads: AtomicUsize,
    fail_person: bool,
}

#[async_trait::async_trait]
impl ItemStore for Slow {
    async fn store(&self, item: StoreItem) -> Result<StoreReceipt> {
        tokio::time::sleep(self.delay).await;
        self.inner.store(item).await
    }

    async fn search(&self, query: &str, tags_any: &[String], limit: usize) -> Result<Vec<Hit>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.inner.search(query, tags_any, limit).await
    }

    async fn latest(&self, tags_any: &[String], limit: usize) -> Result<Vec<Hit>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_person {
            return Err(MemoryError::Store("the person pass failed".into()));
        }
        tokio::time::sleep(self.delay).await;
        self.inner.latest(tags_any, limit).await
    }
}

fn slow(delay: Duration, fail_person: bool) -> Arc<Slow> {
    Arc::new(Slow {
        inner: EngineStore::new(engine(), ROOT).unwrap(),
        delay,
        reads: AtomicUsize::new(0),
        fail_person,
    })
}

fn options(recall_ms: u64, capture_ms: u64) -> CortexOptions {
    CortexOptions {
        recall_timeout: Duration::from_millis(recall_ms),
        capture_timeout: Duration::from_millis(capture_ms),
        cooldown: Duration::from_secs(60),
        ..CortexOptions::default()
    }
}

#[tokio::test]
async fn a_recall_past_its_deadline_times_out_and_pauses_recall_for_the_cooldown() {
    let store = slow(Duration::from_millis(300), false);
    let memory = TinyMemory::new(store.clone()).with_options(options(50, 1_000));
    let error = memory.recall("chart", None).await.unwrap_err();
    assert!(error.is_timeout(), "{error}");
    let reads = store.reads.load(Ordering::SeqCst);

    let paused = memory.recall("chart", None).await.unwrap_err();
    assert!(matches!(paused, MemoryError::Paused(_)), "{paused}");
    assert_eq!(
        store.reads.load(Ordering::SeqCst),
        reads,
        "a paused recall asks the store nothing"
    );
}

#[tokio::test]
async fn too_many_recalls_in_flight_fail_fast() {
    let store = slow(Duration::from_millis(200), false);
    let memory = Arc::new(TinyMemory::new(store).with_options(CortexOptions {
        max_in_flight: 1,
        ..options(1_000, 1_000)
    }));
    let first = {
        let memory = Arc::clone(&memory);
        tokio::spawn(async move { memory.recall("chart", None).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let second = memory.recall("chart", None).await.unwrap_err();
    assert!(matches!(second, MemoryError::Paused(_)), "{second}");
    assert!(first.await.unwrap().is_ok());
}

#[tokio::test]
async fn losing_the_person_pass_still_answers_with_the_general_one() {
    let store = slow(Duration::ZERO, true);
    store
        .inner
        .store(
            item_of(&Experience::activity(
                "chart_post",
                "1",
                json!({ "text": "the chart broke out" }),
                None,
                chrono::Utc::now(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    let memory = TinyMemory::new(store);
    let ana = Person::x("7", Some("ana"));
    let recalled = memory.recall("chart", Some(&ana)).await.unwrap();
    assert!(recalled.starts_with("GENERAL CONTEXT:"), "{recalled}");
}

#[tokio::test]
async fn a_capture_past_its_deadline_times_out() {
    let store = slow(Duration::from_millis(300), false);
    let memory = TinyMemory::new(store).with_options(options(1_000, 50));
    let error = memory.capture_mention(&post("1", "gm")).await.unwrap_err();
    assert!(error.is_timeout(), "{error}");
}

#[tokio::test]
async fn without_the_person_pass_the_handle_leads_the_general_query() {
    let engine = engine();
    let store = EngineStore::new(engine.clone(), ROOT).unwrap();
    let memory = TinyMemory::new(Arc::new(store)).with_options(CortexOptions {
        person_pass: false,
        ..CortexOptions::default()
    });
    memory
        .capture_mention(&post("10", "the lab door"))
        .await
        .unwrap();
    let ana = Person::x("7", Some("ana"));
    let recalled = memory.recall("door", Some(&ana)).await.unwrap();
    assert!(recalled.starts_with("GENERAL CONTEXT:"), "{recalled}");
    assert!(recalled.contains("@ana: the lab door"), "{recalled}");
}

#[test]
fn a_store_refuses_a_root_that_is_not_a_namespace() {
    assert!(EngineStore::new(engine(), "teeny").is_err());
    assert!(EngineStore::new(engine(), "").is_err());
}

#[test]
fn an_assistant_turn_renders_as_the_account_and_a_user_turn_as_the_handle() {
    let experience = Experience::reply(&post("1", "q"), "a", Some("2"), chrono::Utc::now());
    let StoreItem::Conversation { turns, .. } = item_of(&experience).unwrap() else {
        panic!("a reply is a conversation turn");
    };
    assert_eq!(turns[0].role, TurnRole::Assistant);
    assert!(
        turns[0].at.is_none(),
        "the time is the meta's, outside the identity"
    );
}
