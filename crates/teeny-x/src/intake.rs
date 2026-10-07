//! One delivery of the account's activity, whichever way it arrived, ported
//! from `legacy/src/webhooks/intake.ts`.
//!
//! X has two transports for the same subscription: the webhook, and the
//! persistent `GET /2/activity/stream`. Since 2026-08 the webhook carries only
//! the free events (likes, follows) and drops every `post.*` event, while the
//! stream carries all of them, so the process reads both and everything comes
//! through here: identity first, the people record, the mention queue, then
//! memory.
//!
//! The same like arrives on both transports and the people record counts
//! what it is given, so deliveries are deduplicated by X's event id first.
//! The set is bounded and in-process, which is enough: both transports run in
//! one process, and a duplicate that slips past a restart costs a like on
//! somebody's score, not a second reply.
//!
//! The queue, the people record and memory live in other crates; they plug
//! in through the small traits below.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::future::join_all;
use serde_json::Value;
use teeny_core::types::UserId;

use crate::activity::{
    ActivityEntry, ActivityInteraction, activity_event, capturable_entries,
    interactions_from_activity, mentions_from_activity,
};
use crate::client::XApi;
use crate::model::XPost;

/// Whatever a sink fails with.
pub type IntakeError = Box<dyn std::error::Error + Send + Sync>;

/// The mention queue: caches the posts, answers how many were new.
#[async_trait]
pub trait MentionSink: Send + Sync {
    async fn cache(&self, mentions: Vec<XPost>) -> Result<usize, IntakeError>;
}

/// The people record.
#[async_trait]
pub trait PeopleSink: Send + Sync {
    async fn observe(&self, interaction: ActivityInteraction) -> Result<(), IntakeError>;
}

/// Long-term memory, which keeps the non-post activity.
#[async_trait]
pub trait ActivityMemory: Send + Sync {
    async fn capture_activity(&self, entry: ActivityEntry) -> Result<(), IntakeError>;
}

/// The account's own id, so its own posts are never queued.
#[async_trait]
pub trait AccountId: Send + Sync {
    async fn bot_user_id(&self) -> Result<Option<UserId>, IntakeError>;
}

#[async_trait]
impl AccountId for XApi {
    async fn bot_user_id(&self) -> Result<Option<UserId>, IntakeError> {
        Ok(Some(self.user_id().await?))
    }
}

/// Which transport a delivery came on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Webhook,
    Stream,
}

/// What became of a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivered {
    /// Mentions the queue took as new.
    pub queued: usize,
    /// Already handled on the other transport; nothing was done.
    pub duplicate: bool,
}

const DEFAULT_REMEMBER: usize = 10_000;

/// A bounded set of event keys, oldest out first.
struct Seen {
    order: VecDeque<String>,
    keys: HashSet<String>,
    limit: usize,
}

impl Seen {
    /// Whether `key` was already seen; remembers it if not.
    fn check(&mut self, key: String) -> bool {
        if self.keys.contains(&key) {
            return true;
        }
        self.keys.insert(key.clone());
        self.order.push_back(key);
        while self.keys.len() > self.limit {
            match self.order.pop_front() {
                Some(oldest) => self.keys.remove(&oldest),
                None => break,
            };
        }
        false
    }
}

struct Queueing {
    queue: Arc<dyn MentionSink>,
    account: Arc<dyn AccountId>,
    bot_username: String,
}

/// Where both transports hand their deliveries.
pub struct ActivityIntake {
    memory: Arc<dyn ActivityMemory>,
    queueing: Option<Queueing>,
    people: Option<Arc<dyn PeopleSink>>,
    seen: Mutex<Seen>,
}

impl ActivityIntake {
    /// An intake that only captures activity in memory.
    pub fn new(memory: Arc<dyn ActivityMemory>) -> Self {
        Self {
            memory,
            queueing: None,
            people: None,
            seen: Mutex::new(Seen {
                order: VecDeque::new(),
                keys: HashSet::new(),
                limit: DEFAULT_REMEMBER,
            }),
        }
    }

    /// Queues mentions, as `bot_username`, once `account` says who that is.
    pub fn with_queue(
        mut self,
        queue: Arc<dyn MentionSink>,
        account: Arc<dyn AccountId>,
        bot_username: &str,
    ) -> Self {
        self.queueing = Some(Queueing {
            queue,
            account,
            bot_username: bot_username.to_owned(),
        });
        self
    }

    /// Records who interacted with the account. Only runs with a queue,
    /// because it needs the account's id.
    pub fn with_people(mut self, people: Arc<dyn PeopleSink>) -> Self {
        self.people = Some(people);
        self
    }

    /// How many event ids to remember for deduplication; 10 000 by default.
    pub fn remembering(self, limit: usize) -> Self {
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .limit = limit.max(1);
        self
    }

    /// Only the Activity API envelope carries an event id; the old shape is
    /// only ever spoken by the webhook, so it is never a duplicate.
    fn duplicate(&self, delivery: &Value) -> bool {
        let Some(event) = activity_event(delivery) else {
            return false;
        };
        let Some(event_id) = event.event_id else {
            return false;
        };
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .check(format!("{}:{event_id}", event.event_type))
    }

    /// Handles one delivery. Returns once the queue has it; memory is not
    /// waited for, because neither transport can afford to: X drops a webhook
    /// it does not see acknowledged in seconds, and a stream reader stalled on
    /// one event backs up every event behind it.
    pub async fn deliver(
        &self,
        delivery: &Value,
        source: Source,
    ) -> Result<Delivered, IntakeError> {
        if self.duplicate(delivery) {
            tracing::debug!(
                ?source,
                "dropped a delivery already handled on the other transport"
            );
            return Ok(Delivered {
                queued: 0,
                duplicate: true,
            });
        }

        let mut queued = 0;
        if let Some(queueing) = &self.queueing {
            // Identity first, and nothing is queued without it: a delivery is
            // mostly the account's own activity, and queuing that is how a
            // self-reply loop starts. The cron poll catches what is dropped.
            let bot_user_id = match queueing.account.bot_user_id().await {
                Ok(Some(id)) if !id.as_str().is_empty() => id,
                Ok(_) => {
                    tracing::error!(
                        ?source,
                        "no account id available; refusing to queue activity"
                    );
                    return Ok(Delivered {
                        queued: 0,
                        duplicate: false,
                    });
                }
                Err(error) => {
                    tracing::error!(?source, %error, "could not identify the account; queued nothing");
                    return Ok(Delivered {
                        queued: 0,
                        duplicate: false,
                    });
                }
            };
            if let Some(people) = &self.people {
                join_all(interactions_from_activity(delivery, &bot_user_id).into_iter().map(
                    |interaction| async move {
                        if let Err(error) = people.observe(interaction.clone()).await {
                            tracing::warn!(?source, ?interaction, %error, "could not record the interaction");
                        }
                    },
                ))
                .await;
            }
            let mentions = mentions_from_activity(delivery, &bot_user_id, &queueing.bot_username);
            if !mentions.is_empty() {
                let delivered = mentions.len();
                queued = queueing.queue.cache(mentions).await?;
                tracing::info!(
                    ?source,
                    delivered,
                    cached = queued,
                    "queued mentions from activity"
                );
            }
        }

        let entries = capturable_entries(delivery);
        if !entries.is_empty() {
            let memory = self.memory.clone();
            tokio::spawn(async move {
                for result in join_all(
                    entries
                        .into_iter()
                        .map(|entry| memory.capture_activity(entry)),
                )
                .await
                {
                    if let Err(error) = result {
                        tracing::warn!(?source, %error, "could not capture the activity in memory");
                    }
                }
            });
        }
        Ok(Delivered {
            queued,
            duplicate: false,
        })
    }
}

#[cfg(test)]
#[path = "intake_tests.rs"]
mod tests;
