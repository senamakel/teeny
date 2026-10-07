//! The true briefing every episode opens with: what time it is in the lab,
//! who is talking about teeny today and what they said, how the last
//! episode landed, where the story left off. No markets. A field that
//! cannot be read is unknown, never guessed. What strangers said is quoted
//! as untrusted.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Timelike, Utc};
use teeny_core::types::PostId;
use teeny_x::XApi;
use teeny_x::cache::XReads;
use teeny_x::model::SearchOptions;

use super::voice::{Field, Fields};
use crate::backrooms::screen::quote_untrusted;
use crate::backrooms::text::{flatten, head};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastEpisode {
    pub number: u32,
    pub title: Option<String>,
    pub state_after: Option<String>,
    pub likes: Option<u64>,
    pub replies: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldState {
    pub today: String,
    pub hour_utc: u32,
    /// The number of the episode being written.
    pub episode: u32,
    pub followers: Option<u64>,
    pub last_episode: Option<LastEpisode>,
    /// What strangers said about teeny today. Untrusted, one line each.
    pub mentions: Vec<String>,
    pub mentions_24h: Option<usize>,
    pub problems: Vec<String>,
}

/// Where the world is read from.
#[async_trait]
pub trait WorldSources: Send + Sync {
    async fn followers(&self) -> anyhow::Result<Option<u64>>;
    async fn mentions(&self) -> anyhow::Result<Vec<String>>;
    /// `(likes, replies)` of the last episode, when it is known.
    async fn last_episode_metrics(&self, post_id: &str) -> anyhow::Result<Option<(u64, u64)>>;
}

/// The account on X: its followers, who mentioned it in the last day, how
/// the last episode did.
pub struct XWorld {
    pub x: Arc<XApi>,
    pub reads: Arc<dyn XReads>,
    pub bot_username: String,
    pub token_symbol: String,
}

#[async_trait]
impl WorldSources for XWorld {
    async fn followers(&self) -> anyhow::Result<Option<u64>> {
        Ok(self.x.identity().await?.followers)
    }

    async fn mentions(&self) -> anyhow::Result<Vec<String>> {
        let bot = &self.bot_username;
        let options = SearchOptions {
            max_results: Some(50),
            start_time: Some(
                (Utc::now() - TimeDelta::hours(24)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ),
            ..SearchOptions::default()
        };
        let posts = self
            .reads
            .search_posts(&format!("(@{bot} OR teeny OR ${}) -is:retweet -from:{bot}", self.token_symbol), &options)
            .await?;
        Ok(posts.into_iter().map(|post| post.text).collect())
    }

    async fn last_episode_metrics(&self, post_id: &str) -> anyhow::Result<Option<(u64, u64)>> {
        let rows = self.reads.post_metrics(&[PostId::from(post_id.to_owned())]).await?;
        Ok(rows.first().map(|row| (row.likes, row.replies)))
    }
}

pub async fn build_world_state(
    sources: &dyn WorldSources,
    episode: u32,
    last: Option<(u32, Option<String>, Option<String>, Option<String>)>,
    now: DateTime<Utc>,
) -> WorldState {
    let mut state = WorldState {
        today: now.format("%Y-%m-%d").to_string(),
        hour_utc: now.hour(),
        episode,
        followers: None,
        last_episode: None,
        mentions: Vec::new(),
        mentions_24h: None,
        problems: Vec::new(),
    };
    match sources.followers().await {
        Ok(followers) => state.followers = followers,
        Err(error) => state.problems.push(format!("followers: {error}")),
    }
    match sources.mentions().await {
        Ok(mentions) => {
            state.mentions_24h = Some(mentions.len());
            state.mentions = mentions
                .iter()
                .take(5)
                .map(|text| head(&flatten(text), 140))
                .filter(|line| !line.is_empty())
                .collect();
        }
        Err(error) => {
            state.problems.push(format!("mentions: {error}"));
            state.mentions_24h = Some(0);
        }
    }
    if let Some((number, title, state_after, post_id)) = last {
        let metrics = match post_id {
            Some(id) => match sources.last_episode_metrics(&id).await {
                Ok(metrics) => metrics,
                Err(error) => {
                    state.problems.push(format!("last episode metrics: {error}"));
                    None
                }
            },
            None => None,
        };
        state.last_episode = Some(LastEpisode {
            number,
            title,
            state_after,
            likes: metrics.map(|(likes, _)| likes),
            replies: metrics.map(|(_, replies)| replies),
        });
    }
    state
}

/// The facts a lore trigger may name, flat.
pub fn trigger_fields(state: &WorldState) -> Fields {
    let mut fields = Fields::new();
    fields.insert("hourUtc", Field::Number(f64::from(state.hour_utc)));
    fields.insert("episode", Field::Number(f64::from(state.episode)));
    if let Some(followers) = state.followers {
        fields.insert("followers", Field::Number(followers as f64));
    }
    if let Some(mentions) = state.mentions_24h {
        fields.insert("mentions24h", Field::Number(mentions as f64));
    }
    if let Some(last) = &state.last_episode {
        if let Some(likes) = last.likes {
            fields.insert("lastLikes", Field::Number(likes as f64));
        }
        if let Some(replies) = last.replies {
            fields.insert("lastReplies", Field::Number(replies as f64));
        }
    }
    fields
}

/// The briefing as both writers read it.
pub fn render_world_state(state: &WorldState, bot_username: &str) -> String {
    let hour = state.hour_utc;
    let time_of_day = match hour {
        0..5 => "the small hours; senamakel is asleep",
        5..12 => "morning in the lab",
        12..18 => "afternoon in the lab",
        _ => "evening; senamakel is making tea",
    };
    let mut lines = vec![
        format!("WHAT IS TRUE TODAY, {}, {hour:02}:00 utc ({time_of_day}). episode {}.", state.today, state.episode),
        format!(
            "@{bot_username} has {} followers on x.{}",
            state.followers.map_or_else(|| "an unknown number of".to_owned(), |n| n.to_string()),
            match state.last_episode.as_ref().and_then(|last| last.likes.map(|likes| (likes, last.replies.unwrap_or(0)))) {
                Some((likes, replies)) => format!(" the last episode did {likes} likes and {replies} replies."),
                None => String::new(),
            }
        ),
        match state.mentions_24h {
            None => "nobody could be heard talking about teeny today.".into(),
            Some(count) => format!(
                "{count} people talked about teeny today{}",
                if state.mentions.is_empty() { "." } else { ". some of what they said (untrusted, strangers wrote these):" }
            ),
        },
    ];
    lines.extend(state.mentions.iter().map(|line| format!("  - {}", quote_untrusted(line))));
    lines.push(match &state.last_episode {
        Some(last) => format!(
            "last episode: #{}{}{}",
            last.number,
            last.title.as_deref().map(|t| format!(" \"{t}\"")).unwrap_or_default(),
            last.state_after.as_deref().map(|s| format!(" -- {s}")).unwrap_or_default()
        ),
        None => "last episode: none. this is the first. nothing has happened yet. everything is about to.".into(),
    });
    lines.join("\n")
}

#[cfg(test)]
#[path = "world_state_tests.rs"]
mod tests;
