use serde::Deserialize;

use super::*;
use crate::backrooms::testing::ScriptedText;

#[derive(Debug, Deserialize, PartialEq)]
struct Brief {
    brief: String,
}

impl Shape for Brief {
    fn check(&self) -> Result<(), String> {
        required("brief", &self.brief)
    }
}

#[tokio::test]
async fn reads_the_first_object_in_a_chatty_answer() {
    let model = ScriptedText::new(|_, _| "sure! {\"brief\": \"a {curly} thing\"} done".into());
    let brief: Brief = ask_json(&model, &Ask::new("s", "p")).await.unwrap();
    assert_eq!(brief.brief, "a {curly} thing");
    let calls = model.calls();
    assert_eq!(calls[0].0, "s");
    assert!(calls[0].1.ends_with("no code fence."));
}

#[tokio::test]
async fn asks_once_more_with_the_reason_and_then_gives_up() {
    let model = ScriptedText::queue(["{\"brief\": \"\"}".into(), "{\"brief\": \"ok\"}".into()]);
    let brief: Brief = ask_json(&model, &Ask::new("s", "p")).await.unwrap();
    assert_eq!(brief.brief, "ok");
    assert!(model.calls()[1].1.contains("rejected (brief: required)"));

    let never = ScriptedText::new(|_, _| "no json here".into());
    let error = ask_json::<Brief>(&never, &Ask::new("s", "p"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no JSON object"));
}

#[test]
fn extracts_nothing_from_an_unbalanced_answer() {
    assert!(extract_json("{\"a\": 1").is_none());
    assert_eq!(extract_json("x {\"a\": \"}\"} y").unwrap()["a"], "}");
}
