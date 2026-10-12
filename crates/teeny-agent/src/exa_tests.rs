//! The Exa client, and the text the four web tools hand the model, as
//! legacy/src/agent/tools/exa.ts and tools/news.ts rendered it. The
//! rendering cases port legacy/test/agent-tools.test.ts' "Exa tools".

use serde_json::json;
use teeny_market::news::{NewsQuery, NewsSearch};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn exa(server: &MockServer) -> ExaClient {
    ExaClient::new(Secret::new("exa-key")).with_base_url(server.uri())
}

fn hit(title: Option<&str>, url: &str) -> ExaHit {
    ExaHit {
        title: title.map(str::to_owned),
        url: url.to_owned(),
        published_date: None,
        text: None,
        highlights: None,
    }
}

#[tokio::test]
async fn searches_with_contents_and_renders_titles_urls_and_excerpts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("x-api-key", "exa-key"))
        .and(body_json(json!({
            "query": "stonks.rocks",
            "numResults": 5,
            "category": "news",
            "startPublishedDate": "2026-01-01",
            "contents": { "text": { "maxCharacters": 1200 }, "highlights": true }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "title": "Launch notes",
                "url": "https://example.com/a",
                "publishedDate": "2026-01-02",
                "text": "the dapp shipped"
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let hits = exa(&server)
        .search(&WebSearchQuery {
            query: "stonks.rocks".into(),
            num_results: 5,
            category: Some("news".into()),
            start_published_date: Some("2026-01-01".into()),
        })
        .await
        .unwrap();
    assert_eq!(
        render_hits(&hits),
        "[1] Launch notes\nurl: https://example.com/a\npublished: 2026-01-02\nexcerpt: the dapp shipped"
    );
}

#[test]
fn prefers_highlights_cuts_long_excerpts_and_names_the_untitled() {
    let mut first = hit(None, "https://a");
    first.highlights = Some(vec!["one".into(), "two".into()]);
    first.text = Some("ignored".into());
    let mut second = hit(Some("Long"), "https://b");
    second.text = Some(format!("  {}", "x".repeat(1_300)));
    let third = hit(Some("Bare"), "https://c");
    let rendered = render_hits(&[first, second, third]);
    let blocks: Vec<&str> = rendered.split("\n\n").collect();
    assert_eq!(
        blocks[0],
        "[1] (untitled)\nurl: https://a\nexcerpt: one … two"
    );
    assert_eq!(
        blocks[1],
        format!("[2] Long\nurl: https://b\nexcerpt: {}", "x".repeat(1_198))
    );
    assert_eq!(blocks[2], "[3] Bare\nurl: https://c");
    assert_eq!(render_hits(&[]), "No results.");
}

#[tokio::test]
async fn reads_pages_by_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contents"))
        .and(body_json(json!({
            "urls": ["https://example.com/a"],
            "text": { "maxCharacters": 4000 }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "url": "https://example.com/a", "title": "A", "text": "body" }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let hits = exa(&server)
        .contents(&["https://example.com/a".to_owned()])
        .await
        .unwrap();
    assert_eq!(
        render_hits(&hits),
        "[1] A\nurl: https://example.com/a\nexcerpt: body"
    );
}

#[tokio::test]
async fn finds_similar_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/findSimilar"))
        .and(body_json(json!({
            "url": "https://example.com/a",
            "numResults": 3,
            "contents": { "text": { "maxCharacters": 1200 } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let hits = exa(&server)
        .similar("https://example.com/a", 3)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn answers_with_its_sources() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/answer"))
        .and(body_json(json!({ "query": "what" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answer": "42",
            "citations": [
                { "url": "https://example.com/b", "title": "B" },
                { "url": "https://example.com/c", "title": null }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let answer = exa(&server).answer("what").await.unwrap();
    assert_eq!(
        render_answer(&answer),
        "42\n\nSources:\n- B: https://example.com/b\n- https://example.com/c: https://example.com/c"
    );
    let bare = ExaAnswer {
        answer: "just this".into(),
        citations: Vec::new(),
    };
    assert_eq!(render_answer(&bare), "just this");
}

#[tokio::test]
async fn searches_the_news_as_the_crypto_news_tool_did() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(body_json(json!({
            "query": "crypto market news today",
            "numResults": 5,
            "type": "auto",
            "category": "news",
            "startPublishedDate": "2026-10-04T00:00:00.000Z",
            "contents": { "highlights": { "numSentences": 2, "highlightsPerUrl": 2 } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "url": "https://news/1", "title": "BTC up", "highlights": ["a", "b"] }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let stories = exa(&server)
        .search_news(&NewsQuery {
            query: "crypto market news today".into(),
            num_results: 5,
            start_published_date: "2026-10-04T00:00:00.000Z".into(),
        })
        .await
        .unwrap();
    assert_eq!(stories.len(), 1);
    assert_eq!(stories[0].title.as_deref(), Some("BTC up"));
    assert_eq!(stories[0].highlights, Some(vec!["a".into(), "b".into()]));
}

#[tokio::test]
async fn an_error_status_is_an_error_with_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/answer"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&server)
        .await;
    let error = exa(&server).answer("what").await.unwrap_err();
    assert!(error.to_string().contains("401"), "{error}");
    assert!(error.to_string().contains("bad key"), "{error}");
}
