use std::sync::Arc;

use serde_json::json;
use teeny_memory::InMemoryMemory;
use teeny_store::{InMemoryPeople, Interaction, People};
use teeny_x::activity::{ActivityEntry, ActivityInteraction, InteractionKind};
use teeny_x::intake::{ActivityMemory, PeopleSink};

use super::*;

#[tokio::test]
async fn keeps_activity_in_memory_under_its_kind() {
    let memory = Arc::new(InMemoryMemory::default());
    MemorySink(memory.clone())
        .capture_activity(ActivityEntry {
            kind: "like.create".into(),
            id: "905".into(),
            payload: json!({ "id": "905" }),
        })
        .await
        .unwrap();
    let kept = memory.experiences();
    assert_eq!(kept.len(), 1);
    let described = format!("{:?}", kept[0]);
    assert!(
        described.contains("like.create") && described.contains("905"),
        "{described}"
    );
}

#[tokio::test]
async fn credits_each_kind_of_interaction_to_the_person() {
    let people = Arc::new(InMemoryPeople::default());
    let sink = PeopleRecord(people.clone());
    for kind in [
        InteractionKind::Mention,
        InteractionKind::Reply,
        InteractionKind::Quote,
        InteractionKind::Repost,
        InteractionKind::Follow,
        InteractionKind::Like,
    ] {
        sink.observe(ActivityInteraction {
            user_id: "44".into(),
            username: Some("fren".into()),
            kind,
        })
        .await
        .unwrap();
    }
    let top = people.top(1).await.unwrap();
    assert_eq!(top[0].user_id, "44");
    assert_eq!(top[0].username.as_deref(), Some("fren"));
    for kind in [
        Interaction::Mention,
        Interaction::Reply,
        Interaction::Quote,
        Interaction::Repost,
        Interaction::Follow,
        Interaction::Like,
    ] {
        assert_eq!(top[0].counts.get(&kind), Some(&1), "{kind:?}");
    }
}
