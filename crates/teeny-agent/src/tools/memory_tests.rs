//! Port of legacy/test/agent-tools.test.ts, "memory tools".

use std::sync::Arc;

use serde_json::json;
use teeny_memory::{Content, InMemoryMemory, Memory};

use super::*;
use crate::tools::test_support::call;
use crate::tools::{PermissionLevel, text_of};

#[tokio::test]
async fn recalls_context_and_stores_durable_notes() {
    let memory = Arc::new(InMemoryMemory::default());
    let stored = call(
        &remember(memory.clone()),
        json!({ "note": "stonks.rocks launched v3" }),
    )
    .await;
    assert!(
        text_of(&stored).starts_with("Stored as note:"),
        "{}",
        text_of(&stored)
    );
    let experiences = memory.experiences();
    assert_eq!(experiences.len(), 1);
    match &experiences[0].content {
        Content::Json { data } => {
            assert_eq!(data["kind"], "agent_note");
            assert_eq!(data["payload"]["note"], "stonks.rocks launched v3");
            assert_eq!(data["payload"]["labels"], json!([]));
        }
        other => panic!("expected a JSON note, got {other:?}"),
    }

    let recalled = call(
        &recall_memory(memory.clone() as Arc<dyn Memory>),
        json!({ "query": "what launched" }),
    )
    .await;
    assert!(text_of(&recalled).contains("stonks.rocks launched v3"));
}

#[tokio::test]
async fn recalls_about_a_person_by_handle_and_refuses_what_is_not_one() {
    let memory = Arc::new(InMemoryMemory::default());
    memory
        .capture_mention(&teeny_memory::MemoryPost {
            id: "1".into(),
            text: "gm from ana".into(),
            author_id: Some("7".into()),
            author_username: Some("ana".into()),
            ..teeny_memory::MemoryPost::default()
        })
        .await
        .unwrap();
    let tool = recall_memory(memory.clone() as Arc<dyn Memory>);
    let recalled = call(
        &tool,
        json!({ "query": "what did they say", "about": "@ana" }),
    )
    .await;
    assert!(
        text_of(&recalled).contains("ABOUT @ana:"),
        "{}",
        text_of(&recalled)
    );
    assert!(
        call(&tool, json!({ "query": "q", "about": "two words" }))
            .await
            .is_error
    );
}

#[tokio::test]
async fn says_so_when_nothing_is_recalled() {
    let memory = Arc::new(InMemoryMemory::default());
    let recalled = call(&recall_memory(memory), json!({ "query": "anything" })).await;
    assert_eq!(text_of(&recalled), "No relevant memory was recalled.");
}

#[tokio::test]
async fn refuses_a_note_too_long_or_too_labelled() {
    let memory = Arc::new(InMemoryMemory::default());
    let tool = remember(memory.clone());
    assert!(
        call(&tool, json!({ "note": "x".repeat(2_001) }))
            .await
            .is_error
    );
    let labels: Vec<String> = (0..9).map(|n| n.to_string()).collect();
    assert!(
        call(&tool, json!({ "note": "n", "labels": labels }))
            .await
            .is_error
    );
    assert!(memory.experiences().is_empty());
}

#[test]
fn remember_writes_and_recall_reads() {
    let memory = Arc::new(InMemoryMemory::default());
    assert_eq!(
        remember(memory.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert!(remember(memory.clone()).external_effect());
    assert_eq!(
        recall_memory(memory).permission_level(),
        PermissionLevel::ReadOnly
    );
}
