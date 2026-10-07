//! Who interacts with the account, and how much.
//!
//! The reply budget is small on purpose, so who gets one of the answers
//! under a post matters. Every like, follow, reply and quote that reaches the
//! webhook adds to the person's score, and the reply job answers the highest
//! scores first. The score is a sum rather than a rate, so a regular who has
//! been around for a month outranks a stranger who liked three things this
//! morning, which is the intended bias.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::mongo::{Indexes, index, unique};
use crate::support::iso_now;

/// The default collection.
pub const COLLECTION: &str = "people";

/// What somebody did that involved the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interaction {
    Mention,
    Reply,
    Quote,
    Repost,
    Like,
    Follow,
    /// The account answered them.
    Answered,
}

impl Interaction {
    /// How much it is worth when the account decides whom to answer first.
    /// Writing something is worth more than a tap; a quote, which puts the
    /// account in front of the writer's followers, most of all. Being
    /// answered counts a little: the people the account talks with are the
    /// people it should keep talking with.
    pub fn weight(self) -> i64 {
        match self {
            Self::Quote => 4,
            Self::Reply | Self::Mention => 3,
            Self::Repost | Self::Follow => 2,
            Self::Like | Self::Answered => 1,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::Reply => "reply",
            Self::Quote => "quote",
            Self::Repost => "repost",
            Self::Like => "like",
            Self::Follow => "follow",
            Self::Answered => "answered",
        }
    }
}

/// One person, as the log knows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonRecord {
    /// The X user id.
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(deserialize_with = "crate::support::lenient::i64")]
    pub score: i64,
    #[serde(default, deserialize_with = "crate::support::lenient::map")]
    pub counts: BTreeMap<Interaction, i64>,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

/// Something somebody did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub user_id: String,
    pub username: Option<String>,
    pub kind: Interaction,
    /// When, ISO; now when absent.
    pub at: Option<String>,
}

/// The people log.
#[async_trait]
pub trait People: Send + Sync {
    async fn observe(&self, event: &Observation) -> Result<()>;
    /// Scores for the ids given; an unknown person scores 0.
    async fn scores(&self, user_ids: &[String]) -> Result<HashMap<String, i64>>;
    /// The highest scores first.
    async fn top(&self, limit: usize) -> Result<Vec<PersonRecord>>;
}

/// The log in process, for tests and dry runs.
#[derive(Debug, Default)]
pub struct InMemoryPeople {
    people: Mutex<Vec<PersonRecord>>,
}

#[async_trait]
impl People for InMemoryPeople {
    async fn observe(&self, event: &Observation) -> Result<()> {
        let at = event.at.clone().unwrap_or_else(iso_now);
        let mut people = self
            .people
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let index = match people
            .iter()
            .position(|person| person.user_id == event.user_id)
        {
            Some(index) => index,
            None => {
                people.push(PersonRecord {
                    user_id: event.user_id.clone(),
                    username: None,
                    score: 0,
                    counts: BTreeMap::new(),
                    first_seen_at: at.clone(),
                    last_seen_at: at.clone(),
                });
                people.len() - 1
            }
        };
        let person = &mut people[index];
        person.score += event.kind.weight();
        *person.counts.entry(event.kind).or_default() += 1;
        person.last_seen_at = at;
        if let Some(username) = &event.username {
            person.username = Some(username.clone());
        }
        Ok(())
    }

    async fn scores(&self, user_ids: &[String]) -> Result<HashMap<String, i64>> {
        let people = self
            .people
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Ok(user_ids
            .iter()
            .map(|id| {
                let score = people
                    .iter()
                    .find(|person| person.user_id == *id)
                    .map_or(0, |person| person.score);
                (id.clone(), score)
            })
            .collect())
    }

    async fn top(&self, limit: usize) -> Result<Vec<PersonRecord>> {
        let mut people = self
            .people
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        people.sort_by_key(|person| std::cmp::Reverse(person.score));
        people.truncate(limit);
        Ok(people)
    }
}

/// The log in Mongo.
pub struct MongoPeople {
    people: Collection<Document>,
    indexes: Indexes,
}

impl MongoPeople {
    pub fn new(db: &Database) -> Self {
        Self::with_collection(db, COLLECTION)
    }

    pub fn with_collection(db: &Database, name: &str) -> Self {
        Self {
            people: db.collection(name),
            indexes: Indexes::default(),
        }
    }
}

impl MongoPeople {
    async fn ready(&self) -> Result<()> {
        self.indexes
            .ensure(|| async {
                self.people
                    .create_indexes([unique(doc! { "userId": 1 }), index(doc! { "score": -1 })])
                    .await?;
                Ok(())
            })
            .await
    }
}

#[async_trait]
impl People for MongoPeople {
    async fn observe(&self, event: &Observation) -> Result<()> {
        self.ready().await?;
        let at = event.at.clone().unwrap_or_else(iso_now);
        let mut set = doc! { "lastSeenAt": &at };
        if let Some(username) = &event.username {
            set.insert("username", username);
        }
        self.people
            .update_one(
                doc! { "userId": &event.user_id },
                doc! {
                    "$inc": { "score": event.kind.weight(), format!("counts.{}", event.kind.as_str()): 1 },
                    "$set": set,
                    "$setOnInsert": { "userId": &event.user_id, "firstSeenAt": &at },
                },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    async fn scores(&self, user_ids: &[String]) -> Result<HashMap<String, i64>> {
        if user_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.ready().await?;
        // Everyone asked about is in the answer; somebody never seen scores 0.
        let mut scores: HashMap<String, i64> = user_ids.iter().map(|id| (id.clone(), 0)).collect();
        let mut rows = self
            .people
            .clone_with_type::<PersonRecord>()
            .find(doc! { "userId": { "$in": user_ids } })
            .await?;
        while let Some(row) = rows.try_next().await? {
            scores.insert(row.user_id, row.score);
        }
        Ok(scores)
    }

    async fn top(&self, limit: usize) -> Result<Vec<PersonRecord>> {
        self.ready().await?;
        let people = self
            .people
            .clone_with_type::<PersonRecord>()
            .find(doc! {})
            .sort(doc! { "score": -1 })
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .await?
            .try_collect()
            .await?;
        Ok(people)
    }
}

#[cfg(test)]
#[path = "people_tests.rs"]
mod tests;
