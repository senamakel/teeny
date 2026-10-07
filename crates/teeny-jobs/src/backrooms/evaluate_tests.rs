use serde_json::json;

use super::*;
use crate::backrooms::distill::{DEFAULT_EDITOR, distill, distill_prompt};
use crate::backrooms::testing::{ScriptedText, run};

#[test]
fn counts_what_cannot_be_argued_with() {
    let counted = stats(&run());
    assert_eq!(counted.turns, 4);
    assert_eq!(counted.repeated_lines, 1);
    assert_eq!(counted.tool_calls["teeny"], 0);
    assert_eq!(counted.tool_calls["swarm"], 1);
    assert_eq!(counted.turns_using_tools, 1);
    assert_eq!(counted.price_mentions, 1);
    assert_eq!(counted.winks, 1);
    assert_eq!(counted.one_liners, 3);
}

#[tokio::test]
async fn asks_a_cold_model_for_the_craziness_and_reads_its_answer() {
    let answer = json!({
        "reasons": "it circled",
        "scores": { "novelty": 1, "surprise": 0, "escalation": 0, "voice": 2, "coherence": 3, "wildness": 1, "toolPayoff": 2 },
        "quotables": ["the door is one inch open"],
        "deadSpots": [2],
        "verdict": "hotter"
    });
    let text = answer.to_string();
    let model = ScriptedText::new(move |_, _| text.clone());
    let read = judge(&run(), &model).await.unwrap();
    assert_eq!(read.scores.tool_payoff, 2.0);
    assert_eq!(read.dead_spots, [2]);
    assert_eq!(craziness_total(&read), 9.0);
    let prompt = &model.calls()[0].1;
    assert!(prompt.contains(
        "1. swarm [looked up: x_search_posts {\"query\":\"inch\"}]: we counted"
    ));
    assert!(prompt.contains("2. teeny (mood → giddy)"));

    let mut over = answer.clone();
    over["scores"]["novelty"] = json!(4);
    let over = over.to_string();
    let refuses = ScriptedText::new(move |_, _| over.clone());
    assert!(judge(&run(), &refuses).await.is_err());
}

#[tokio::test]
async fn reads_the_room_down_to_one_thought_a_stranger_can_follow() {
    let answer = json!({
        "reasons": "kept the inch, dropped the rest",
        "premise": "a robot learns the door was never locked.",
        "wire": "the car outside is why the door is open",
        "objects": ["the door", "the ledger"],
        "thought": "the door is one inch open. the agents measured it. i stop believing in the gap.",
        "post": "the door is one inch open. i stop believing in the gap.",
        "kept": ["the door is one inch open"]
    });
    let text = answer.to_string();
    let model = ScriptedText::new(move |_, _| text.clone());
    let thought = distill(&run(), &model, None).await.unwrap();
    assert_eq!(thought.objects, ["the door", "the ledger"]);
    let (system, prompt) = &model.calls()[0];
    assert!(system.contains("editor for teeny"));
    assert!(system.contains("no \"nfa\""));
    assert!(prompt.contains("0. teeny: the door is one inch open"));
    // The editor's instructions can be swapped for the file's.
    assert_eq!(distill_prompt(&run(), None).system, DEFAULT_EDITOR);
    assert_eq!(distill_prompt(&run(), Some("be brief")).system, "be brief");

    let mut long = answer.clone();
    long["post"] = json!("x".repeat(281));
    let long = long.to_string();
    let refuses = ScriptedText::new(move |_, _| long.clone());
    assert!(distill(&run(), &refuses, None).await.is_err());
}

#[test]
fn renders_a_turn_for_a_reader() {
    let run = run();
    assert_eq!(
        render_turn(&run.turns[1], 1),
        "### 2. swarm\n\n> looked up · x_search_posts · \"inch\"\n\nwe counted the inch. the inch is twelve millimetres. the ledger disagrees.\n"
    );
    assert!(render_turn(&run.turns[2], 2).starts_with("### 3. teeny · mood → giddy"));
}
