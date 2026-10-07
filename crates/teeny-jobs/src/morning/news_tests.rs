use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::json;
use teeny_agent::exa::ExaClient;
use teeny_core::Secret;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn story(title: &str) -> Story {
    Story {
        title: title.into(),
        url: format!(
            "https://news.example/{}",
            title.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-")
        ),
        source: "news.example".into(),
        published_at: None,
        excerpt: String::new(),
        topic: "ai".into(),
        kind: StoryKind::News,
        metrics: None,
        media: None,
    }
}

fn with_excerpt(title: &str, excerpt: &str) -> Story {
    Story {
        excerpt: excerpt.into(),
        ..story(title)
    }
}

#[test]
fn drops_governments_wars_and_named_politicians_wherever_the_word_sits() {
    assert!(is_political(&story("Senate passes AI bill")));
    assert!(is_political(&with_excerpt(
        "New model released",
        "The White House responded."
    )));
    assert!(is_political(&story("Trump on crypto")));
    assert!(is_political(&story("Chip tariffs land")));
    assert!(is_political(&story("War in the east")));
    assert!(is_political(&with_excerpt(
        "Washington scrambles to meet calls for AI guardrails",
        "Lawmakers are seeking to address..."
    )));
    assert!(is_political(&story("Feds Have Now Frozen $938M in Scam Crypto")));
}

#[test]
fn keeps_ordinary_stories_and_matches_whole_words_only() {
    assert!(!is_political(&story("Open-source agent framework hits 40k stars")));
    assert!(!is_political(&story("Solana devs ship a new runtime")));
    assert!(!is_political(&story("Robots sort a warehouse")));
    assert!(!is_political(&story("A trumpet-playing robot")));
}

struct Scripted {
    answers: Mutex<VecDeque<Result<Vec<Story>, String>>>,
    asked: Mutex<Vec<(String, u32, u32)>>,
}

#[async_trait::async_trait]
impl NewsSource for Scripted {
    async fn search(&self, topic: &str, days: u32, limit: u32) -> Result<Vec<Story>, String> {
        self.asked.lock().unwrap().push((topic.into(), days, limit));
        self.answers.lock().unwrap().pop_front().unwrap_or(Ok(Vec::new()))
    }
    async fn read(&self, _url: &str, _max_characters: usize) -> String {
        String::new()
    }
}

#[tokio::test]
async fn searches_every_topic_drops_politics_folds_duplicates_and_survives_a_failure() {
    let source = Scripted {
        answers: Mutex::new(VecDeque::from([
            Ok(vec![story("A new open model"), story("Congress hearing on AI")]),
            Err("exa down".to_owned()),
            Ok(vec![
                Story {
                    url: "https://news.example/a-new-open-model?utm=x".into(),
                    ..story("A new open model")
                },
                story("Something about phones"),
            ]),
        ])),
        asked: Mutex::default(),
    };
    let topics = ["ai".to_owned(), "crypto".to_owned(), "tech".to_owned()];
    let (stories, dropped) = gather_news(&source, &topics, 2, 5).await;
    let titles: Vec<_> = stories.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["A new open model", "Something about phones"]);
    assert_eq!(dropped, 1);
    let asked = source.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 3);
    assert_eq!(asked[1], ("crypto".to_owned(), 2, 5));
}

#[test]
fn numbers_the_stories_from_one() {
    let rendered = render_stories(
        &[
            Story {
                published_at: Some("2026-09-14T06:00:00Z".into()),
                ..with_excerpt("First", "It happened.")
            },
            Story {
                topic: "crypto".into(),
                ..story("Second")
            },
        ],
        1,
    );
    assert!(rendered.contains("[1] First\n    news.example, 2026-09-14 (ai)\n    It happened."));
    assert!(rendered.contains("[2] Second\n    news.example (crypto)"));
}

#[tokio::test]
async fn asks_exa_for_recent_news_and_reads_what_comes_back() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(body_partial_json(json!({ "query": "ai news", "numResults": 4, "category": "news" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [
            { "title": " A story ", "url": "https://www.example.com/a", "publishedDate": "2026-09-14", "highlights": ["one", "two"] },
            { "title": null, "url": "https://example.com/untitled" }
        ] })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/contents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [
            { "url": "https://www.example.com/a", "text": "The  full\n article." }
        ] })))
        .mount(&server)
        .await;
    let source = ExaNewsSource::new(std::sync::Arc::new(
        ExaClient::new(Secret::new("k")).with_base_url(server.uri()),
    ));
    assert_eq!(source.read("https://www.example.com/a", 500).await, "The full article.");
    let found = source.search("ai news", 2, 4).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "A story");
    assert_eq!(found[0].source, "example.com");
    assert_eq!(found[0].published_at.as_deref(), Some("2026-09-14"));
    assert_eq!(found[0].excerpt, "one … two");
    assert_eq!(found[0].topic, "ai news");
    let sent: serde_json::Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[1].body).unwrap();
    assert!(sent["startPublishedDate"].is_string());
}
