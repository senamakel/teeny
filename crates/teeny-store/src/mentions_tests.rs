use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Post {
    id: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author_id: Option<String>,
}

impl QueuedPost for Post {
    fn post_id(&self) -> &str {
        &self.id
    }
}

fn post(id: &str) -> Post {
    Post {
        id: id.into(),
        text: format!("post {id}"),
        author_id: Some("u1".into()),
    }
}

fn reply(text: &str) -> Reply {
    Reply {
        text: text.into(),
        ..Reply::default()
    }
}

/// The queue's contract, run against every implementation.
async fn contract(queue: &dyn MentionQueue<Post>) {
    // Caching is upsert-if-absent: a re-read from the overlapping search
    // window never resets work, and only new posts count.
    assert_eq!(queue.cache(&[]).await.unwrap(), 0);
    assert_eq!(queue.cache(&[post("1"), post("2")]).await.unwrap(), 2);
    queue.mark_replied("1", &reply("gm")).await.unwrap();
    assert_eq!(
        queue
            .cache(&[post("1"), post("2"), post("3")])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        queue.counts().await.unwrap(),
        StatusCounts {
            pending: 2,
            replied: 1,
            failed: 0,
            skipped: 0
        }
    );

    // Oldest first, pending only, up to the limit.
    let pending = queue.take_pending(1).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].post_id, "2");
    assert_eq!(pending[0].post, post("2"));
    assert_eq!(pending[0].status, MentionStatus::Pending);
    assert_eq!(pending[0].attempts, 0);
    assert_eq!(pending[0].cached_at, pending[0].updated_at);
    assert_eq!(
        queue
            .take_pending(10)
            .await
            .unwrap()
            .iter()
            .map(|m| m.post_id.as_str())
            .collect::<Vec<_>>(),
        ["2", "3"]
    );

    // A failure counts as an attempt and keeps the first 500 characters.
    queue.mark_failed("2", &"e".repeat(600)).await.unwrap();
    // A skip is not a failure and not an attempt.
    queue.mark_skipped("3", "over budget").await.unwrap();
    queue.mark_liked("3").await.unwrap();
    assert!(queue.take_pending(10).await.unwrap().is_empty());
    assert_eq!(
        queue.counts().await.unwrap(),
        StatusCounts {
            pending: 0,
            replied: 1,
            failed: 1,
            skipped: 1
        }
    );

    // An approved hold goes back to pending with the content check waived
    // and the error cleared; an unknown one is refused.
    assert!(queue.requeue_approved("2").await.unwrap());
    assert!(!queue.requeue_approved("404").await.unwrap());
    let [requeued] = queue.take_pending(10).await.unwrap().try_into().unwrap();
    assert_eq!(requeued.post_id, "2");
    assert_eq!(requeued.attempts, 1);
    assert_eq!(requeued.filter_approved, Some(true));
    assert_eq!(requeued.error, None);

    // Replies count by lane, by own post, and since a time.
    let before = crate::iso_now();
    queue
        .mark_replied(
            "2",
            &Reply {
                id: Some("r2".into()),
                text: "hi".into(),
                lane: Some(ReplyLane::Auto),
                own_post_id: Some("own1".into()),
            },
        )
        .await
        .unwrap();
    queue.cache(&[post("4")]).await.unwrap();
    queue
        .mark_replied(
            "4",
            &Reply {
                lane: Some(ReplyLane::Smart),
                ..reply("yo")
            },
        )
        .await
        .unwrap();
    let count = |lane, own_post_id: Option<&str>, since: Option<&str>| {
        let filter = RepliedFilter {
            lane,
            own_post_id: own_post_id.map(Into::into),
            since: since.map(Into::into),
        };
        async move { queue.count_replied(&filter).await.unwrap() }
    };
    assert_eq!(count(ReplyLane::Auto, None, None).await, 1);
    assert_eq!(count(ReplyLane::Auto, Some("own1"), None).await, 1);
    assert_eq!(count(ReplyLane::Auto, Some("own2"), None).await, 0);
    assert_eq!(count(ReplyLane::Smart, None, Some(&before)).await, 1);
    assert_eq!(
        count(ReplyLane::Smart, None, Some("9999-01-01T00:00:00.000Z")).await,
        0
    );
    assert_eq!(count(ReplyLane::Called, None, None).await, 0);
}

#[tokio::test]
async fn the_in_memory_queue_keeps_the_contract() {
    contract(&InMemoryMentionQueue::default()).await;
}

#[tokio::test]
async fn the_in_memory_queue_records_what_happened_to_each_mention() {
    let queue = InMemoryMentionQueue::default();
    queue.cache(&[post("1"), post("2")]).await.unwrap();
    queue
        .mark_replied(
            "1",
            &Reply {
                id: Some("99".into()),
                text: "gm".into(),
                lane: Some(ReplyLane::Called),
                own_post_id: None,
            },
        )
        .await
        .unwrap();
    queue.mark_failed("2", &"é".repeat(600)).await.unwrap();
    queue.mark_liked("2").await.unwrap();

    let replied = queue.get("1").unwrap();
    assert_eq!(replied.status, MentionStatus::Replied);
    assert_eq!(replied.attempts, 1);
    assert_eq!(replied.reply_id.as_deref(), Some("99"));
    assert_eq!(replied.reply_text.as_deref(), Some("gm"));
    assert_eq!(replied.lane, Some(ReplyLane::Called));
    assert_eq!(replied.own_post_id, None);

    let failed = queue.get("2").unwrap();
    assert_eq!(
        failed.error.as_deref().map(|e| e.chars().count()),
        Some(500)
    );
    assert!(failed.liked_at.is_some());
}

#[test]
fn a_queued_mention_is_spelled_as_the_typescript_service_stored_it() {
    let mention = QueuedMention {
        post_id: "1".into(),
        post: post("1"),
        status: MentionStatus::Replied,
        attempts: 1,
        cached_at: "2026-09-13T01:00:00.000Z".into(),
        updated_at: "2026-09-13T02:00:00.000Z".into(),
        reply_id: Some("9".into()),
        reply_text: Some("gm".into()),
        error: None,
        filter_approved: Some(true),
        lane: Some(ReplyLane::Auto),
        own_post_id: Some("own".into()),
        liked_at: None,
    };
    assert_eq!(
        serde_json::to_value(&mention).unwrap(),
        serde_json::json!({
            "postId": "1",
            "post": { "id": "1", "text": "post 1", "authorId": "u1" },
            "status": "replied",
            "attempts": 1,
            "cachedAt": "2026-09-13T01:00:00.000Z",
            "updatedAt": "2026-09-13T02:00:00.000Z",
            "replyId": "9",
            "replyText": "gm",
            "filterApproved": true,
            "lane": "auto",
            "ownPostId": "own",
        })
    );
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_queue_keeps_the_contract() {
    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("mentions");
    contract(&MongoMentionQueue::<Post>::with_collection(&db, &name)).await;
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
}
