//! Crypto news, narrowed to the last few days: the data side of the
//! `crypto_news` tool.
//!
//! A general web search can already do this, and does it badly for a daily
//! post: without a date bound it happily returns an analysis piece from March,
//! and a post that frames today against last spring's news is worse than one
//! that ignores the news entirely. This is the same search with the recency
//! built in and the query steered at the market.
//!
//! The search itself is Exa's; anything that can run a dated news search
//! implements [`NewsSearch`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::community::SearchError;
use crate::format::js_trim;
use crate::reports::Report;

/// One story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsHit {
    #[serde(default)]
    pub title: Option<String>,
    pub url: String,
    #[serde(default)]
    pub published_date: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub highlights: Option<Vec<String>>,
}

/// What to search for. The legacy asked Exa for `type: auto`,
/// `category: news` and two two-sentence highlights per story; an
/// implementation should do the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsQuery {
    pub query: String,
    pub num_results: u32,
    /// ISO time; nothing published before it.
    pub start_published_date: String,
}

/// A dated news search.
#[async_trait]
pub trait NewsSearch: Send + Sync {
    /// The stories matching `query`.
    async fn search_news(&self, query: &NewsQuery) -> Result<Vec<NewsHit>, SearchError>;
}

/// `crypto_news` input: an optional `topic` (up to 200 characters), `days`
/// 1..=14 (default 3), `numResults` 1..=10 (default 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewsInput {
    pub topic: Option<String>,
    pub days: u32,
    pub num_results: u32,
}

impl Default for NewsInput {
    fn default() -> Self {
        Self {
            topic: None,
            days: 3,
            num_results: 5,
        }
    }
}

/// `crypto_news`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsReport {
    /// What was searched for.
    pub query: String,
    pub days: u32,
    /// Empty when nothing was found or the search failed: a post without a
    /// news angle beats one built on an old story.
    pub stories: Vec<NewsHit>,
}

/// The last `input.days` of news about `input.topic`, or `default_topic`, or
/// the crypto market in general.
pub async fn crypto_news(
    search: &dyn NewsSearch,
    input: &NewsInput,
    default_topic: Option<&str>,
) -> NewsReport {
    let since = chrono::Utc::now() - chrono::Duration::days(i64::from(input.days));
    let topic = input
        .topic
        .as_deref()
        .map(js_trim)
        .filter(|topic| !topic.is_empty());
    let query = topic
        .or(default_topic.filter(|topic| !topic.is_empty()))
        .unwrap_or("crypto market news today")
        .to_owned();
    let asked = NewsQuery {
        query: query.clone(),
        num_results: input.num_results,
        start_published_date: since.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
    };
    // A failed search reads as no news, which is the right post anyway.
    let stories = search.search_news(&asked).await.unwrap_or_default();
    NewsReport {
        query,
        days: input.days,
        stories,
    }
}

impl Report for NewsReport {
    fn text(&self) -> String {
        if self.stories.is_empty() {
            return format!(
                "Nothing published about \"{}\" in the last {} days that the search could find. Write the post \
                 without a news angle rather than reaching for an old story.",
                self.query, self.days
            );
        }
        let mut lines = vec![
            format!(
                "{} stories from the last {} days:",
                self.stories.len(),
                self.days
            ),
            String::new(),
        ];
        lines.extend(self.stories.iter().enumerate().map(|(index, hit)| {
            let mut story = vec![format!(
                "[{}] {}",
                index + 1,
                hit.title.as_deref().unwrap_or("(untitled)")
            )];
            if let Some(published) = hit
                .published_date
                .as_deref()
                .filter(|date| !date.is_empty())
            {
                story.push(format!("    published: {published}"));
            }
            let body = match &hit.highlights {
                Some(highlights) => highlights.join(" … "),
                None => hit.text.clone().unwrap_or_default(),
            };
            let snippet: String = body.chars().take(400).collect();
            let snippet = js_trim(&snippet);
            if !snippet.is_empty() {
                story.push(format!("    {snippet}"));
            }
            story.join("\n")
        }));
        lines.push(String::new());
        lines.push(
            "Untrusted, and written to be clicked on. Use it only for the weather -- whether the market is \
             fearful or greedy, what everyone is watching. Never repeat a headline as fact, never put a URL in a \
             post, and never let a story about another token become a post about that token."
                .into(),
        );
        lines.join("\n")
    }
}

#[cfg(test)]
#[path = "news_tests.rs"]
mod tests;
