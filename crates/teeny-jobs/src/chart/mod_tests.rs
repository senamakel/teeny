use std::sync::Arc;

use chrono::TimeDelta;
use serde_json::json;
use teeny_agent::posture::Belt;

use super::*;
use crate::morning::test_support::{Remembers, at, ctx_with};
use crate::runner::{AgentKey, AgentOutcome, Origin, ScriptedRunner, ToolCallRecord};
use teeny_cadence::rota::Rota;

fn posted(text: &str) -> AgentOutcome {
    AgentOutcome {
        reply: "posted the 1h read".into(),
        session_id: "s".into(),
        tool_calls: vec![
            ToolCallRecord {
                name: "render_chart".into(),
                arguments: json!({ "timeframe": "1h" }),
                result: "chart_1".into(),
                is_error: false,
            },
            ToolCallRecord {
                name: "post_chart".into(),
                arguments: json!({ "text": text, "chartId": "chart_1" }),
                result: "Published post 9001.".into(),
                is_error: false,
            },
        ],
        usage: None,
        model: None,
    }
}

/// A context on the chart's day of the rota, with `runner` drafting.
fn ctx(
    runner: Arc<ScriptedRunner>,
    memory: Arc<Remembers>,
) -> (JobCtx, Arc<teeny_cadence::clock::ManualClock>) {
    let probe = JobCtx::fake(Arc::new(ScriptedRunner::new()));
    let rota = Rota::from_config(&probe.config.rota).unwrap();
    let mut day = at("2026-09-08T15:00:00Z");
    while rota.slot(day) != probe.config.rota.chart_slot {
        day += TimeDelta::days(1);
    }
    let (ctx, clock) = ctx_with(day, Arc::default(), |deps| deps.memory = memory);
    let ctx = JobCtx::new(
        ctx.deps.clone(),
        runner,
        ctx.config.clone(),
        ctx.journal.clone(),
    )
    .with_clock(clock.clone());
    (ctx, clock)
}

#[tokio::test]
async fn runs_the_chart_agent_once_a_day_on_its_slot_and_journals_what_it_posted() {
    let runner = Arc::new(ScriptedRunner::new().outcome(posted("the level held")));
    let (ctx, clock) = ctx(
        runner.clone(),
        Remembers::saying("yesterday you posted about the 0.0007 reclaim"),
    );
    let report = run(&ctx).await.unwrap();
    assert!(report.ran);
    assert_eq!(report.outcome, Outcome::DryRun);
    assert_eq!(report.drafts, ["the level held"]);

    let seen = runner.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].agent, AgentKey::Belt(Belt::Chart));
    assert_eq!(seen[0].origin, Origin::Scheduled { once: true });
    assert!(
        seen[0]
            .prompt
            .starts_with("Post today's TINY chart. Start on the 1h timeframe")
    );
    assert!(
        seen[0]
            .prompt
            .contains("yesterday you posted about the 0.0007 reclaim")
    );
    assert!(seen[0].prompt.contains("THE TINY COMMUNITY"));
    let day = ctx.now().format("%Y-%m-%d").to_string();
    assert_eq!(
        seen[0].session.as_deref(),
        Some(format!("chart:{day}").as_str())
    );

    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.id, format!("chart:{day}"));
    assert_eq!(entry.trigger, Trigger::Chart);
    assert_eq!(entry.said, "the level held");
    assert_eq!(entry.thinking, "posted the 1h read");
    assert_eq!(entry.tools.len(), 2);

    clock.advance(TimeDelta::hours(1));
    assert!(!run(&ctx).await.unwrap().ran);
    assert_eq!(runner.seen().len(), 1);
}

#[tokio::test]
async fn asks_memory_about_the_community_by_name_and_runs_when_it_is_down() {
    let memory = Remembers::down();
    let runner = Arc::new(ScriptedRunner::new().outcome(posted("x")));
    let (ctx, _) = ctx(runner.clone(), memory.clone());
    run(&ctx).await.unwrap();
    assert!(memory.queries().join(" ").contains("TINY holders"));
    let prompt = runner.seen()[0].prompt.clone();
    assert!(prompt.contains(
        "nothing recalled -- treat today as your first post and do not pretend otherwise"
    ));
    assert!(prompt.contains("nothing recalled -- write for holders in general"));
}

#[tokio::test]
async fn journals_a_turn_that_ended_without_posting_as_skipped() {
    let runner = Arc::new(ScriptedRunner::new().reply("nothing worth saying today"));
    let (ctx, _) = ctx(runner, Remembers::saying(""));
    let report = run(&ctx).await.unwrap();
    assert_eq!(report.outcome, Outcome::Skipped);
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(
        entry.outcome_note.as_deref(),
        Some("the agent finished without posting")
    );
}

#[tokio::test]
async fn journals_a_failed_turn_and_returns_the_error() {
    let runner = Arc::new(ScriptedRunner::new().fail("ladder down"));
    let (ctx, _) = ctx(runner, Remembers::saying(""));
    assert!(run(&ctx).await.is_err());
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.outcome, Outcome::Failed);
}

#[tokio::test]
async fn the_rehearsal_ignores_the_rota_but_refuses_unless_dry_run_is_on() {
    let runner = Arc::new(ScriptedRunner::new().outcome(posted("the level held")));
    let (ctx, clock) = ctx(runner.clone(), Remembers::saying(""));
    clock.advance(TimeDelta::days(1));
    let report = dry_run(&ctx).await.unwrap();
    assert_eq!(report.drafts, ["the level held"]);
    assert!(ctx.journal.recent(5).await.unwrap().is_empty());

    let (live, _) = ctx_with(at("2026-09-08T15:00:00Z"), Arc::default(), |deps| {
        deps.settings.dry_run = false
    });
    assert!(
        dry_run(&live)
            .await
            .unwrap_err()
            .to_string()
            .contains("DRY_RUN")
    );
}
