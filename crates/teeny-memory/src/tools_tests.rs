use serde_json::json;

use super::*;
use crate::{Content, InMemoryMemory, MemoryPost};

#[tokio::test]
async fn recall_memory_trims_what_came_back_and_says_so_when_nothing_did() {
    let memory = InMemoryMemory::default();
    assert_eq!(
        recall_memory(&memory, "anything", None).await.unwrap(),
        "No relevant memory was recalled."
    );

    memory
        .capture_mention(&MemoryPost {
            id: "1".into(),
            text: "the chart is hourly".into(),
            ..MemoryPost::default()
        })
        .await
        .unwrap();
    assert_eq!(
        recall_memory(&memory, "chart", None).await.unwrap(),
        "GENERAL CONTEXT:\n- the chart is hourly"
    );
    assert!(matches!(
        recall_memory(&memory, "", None).await,
        Err(MemoryError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn remember_stores_an_agent_note_and_names_it() {
    let memory = InMemoryMemory::default();
    let answer = remember(
        &memory,
        "they prefer charts in UTC",
        &["preference".to_owned()],
    )
    .await
    .unwrap();

    let [stored] = memory.experiences().try_into().unwrap();
    let id = stored
        .idempotency_key
        .strip_prefix("x:agent_note:2:")
        .unwrap()
        .to_owned();
    assert!(id.starts_with("note:"), "{id}");
    assert_eq!(answer, format!("Stored as {id}."));
    assert_eq!(
        stored.content,
        Content::Json {
            data: json!({ "kind": "agent_note", "payload": { "note": "they prefer charts in UTC", "labels": ["preference"] } })
        }
    );
}

#[tokio::test]
async fn remember_refuses_what_its_schema_refuses() {
    let memory = InMemoryMemory::default();
    assert!(remember(&memory, "", &[]).await.is_err());
    assert!(remember(&memory, &"x".repeat(2_001), &[]).await.is_err());
    assert!(
        remember(&memory, "ok", &vec!["l".to_owned(); 9])
            .await
            .is_err()
    );
    assert!(
        remember(&memory, &"é".repeat(2_000), &vec!["l".to_owned(); 8])
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn recall_memory_about_a_handle_asks_what_was_said_with_that_person() {
    let memory = InMemoryMemory::default();
    memory
        .capture_mention(&MemoryPost {
            id: "1".into(),
            text: "is the serial back?".into(),
            author_id: Some("7".into()),
            author_username: Some("ana".into()),
            ..MemoryPost::default()
        })
        .await
        .unwrap();
    // By the handle label, with or without the @, whatever the case.
    for about in ["@ana", "ana", "@Ana"] {
        let recalled = recall_memory(&memory, "the serial", Some(about))
            .await
            .unwrap();
        assert!(recalled.starts_with("ABOUT @"), "{recalled}");
        assert!(recalled.contains("is the serial back?"), "{recalled}");
    }
    assert!(matches!(
        recall_memory(&memory, "q", Some("not a handle!")).await,
        Err(MemoryError::InvalidInput(_))
    ));
}
