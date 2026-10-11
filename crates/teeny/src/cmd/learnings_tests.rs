use std::sync::Arc;

use serde_json::json;
use teeny_jobs::ctx::JobCtx;
use teeny_jobs::learnings::LEARNINGS_SECTIONS;
use teeny_jobs::runner::ScriptedRunner;

use super::*;
use crate::cmd::test_support::printed;

fn answer() -> String {
    let sections: serde_json::Map<String, serde_json::Value> = LEARNINGS_SECTIONS
        .iter()
        .map(|section| ((*section).to_owned(), json!(["something true"])))
        .collect();
    json!({ "sections": sections }).to_string()
}

fn ctx(path: &std::path::Path) -> JobCtx {
    let mut ctx = JobCtx::fake(Arc::new(ScriptedRunner::new().reply(answer())));
    let mut config = (*ctx.config).clone();
    config.learnings.path = path.display().to_string();
    ctx.config = Arc::new(config);
    ctx
}

fn scratch(name: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("teeny-cmd-learnings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root.join("LEARNINGS.md")
}

#[tokio::test]
async fn the_rehearsal_prints_the_draft_and_writes_nothing() {
    let path = scratch("dry");
    let mut out = Vec::new();
    run(&ctx(&path), true, &mut out).await.unwrap();
    let out = printed(out);
    assert!(out.contains("drafted, not written"), "{out}");
    assert!(
        out.contains("# Learnings\n\n## Who I am right now\n- something true"),
        "{out}"
    );
    assert!(!path.exists());
}

#[tokio::test]
async fn writes_the_brief_now_and_prints_it() {
    let path = scratch("write");
    let mut out = Vec::new();
    run(&ctx(&path), false, &mut out).await.unwrap();
    let out = printed(out);
    assert!(
        out.contains(&format!("written to {}", path.display())),
        "{out}"
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("# Learnings")
    );
}

#[tokio::test]
async fn says_why_it_kept_the_previous_brief() {
    let path = scratch("kept");
    let mut ctx = ctx(&path);
    ctx.runner = Arc::new(ScriptedRunner::new().reply("an essay").reply("an essay"));
    let mut out = Vec::new();
    run(&ctx, false, &mut out).await.unwrap();
    assert!(printed(out).contains("kept the previous brief: the draft was not a brief"));
}

#[test]
fn the_command_writes_the_brief_even_when_the_daily_job_is_off() {
    let mut config = crate::cmd::test_support::config(true);
    config.learnings.enabled = false;
    assert!(for_command(config, false).learnings.enabled);
}
