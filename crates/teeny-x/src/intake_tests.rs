//! Port of legacy/test/activity-intake.test.ts, plus the identity cases of
//! legacy/test/webhooks.test.ts.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_core::types::UserId;

use super::*;
use crate::activity::{ActivityEntry, ActivityInteraction, InteractionKind};
use crate::model::XPost;

const BOT: &str = "2094854245468000256";

#[derive(Default)]
struct Queue(Mutex<Vec<Vec<XPost>>>);
#[async_trait]
impl MentionSink for Queue {
    async fn cache(&self, mentions: Vec<XPost>) -> Result<usize, IntakeError> {
        let count = mentions.len();
        self.0.lock().unwrap().push(mentions);
        Ok(count)
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

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Store,
    Hang,
    Fail,
}

struct Memory {
    captured: Mutex<Vec<ActivityEntry>>,
    behaviour: Mutex<Behaviour>,
}
impl Memory {
    fn new(behaviour: Behaviour) -> Arc<Self> {
        Arc::new(Self {
            captured: Mutex::new(Vec::new()),
            behaviour: Mutex::new(behaviour),
        })
    }
    async fn wait_for(&self, count: usize) {
        for _ in 0..100 {
            if self.captured.lock().unwrap().len() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "memory captured {} entries, expected {count}",
            self.captured.lock().unwrap().len()
        );
    }
}
#[async_trait]
impl ActivityMemory for Memory {
    async fn capture_activity(&self, entry: ActivityEntry) -> Result<(), IntakeError> {
        let behaviour = *self.behaviour.lock().unwrap();
        match behaviour {
            Behaviour::Store => {
                self.captured.lock().unwrap().push(entry);
                Ok(())
            }
            Behaviour::Hang => futures::future::pending().await,
            Behaviour::Fail => Err("cortex timed out".into()),
        }
    }
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

fn mention(id: &str) -> Value {
    json!({ "data": {
        "event_uuid": id, "event_type": "post.mention.create", "filter": { "user_id": BOT },
        "payload": { "id": id, "text": "hey @teenydroid", "author_id": "42", "created_at": "2026-09-21T12:00:00.000Z" },
        "includes": { "users": [{ "id": "42", "username": "someone" }] }
    }})
}

fn like(event_id: &str) -> Value {
    json!({ "data": {
        "event_uuid": event_id, "event_type": "like.create", "filter": { "user_id": BOT },
        "payload": { "id": "l1", "liked_tweet_id": "1", "liking_user_id": "42" },
        "includes": { "users": [{ "id": "42", "username": "someone" }] }
    }})
}

struct Rig {
    intake: ActivityIntake,
    queue: Arc<Queue>,
    people: Arc<People>,
    memory: Arc<Memory>,
}

fn rig(account: Account, remember: usize) -> Rig {
    let queue = Arc::new(Queue::default());
    let people = Arc::new(People::default());
    let memory = Memory::new(Behaviour::Store);
    let intake = ActivityIntake::new(memory.clone())
        .with_queue(queue.clone(), Arc::new(account), "teenydroid")
        .with_people(people.clone())
        .remembering(remember);
    Rig {
        intake,
        queue,
        people,
        memory,
    }
}

#[tokio::test]
async fn queues_a_streamed_mention_and_scores_the_author() {
    let rig = rig(Account(Ok(Some(BOT))), 10_000);
    let delivered = rig
        .intake
        .deliver(&mention("900"), Source::Stream)
        .await
        .unwrap();
    assert_eq!(
        delivered,
        Delivered {
            queued: 1,
            duplicate: false
        }
    );
    let queued = rig.queue.0.lock().unwrap();
    assert_eq!(queued[0][0].id.as_str(), "900");
    assert_eq!(queued[0][0].author_username.as_deref(), Some("someone"));
    assert_eq!(
        rig.people.0.lock().unwrap().as_slice(),
        [ActivityInteraction {
            user_id: "42".into(),
            username: Some("someone".into()),
            kind: InteractionKind::Mention
        }]
    );
}

#[tokio::test]
async fn handles_an_event_once_when_both_transports_deliver_it() {
    let rig = rig(Account(Ok(Some(BOT))), 10_000);
    rig.intake
        .deliver(&like("e1"), Source::Stream)
        .await
        .unwrap();
    assert_eq!(
        rig.intake
            .deliver(&like("e1"), Source::Webhook)
            .await
            .unwrap(),
        Delivered {
            queued: 0,
            duplicate: true
        }
    );
    rig.intake
        .deliver(&like("e2"), Source::Webhook)
        .await
        .unwrap();
    // A like counts once per event, not once per transport.
    assert_eq!(rig.people.0.lock().unwrap().len(), 2);
    rig.memory.wait_for(2).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(rig.memory.captured.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn forgets_the_oldest_ids_once_the_set_is_full() {
    let rig = rig(Account(Ok(Some(BOT))), 2);
    for id in ["e1", "e2", "e3"] {
        rig.intake.deliver(&like(id), Source::Stream).await.unwrap();
    }
    assert!(
        !rig.intake
            .deliver(&like("e1"), Source::Webhook)
            .await
            .unwrap()
            .duplicate
    );
    assert!(
        rig.intake
            .deliver(&like("e3"), Source::Webhook)
            .await
            .unwrap()
            .duplicate
    );
}

#[tokio::test]
async fn answers_before_memory_has_the_event_and_survives_memory_failing() {
    let memory = Memory::new(Behaviour::Hang);
    let intake = ActivityIntake::new(memory.clone());
    let delivered = tokio::time::timeout(
        Duration::from_millis(200),
        intake.deliver(&like("e1"), Source::Webhook),
    )
    .await;
    assert!(delivered.is_ok(), "the delivery waited on memory");

    *memory.behaviour.lock().unwrap() = Behaviour::Fail;
    assert_eq!(
        intake.deliver(&like("e2"), Source::Webhook).await.unwrap(),
        Delivered {
            queued: 0,
            duplicate: false
        }
    );
}

#[tokio::test]
async fn queues_nothing_when_the_account_cannot_be_identified() {
    let own = json!({ "for_user_id": BOT, "tweet_create_events": [
        { "id_str": "901", "text": "gm from the bot", "user": { "id_str": BOT, "screen_name": "teenydroid" } }
    ]});
    for account in [
        Account(Ok(None)),
        Account(Err("Could not find user with username: [TinyMascot].")),
    ] {
        let rig = rig(account, 10_000);
        assert_eq!(
            rig.intake.deliver(&own, Source::Webhook).await.unwrap(),
            Delivered {
                queued: 0,
                duplicate: false
            }
        );
        assert!(rig.queue.0.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn an_old_shape_delivery_is_never_a_duplicate() {
    let rig = rig(Account(Ok(Some(BOT))), 10_000);
    let payload = json!({ "favorite_events": [{ "id": "456" }] });
    assert!(
        !rig.intake
            .deliver(&payload, Source::Webhook)
            .await
            .unwrap()
            .duplicate
    );
    assert!(
        !rig.intake
            .deliver(&payload, Source::Webhook)
            .await
            .unwrap()
            .duplicate
    );
    rig.memory.wait_for(2).await;
}
