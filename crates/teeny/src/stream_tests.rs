use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_jobs::ingest::FixedAccount;
use teeny_store::{InMemoryMentionQueue, MentionQueue};
use teeny_x::activity::ActivityEntry;
use teeny_x::intake::{ActivityIntake, ActivityMemory, IntakeError, Source};

use super::*;
use crate::mentions::{QueueAdapter, QueuedXPost};

const BOT: &str = "2094854245468000256";

struct Forget;

#[async_trait]
impl ActivityMemory for Forget {
    async fn capture_activity(&self, _entry: ActivityEntry) -> Result<(), IntakeError> {
        Ok(())
    }
}

fn mention(id: &str) -> Value {
    json!({ "data": {
        "event_uuid": id, "event_type": "post.mention.create", "filter": { "user_id": BOT },
        "payload": { "id": id, "text": "hey @teenydroid", "author_id": "42", "created_at": "2026-09-21T12:00:00.000Z" },
        "includes": { "users": [{ "id": "42", "username": "someone" }] }
    }})
}

#[tokio::test]
async fn the_stream_and_the_webhook_feed_one_intake_into_one_queue() {
    let queue = Arc::new(InMemoryMentionQueue::<QueuedXPost>::default());
    let intake = Arc::new(ActivityIntake::new(Arc::new(Forget)).with_queue(
        Arc::new(QueueAdapter(queue.clone())),
        Arc::new(FixedAccount::new(BOT)),
        "teenydroid",
    ));
    // The webhook saw 900 first.
    intake
        .deliver(&mention("900"), Source::Webhook)
        .await
        .unwrap();

    let read = feed(
        futures::stream::iter(vec![mention("900"), mention("901"), json!({})]),
        Arc::clone(&intake),
    )
    .await;

    assert_eq!(read, 3);
    assert!(queue.get("900").is_some());
    assert!(queue.get("901").is_some());
    assert_eq!(queue.counts().await.unwrap().pending, 2);
}
