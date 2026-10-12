//! Where the wire comes from: the web (Exa), reddit's and 4chan's public
//! JSON, and Google News as a feed. Everything read here is untrusted; the
//! callers screen it ([`super::screen`]) before any of it reaches a room.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use teeny_agent::exa::ExaClient;
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::text::{clip, strip_html, unescape_entities};
use super::{ForumPost, Site};

/// The public forum APIs ask for a real user agent and one request a second.
pub const USER_AGENT: &str =
    "teeny-backrooms/1.0 (research; contact via github tinyhumansai/teeny)";

/// Keeps consecutive requests at least `gap` apart.
pub struct Pacer {
    gap: Duration,
    last: Mutex<Option<Instant>>,
}

impl Pacer {
    pub fn new(gap: Duration) -> Self {
        Self {
            gap,
            last: Mutex::new(None),
        }
    }

    /// Waits for this request's turn.
    pub async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(at) = *last {
            let ready = at + self.gap;
            if ready > Instant::now() {
                tokio::time::sleep_until(ready).await;
            }
        }
        *last = Some(Instant::now());
    }
}

/// A board or a sub, searched.
#[async_trait]
pub trait Forum: Send + Sync {
    fn site(&self) -> Site;
    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<ForumPost>>;
}

/// A web search, with the options the long-form side asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebQuery {
    pub query: String,
    pub num_results: u32,
    /// Only things published in the last `days`.
    pub days: Option<u32>,
    /// `news`, for a news search.
    pub category: Option<String>,
    pub include_domains: Vec<String>,
    /// Two-sentence highlights, this many per result.
    pub highlights: Option<(u32, u32)>,
    /// Image links per result, for a photograph.
    pub image_links: u32,
}

/// One result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebHit {
    pub title: Option<String>,
    pub url: String,
    pub published_date: Option<String>,
    pub text: Option<String>,
    pub highlights: Vec<String>,
    pub image: Option<String>,
    pub image_links: Vec<String>,
}

#[async_trait]
pub trait WebSearch: Send + Sync {
    async fn search(&self, query: &WebQuery) -> anyhow::Result<Vec<WebHit>>;
}

/// Exa's `searchAndContents`, as the legacy asked it.
#[async_trait]
impl WebSearch for ExaClient {
    async fn search(&self, query: &WebQuery) -> anyhow::Result<Vec<WebHit>> {
        let mut body =
            json!({ "query": query.query, "numResults": query.num_results, "type": "auto" });
        if let Some(days) = query.days {
            body["startPublishedDate"] = json!(
                (Utc::now() - TimeDelta::days(i64::from(days)))
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            );
        }
        if let Some(category) = &query.category {
            body["category"] = json!(category);
        }
        if !query.include_domains.is_empty() {
            body["includeDomains"] = json!(query.include_domains);
        }
        let mut contents = serde_json::Map::new();
        if let Some((sentences, per_url)) = query.highlights {
            contents.insert(
                "highlights".into(),
                json!({ "numSentences": sentences, "highlightsPerUrl": per_url }),
            );
        }
        if query.image_links > 0 {
            contents.insert("extras".into(), json!({ "imageLinks": query.image_links }));
        }
        if !contents.is_empty() {
            body["contents"] = Value::Object(contents);
        }
        let results = self.search_raw(&body).await?;
        Ok(results.iter().filter_map(read_hit).collect())
    }
}

fn read_hit(raw: &Value) -> Option<WebHit> {
    let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_owned);
    let list = |value: Option<&Value>| -> Vec<String> {
        value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(WebHit {
        url: text("url")?,
        title: text("title"),
        published_date: text("publishedDate"),
        text: text("text"),
        highlights: list(raw.get("highlights")),
        image: text("image"),
        image_links: list(
            raw.get("extras")
                .and_then(|extras| extras.get("imageLinks")),
        ),
    })
}

/// The host of a URL, without `www.`.
pub fn safe_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_else(|| url.to_owned())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_default()
}

/// Reddit's public JSON, with Exa on reddit.com when reddit throttles.
pub struct RedditForum {
    http: reqwest::Client,
    base: String,
    subs: Vec<String>,
    web: Option<Arc<dyn WebSearch>>,
    pacer: Arc<Pacer>,
}

#[derive(Deserialize)]
struct Listing {
    data: Option<ListingData>,
}

#[derive(Deserialize)]
struct ListingData {
    #[serde(default)]
    children: Vec<Child>,
}

#[derive(Deserialize)]
struct Child {
    data: Option<RawRedditPost>,
}

#[derive(Deserialize)]
struct RawRedditPost {
    title: Option<String>,
    selftext: Option<String>,
    permalink: Option<String>,
    score: Option<i64>,
    created_utc: Option<f64>,
    subreddit: Option<String>,
}

impl RedditForum {
    pub fn new(subs: Vec<String>, web: Option<Arc<dyn WebSearch>>, pacer: Arc<Pacer>) -> Self {
        Self {
            http: http(),
            base: "https://www.reddit.com".into(),
            subs,
            web,
            pacer,
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    /// The posts, and whether reddit refused.
    async fn search_sub(&self, sub: &str, query: &str, limit: usize) -> (Vec<ForumPost>, bool) {
        self.pacer.wait().await;
        let url = format!("{}/r/{}/search.json", self.base, sub);
        let response = self
            .http
            .get(url)
            .query(&[
                ("q", query),
                ("restrict_sr", "1"),
                ("sort", "new"),
                ("t", "week"),
                ("limit", &limit.to_string()),
            ])
            .header("accept", "application/json")
            .send()
            .await;
        let Ok(response) = response else {
            return (Vec::new(), false);
        };
        if matches!(response.status().as_u16(), 403 | 429) {
            return (Vec::new(), true);
        }
        if !response.status().is_success() {
            return (Vec::new(), false);
        }
        let Ok(listing) = response.json::<Listing>().await else {
            return (Vec::new(), false);
        };
        let posts = listing
            .data
            .map(|data| data.children)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|child| child.data)
            .filter_map(|post| {
                let title = unescape_entities(post.title.as_deref().filter(|t| !t.is_empty())?);
                let body = unescape_entities(post.selftext.as_deref().unwrap_or_default());
                let body = body.trim();
                Some(ForumPost {
                    site: Site::Reddit,
                    place: format!("r/{}", post.subreddit.as_deref().unwrap_or(sub)),
                    text: clip(if body.is_empty() { &title } else { body }, 600),
                    title: Some(title),
                    url: format!(
                        "https://www.reddit.com{}",
                        post.permalink.unwrap_or_default()
                    ),
                    score: post.score,
                    at: post.created_utc.and_then(|secs| {
                        chrono::DateTime::from_timestamp(secs as i64, 0)
                            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    }),
                })
            })
            .collect();
        (posts, false)
    }

    async fn via_web(&self, web: &dyn WebSearch, query: &str, limit: usize) -> Vec<ForumPost> {
        static SUB: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)reddit\.com/(r/[^/]+)").expect("valid"));
        let hits = web
            .search(&WebQuery {
                query: query.to_owned(),
                num_results: limit as u32,
                days: Some(7),
                include_domains: vec!["reddit.com".into()],
                highlights: Some((3, 1)),
                ..WebQuery::default()
            })
            .await
            .unwrap_or_default();
        hits.into_iter()
            .map(|hit| {
                let body = if hit.highlights.is_empty() {
                    hit.text
                        .clone()
                        .or_else(|| hit.title.clone())
                        .unwrap_or_default()
                } else {
                    hit.highlights.join(" ")
                };
                ForumPost {
                    site: Site::Reddit,
                    place: SUB
                        .captures(&hit.url)
                        .map_or_else(|| "reddit".to_owned(), |c| c[1].to_owned()),
                    title: hit.title,
                    text: clip(body.trim(), 600),
                    url: hit.url,
                    score: None,
                    at: hit.published_date,
                }
            })
            .collect()
    }
}

#[async_trait]
impl Forum for RedditForum {
    fn site(&self) -> Site {
        Site::Reddit
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<ForumPost>> {
        let per_sub = (limit.div_ceil(self.subs.len().max(1))).max(3);
        let mut direct = Vec::new();
        let mut blocked = false;
        for sub in &self.subs {
            let (posts, refused) = self.search_sub(sub, query, per_sub).await;
            direct.extend(posts);
            blocked |= refused;
        }
        if direct.is_empty()
            && blocked
            && let Some(web) = &self.web
        {
            return Ok(self.via_web(web.as_ref(), query, limit).await);
        }
        direct.sort_by_key(|post| std::cmp::Reverse(post.score.unwrap_or(0)));
        direct.truncate(limit);
        Ok(direct)
    }
}

/// 4chan's read-only JSON: a board's catalog, matched against the query's
/// words, read once per instance; the best thread's first replies after.
pub struct FourchanForum {
    http: reqwest::Client,
    base: String,
    boards: Vec<String>,
    replies: usize,
    pacer: Arc<Pacer>,
    catalogs: Mutex<HashMap<String, Vec<CatalogThread>>>,
}

#[derive(Debug, Clone)]
struct CatalogThread {
    board: String,
    no: u64,
    replies: i64,
    time: i64,
    subject: Option<String>,
    text: String,
    haystack: String,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    threads: Vec<RawThread>,
}

#[derive(Deserialize)]
struct RawThread {
    no: u64,
    sub: Option<String>,
    com: Option<String>,
    replies: Option<i64>,
    time: Option<i64>,
}

#[derive(Deserialize)]
struct RawThreadPage {
    #[serde(default)]
    posts: Vec<RawThread>,
}

impl FourchanForum {
    pub fn new(boards: Vec<String>, pacer: Arc<Pacer>) -> Self {
        Self {
            http: http(),
            base: "https://a.4cdn.org".into(),
            boards,
            replies: 3,
            pacer,
            catalogs: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    /// How many replies of the best thread to read.
    pub fn with_replies(mut self, replies: usize) -> Self {
        self.replies = replies;
        self
    }

    async fn catalog(&self, board: &str) -> Vec<CatalogThread> {
        let mut catalogs = self.catalogs.lock().await;
        if let Some(threads) = catalogs.get(board) {
            return threads.clone();
        }
        let threads = self.read_catalog(board).await.unwrap_or_default();
        catalogs.insert(board.to_owned(), threads.clone());
        threads
    }

    async fn read_catalog(&self, board: &str) -> anyhow::Result<Vec<CatalogThread>> {
        self.pacer.wait().await;
        let response = self
            .http
            .get(format!("{}/{board}/catalog.json", self.base))
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "4chan /{board}/ answered {}",
            response.status()
        );
        let pages: Vec<Page> = response.json().await?;
        Ok(pages
            .into_iter()
            .flat_map(|page| page.threads)
            .map(|thread| {
                let subject = thread
                    .sub
                    .as_deref()
                    .map(strip_html)
                    .filter(|s| !s.is_empty());
                let text = thread.com.as_deref().map(strip_html).unwrap_or_default();
                CatalogThread {
                    board: board.to_owned(),
                    no: thread.no,
                    replies: thread.replies.unwrap_or(0),
                    time: thread.time.unwrap_or(0),
                    haystack: format!("{} {text}", subject.as_deref().unwrap_or_default())
                        .to_lowercase(),
                    subject,
                    text,
                }
            })
            .collect())
    }

    async fn thread_replies(&self, thread: &CatalogThread, count: usize) -> Vec<ForumPost> {
        static QUOTE_LINK: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?m)^>>\d+\s*").expect("valid"));
        self.pacer.wait().await;
        let Ok(response) = self
            .http
            .get(format!(
                "{}/{}/thread/{}.json",
                self.base, thread.board, thread.no
            ))
            .send()
            .await
        else {
            return Vec::new();
        };
        if !response.status().is_success() {
            return Vec::new();
        }
        let Ok(page) = response.json::<RawThreadPage>().await else {
            return Vec::new();
        };
        page.posts
            .into_iter()
            .skip(1)
            .filter_map(|post| post.com)
            .map(|com| {
                QUOTE_LINK
                    .replace_all(&strip_html(&com), "")
                    .trim()
                    .to_owned()
            })
            .filter(|text| text.chars().count() > 20)
            .take(count)
            .map(|text| ForumPost {
                site: Site::Fourchan,
                place: format!("/{}/ reply", thread.board),
                title: None,
                text: clip(&text, 400),
                url: format!(
                    "https://boards.4chan.org/{}/thread/{}",
                    thread.board, thread.no
                ),
                score: None,
                at: None,
            })
            .collect()
    }
}

#[async_trait]
impl Forum for FourchanForum {
    fn site(&self) -> Site {
        Site::Fourchan
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<ForumPost>> {
        let words = super::text::keywords(query, 6);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut threads = Vec::new();
        for board in &self.boards {
            threads.extend(self.catalog(board).await);
        }
        let mut scored: Vec<(CatalogThread, usize)> = threads
            .into_iter()
            .map(|thread| {
                let hits = words
                    .iter()
                    .filter(|word| thread.haystack.contains(word.as_str()))
                    .count();
                (thread, hits)
            })
            .filter(|(_, hits)| *hits > 0)
            .collect();
        scored.sort_by(|(a, ha), (b, hb)| hb.cmp(ha).then(b.replies.cmp(&a.replies)));
        scored.truncate(limit);
        let mut posts: Vec<ForumPost> = scored
            .iter()
            .map(|(thread, _)| ForumPost {
                site: Site::Fourchan,
                place: format!("/{}/", thread.board),
                title: thread.subject.clone(),
                text: clip(&thread.text, 600),
                url: format!(
                    "https://boards.4chan.org/{}/thread/{}",
                    thread.board, thread.no
                ),
                score: Some(thread.replies),
                at: chrono::DateTime::from_timestamp(thread.time, 0)
                    .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            })
            .collect();
        if let Some((best, _)) = scored.first()
            && self.replies > 0
        {
            posts.extend(self.thread_replies(best, self.replies).await);
        }
        Ok(posts)
    }
}

/// One item of a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssItem {
    pub title: String,
    pub source: Option<String>,
    pub at: Option<String>,
    pub description: Option<String>,
}

/// Google News as a feed: no key, a title and a source per item. The
/// fallback seed when Exa is over capacity. Read only; nothing is stored.
pub struct NewsFeed {
    http: reqwest::Client,
    base: String,
}

impl Default for NewsFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl NewsFeed {
    pub fn new() -> Self {
        Self {
            http: http(),
            base: "https://news.google.com".into(),
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    pub async fn search(&self, query: &str) -> anyhow::Result<Vec<RssItem>> {
        let response = self
            .http
            .get(format!("{}/rss/search", self.base))
            .query(&[
                ("q", query),
                ("hl", "en-US"),
                ("gl", "US"),
                ("ceid", "US:en"),
            ])
            .header("user-agent", "teeny-backrooms/1.0")
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "google news answered {}",
            response.status()
        );
        Ok(parse_rss(&response.text().await?))
    }
}

static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<item>([\s\S]*?)</item>").expect("valid"));
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("valid"));

fn field(body: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r"<{name}[^>]*>(?:<!\[CDATA\[)?([\s\S]*?)(?:\]\]>)?</{name}>"
    ))
    .ok()?;
    let raw = pattern.captures(body)?.get(1)?.as_str();
    if raw.is_empty() {
        return None;
    }
    // Descriptions arrive as escaped html inside the xml: unescape, strip, unescape again.
    let once = unescape_entities(raw);
    let stripped = TAGS.replace_all(&once, "");
    Some(super::text::flatten(&unescape_entities(&stripped))).filter(|text| !text.is_empty())
}

pub fn parse_rss(xml: &str) -> Vec<RssItem> {
    ITEM.captures_iter(xml)
        .filter_map(|captures| {
            let body = captures.get(1)?.as_str();
            let raw_title = field(body, "title")?;
            let source = field(body, "source");
            // Google appends " - Publisher" to every title; the source element already has it.
            let title = match &source {
                Some(source) => raw_title
                    .strip_suffix(&format!(" - {source}"))
                    .map(str::to_owned)
                    .unwrap_or(raw_title),
                None => raw_title,
            };
            Some(RssItem {
                title,
                source,
                at: field(body, "pubDate"),
                description: field(body, "description"),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "sources_tests.rs"]
pub(crate) mod tests;
