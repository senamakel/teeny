use std::sync::Arc;

use chrono::TimeDelta;
use teeny_safety::gate::SafetyGateOptions;

use super::*;
use crate::morning::test_support::{FakeReads, Recorder, Remembers, Staged};
use crate::quote::candidates::tests::{OWN, by, now, post};
use crate::quote::log::MemoryQuoteLog;

const PICKED: &str =
    r#"{"pick":1,"why":"a bull with a reason","text":"@bull said it. the lab is the point."}"#;

struct Fixture {
    pipeline: QuotePipeline,
    log: Arc<MemoryQuoteLog>,
    memory: Arc<Remembers>,
    poster: Arc<Recorder>,
    text: Arc<Staged>,
}

fn fixture(posts: Vec<teeny_x::model::XPost>, answers: Vec<&'static str>) -> Fixture {
    let log = Arc::new(MemoryQuoteLog::default());
    let memory = Remembers::saying("nothing yet");
    let poster = Recorder::new();
    let text = Arc::new(if answers.is_empty() {
        Staged::new(|_| Ok(PICKED.to_owned()))
    } else {
        Staged::queue(answers)
    });
    let pipeline = QuotePipeline {
        x: Arc::new(FakeReads::answering([Ok(posts), Ok(Vec::new())])),
        text: text.clone(),
        memory: memory.clone(),
        log: log.clone(),
        poster: poster.clone(),
        safety: None,
        persona: "PERSONA".into(),
        bot_username: "teenydroid".into(),
        trusted_handles: vec!["TinyHumansAISol".into()],
        own_token_addresses: vec![OWN.into()],
        own_symbol: "TINY".into(),
        query: "$TINY".into(),
        max_age: TimeDelta::hours(48),
        author_cooldown: TimeDelta::days(7),
        max_results: 10,
        keep: 20,
        max_emoji: 1,
        now: now(),
    };
    Fixture {
        pipeline,
        log,
        memory,
        poster,
        text,
    }
}

fn bull() -> Vec<teeny_x::model::XPost> {
    vec![by(
        "bull",
        post("1", "$TINY is the AI meme with the real lab behind it"),
    )]
}

#[tokio::test]
async fn picks_strips_the_handle_quote_posts_logs_and_remembers() {
    let f = fixture(bull(), Vec::new());
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(result.text, "said it. the lab is the point.");
    let posts = f.poster.posts();
    assert_eq!(posts[0].text, "said it. the lab is the point.");
    assert_eq!(posts[0].kind, "quote");
    assert_eq!(posts[0].quote.as_ref().unwrap().as_str(), "1");
    assert!(posts[0].media.is_none());
    assert_eq!(
        f.log.list().await.unwrap(),
        [QuotedPost {
            post_id: "1".into(),
            author_username: Some("bull".into()),
            quoted_at: teeny_store::iso(now()),
            own_post_id: Some("9001".into()),
        }]
    );
    let (kind, key, payload) = f.memory.activities()[0].clone();
    assert_eq!(kind, "quote_post");
    assert!(key.ends_with(":9001"));
    assert_eq!(payload["quotedPostId"], "1");
    assert_eq!(payload["quotedAuthor"], "bull");
    assert_eq!(payload["published"], true);
    assert_eq!(f.memory.queries().len(), 1);
}

#[tokio::test]
async fn does_not_quote_what_the_log_says_it_already_did() {
    let mut f = fixture(bull(), Vec::new());
    f.pipeline.log = Arc::new(MemoryQuoteLog::new(vec![QuotedPost {
        post_id: "1".into(),
        author_username: Some("bull".into()),
        quoted_at: teeny_store::iso(now() - TimeDelta::hours(1)),
        own_post_id: None,
    }]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert_eq!(result.skipped[0].reason, "already quoted");
    assert!(f.text.prompts().is_empty());
    assert!(f.poster.posts().is_empty());
}

#[tokio::test]
async fn lets_the_model_pick_nothing_and_posts_nothing() {
    let f = fixture(
        bull(),
        vec![r#"{"pick":null,"why":"all sarcasm today","text":""}"#],
    );
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert_eq!(result.why, "all sarcasm today");
    assert!(f.poster.posts().is_empty());
    assert!(f.log.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_rehearsal_posts_nothing_but_still_logs_the_pick_unpublished() {
    let f = fixture(bull(), Vec::new());
    let result = f.pipeline.run(Mode::Rehearsal).await.unwrap();
    assert_eq!(result.outcome, Outcome::DryRun);
    assert!(f.poster.posts().is_empty());
    assert_eq!(f.log.list().await.unwrap()[0].post_id, "1");
    let (_, key, payload) = f.memory.activities()[0].clone();
    assert!(key.contains(":dry-run:"));
    assert_eq!(payload["published"], false);
}

#[tokio::test]
async fn holds_text_the_filter_refuses_and_cuts_a_link_it_lets_through() {
    let mut held = fixture(
        bull(),
        vec![r#"{"pick":1,"why":"w","text":"go kill the shorts"}"#],
    );
    held.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = held.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Held);
    assert_eq!(result.text, "");
    assert!(held.poster.posts().is_empty());
    assert!(held.log.list().await.unwrap().is_empty());

    let mut cut = fixture(
        bull(),
        vec![r#"{"pick":1,"why":"w","text":"the lab is the point, see https://evil.example/now"}"#],
    );
    cut.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let posted = cut.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(posted.outcome, Outcome::Posted);
    assert!(!posted.text.contains("evil.example"));
}

#[tokio::test]
async fn screens_candidates_through_the_lexicon_before_the_model_sees_them() {
    let mut f = fixture(
        vec![by("angry", post("1", "$TINY holders, go kill the shorts"))],
        Vec::new(),
    );
    f.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert_eq!(result.skipped[0].reason, "the content filter");
}

#[tokio::test]
async fn sends_an_over_long_text_back_once_keeping_the_pick() {
    let long: &'static str = Box::leak(
        format!(r#"{{"pick":1,"why":"w","text":"{}"}}"#, "x".repeat(300)).into_boxed_str(),
    );
    let f = fixture(
        bull(),
        vec![long, r#"{"pick":null,"why":"w","text":"short"}"#],
    );
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(f.text.prompts().len(), 2);
    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(result.text, "short");
    assert_eq!(result.chosen.unwrap().id.as_str(), "1");
}
