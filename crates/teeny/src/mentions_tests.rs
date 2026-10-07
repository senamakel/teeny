use std::sync::Arc;

use teeny_safety::review::Requeue;
use teeny_store::{InMemoryMentionQueue, MentionQueue, MentionStatus};
use teeny_x::intake::MentionSink;
use teeny_x::model::XPost;

use super::*;

#[tokio::test]
async fn caches_posts_through_the_intake_sink_and_counts_only_new_ones() {
    let queue = Arc::new(InMemoryMentionQueue::<QueuedXPost>::default());
    let sink = QueueAdapter(queue.clone());

    let first = sink.cache(vec![XPost::new("900", "hey"), XPost::new("901", "yo")]).await.unwrap();
    let again = sink.cache(vec![XPost::new("900", "hey")]).await.unwrap();

    assert_eq!((first, again), (2, 0));
    let stored = queue.get("900").unwrap();
    assert_eq!(stored.post.0.text, "hey");
    assert_eq!(stored.status, MentionStatus::Pending);
}

#[tokio::test]
async fn stores_the_post_in_the_shape_the_typescript_service_wrote() {
    let post = QueuedXPost(XPost::new("900", "hey"));
    assert_eq!(serde_json::to_value(&post).unwrap(), serde_json::json!({ "id": "900", "text": "hey" }));
}

#[tokio::test]
async fn requeues_a_held_mention_for_the_review() {
    let queue = Arc::new(InMemoryMentionQueue::<QueuedXPost>::default());
    queue.cache(&[QueuedXPost(XPost::new("900", "hey"))]).await.unwrap();
    queue.mark_skipped("900", "held").await.unwrap();
    let adapter = QueueAdapter(queue.clone());

    assert!(adapter.requeue("900").await.unwrap());
    assert!(!adapter.requeue("nope").await.unwrap());
    assert_eq!(queue.get("900").unwrap().filter_approved, Some(true));
}
