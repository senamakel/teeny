//! What a delivery of the account's activity says, ported from
//! `legacy/src/webhooks/activity.ts`.
//!
//! Two shapes arrive, because X has two webhook products and the account
//! moved from one to the other on 2026-09-14:
//!
//! - Account Activity (old, deprecated): v1.1 tweets under
//!   `tweet_create_events`, with `id_str`, the author under `user`, and the
//!   text in `extended_tweet.full_text` once past 140 characters.
//! - The X Activity API: one event per delivery as
//!   `{ data: { event_type, event_uuid, filter, tag, payload, includes } }`,
//!   where a `post.*` payload is a v2 post and the author's handle is in
//!   `includes.users`, when it is anywhere. The activity stream speaks this
//!   shape too.
//!
//! Everything here is untrusted input: absent or mistyped fields are skipped,
//! never an error.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use teeny_core::types::UserId;

use crate::mention::{is_retweet_text, is_worth_answering};
use crate::model::XPost;

/// The Activity API events whose payload is somebody's post about the account.
const POST_EVENTS: [&str; 3] = [
    "post.mention.create",
    "post.reply.create",
    "post.quote.create",
];

/// An X Activity API envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEvent {
    pub event_type: String,
    /// `event_uuid`: the same event on both transports carries the same one.
    pub event_id: Option<String>,
    pub payload: Map<String, Value>,
    /// `includes.users`, as `(id, username)`.
    pub users: Vec<(String, Option<String>)>,
}

impl ActivityEvent {
    fn username_of(&self, user_id: &str) -> Option<String> {
        self.users
            .iter()
            .find(|(id, _)| id == user_id)
            .and_then(|(_, username)| username.clone())
    }
}

/// One thing that happened, for memory: its kind (an event type, or a v1.1
/// `*_events` key), an id, and the raw payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub kind: String,
    pub id: String,
    pub payload: Value,
}

/// How somebody engaged with the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionKind {
    Mention,
    Reply,
    Quote,
    Repost,
    Follow,
    Like,
}

/// Who did what to the account, for the people record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityInteraction {
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub kind: InteractionKind,
}

fn str_field<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    object.get(name).and_then(Value::as_str)
}

/// `new Date(x).toISOString()`: UTC, milliseconds, `Z`. Reads RFC 3339 and
/// v1.1's `Wed Sep 02 13:30:16 +0000 2026`; anything else is dropped.
fn iso_time(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw)
        .or_else(|_| DateTime::parse_from_str(raw, "%a %b %d %H:%M:%S %z %Y"))
        .ok()
        .map(|time| {
            time.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        })
}

/// The Activity API envelope, when the delivery is one.
pub fn activity_event(delivery: &Value) -> Option<ActivityEvent> {
    let envelope = delivery.get("data")?.as_object()?;
    let event_type = str_field(envelope, "event_type")?.to_owned();
    let payload = envelope
        .get("payload")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let users = envelope
        .get("includes")
        .and_then(|includes| includes.get("users"))
        .and_then(Value::as_array)
        .map(|users| {
            users
                .iter()
                .filter_map(Value::as_object)
                .filter_map(|user| {
                    Some((
                        str_field(user, "id")?.to_owned(),
                        str_field(user, "username").map(str::to_owned),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ActivityEvent {
        event_type,
        event_id: str_field(envelope, "event_uuid").map(str::to_owned),
        payload,
        users,
    })
}

fn from_v2_post(event: &ActivityEvent) -> Option<XPost> {
    let post = &event.payload;
    let id = str_field(post, "id")?;
    let text = str_field(post, "text")?;
    if id.is_empty() || text.is_empty() {
        return None;
    }
    let author_id = str_field(post, "author_id");
    Some(XPost {
        author_id: author_id.map(UserId::from),
        author_username: author_id.and_then(|author| event.username_of(author)),
        created_at: str_field(post, "created_at").and_then(iso_time),
        // The conversation tells a comment under the account's own post from
        // a reply out in somebody else's thread.
        conversation_id: str_field(post, "conversation_id").map(Into::into),
        in_reply_to_user_id: str_field(post, "in_reply_to_user_id").map(Into::into),
        ..XPost::new(id, text)
    })
}

fn from_v1_tweet(tweet: &Map<String, Value>) -> Option<XPost> {
    let text = tweet
        .get("extended_tweet")
        .and_then(|extended| extended.get("full_text"))
        .and_then(Value::as_str)
        .or_else(|| str_field(tweet, "text"))?;
    let id = str_field(tweet, "id_str")?;
    if id.is_empty() || text.is_empty() {
        return None;
    }
    let user = tweet.get("user").and_then(Value::as_object);
    Some(XPost {
        author_id: user
            .and_then(|user| str_field(user, "id_str"))
            .map(Into::into),
        author_username: user
            .and_then(|user| str_field(user, "screen_name"))
            .map(str::to_owned),
        created_at: str_field(tweet, "created_at").and_then(iso_time),
        in_reply_to_user_id: str_field(tweet, "in_reply_to_user_id_str")
            .filter(|parent| !parent.is_empty())
            .map(Into::into),
        ..XPost::new(id, text)
    })
}

/// The mentions in a delivery worth answering.
///
/// The account's own posts arrive too (the subscription is to the account),
/// and replying to itself is the one loop this must never enter, so without
/// the account's id nothing is a mention. A retweet of the account reads as
/// a mention and is dropped: X attaches a reply to the original.
pub fn mentions_from_activity(
    delivery: &Value,
    bot_user_id: &UserId,
    bot_username: &str,
) -> Vec<XPost> {
    if bot_user_id.as_str().is_empty() {
        return Vec::new();
    }
    let keep = |post: &XPost| {
        !is_retweet_text(&post.text)
            && post.author_id.as_ref() != Some(bot_user_id)
            && is_worth_answering(post, Some(bot_user_id), bot_username)
    };

    if let Some(event) = activity_event(delivery) {
        if !POST_EVENTS.contains(&event.event_type.as_str()) {
            return Vec::new();
        }
        return from_v2_post(&event).filter(keep).into_iter().collect();
    }

    let Some(tweets) = delivery
        .get("tweet_create_events")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    tweets
        .iter()
        .filter_map(Value::as_object)
        .filter(|tweet| tweet.get("retweeted_status").is_none_or(Value::is_null))
        .filter_map(from_v1_tweet)
        .filter(keep)
        .collect()
}

/// Who did what to the account. Only the Activity API envelope says enough:
/// a post event names its author, a follow its source, a like is credited to
/// whoever `includes` names that is not the account.
pub fn interactions_from_activity(
    delivery: &Value,
    bot_user_id: &UserId,
) -> Vec<ActivityInteraction> {
    let Some(event) = activity_event(delivery) else {
        return Vec::new();
    };
    let named = |user_id: Option<&str>, kind: InteractionKind| -> Vec<ActivityInteraction> {
        match user_id {
            Some(id) if !id.is_empty() && id != bot_user_id.as_str() => vec![ActivityInteraction {
                user_id: id.into(),
                username: event.username_of(id),
                kind,
            }],
            _ => Vec::new(),
        }
    };
    let author = str_field(&event.payload, "author_id");
    match event.event_type.as_str() {
        "post.mention.create" => named(author, InteractionKind::Mention),
        "post.reply.create" => named(author, InteractionKind::Reply),
        "post.quote.create" => named(author, InteractionKind::Quote),
        "post.repost.create" => named(author, InteractionKind::Repost),
        "follow.follow" => named(
            event
                .payload
                .get("source")
                .and_then(Value::as_object)
                .and_then(|source| str_field(source, "id")),
            InteractionKind::Follow,
        ),
        "like.create" => named(
            event
                .users
                .iter()
                .map(|(id, _)| id.as_str())
                .find(|id| !id.is_empty() && *id != bot_user_id.as_str()),
            InteractionKind::Like,
        ),
        _ => Vec::new(),
    }
}

fn id_of(value: &Value, fallback: usize) -> String {
    let field = |name: &str| match value.get(name) {
        Some(Value::String(id)) => Some(id.clone()),
        Some(Value::Number(id)) => Some(id.to_string()),
        _ => None,
    };
    field("id")
        .or_else(|| field("id_str"))
        .unwrap_or_else(|| fallback.to_string())
}

/// Every entry in a delivery: the one event of an envelope, or each item of
/// every v1.1 `*_events` list.
pub fn activity_entries(delivery: &Value) -> Vec<ActivityEntry> {
    if let Some(event) = activity_event(delivery) {
        let id = str_field(&event.payload, "id")
            .map(str::to_owned)
            .or(event.event_id)
            .unwrap_or_else(|| "0".to_owned());
        return vec![ActivityEntry {
            kind: event.event_type,
            id,
            payload: Value::Object(event.payload),
        }];
    }
    let Some(object) = delivery.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter(|(kind, _)| kind.ends_with("_events"))
        .filter_map(|(kind, value)| Some((kind, value.as_array()?)))
        .flat_map(|(kind, events)| {
            events
                .iter()
                .enumerate()
                .map(move |(index, event)| ActivityEntry {
                    kind: kind.clone(),
                    id: id_of(event, index),
                    payload: event.clone(),
                })
        })
        .collect()
}

/// Likes, reposts, follows, blocks, mutes and deletions: counted, not
/// remembered. `like.create`, `post.repost.create`, `follow.follow` and the
/// old shape's `favorite_events`, `follow_events` and the like.
pub fn is_engagement(kind: &str) -> bool {
    const PREFIXES: [&str; 10] = [
        "like",
        "favorite",
        "post.repost",
        "repost",
        "retweet",
        "follow",
        "unfollow",
        "block",
        "mute",
        "tweet_delete",
    ];
    const OLD_SHAPE: [&str; 6] = [
        "favorite_events",
        "follow_events",
        "block_events",
        "mute_events",
        "tweet_delete_events",
        "retweet_events",
    ];
    OLD_SHAPE.contains(&kind)
        || PREFIXES.iter().any(|prefix| {
            kind.strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(['.', '_']))
        })
}

/// The entries memory captures: neither posts nor engagement. Posts reach
/// memory through the reply harness as a chat turn attributed to their
/// author; capturing the raw delivery too would store the conversation
/// twice. Engagement is counted in the people store, which is what decides
/// who gets answered; stored as raw payloads it was two thirds of memory,
/// and recall returned JSON about who liked what instead of what anybody said.
pub fn capturable_entries(delivery: &Value) -> Vec<ActivityEntry> {
    activity_entries(delivery)
        .into_iter()
        .filter(|entry| {
            entry.kind != "tweet_create_events"
                && !POST_EVENTS.contains(&entry.kind.as_str())
                && !is_engagement(&entry.kind)
        })
        .collect()
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
