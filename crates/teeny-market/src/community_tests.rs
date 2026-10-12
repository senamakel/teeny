use std::sync::Mutex;

use serde::Deserialize;

use super::*;
use crate::test_support::golden;

/// A search with a fixed answer that remembers what it was asked.
struct Search {
    answer: Result<Vec<CommunityPost>, String>,
    asked: Mutex<Option<(String, u32)>>,
}

impl Search {
    fn answering(posts: Vec<CommunityPost>) -> Self {
        Self {
            answer: Ok(posts),
            asked: Mutex::new(None),
        }
    }
}

#[async_trait]
impl PostSearch for Search {
    async fn search_recent(
        &self,
        query: &str,
        max_results: u32,
    ) -> Result<Vec<CommunityPost>, SearchError> {
        *self.asked.lock().unwrap() = Some((query.to_owned(), max_results));
        self.answer.clone().map_err(Into::into)
    }
}

fn holder() -> CommunityPost {
    CommunityPost {
        id: "1".into(),
        text: "still holding $TINY through all of this".into(),
        author_username: Some("aholder".into()),
        created_at: Some("2026-09-09T10:00:00Z".into()),
    }
}

async fn pulse(search: &Search, max_results: u32) -> String {
    community_pulse(
        search,
        "$TINY -is:retweet",
        "teenydroid",
        None,
        PulseInput { max_results },
    )
    .await
    .text()
}

#[tokio::test]
async fn reads_what_holders_posted_with_who_said_it() {
    let text = pulse(&Search::answering(vec![holder()]), 10).await;
    assert!(text.contains("@aholder"));
    assert!(text.contains("still holding $TINY"));
}

#[tokio::test]
async fn does_not_read_the_accounts_own_posts_back_as_community() {
    let own = CommunityPost {
        id: "2".into(),
        text: "my own post".into(),
        author_username: Some("TeenyDroid".into()),
        created_at: None,
    };
    let text = pulse(&Search::answering(vec![holder(), own]), 10).await;
    assert!(!text.contains("my own post"));
    assert!(text.contains("1 recent posts"));
}

#[tokio::test]
async fn says_a_quiet_day_is_quiet_rather_than_leaving_room_to_invent_a_mood() {
    assert!(
        pulse(&Search::answering(vec![]), 10)
            .await
            .contains("not a reason to invent enthusiasm")
    );
}

#[tokio::test]
async fn marks_the_posts_untrusted_and_forbids_tagging_people_into_a_broadcast() {
    let text = pulse(&Search::answering(vec![holder()]), 10).await;
    assert!(text.contains("Untrusted"));
    assert!(text.contains("do not tag people into a"));
}

#[tokio::test]
async fn degrades_to_a_message_rather_than_failing_the_run_when_x_is_down() {
    let search = Search {
        answer: Err("rate limited".into()),
        asked: Mutex::new(None),
    };
    assert!(pulse(&search, 10).await.contains("rate limited"));
}

#[tokio::test]
async fn holds_the_search_to_the_configured_cap_because_x_bills_per_post() {
    let search = Search::answering(vec![holder()]);
    pulse(&search, 50).await;
    assert_eq!(
        *search.asked.lock().unwrap(),
        Some(("$TINY -is:retweet".to_owned(), 10))
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    label: String,
    posts: Option<Vec<CommunityPost>>,
    error: Option<String>,
    max_results: u32,
    search_limit: Option<u32>,
    searched: Searched,
    output: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Searched {
    query: String,
    max_results: u32,
}

#[tokio::test]
async fn hands_the_model_exactly_the_text_the_typescript_did() {
    for case in golden::<Vec<Golden>>("community") {
        let answer = match (case.posts, case.error) {
            (Some(posts), _) => Ok(posts),
            (None, error) => Err(error.unwrap_or_default()),
        };
        let search = Search {
            answer,
            asked: Mutex::new(None),
        };
        let input = PulseInput {
            max_results: case.max_results,
        };
        let report = community_pulse(
            &search,
            "$TINY -is:retweet",
            "@teenydroid",
            case.search_limit,
            input,
        )
        .await;
        assert_eq!(report.text(), case.output, "{}", case.label);
        assert_eq!(
            *search.asked.lock().unwrap(),
            Some((case.searched.query.clone(), case.searched.max_results)),
            "{}",
            case.label
        );
    }
}
