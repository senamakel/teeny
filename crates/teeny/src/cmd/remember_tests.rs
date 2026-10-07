use teeny_memory::{Content, InMemoryMemory};

use super::*;

fn args(note: &[&str], labels: &[&str]) -> RememberArgs {
    RememberArgs {
        note: note.iter().map(|word| (*word).to_owned()).collect(),
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
    }
}

#[tokio::test]
async fn stores_the_note_as_an_agent_note_activity_with_its_labels() {
    let memory = InMemoryMemory::default();
    let mut out = Vec::new();

    run(
        &memory,
        &args(&["Teeny", "is a meme bot"], &["identity"]),
        "agent:teeny at http://cortex",
        &mut out,
    )
    .await
    .unwrap();

    let stored = memory.experiences();
    assert_eq!(stored.len(), 1);
    let Content::Json { data } = &stored[0].content else {
        panic!("not json: {:?}", stored[0].content);
    };
    assert_eq!(data["kind"], "agent_note");
    assert_eq!(data["payload"]["note"], "Teeny is a meme bot");
    assert_eq!(data["payload"]["labels"][0], "identity");
    assert!(stored[0].idempotency_key.starts_with("x:agent_note:"));
    let printed = String::from_utf8(out).unwrap();
    assert!(printed.starts_with("Stored as note:"), "{printed}");
    assert!(printed.contains("in agent:teeny at http://cortex"), "{printed}");
}

#[tokio::test]
async fn refuses_an_empty_note() {
    let memory = InMemoryMemory::default();

    let error = run(&memory, &args(&["  "], &[]), "here", &mut Vec::new())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("usage"), "{error}");
    assert!(memory.experiences().is_empty());
}

#[test]
fn every_note_gets_its_own_id() {
    assert_ne!(note_id(), note_id());
    assert!(note_id().starts_with("note:"));
}
