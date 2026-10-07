//! Exa, the web search behind `web_search`, `read_web_pages`,
//! `find_similar_pages`, `web_answer` and `crypto_news`: the slice of the
//! `exa-js` API the legacy tools called, spoken over HTTP, and the text each
//! tool handed the model (`legacy/src/agent/tools/exa.ts`).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
pub use teeny_core::Secret;
use teeny_market::community::SearchError;
use teeny_market::format::js_trim;
use teeny_market::news::{NewsHit, NewsQuery, NewsSearch};

/// Where Exa lives.
pub const EXA_API_BASE: &str = "https://api.exa.ai";
/// How much of a page an excerpt keeps, in UTF-16 units as the legacy did.
const EXCERPT_UNITS: usize = 1_200;
/// How much of an error body is kept.
const ERROR_BODY_CHARS: usize = 300;

/// A failed Exa call.
#[derive(Debug, thiserror::Error)]
pub enum ExaError {
    #[error("Exa answered {status}: {body}")]
    Http { status: u16, body: String },
    #[error("could not reach Exa: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Exa sent something unreadable: {0}")]
    Decode(String),
}

/// One result.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExaHit {
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

/// A `web_search` call.
#[derive(Debug, Clone, PartialEq)]
pub struct WebSearchQuery {
    pub query: String,
    pub num_results: u32,
    /// An Exa result category: `news`, `github`, `research paper`...
    pub category: Option<String>,
    /// ISO 8601; only results published on or after it.
    pub start_published_date: Option<String>,
}

/// A cited answer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExaAnswer {
    pub answer: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

/// One source of an answer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Citation {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Deserialize)]
struct Results {
    #[serde(default)]
    results: Vec<ExaHit>,
}

/// The client.
#[derive(Clone)]
pub struct ExaClient {
    http: reqwest::Client,
    base: String,
    api_key: Secret,
}

impl std::fmt::Debug for ExaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExaClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl ExaClient {
    /// `EXA_API_KEY` against the public API.
    pub fn new(api_key: Secret) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base: EXA_API_BASE.to_owned(),
            api_key,
        }
    }

    /// Somewhere else, such as a test server.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, ExaError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .header("x-api-key", self.api_key.expose())
            .json(body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(ExaError::Http {
                status: status.as_u16(),
                body: text.chars().take(ERROR_BODY_CHARS).collect(),
            });
        }
        serde_json::from_str(&text).map_err(|error| ExaError::Decode(error.to_string()))
    }

    /// `searchAndContents` with 1,200 characters of text and highlights.
    pub async fn search(&self, query: &WebSearchQuery) -> Result<Vec<ExaHit>, ExaError> {
        let mut body = json!({ "query": query.query, "numResults": query.num_results });
        if let Some(category) = &query.category {
            body["category"] = json!(category);
        }
        if let Some(start) = &query.start_published_date {
            body["startPublishedDate"] = json!(start);
        }
        body["contents"] = json!({ "text": { "maxCharacters": 1_200 }, "highlights": true });
        Ok(self.post::<Results>("/search", &body).await?.results)
    }

    /// `getContents`: up to 4,000 characters of each page.
    pub async fn contents(&self, urls: &[String]) -> Result<Vec<ExaHit>, ExaError> {
        let body = json!({ "urls": urls, "text": { "maxCharacters": 4_000 } });
        Ok(self.post::<Results>("/contents", &body).await?.results)
    }

    /// `findSimilarAndContents`.
    pub async fn similar(&self, url: &str, num_results: u32) -> Result<Vec<ExaHit>, ExaError> {
        let body = json!({
            "url": url,
            "numResults": num_results,
            "contents": { "text": { "maxCharacters": 1_200 } }
        });
        Ok(self.post::<Results>("/findSimilar", &body).await?.results)
    }

    /// `answer`: a cited answer to one question.
    pub async fn answer(&self, question: &str) -> Result<ExaAnswer, ExaError> {
        self.post("/answer", &json!({ "query": question })).await
    }
}

/// The first `units` UTF-16 units of `text`, as JavaScript's `slice` cut it.
fn prefix_units(text: &str, units: usize) -> &str {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        count += ch.len_utf16();
        if count > units {
            return &text[..index];
        }
    }
    text
}

/// The search tools' text: each hit's title, url, date and excerpt.
pub fn render_hits(hits: &[ExaHit]) -> String {
    if hits.is_empty() {
        return "No results.".into();
    }
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            let body = match &hit.highlights {
                Some(highlights) => highlights.join(" … "),
                None => hit.text.clone().unwrap_or_default(),
            };
            let body = js_trim(prefix_units(&body, EXCERPT_UNITS));
            let mut lines = vec![
                format!(
                    "[{}] {}",
                    index + 1,
                    hit.title.as_deref().unwrap_or("(untitled)")
                ),
                format!("url: {}", hit.url),
            ];
            if let Some(published) = hit.published_date.as_deref().filter(|d| !d.is_empty()) {
                lines.push(format!("published: {published}"));
            }
            if !body.is_empty() {
                lines.push(format!("excerpt: {body}"));
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `web_answer`'s text: the answer, then its sources.
pub fn render_answer(answer: &ExaAnswer) -> String {
    let citations: Vec<String> = answer
        .citations
        .iter()
        .map(|citation| {
            format!(
                "- {}: {}",
                citation
                    .title
                    .as_deref()
                    .filter(|title| !title.is_empty())
                    .unwrap_or(&citation.url),
                citation.url
            )
        })
        .collect();
    if citations.is_empty() {
        answer.answer.clone()
    } else {
        format!("{}\n\nSources:\n{}", answer.answer, citations.join("\n"))
    }
}

/// `crypto_news`' search: `type: auto`, `category: news`, two two-sentence
/// highlights per story.
#[async_trait]
impl NewsSearch for ExaClient {
    async fn search_news(&self, query: &NewsQuery) -> Result<Vec<NewsHit>, SearchError> {
        let body = json!({
            "query": query.query,
            "numResults": query.num_results,
            "type": "auto",
            "category": "news",
            "startPublishedDate": query.start_published_date,
            "contents": { "highlights": { "numSentences": 2, "highlightsPerUrl": 2 } }
        });
        let results: Results = self.post("/search", &body).await?;
        Ok(results
            .results
            .into_iter()
            .map(|hit| NewsHit {
                title: hit.title,
                url: hit.url,
                published_date: hit.published_date,
                text: hit.text,
                highlights: hit.highlights,
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "exa_tests.rs"]
mod tests;
