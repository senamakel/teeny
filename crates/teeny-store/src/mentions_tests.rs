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

    fn author_id(&self) -> Option<&str> {
        self.author_id.as_deref()
    }

    fn text(&self) -> &str {
        &self.text
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

/// `unsay`'s half: a mention whose reply was deleted goes back to pending
/// with its attempts, error and reply cleared, and its status is readable.
async fn requeue_contract(queue: &dyn MentionQueue<Post>) {
    assert_eq!(queue.status_of("9").await.unwrap(), None);
    assert!(!queue.requeue_fresh("9").await.unwrap());

    queue.cache(&[post("9")]).await.unwrap();
    queue.mark_failed("9", "boom").await.unwrap();
    queue
        .mark_replied(
            "9",
            &Reply {
                id: Some("r9".into()),
                ..reply("rude")
            },
        )
        .await
        .unwrap();
    assert_eq!(
        queue.status_of("9").await.unwrap(),
        Some(MentionStatus::Replied)
    );

    assert!(queue.requeue_fresh("9").await.unwrap());

    assert_eq!(
        queue.status_of("9").await.unwrap(),
        Some(MentionStatus::Pending)
    );
    let row = queue.take_pending(10).await.unwrap().remove(0);
    assert_eq!(row.attempts, 0);
    assert_eq!(row.error, None);
    assert_eq!(row.reply_id, None);
    assert_eq!(row.reply_text, None);
}

/// The claim, on a queue with the default lease: a mention is drafted by
/// one consumer at a time.
async fn claim_contract(queue: &dyn MentionQueue<Post>) {
    queue.cache(&[post("1"), post("2")]).await.unwrap();
    let (a, b) = tokio::join!(queue.claim("1"), queue.claim("1"));
    let claimed: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(claimed.len(), 1, "two consumers, one claim");
    assert_eq!(claimed[0].post_id, "1");
    assert_eq!(claimed[0].status, MentionStatus::Replying);
    assert!(claimed[0].claimed_at.is_some());
    assert_eq!(claimed[0].post, post("1"));

    // A live claim is nobody else's work.
    let pending = queue.take_pending(10).await.unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|m| m.post_id.as_str())
            .collect::<Vec<_>>(),
        ["2"]
    );
    assert!(queue.claim("1").await.unwrap().is_none());
    assert!(queue.claim("404").await.unwrap().is_none());
    assert_eq!(queue.counts().await.unwrap().replying, 1);

    // Done is done.
    queue.mark_replied("1", &reply("gm")).await.unwrap();
    assert!(queue.claim("1").await.unwrap().is_none());
    queue.mark_skipped("2", "over budget").await.unwrap();
    assert!(queue.claim("2").await.unwrap().is_none());
}

/// The claim, on a queue whose lease runs out at once: a consumer that died
/// mid-draft gives its mention back, one that died after posting does not.
async fn stale_claim_contract(queue: &dyn MentionQueue<Post>) {
    queue.cache(&[post("1"), post("2")]).await.unwrap();
    queue.claim("1").await.unwrap().unwrap();
    queue.claim("2").await.unwrap().unwrap();
    // The reply to "2" went out; the process died before the rest.
    queue.record_reply_id("2", "r2").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let pending = queue.take_pending(10).await.unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|m| m.post_id.as_str())
            .collect::<Vec<_>>(),
        ["1"],
        "an unposted stale claim is work again"
    );
    assert!(queue.claim("1").await.unwrap().is_some());
    assert!(
        queue.claim("2").await.unwrap().is_none(),
        "never answered twice"
    );
    assert_eq!(
        queue.status_of("2").await.unwrap(),
        Some(MentionStatus::Replying)
    );

    queue.mark_replied("2", &reply("gm")).await.unwrap();
    assert_eq!(
        queue.status_of("2").await.unwrap(),
        Some(MentionStatus::Replied)
    );

    // Requeued fresh, the claim and the reply are gone with it.
    assert!(queue.requeue_fresh("2").await.unwrap());
    let row = queue.claim("2").await.unwrap().unwrap();
    assert_eq!(row.reply_id, None);
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
            skipped: 0,
            replying: 0
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
            skipped: 1,
            replying: 0
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

/// The exact record of what was said to whom: the last answered mentions
/// from one author, newest first, the one being answered left out.
async fn exchange_contract<Q: MentionQueue<Post> + ExchangeHistory>(queue: &Q) {
    let by = |id: &str, author: &str| Post {
        id: id.into(),
        text: format!("question {id}"),
        author_id: Some(author.into()),
    };
    queue
        .cache(&[
            by("1", "7"),
            by("2", "7"),
            by("3", "8"),
            by("4", "7"),
            by("5", "7"),
        ])
        .await
        .unwrap();
    for id in ["1", "2", "3", "5"] {
        // A reply's time is when it was marked, so the order is the marking's.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        queue
            .mark_replied(id, &reply(&format!("answer {id}")))
            .await
            .unwrap();
    }
    queue.mark_failed("4", "boom").await.unwrap();

    let exchanges = queue.exchanges_with("7", 6, None).await.unwrap();
    let pairs: Vec<(&str, &str)> = exchanges
        .iter()
        .map(|exchange| (exchange.incoming.as_str(), exchange.reply.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("question 5", "answer 5"),
            ("question 2", "answer 2"),
            ("question 1", "answer 1"),
        ]
    );
    assert!(!exchanges[0].at.is_empty());
    assert_eq!(
        queue.exchanges_with("7", 1, Some("5")).await.unwrap()[0].incoming,
        "question 2"
    );
    assert!(queue.exchanges_with("7", 0, None).await.unwrap().is_empty());
    assert!(queue.exchanges_with("9", 6, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_in_memory_queue_keeps_the_contract() {
    exchange_contract(&InMemoryMentionQueue::default()).await;
    contract(&InMemoryMentionQueue::default()).await;
    requeue_contract(&InMemoryMentionQueue::default()).await;
    claim_contract(&InMemoryMentionQueue::default()).await;
    stale_claim_contract(
        &InMemoryMentionQueue::default().with_claim_lease(std::time::Duration::ZERO),
    )
    .await;
}

#[test]
fn a_claim_lasts_fifteen_minutes_unless_told_otherwise() {
    assert_eq!(DEFAULT_CLAIM_LEASE, std::time::Duration::from_secs(15 * 60));
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
        claimed_at: None,
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
    let fresh = crate::mongo::testing::collection("mentions");
    requeue_contract(&MongoMentionQueue::<Post>::with_collection(&db, &fresh)).await;
    let claims = crate::mongo::testing::collection("mentions");
    claim_contract(&MongoMentionQueue::<Post>::with_collection(&db, &claims)).await;
    let exchanges = crate::mongo::testing::collection("mentions");
    exchange_contract(&MongoMentionQueue::<Post>::with_collection(&db, &exchanges)).await;
    let stale = crate::mongo::testing::collection("mentions");
    stale_claim_contract(
        &MongoMentionQueue::<Post>::with_collection(&db, &stale)
            .with_claim_lease(std::time::Duration::ZERO),
    )
    .await;
    for name in [&fresh, &claims, &exchanges, &stale] {
        db.collection::<mongodb::bson::Document>(name)
            .drop()
            .await
            .unwrap();
    }
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
}
