use std::sync::Mutex;

use serde::Deserialize;

use super::*;
use crate::test_support::golden;

struct Search {
    answer: Result<Vec<NewsHit>, String>,
    asked: Mutex<Option<NewsQuery>>,
}

#[async_trait]
impl NewsSearch for Search {
    async fn search_news(&self, query: &NewsQuery) -> Result<Vec<NewsHit>, SearchError> {
        *self.asked.lock().unwrap() = Some(query.clone());
        self.answer.clone().map_err(Into::into)
    }
}

#[tokio::test]
async fn bounds_the_search_to_the_days_asked_for() {
    let search = Search {
        answer: Ok(vec![]),
        asked: Mutex::new(None),
    };
    let input = NewsInput {
        topic: None,
        days: 3,
        num_results: 5,
    };
    crypto_news(&search, &input, None).await;
    let asked = search.asked.lock().unwrap().clone().unwrap();
    assert_eq!(asked.query, "crypto market news today");
    let since = chrono::DateTime::parse_from_rfc3339(&asked.start_published_date).unwrap();
    let age = chrono::Utc::now().signed_duration_since(since);
    assert!(
        (age - chrono::Duration::days(3)).num_seconds().abs() < 60,
        "{age}"
    );
}

#[tokio::test]
async fn says_to_write_without_a_news_angle_rather_than_reach_for_an_old_story() {
    let search = Search {
        answer: Err("exa down".into()),
        asked: Mutex::new(None),
    };
    let text = crypto_news(&search, &NewsInput::default(), None)
        .await
        .text();
    assert!(
        text.contains("Write the post without a news angle"),
        "{text}"
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    label: String,
    hits: Option<Vec<NewsHit>>,
    error: Option<String>,
    input: NewsInput,
    default_topic: Option<String>,
    searched: Searched,
    output: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Searched {
    query: String,
    num_results: u32,
}

#[tokio::test]
async fn hands_the_model_exactly_the_text_the_typescript_did() {
    for case in golden::<Vec<Golden>>("news") {
        let answer = match (case.hits, case.error) {
            (Some(hits), _) => Ok(hits),
            (None, error) => Err(error.unwrap_or_default()),
        };
        let search = Search {
            answer,
            asked: Mutex::new(None),
        };
        let report = crypto_news(&search, &case.input, case.default_topic.as_deref()).await;
        assert_eq!(report.text(), case.output, "{}", case.label);
        let asked = search.asked.lock().unwrap().clone().unwrap();
        assert_eq!(
            (asked.query.as_str(), asked.num_results),
            (case.searched.query.as_str(), case.searched.num_results)
        );
    }
}
