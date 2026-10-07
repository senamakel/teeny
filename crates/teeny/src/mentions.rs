//! The mention queue as the service uses it: X posts, in the store's generic
//! queue, plugged into the activity intake and the hold review.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use teeny_safety::review::{Requeue, ReviewError};
use teeny_store::{MentionQueue, QueuedPost};
use teeny_x::intake::{IntakeError, MentionSink};
use teeny_x::model::XPost;

/// An X post as the queue stores it: whole, under `post`, the shape the
/// TypeScript service wrote. A newtype because the store is generic over its
/// post and does not depend on the X client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueuedXPost(pub XPost);

impl QueuedPost for QueuedXPost {
    fn post_id(&self) -> &str {
        self.0.id.as_str()
    }
}

/// The queue the service runs on.
pub type Mentions = dyn MentionQueue<QueuedXPost>;

/// The queue as the intake's sink and the review's requeue.
#[derive(Clone)]
pub struct QueueAdapter(pub Arc<Mentions>);

#[async_trait]
impl MentionSink for QueueAdapter {
    async fn cache(&self, mentions: Vec<XPost>) -> Result<usize, IntakeError> {
        let posts: Vec<QueuedXPost> = mentions.into_iter().map(QueuedXPost).collect();
        let new = self.0.cache(&posts).await?;
        Ok(usize::try_from(new).unwrap_or(usize::MAX))
    }
}

#[async_trait]
impl Requeue for QueueAdapter {
    async fn requeue(&self, post_id: &str) -> Result<bool, ReviewError> {
        Ok(self.0.requeue_approved(post_id).await?)
    }
}

#[cfg(test)]
#[path = "mentions_tests.rs"]
mod tests;
