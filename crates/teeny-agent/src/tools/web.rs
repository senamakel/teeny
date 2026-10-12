//! `web_search`, `read_web_pages`, `find_similar_pages` and `web_answer`:
//! the live web through Exa, as `legacy/src/agent/tools/exa.ts` offered it.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use super::{Access, HostTool, Tool, ToolResult, failed, invalid, parse};
use crate::exa::{ExaClient, WebSearchQuery, render_answer, render_hits};

fn check_results(num_results: u32) -> Result<(), ToolResult> {
    if (1..=10).contains(&num_results) {
        Ok(())
    } else {
        Err(invalid("numResults must be between 1 and 10"))
    }
}

/// An http(s) url, as zod's `.url()` accepted for these tools.
fn is_web_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// The result categories Exa filters on.
pub const CATEGORIES: [&str; 7] = [
    "company",
    "research paper",
    "news",
    "tweet",
    "pdf",
    "github",
    "financial report",
];

fn five() -> u32 {
    5
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchArgs {
    query: String,
    #[serde(default = "five")]
    num_results: u32,
    category: Option<String>,
    start_published_date: Option<String>,
}

#[derive(Deserialize)]
struct PagesArgs {
    urls: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimilarArgs {
    url: String,
    #[serde(default = "five")]
    num_results: u32,
}

#[derive(Deserialize)]
struct AnswerArgs {
    question: String,
}

/// The four web tools, in the legacy order.
pub fn web_tools(exa: Arc<ExaClient>) -> Vec<Arc<dyn Tool>> {
    vec![
        web_search(Arc::clone(&exa)),
        read_web_pages(Arc::clone(&exa)),
        find_similar_pages(Arc::clone(&exa)),
        web_answer(exa),
    ]
}

/// `web_search`.
pub fn web_search(exa: Arc<ExaClient>) -> Arc<dyn Tool> {
    HostTool::build(
        "web_search",
        "Search the live web with Exa and return titles, urls, and excerpts. Use for news, prices, project \
         background, and anything that may have changed recently.",
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "minLength": 1, "description": "A natural-language search query." },
                "numResults": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 },
                "category": { "type": "string", "enum": CATEGORIES, "description": "Optional Exa result category filter." },
                "startPublishedDate": { "type": "string", "description": "ISO 8601 date; only return results published on or after it." }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let exa = Arc::clone(&exa);
            async move {
                let args = match parse::<SearchArgs>(args) {
                    Ok(args) => args,
                    Err(refused) => return refused,
                };
                if args.query.is_empty() {
                    return invalid("query must not be empty");
                }
                if let Err(refused) = check_results(args.num_results) {
                    return refused;
                }
                if let Some(category) = &args.category
                    && !CATEGORIES.contains(&category.as_str())
                {
                    return invalid(format!("category must be one of {}", CATEGORIES.join(", ")));
                }
                let query = WebSearchQuery {
                    query: args.query,
                    num_results: args.num_results,
                    category: args.category,
                    start_published_date: args.start_published_date.filter(|d| !d.is_empty()),
                };
                match exa.search(&query).await {
                    Ok(hits) => ToolResult::success(render_hits(&hits)),
                    Err(error) => failed(error),
                }
            }
        },
    )
}

/// `read_web_pages`.
pub fn read_web_pages(exa: Arc<ExaClient>) -> Arc<dyn Tool> {
    HostTool::build(
        "read_web_pages",
        "Fetch the readable text of specific urls found by web_search.",
        json!({
            "type": "object",
            "properties": {
                "urls": { "type": "array", "items": { "type": "string", "format": "uri" }, "minItems": 1, "maxItems": 5 }
            },
            "required": ["urls"],
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let exa = Arc::clone(&exa);
            async move {
                let args = match parse::<PagesArgs>(args) {
                    Ok(args) => args,
                    Err(refused) => return refused,
                };
                if !(1..=5).contains(&args.urls.len()) {
                    return invalid("urls must hold 1 to 5 urls");
                }
                if let Some(bad) = args.urls.iter().find(|url| !is_web_url(url)) {
                    return invalid(format!("{bad} is not a web url"));
                }
                match exa.contents(&args.urls).await {
                    Ok(hits) => ToolResult::success(render_hits(&hits)),
                    Err(error) => failed(error),
                }
            }
        },
    )
}

/// `find_similar_pages`.
pub fn find_similar_pages(exa: Arc<ExaClient>) -> Arc<dyn Tool> {
    HostTool::build(
        "find_similar_pages",
        "Find pages similar to a url, for broadening research around one good source.",
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "format": "uri" },
                "numResults": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let exa = Arc::clone(&exa);
            async move {
                let args = match parse::<SimilarArgs>(args) {
                    Ok(args) => args,
                    Err(refused) => return refused,
                };
                if !is_web_url(&args.url) {
                    return invalid(format!("{} is not a web url", args.url));
                }
                if let Err(refused) = check_results(args.num_results) {
                    return refused;
                }
                match exa.similar(&args.url, args.num_results).await {
                    Ok(hits) => ToolResult::success(render_hits(&hits)),
                    Err(error) => failed(error),
                }
            }
        },
    )
}

/// `web_answer`.
pub fn web_answer(exa: Arc<ExaClient>) -> Arc<dyn Tool> {
    HostTool::build(
        "web_answer",
        "Ask Exa a factual question and get a cited answer. Cheaper than searching and reading when one \
         specific fact is needed.",
        json!({
            "type": "object",
            "properties": { "question": { "type": "string", "minLength": 1 } },
            "required": ["question"],
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let exa = Arc::clone(&exa);
            async move {
                let args = match parse::<AnswerArgs>(args) {
                    Ok(args) => args,
                    Err(refused) => return refused,
                };
                if args.question.is_empty() {
                    return invalid("question must not be empty");
                }
                match exa.answer(&args.question).await {
                    Ok(answer) => ToolResult::success(render_answer(&answer)),
                    Err(error) => failed(error),
                }
            }
        },
    )
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
