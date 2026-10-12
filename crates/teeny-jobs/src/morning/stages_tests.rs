use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;
use teeny_media::reference::TextModel;

use super::*;

/// A model that answers from a script and remembers every prompt.
#[derive(Default)]
struct Script {
    answers: Mutex<VecDeque<Result<String, String>>>,
    prompts: Mutex<Vec<String>>,
}

impl Script {
    fn new(answers: impl IntoIterator<Item = Result<&'static str, &'static str>>) -> Self {
        Self {
            answers: Mutex::new(
                answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned).map_err(str::to_owned))
                    .collect(),
            ),
            prompts: Mutex::default(),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

#[async_trait]
impl TextModel for Script {
    async fn complete(&self, _system: Option<&str>, prompt: &str) -> teeny_media::Result<String> {
        self.prompts.lock().unwrap().push(prompt.to_owned());
        match self.answers.lock().unwrap().pop_front() {
            Some(Ok(text)) => Ok(text),
            Some(Err(error)) => Err(teeny_media::Error::failed(error)),
            None => Err(teeny_media::Error::failed("script exhausted")),
        }
    }
}

const PICK: &str = r#"{"story": 3, "headline": "h", "topic": "ai", "angle": "a", "visual": "v"}"#;

#[test]
fn finds_the_first_balanced_object_in_prose_or_a_fence_and_nothing_else() {
    assert_eq!(
        extract_json("Sure! ```json\n{\"a\": 1, \"b\": {\"c\": \"}\"}}\n```"),
        Some(json!({ "a": 1, "b": { "c": "}" } }))
    );
    assert_eq!(extract_json("no json here"), None);
    assert_eq!(extract_json("{\"broken\": "), None);
}

#[tokio::test]
async fn sends_a_rejected_shape_back_once_with_the_complaint() {
    let model = Script::new([Ok(r#"{"story": "three"}"#), Ok(PICK)]);
    let pick: Pick = ask_json(&model, "s", "p").await.unwrap();
    assert_eq!(pick.story, 3);
    let prompts = model.prompts();
    assert!(prompts[0].ends_with(
        "Answer with one JSON object and nothing else: no prose before or after, no code fence."
    ));
    assert!(
        prompts[1].contains("previous answer was rejected (story:"),
        "{}",
        prompts[1]
    );
}

#[tokio::test]
async fn tries_a_failed_call_once_more_with_the_same_prompt() {
    let flaky = Script::new([Err("timed out"), Ok(PICK)]);
    let pick: Pick = ask_json(&flaky, "s", "p").await.unwrap();
    assert_eq!(pick.story, 3);
    let prompts = flaky.prompts();
    assert_eq!(prompts[0], prompts[1]);

    let dead = Script::new([Err("502"), Err("502")]);
    let error = ask_json::<Pick>(&dead, "s", "p").await.unwrap_err();
    assert!(error.to_string().contains("502"));
    assert_eq!(dead.prompts().len(), 2);
}

#[tokio::test]
async fn gives_up_after_two_answers_without_json() {
    let hopeless = Script::new([Ok("nope"), Ok("nope")]);
    let error = ask_json::<Pick>(&hopeless, "s", "p").await.unwrap_err();
    assert!(error.to_string().contains("no JSON object"), "{error}");
    assert!(error.to_string().contains("the answer ended: \"nope\""));
    assert_eq!(hopeless.prompts().len(), 2);
}

#[test]
fn reads_a_pick_with_its_defaults_and_cuts_a_long_shortlist_line() {
    let long = "x".repeat(400);
    let value = json!({
        "shortlist": [{ "story": 2, "talk": long }],
        "story": 0, "headline": "h", "topic": "culture", "angle": "a", "visual": "v",
        "fallback": "our own thing"
    });
    let pick = Pick::read(&value).unwrap();
    assert_eq!(pick.shortlist[0].talk.len(), 300);
    assert_eq!(pick.hook, "");
    assert_eq!(pick.topic, "culture");
    assert_eq!(pick.fallback.as_deref(), Some("our own thing"));
}

#[test]
fn refuses_a_pick_off_the_topic_list_or_missing_a_field() {
    let issues =
        Pick::read(&json!({ "story": 1, "headline": "h", "topic": "politics", "angle": "a" }))
            .unwrap_err();
    assert!(
        issues.iter().any(|issue| issue.starts_with("topic:")),
        "{issues:?}"
    );
    assert!(
        issues.iter().any(|issue| issue == "visual: Required"),
        "{issues:?}"
    );
}

#[test]
fn measures_lengths_as_javascript_did() {
    // Twenty UTF-16 units: ten astral emoji.
    let scene = "🌅".repeat(10);
    let value = json!({ "scene": scene, "storyCue": "a cue", "alt": "an alt" });
    assert!(Scene::read(&value).is_ok());
    assert_eq!(Scene::read(&value).unwrap().concepts, Vec::<String>::new());
}

#[test]
fn reads_freshness_and_the_picture_check() {
    let fresh = Freshness::read(&json!({ "reason": "r", "verdict": "stale" })).unwrap();
    assert_eq!(fresh.verdict, Verdict::Stale);
    assert_eq!(fresh.said, "");
    let check = Check::read(&json!({ "ok": false })).unwrap();
    assert!(!check.ok);
    assert_eq!(check.reason, "");
}
