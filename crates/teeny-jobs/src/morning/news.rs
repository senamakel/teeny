//! The morning's reading: Exa's news index, the political filter, and the
//! stories as the model reads them. A port of `legacy/src/morning/news.ts`.

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use regex::Regex;
use teeny_agent::exa::{ExaClient, WebSearchQuery};
use teeny_x::model::XPostMetrics;

/// Where a story came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryKind {
    /// A newsroom's article.
    News,
    /// A post from X's own timeline; the excerpt is the post.
    X,
}

/// One story as the pipeline sees it: enough to choose from, not to quote.
#[derive(Debug, Clone, PartialEq)]
pub struct Story {
    pub title: String,
    pub url: String,
    pub source: String,
    pub published_at: Option<String>,
    pub excerpt: String,
    /// Which topic search (or timeline corner) found it.
    pub topic: String,
    pub kind: StoryKind,
    pub metrics: Option<XPostMetrics>,
    /// What the picture on the post shows, when X described it.
    pub media: Option<String>,
}

/// Recent news, and the article behind a story.
#[async_trait]
pub trait NewsSource: Send + Sync {
    /// Recent stories on a topic.
    async fn search(&self, topic: &str, days: u32, limit: u32) -> Result<Vec<Story>, String>;
    /// The article, up to `max_characters`, or empty when it cannot be read.
    async fn read(&self, url: &str, max_characters: usize) -> String;
}

/// Exa, bounded to the last day or two and to its news index.
pub struct ExaNewsSource {
    exa: Arc<ExaClient>,
}

impl ExaNewsSource {
    pub fn new(exa: Arc<ExaClient>) -> Self {
        Self { exa }
    }
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .map_or_else(
            || "unknown".to_owned(),
            |host| host.strip_prefix("www.").unwrap_or(&host).to_owned(),
        )
}

#[async_trait]
impl NewsSource for ExaNewsSource {
    async fn search(&self, topic: &str, days: u32, limit: u32) -> Result<Vec<Story>, String> {
        let since = Utc::now() - TimeDelta::days(i64::from(days));
        let query = WebSearchQuery {
            query: topic.to_owned(),
            num_results: limit,
            category: Some("news".into()),
            start_published_date: Some(teeny_store::iso(since)),
        };
        let hits = self.exa.search(&query).await.unwrap_or_default();
        Ok(hits
            .into_iter()
            .filter_map(|hit| {
                let title = hit.title.as_deref().map(str::trim).unwrap_or("");
                if title.is_empty() {
                    return None;
                }
                let excerpt = match &hit.highlights {
                    Some(highlights) => highlights.join(" … "),
                    None => hit.text.clone().unwrap_or_default(),
                };
                Some(Story {
                    title: title.to_owned(),
                    source: host_of(&hit.url),
                    published_at: hit.published_date.clone(),
                    excerpt: super::stages::js_prefix(&squash(&excerpt), 400)
                        .trim()
                        .to_owned(),
                    topic: topic.to_owned(),
                    url: hit.url,
                    kind: StoryKind::News,
                    metrics: None,
                    media: None,
                })
            })
            .collect())
    }

    async fn read(&self, url: &str, max_characters: usize) -> String {
        let hits = self
            .exa
            .contents(&[url.to_owned()])
            .await
            .unwrap_or_default();
        let text = hits
            .into_iter()
            .next()
            .and_then(|hit| hit.text)
            .unwrap_or_default();
        super::stages::js_prefix(&squash(&text), max_characters)
    }
}

/// Words that mark a story as political, whatever else it is about. Word
/// boundaries, case-insensitive, over the title and the excerpt.
pub const POLITICAL_TERMS: &[&str] = &[
    "election", "elections", "electoral", "ballot", "president", "presidential",
    "prime minister", "senate", "senator", "lawmaker", "lawmakers", "house speaker",
    "policymaker", "policymakers", "minister", "ministry", "federal", "feds", "treasury",
    "justice department", "north korea", "north korean", "congress", "congressional",
    "parliament", "legislation", "legislature", "governor", "white house", "kremlin",
    "republican", "republicans", "democrat", "democrats", "gop", "labour party", "tory",
    "tories", "trump", "biden", "harris", "putin", "zelensky", "netanyahu", "xi jinping",
    "modi", "war", "warfare", "invasion", "ceasefire", "missile", "missiles", "airstrike",
    "airstrikes", "troops", "military", "gaza", "israel", "israeli", "palestine",
    "palestinian", "ukraine", "ukrainian", "russia", "russian", "iran", "iranian", "taiwan",
    "nato", "tariff", "tariffs", "sanction", "sanctions", "immigration", "immigrants",
    "deportation", "abortion", "protest", "protests", "protesters", "riot", "riots",
    "impeach", "impeachment", "indictment", "indicted", "lawsuit", "supreme court",
    "regulator", "regulators", "regulation", "regulatory", "doj", "fbi", "cia", "shooting",
    "terror", "terrorist", "genocide",
];

static POLITICAL: LazyLock<Regex> = LazyLock::new(|| {
    let terms = POLITICAL_TERMS
        .iter()
        .map(|term| regex::escape(term.trim()))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?i)(?:^|[^a-z])(?:{terms})(?:$|[^a-z])")).expect("a valid pattern")
});

/// Whether a story is political, by the blunt word list.
pub fn is_political(story: &Story) -> bool {
    is_political_text(&format!("{} {}", story.title, story.excerpt))
}

pub(crate) fn is_political_text(text: &str) -> bool {
    POLITICAL.is_match(text)
}

/// Every topic searched, the political stories dropped, duplicates by URL
/// folded. A search that fails contributes nothing. Returns the stories and
/// how many were dropped as political.
pub async fn gather_news(
    source: &dyn NewsSource,
    topics: &[String],
    days: u32,
    per_topic: u32,
) -> (Vec<Story>, usize) {
    let mut seen: Vec<String> = Vec::new();
    let mut stories = Vec::new();
    let mut dropped = 0;
    for topic in topics {
        let found = source
            .search(topic, days, per_topic)
            .await
            .unwrap_or_default();
        for story in found {
            let key = story
                .url
                .split(['#', '?'])
                .next()
                .unwrap_or("")
                .to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            if is_political(&story) {
                dropped += 1;
                continue;
            }
            stories.push(story);
        }
    }
    (stories, dropped)
}

/// The stories as the model reads them, numbered from `from`.
pub fn render_stories(stories: &[Story], from: usize) -> String {
    stories
        .iter()
        .enumerate()
        .map(|(index, story)| {
            let mut lines = vec![format!("[{}] {}", index + from, story.title)];
            let date = story
                .published_at
                .as_deref()
                .map(|at| format!(", {}", super::stages::js_prefix(at, 10)))
                .unwrap_or_default();
            lines.push(format!("    {}{date} ({})", story.source, story.topic));
            if !story.excerpt.is_empty() && story.excerpt != story.title {
                lines.push(format!("    {}", story.excerpt));
            }
            if let Some(media) = &story.media {
                lines.push(format!("    picture: {media}"));
            }
            if let Some(metrics) = &story.metrics {
                lines.push(format!("    {}", render_metrics(metrics)));
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `310 replies, 140 quotes, 90 reposts, 4.2k likes, 900k views`.
pub fn render_metrics(metrics: &XPostMetrics) -> String {
    let mut parts = vec![
        format!("{} replies", compact(metrics.replies)),
        format!("{} quotes", compact(metrics.quotes)),
        format!("{} reposts", compact(metrics.reposts)),
        format!("{} likes", compact(metrics.likes)),
    ];
    if let Some(views) = metrics.impressions.filter(|views| *views > 0) {
        parts.push(format!("{} views", compact(views)));
    }
    parts.join(", ")
}

fn compact(n: u64) -> String {
    let short = |value: f64, unit: &str| {
        let fixed = format!("{value:.1}");
        format!("{}{unit}", fixed.strip_suffix(".0").unwrap_or(&fixed))
    };
    if n >= 1_000_000 {
        short(n as f64 / 1_000_000.0, "m")
    } else if n >= 1_000 {
        short(n as f64 / 1_000.0, "k")
    } else {
        n.to_string()
    }
}

#[cfg(test)]
#[path = "news_tests.rs"]
mod tests;
