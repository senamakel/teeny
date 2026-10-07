//! What the morning reads off X itself: the corners of the timeline, best
//! conversations first, and the trends. A port of
//! `legacy/src/morning/timeline.ts`.

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use regex::Regex;
use teeny_safety::critic::Action;
use teeny_x::cache::XReads;
use teeny_x::model::{SearchOptions, SortOrder, Trend, TrendOptions, XPost};

use super::news::{Story, StoryKind, is_political};

/// One search over X's recent posts, standing for one corner of the timeline.
/// `share` is how many of the day's posts it may bring relative to the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineQuery {
    pub label: String,
    pub query: String,
    pub topic: String,
    pub share: u32,
}

/// The built-in corners: AI memes (twice the share), AI, crypto builders.
pub fn default_queries() -> Vec<TimelineQuery> {
    let corner = |label: &str, topic: &str, share: u32, query: &str| TimelineQuery {
        label: label.into(),
        topic: topic.into(),
        share,
        query: query.into(),
    };
    vec![
        corner(
            "ai memes",
            "ai",
            2,
            "(chatgpt OR claude OR gemini OR grok OR openai OR anthropic OR llm OR \"the model\" OR \"vibe coding\" OR \"ai agents\") \
             (meme OR lol OR lmao OR cooked OR unhinged OR \"so back\" OR \"it's over\" OR \"absolutely right\" OR \"skill issue\" OR \"hallucinated\") \
             has:media -has:cashtags -is:retweet -is:reply -is:quote lang:en -airdrop -presale -giveaway -launchpad",
        ),
        corner(
            "ai",
            "ai",
            1,
            "(openai OR anthropic OR deepmind OR mistral OR \"open source model\" OR \"open-source model\" OR \"open weights\" OR \
             \"ai agents\" OR agentic OR llm) -has:cashtags -is:retweet -is:reply -is:quote lang:en -airdrop -presale -giveaway",
        ),
        corner(
            "crypto builders",
            "crypto",
            1,
            "(ethereum OR solana OR onchain OR defi OR \"ai agent\" OR \"agent economy\") (shipped OR launched OR \"open source\" OR \
             builders OR lol OR meme) -has:cashtags -is:retweet -is:reply -is:quote lang:en -airdrop -presale -giveaway -launchpad -whitelist",
        ),
    ]
}

/// Below this a post is not a conversation to walk into.
pub const MIN_CONVERSATION: f64 = 10.0;

/// The timeline and the trends.
#[async_trait]
pub trait TimelineSource: Send + Sync {
    /// What X says is trending, or nothing.
    async fn trending(&self) -> Vec<Trend>;
    /// Recent original posts for one corner, as stories, best first.
    async fn pulse(&self, query: &TimelineQuery, hours: u32, limit: u32) -> Vec<Story>;
}

/// The timeline through the X API: recent search by relevancy, and the
/// trends endpoint when a WOEID is configured.
pub struct XTimelineSource {
    reads: Arc<dyn XReads>,
    woeid: Option<u32>,
    now: DateTime<Utc>,
}

impl XTimelineSource {
    pub fn new(reads: Arc<dyn XReads>, woeid: Option<u32>, now: DateTime<Utc>) -> Self {
        Self { reads, woeid, now }
    }
}

#[async_trait]
impl TimelineSource for XTimelineSource {
    async fn trending(&self) -> Vec<Trend> {
        let Some(woeid) = self.woeid else {
            return Vec::new();
        };
        self.reads
            .trends(TrendOptions {
                woeid: Some(woeid),
                max: None,
            })
            .await
            .unwrap_or_default()
    }

    async fn pulse(&self, query: &TimelineQuery, hours: u32, limit: u32) -> Vec<Story> {
        // X wants end_time at least ten seconds in the past; a minute is safe.
        let end = self.now - TimeDelta::minutes(1);
        let options = SearchOptions {
            max_results: Some(limit),
            start_time: Some(teeny_store::iso(end - TimeDelta::hours(i64::from(hours)))),
            end_time: Some(teeny_store::iso(end)),
            sort_order: Some(SortOrder::Relevancy),
            ..SearchOptions::default()
        };
        let posts = self
            .reads
            .search_posts(&query.query, &options)
            .await
            .unwrap_or_default();
        let mut stories: Vec<Story> = posts.iter().map(|post| story_of(post, query)).collect();
        stories.sort_by(|a, b| conversation_score(b).total_cmp(&conversation_score(a)));
        stories
    }
}

static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").expect("valid"));
static SPAM_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(giveaway|airdrop|presale|pre-sale|whitelist|pump|100x|1000x|dm me|link in bio|position yourself|alpha list|join (?:my|our) (?:telegram|discord))\b|0x[a-f0-9]{40}|\bca:")
        .expect("valid")
});
static CASHTAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$([A-Za-z]{2,10})(?-u:\b)").expect("valid"));

/// A post that is somebody's promotion: the words, a contract address, or
/// a cashtag that is not our own.
pub fn is_spam(text: &str) -> bool {
    SPAM_WORDS.is_match(text)
        || CASHTAG
            .captures_iter(text)
            .any(|caps| !caps[1].eq_ignore_ascii_case("TINY"))
}

/// A post as a story: the first line the headline, the text the excerpt,
/// the author the source, the counters carried across.
pub fn story_of(post: &XPost, query: &TimelineQuery) -> Story {
    let text = URL
        .replace_all(&post.text, "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let first_line = post
        .text
        .split('\n')
        .map(|line| URL.replace_all(line, "").trim().to_owned())
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| text.clone());
    let title = if super::stages::js_len(&first_line) > 140 {
        format!(
            "{}…",
            super::stages::js_prefix(&first_line, 137).trim_end()
        )
    } else {
        first_line
    };
    let author = post.author_username.as_deref();
    Story {
        title: if title.is_empty() {
            "(a picture)".into()
        } else {
            title
        },
        url: format!("https://x.com/{}/status/{}", author.unwrap_or("i"), post.id),
        source: author.map_or_else(|| "a post on X".to_owned(), |a| format!("@{a} on X")),
        published_at: post.created_at.clone(),
        excerpt: super::stages::js_prefix(&text, 400),
        topic: query.label.clone(),
        kind: StoryKind::X,
        metrics: post.metrics.clone(),
        media: post
            .media
            .iter()
            .filter_map(|item| item.alt_text.clone())
            .find(|alt| !alt.is_empty()),
    }
}

/// What a post already earned, valued as the ranker values it.
pub fn conversation_score(story: &Story) -> f64 {
    let Some(m) = &story.metrics else {
        return 0.0;
    };
    m.replies as f64 * Action::Reply.weight()
        + m.quotes as f64 * Action::Quote.weight()
        + m.reposts as f64 * Action::Repost.weight()
        + m.likes as f64 * Action::Like.weight()
}

/// The morning's read of the timeline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimelineRead {
    pub posts: Vec<Story>,
    /// Promotions and politics.
    pub dropped: usize,
    /// Posts with nobody talking under them.
    pub quiet: usize,
    pub trends: Vec<Trend>,
}

/// Every corner searched, promotions and politics dropped, duplicates
/// folded, each corner's share honoured. Off when `per_query` is zero.
pub async fn gather_timeline(
    source: &dyn TimelineSource,
    queries: &[TimelineQuery],
    hours: u32,
    per_query: u32,
) -> TimelineRead {
    if per_query == 0 {
        return TimelineRead::default();
    }
    let mut read = TimelineRead {
        trends: source.trending().await,
        ..TimelineRead::default()
    };
    let mut seen: Vec<String> = Vec::new();
    for query in queries {
        let limit = per_query * query.share.max(1);
        let found = source.pulse(query, hours, limit).await;
        for story in found.into_iter().take(limit as usize) {
            if seen.contains(&story.url) {
                continue;
            }
            seen.push(story.url.clone());
            if is_political(&story) || is_spam(&story.excerpt) {
                read.dropped += 1;
                continue;
            }
            if conversation_score(&story) < MIN_CONVERSATION {
                read.quiet += 1;
                continue;
            }
            read.posts.push(story);
        }
    }
    read
}

/// The trends as the model reads them, most posts first.
pub fn render_trends(trends: &[Trend]) -> String {
    let mut sorted = trends.to_vec();
    sorted.sort_by(|a, b| b.posts.unwrap_or(0).cmp(&a.posts.unwrap_or(0)));
    sorted
        .iter()
        .map(|trend| match trend.posts.filter(|posts| *posts > 0) {
            Some(posts) => format!("- {} ({} posts)", trend.name, thousands(posts)),
            None => format!("- {}", trend.name),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Corners from configuration (`label=query` entries split by `;`), or the
/// built-in ones when the string is empty.
pub fn parse_timeline_queries(value: &str) -> Vec<TimelineQuery> {
    if value.trim().is_empty() {
        return default_queries();
    }
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (label, query) = match entry.find('=') {
                Some(at) if at > 0 => (entry[..at].trim(), entry[at + 1..].trim()),
                _ => (entry, entry),
            };
            TimelineQuery {
                label: label.into(),
                query: query.into(),
                topic: label.into(),
                share: 1,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
