use std::sync::Arc;

use chrono::{TimeZone, Utc};
use serde_json::json;
use teeny_cadence::clock::ManualClock;

use super::*;
use crate::backrooms::testing::{FixedMemory, ScriptedText, scratch, seed};
use crate::runner::{AgentKey, Origin, ScriptedRunner};

const THOUGHT: &str = r#"{"reasons":"r","premise":"the agents measured the door.","wire":"a car","objects":["the door"],"thought":"the door is one inch open. the agents measured it.","post":"the door is one inch open.","kept":[]}"#;

pub(crate) fn backrooms(runner: Arc<ScriptedRunner>, editor: Arc<ScriptedText>, dir: Option<PathBuf>) -> Backrooms {
    Backrooms::new(
        runner,
        ContextSources {
            memory: Some(Arc::new(FixedMemory::default())),
            x: None,
            web: None,
            forums: Vec::new(),
            voice: "rules the terminal enforces:\n- never wink.".into(),
            lore: "# lore\n\n## engines\n\n- the kettle: it boils.".into(),
            bot_username: "teenydroid".into(),
        },
        editor.clone(),
        editor,
        RoomDefaults { agents: vec!["teeny".into(), "swarm".into()], turns: 4, budget: 4_000, max_tool_calls: 3 },
        GmSources {
            memory: None,
            x: None,
            persona: "smol yellow robot".into(),
            lore: "# lore".into(),
            bot_username: "teenydroid".into(),
            max_emoji: 1,
            history_query: None,
        },
        Arc::new(ManualClock::new(Utc.with_ymd_and_hms(2026, 9, 21, 13, 35, 0).unwrap())),
    )
    .with_store_dir(dir)
}

#[tokio::test]
async fn opens_a_room_of_persona_agents_reads_it_down_to_a_thought_and_keeps_the_notebook() {
    let runner = Arc::new(
        ScriptedRunner::new()
            .reply("the door is one inch open")
            .reply("we counted the inch")
            .reply("the ledger disagrees")
            .reply("the kettle boils"),
    );
    let editor = Arc::new(ScriptedText::new(|_, _| THOUGHT.to_owned()));
    let dir = scratch("backrooms-wiring");
    let rooms = backrooms(runner.clone(), editor.clone(), Some(dir.clone()));
    let result = rooms
        .run(&seed(), &RunOptions { rng: Some(42), ..RunOptions::default() })
        .await
        .unwrap();
    assert_eq!(result.run.turns.len(), 4);
    assert_eq!(result.run.id, "20260921-1335-a-car-pulled-up-outside-the-lab-at-3am-16");
    assert_eq!(result.thought.unwrap().premise, "the agents measured the door.");
    let seen = runner.seen();
    assert_eq!(seen.len(), 4);
    for (turn, persona) in seen.iter().zip(["teeny", "swarm", "teeny", "swarm"]) {
        match &turn.agent {
            AgentKey::Room { persona: who, system_prompt } => {
                assert_eq!(who, persona);
                assert!(system_prompt.contains("## what came through the wire"));
            }
            other => panic!("a room turn ran as {other:?}"),
        }
        assert_eq!(turn.origin, Origin::Scheduled { once: true });
        assert_eq!(turn.session.as_deref(), Some(format!("backrooms:{}:{persona}", result.run.id).as_str()));
    }
    let saved = result.saved.unwrap();
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(saved.json).unwrap()).unwrap();
    assert_eq!(json["thought"]["premise"], json!("the agents measured the door."));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn refuses_an_agent_it_does_not_know() {
    let rooms = backrooms(Arc::new(ScriptedRunner::new()), Arc::new(ScriptedText::new(|_, _| String::new())), None);
    let error = rooms
        .run(&seed(), &RunOptions { agents: Some(vec!["ghost".into()]), ..RunOptions::default() })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no such backrooms agent: ghost"));
}
