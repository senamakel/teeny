use std::sync::Arc;

use serde_json::json;
use teeny_core::Config;

use super::*;
use crate::runner::{AgentKey, Origin, ScriptedRunner};

fn answer() -> String {
    let sections: serde_json::Map<String, serde_json::Value> = LEARNINGS_SECTIONS
        .iter()
        .map(|section| ((*section).to_owned(), json!(["something true"])))
        .collect();
    json!({ "sections": sections }).to_string()
}

fn ctx(runner: Arc<ScriptedRunner>, path: &std::path::Path) -> JobCtx {
    let mut ctx = JobCtx::fake(runner);
    let vars = teeny_agent::deps::fake_vars()
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([("LEARNINGS_PATH".to_owned(), path.display().to_string())]);
    ctx.config = Arc::new(Config::from_vars(vars).unwrap());
    ctx
}

fn scratch(name: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("teeny-learnings-job-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root.join("memory").join("LEARNINGS.md")
}

#[tokio::test]
async fn asks_the_learner_on_a_scheduled_turn_and_writes_the_brief() {
    let path = scratch("run");
    let runner = Arc::new(ScriptedRunner::new().reply(answer()));
    let ctx = ctx(runner.clone(), &path);
    assert!(needs_first_brief(&ctx));

    let report = run(&ctx).await.unwrap();
    assert!(report.ran);
    assert_eq!(report.outcome, Outcome::Posted);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("# Learnings")
    );
    assert!(!needs_first_brief(&ctx));

    let turn = &runner.seen()[0];
    match &turn.agent {
        AgentKey::Learner { system_prompt } => {
            assert!(system_prompt.contains("You maintain LEARNINGS.md"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(turn.origin, Origin::Scheduled { once: true });
    assert_eq!(turn.label, "learnings");
    assert!(turn.prompt.contains("THE PREVIOUS BRIEF:"));
}

#[tokio::test]
async fn the_rehearsal_returns_the_draft_and_writes_nothing() {
    let path = scratch("dry");
    let runner = Arc::new(ScriptedRunner::new().reply(answer()));
    let report = dry_run(&ctx(runner, &path)).await.unwrap();
    assert_eq!(report.outcome, Outcome::DryRun);
    assert!(report.drafts[0].starts_with("# Learnings\n\n## Who I am right now"));
    assert!(!path.exists());
}

#[tokio::test]
async fn a_draft_that_is_not_a_brief_is_a_skip_not_a_failure() {
    let path = scratch("kept");
    let runner = Arc::new(
        ScriptedRunner::new()
            .reply("Sure! Here is an essay.")
            .reply("Still an essay."),
    );
    let report = run(&ctx(runner, &path)).await.unwrap();
    assert_eq!(report.outcome, Outcome::Skipped);
    assert!(report.note.unwrap().contains("not a brief"));
    assert!(!path.exists());
}

#[tokio::test]
async fn is_off_when_the_learnings_are() {
    let path = scratch("off");
    let runner = Arc::new(ScriptedRunner::new());
    let mut ctx = ctx(runner.clone(), &path);
    let mut config = (*ctx.config).clone();
    config.learnings.enabled = false;
    ctx.config = Arc::new(config);
    assert!(!needs_first_brief(&ctx));
    let report = run(&ctx).await.unwrap();
    assert!(!report.ran);
    assert!(runner.seen().is_empty());
}
