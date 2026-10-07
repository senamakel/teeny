//! Where a room's seed comes from: the news (Exa, or the Google News feed
//! when Exa has nothing), the boards, memory, a mention. Every finder
//! screens what it found; a seed is untrusted text all the same, and the
//! room quotes it ([`super::room`]).

use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use regex::Regex;
use teeny_memory::Memory;
use teeny_x::cache::XReads;
use teeny_x::model::SearchOptions;

use super::screen::{admissible, admissible_post, is_political};
use super::sources::{Forum, NewsFeed, WebQuery, WebSearch, safe_host};
use super::text::{flatten, head};
use super::tokens::{Random, Rng};
use super::{Seed, SeedKind};

/// Finds a seed on a topic, or nothing.
#[async_trait]
pub trait SeedFinder: Send + Sync {
    async fn find(&self, topic: &str) -> anyhow::Result<Option<Seed>>;
}

/// A story from Exa's news index, or from the Google News feed when Exa
/// has nothing or is down.
pub struct NewsSeeds {
    pub web: Option<Arc<dyn WebSearch>>,
    pub feed: NewsFeed,
    pub random: Mutex<Rng>,
}

#[async_trait]
impl SeedFinder for NewsSeeds {
    async fn find(&self, topic: &str) -> anyhow::Result<Option<Seed>> {
        if let Some(web) = &self.web {
            let hits = web
                .search(&WebQuery {
                    query: topic.to_owned(),
                    num_results: 6,
                    days: Some(2),
                    category: Some("news".into()),
                    highlights: Some((2, 2)),
                    ..WebQuery::default()
                })
                .await
                .unwrap_or_default();
            for hit in hits {
                let Some(title) = hit
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                else {
                    continue;
                };
                let excerpt = if hit.highlights.is_empty() {
                    hit.text.clone().unwrap_or_default()
                } else {
                    hit.highlights.join(" … ")
                };
                let excerpt = head(&flatten(&excerpt), 400);
                if is_political(&format!("{title} {excerpt}"))
                    || !admissible(&format!("{title} {excerpt}"))
                {
                    continue;
                }
                return Ok(Some(Seed {
                    kind: SeedKind::News,
                    title: Some(title.to_owned()),
                    text: excerpt,
                    source: Some(safe_host(&hit.url)),
                }));
            }
        }
        let items: Vec<_> = self
            .feed
            .search(topic)
            .await?
            .into_iter()
            .filter(|item| {
                admissible(&format!(
                    "{} {}",
                    item.title,
                    item.description.as_deref().unwrap_or_default()
                ))
            })
            .take(10)
            .collect();
        if items.is_empty() {
            return Ok(None);
        }
        let at = self
            .random
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .index(items.len());
        let item = items[at].clone();
        let text = item
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map_or_else(|| item.title.clone(), str::to_owned);
        Ok(Some(Seed {
            kind: SeedKind::News,
            title: Some(item.title),
            text,
            source: item.source,
        }))
    }
}

/// A random admissible post from the boards.
pub struct ForumSeeds {
    pub forums: Vec<Arc<dyn Forum>>,
    pub random: Mutex<Rng>,
}

#[async_trait]
impl SeedFinder for ForumSeeds {
    async fn find(&self, topic: &str) -> anyhow::Result<Option<Seed>> {
        let mut posts = Vec::new();
        for forum in &self.forums {
            posts.extend(forum.search(topic, 6).await.unwrap_or_default());
        }
        posts.retain(admissible_post);
        if posts.is_empty() {
            return Ok(None);
        }
        let at = self
            .random
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .index(posts.len());
        let post = posts.swap_remove(at);
        Ok(Some(Seed {
            kind: SeedKind::Forum,
            title: post.title,
            text: post.text,
            source: Some(post.place),
        }))
    }
}

static BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[[^\]]*\]\s*(\[[^\]]*\]\s*)?").expect("valid"));
static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(GENERAL CONTEXT|ABOUT\b)").expect("valid"));
static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-{3,}$").expect("valid"));

/// A recall, read as prose: raw captures and headers are left out.
pub struct MemorySeeds {
    pub memory: Arc<dyn Memory>,
}

#[async_trait]
impl SeedFinder for MemorySeeds {
    async fn find(&self, topic: &str) -> anyhow::Result<Option<Seed>> {
        let recalled = self.memory.recall(topic, None).await?;
        let text = recalled
            .split('\n')
            .map(str::trim)
            .filter(|line| {
                !line.is_empty()
                    && !line.starts_with('{')
                    && !HEADER.is_match(line)
                    && !RULE.is_match(line)
            })
            .map(|line| BRACKETS.replace(line, "").into_owned())
            .filter(|line| line.chars().count() > 8)
            .collect::<Vec<_>>()
            .join("\n");
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        Ok(Some(Seed {
            kind: SeedKind::Memory,
            title: Some(topic.to_owned()),
            text: head(text, 1_200),
            source: Some("memory".into()),
        }))
    }
}

/// The first admissible mention of the account.
pub async fn seed_from_mention(x: &dyn XReads, bot_username: &str) -> anyhow::Result<Option<Seed>> {
    let options = SearchOptions {
        max_results: Some(20),
        ..SearchOptions::default()
    };
    let posts = x
        .search_posts(
            &format!("(@{bot_username} OR to:{bot_username}) -from:{bot_username} -is:retweet"),
            &options,
        )
        .await?;
    Ok(posts
        .into_iter()
        .find(|post| admissible(&post.text))
        .map(|post| Seed {
            kind: SeedKind::Mention,
            text: post.text,
            title: None,
            source: Some(
                post.author_username
                    .map_or_else(|| "x".to_owned(), |u| format!("@{u}")),
            ),
        }))
}

#[cfg(test)]
#[path = "seeds_tests.rs"]
mod tests;
