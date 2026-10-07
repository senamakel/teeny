use serde_json::json;

use super::*;
use crate::{Memory, MemoryPost, Person};

fn post(id: &str, text: &str) -> MemoryPost {
    MemoryPost {
        id: id.into(),
        text: text.into(),
        author_id: Some("7".into()),
        author_username: Some("fren".into()),
        ..MemoryPost::default()
    }
}

#[tokio::test]
async fn keeps_every_capture_once_by_its_idempotency_key() {
    let memory = InMemoryMemory::default();
    memory
        .capture_mention(&post("1", "what is the chart"))
        .await
        .unwrap();
    memory
        .capture_mention(&post("1", "what is the chart"))
        .await
        .unwrap();
    memory
        .capture_reply(&post("1", "what is the chart"), "hourly candles", Some("9"))
        .await
        .unwrap();
    memory
        .capture_activity(
            "agent_note",
            "n1",
            json!({ "note": "the roadmap ships in Q4" }),
            None,
        )
        .await
        .unwrap();

    let keys: Vec<_> = memory
        .experiences()
        .into_iter()
        .map(|e| e.idempotency_key)
        .collect();
    assert_eq!(
        keys,
        ["x:mention:2:1", "x:reply:2:1:9", "x:agent_note:2:n1"]
    );
}

#[tokio::test]
async fn recalls_what_it_holds_about_the_person_and_the_topic() {
    let memory = InMemoryMemory::default();
    memory
        .capture_mention(&post("1", "what is the chart"))
        .await
        .unwrap();
    memory
        .capture_activity(
            "agent_note",
            "n1",
            json!({ "note": "the roadmap ships in Q4" }),
            None,
        )
        .await
        .unwrap();

    let fren = Person::x("7", Some("fren"));
    assert_eq!(
        memory.recall("roadmap", Some(&fren)).await.unwrap(),
        "ABOUT @fren:\n- what is the chart\n\nGENERAL CONTEXT:\n- {\"kind\":\"agent_note\",\"payload\":{\"note\":\"the roadmap ships in Q4\"}}"
    );
    assert_eq!(memory.recall("nothing matches", None).await.unwrap(), "");
}
