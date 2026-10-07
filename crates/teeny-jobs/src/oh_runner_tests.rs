//! The runner over the real runtime and a scripted provider
//! (`teeny_agent::testkit`), plus the pure parts: which origin a turn runs
//! with, how often it may be tried, and what its progress events add up to.

use std::sync::Arc;
use std::time::Duration;

use openhuman_embed::agent_progress::AgentProgress;
use openhuman_embed::{AgentTurnOrigin, TrustedAutomationSource};
use serde_json::json;
use teeny_agent::posture::Belt;
use teeny_agent::testkit::{self, FINAL_REPLY, SLOW, booted, call, chat_requests};
use teeny_store::inference::{Lane, RecentFilter};
use teeny_store::{InMemoryLedger, InferenceLedger};

use super::*;
use crate::runner::{AgentKey, AgentRun, AgentRunner, Origin, RunError};

fn turn(agent: AgentKey, origin: Origin, prompt: &str, label: &str) -> AgentRun {
    AgentRun {
        agent,
        origin,
        prompt: prompt.to_owned(),
        session: None,
        model: None,
        label: label.to_owned(),
    }
}

fn runner(ledger: &Arc<InMemoryLedger>) -> OhRunner {
    OhRunner::new(Arc::clone(&booted().agents))
        .with_ledger(Arc::clone(ledger) as Arc<dyn InferenceLedger>)
}

async fn rows(ledger: &InMemoryLedger) -> Vec<teeny_store::Inference> {
    ledger.recent(100, &RecentFilter::default()).await.unwrap()
}

#[test]
fn a_public_turn_runs_as_the_public_channel() {
    assert!(matches!(
        turn_origin(Origin::Public, "reply"),
        AgentTurnOrigin::ExternalChannel { channel, .. } if channel == "public"
    ));
}

#[test]
fn a_scheduled_turn_runs_as_cron_named_by_its_label() {
    for once in [true, false] {
        match turn_origin(Origin::Scheduled { once }, "calendar.gm") {
            AgentTurnOrigin::TrustedAutomation { job_id, source } => {
                assert_eq!(job_id, "teeny:calendar.gm");
                assert!(matches!(source, TrustedAutomationSource::Cron));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn an_operator_turn_runs_as_trusted_automation_without_approval() {
    match turn_origin(Origin::Operator, "agent.run") {
        AgentTurnOrigin::TrustedAutomation { job_id, source } => {
            assert_eq!(job_id, "teeny:agent.run");
            assert!(matches!(
                source,
                TrustedAutomationSource::Workflow {
                    require_approval: false
                }
            ));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn only_a_scheduled_turn_that_may_repeat_is_tried_twice() {
    assert_eq!(attempts(Origin::Scheduled { once: true }), 1);
    assert_eq!(attempts(Origin::Scheduled { once: false }), 2);
    assert_eq!(attempts(Origin::Public), 1);
    assert_eq!(attempts(Origin::Operator), 1);
}

fn started(id: &str, tool: &str) -> AgentProgress {
    AgentProgress::ToolCallStarted {
        call_id: id.into(),
        tool_name: tool.into(),
        arguments: json!({"q": tool}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    }
}

fn completed(id: &str, tool: &str, success: bool, output: &str) -> AgentProgress {
    AgentProgress::ToolCallCompleted {
        call_id: id.into(),
        tool_name: tool.into(),
        success,
        output_chars: output.len(),
        output: output.into(),
        arguments: None,
        elapsed_ms: 3,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    }
}

fn model_call(model: &str, input: u64, output: u64, reasoning: u64) -> AgentProgress {
    AgentProgress::ModelCallCompleted {
        model: model.into(),
        provider_id: "openai".into(),
        subagent_task_id: None,
        input: None,
        output: None,
        iteration: 1,
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: 2,
        cache_creation_tokens: 0,
        reasoning_tokens: reasoning,
        cost_usd: 0.0,
    }
}

#[test]
fn progress_adds_up_to_the_tool_calls_and_the_model_calls() {
    let mut progress = Progress::default();
    for event in [
        started("a", "recall_memory"),
        model_call("chat", 10, 4, 1),
        completed("a", "recall_memory", true, "nothing recalled"),
        started("b", "x_post"),
        completed("b", "x_post", false, "Refused: public turn"),
        model_call("chat", 20, 6, 2),
    ] {
        progress.observe(event);
    }
    let calls = progress.tool_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "recall_memory");
    assert_eq!(calls[0].arguments, json!({"q": "recall_memory"}));
    assert_eq!(calls[0].result, "nothing recalled");
    assert!(!calls[0].is_error);
    assert!(calls[1].is_error);
    assert_eq!(progress.model_calls().len(), 2);
    assert_eq!(
        progress.usage(),
        Some(TurnUsage {
            input: 30,
            output: 10,
            reasoning: 3,
            cached: 4
        })
    );
    assert_eq!(progress.model().as_deref(), Some("chat"));
}

#[test]
fn a_turn_with_no_model_call_reports_no_usage() {
    assert_eq!(Progress::default().usage(), None);
}

#[test]
fn a_public_reply_turn_collects_its_tool_calls_usage_and_ledger_rows() {
    let ledger = Arc::new(InMemoryLedger::default());
    let runner = runner(&ledger);
    let outcome = testkit::run({
        let ledger = Arc::clone(&ledger);
        async move {
            let outcome = runner
                .run(turn(
                    AgentKey::Belt(Belt::Reply),
                    Origin::Public,
                    &call("recall_memory", json!({"query": "gm"})),
                    "reply",
                ))
                .await
                .expect("the turn runs");
            (outcome, rows(&ledger).await)
        }
    });
    let (outcome, rows) = outcome;
    assert_eq!(outcome.reply, FINAL_REPLY);
    assert!(outcome.session_id.starts_with("embed-"));
    assert_eq!(outcome.tool_calls.len(), 1, "{:?}", outcome.tool_calls);
    assert_eq!(outcome.tool_calls[0].name, "recall_memory");
    assert_eq!(outcome.tool_calls[0].arguments, json!({"query": "gm"}));
    let usage = outcome.usage.expect("the provider reported usage");
    assert!(usage.input > 0 && usage.output > 0, "{usage:?}");
    assert_eq!(
        rows.len(),
        2,
        "a ledger row per model call: the tool call, then the answer"
    );
    assert!(
        rows.iter()
            .all(|row| row.source == "reply" && row.lane == Lane::Mascot)
    );
    let spent: i64 = rows.iter().map(|row| row.tokens.input).sum();
    assert_eq!(spent, i64::try_from(usage.input).unwrap());
}

#[test]
fn a_session_continues_and_a_model_override_reaches_the_provider() {
    let ledger = Arc::new(InMemoryLedger::default());
    let runner = Arc::new(runner(&ledger));
    let (first, second) = testkit::run(async move {
        let first = runner
            .run(turn(
                AgentKey::Belt(Belt::Chat),
                Origin::Public,
                "hello there, first",
                "chat",
            ))
            .await
            .unwrap();
        let mut next = turn(
            AgentKey::Belt(Belt::Chat),
            Origin::Public,
            "and again, second",
            "chat",
        );
        next.session = Some(first.session_id.clone());
        next.model = Some("override-model".into());
        let second = runner.run(next).await.unwrap();
        (first, second)
    });
    assert_eq!(second.session_id, first.session_id);
    let requests = testkit::run(async { chat_requests(&booted().server).await });
    let continued = requests
        .iter()
        .rev()
        .find(|request| testkit::asks(request, "and again, second"))
        .expect("the second turn reached the provider");
    assert!(
        testkit::asks(continued, "hello there, first"),
        "the second turn carries the first"
    );
    let body: serde_json::Value = serde_json::from_slice(&continued.body).unwrap();
    assert!(
        body["model"]
            .as_str()
            .unwrap_or("")
            .contains("override-model"),
        "{}",
        body["model"]
    );
}

#[test]
fn a_room_persona_is_its_own_agent() {
    let ledger = Arc::new(InMemoryLedger::default());
    let runner = runner(&ledger);
    let outcome = testkit::run(async move {
        runner
            .run(turn(
                AgentKey::Room {
                    persona: "skeptic".into(),
                    system_prompt: "You doubt everything.".into(),
                },
                Origin::Scheduled { once: true },
                "is the moon real",
                "backrooms",
            ))
            .await
    })
    .unwrap();
    assert_eq!(outcome.reply, FINAL_REPLY);
    let bad = testkit::run(async move {
        OhRunner::new(Arc::clone(&booted().agents))
            .run(turn(
                AgentKey::Room {
                    persona: "Not A Persona".into(),
                    system_prompt: "x".into(),
                },
                Origin::Scheduled { once: true },
                "x",
                "backrooms",
            ))
            .await
    });
    assert!(matches!(bad, Err(RunError::Failed(_))), "{bad:?}");
}

/// How many times the provider saw a turn whose message carries `tag`.
fn tries(tag: &str) -> usize {
    let tag = tag.to_owned();
    testkit::run(async move {
        chat_requests(&booted().server)
            .await
            .iter()
            .filter(|request| testkit::asks(request, &tag))
            .count()
    })
}

#[test]
fn a_turn_that_must_run_once_times_out_and_is_never_retried() {
    let runner =
        OhRunner::new(Arc::clone(&booted().agents)).with_timeout(Duration::from_millis(500));
    let once = format!("{SLOW} post-once");
    let result = testkit::run({
        let once = once.clone();
        async move {
            runner
                .run(turn(
                    AgentKey::Belt(Belt::Drafter),
                    Origin::Scheduled { once: true },
                    &once,
                    "calendar.gm",
                ))
                .await
        }
    });
    assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
    assert_eq!(tries("post-once"), 1);
}

#[test]
fn a_turn_that_may_repeat_is_tried_again() {
    let runner =
        OhRunner::new(Arc::clone(&booted().agents)).with_timeout(Duration::from_millis(500));
    let result = testkit::run(async move {
        runner
            .run(turn(
                AgentKey::Belt(Belt::Drafter),
                Origin::Scheduled { once: false },
                &format!("{SLOW} may-repeat"),
                "lore",
            ))
            .await
    });
    assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
    assert_eq!(tries("may-repeat"), 2);
}

/// The operator's belt posts and remembers, so only the bearer may start
/// it: a cron turn on it is refused before it starts.
#[test]
fn the_operator_runs_only_operator_turns() {
    let agents = Arc::clone(&booted().agents);
    for origin in [
        Origin::Scheduled { once: true },
        Origin::Scheduled { once: false },
    ] {
        let agents = Arc::clone(&agents);
        let result = testkit::run(async move {
            OhRunner::new(agents)
                .run(turn(
                    AgentKey::Belt(Belt::Operator),
                    origin,
                    "post this (operator-from-cron)",
                    "probe",
                ))
                .await
        });
        match result {
            Err(RunError::Failed(why)) => assert!(why.contains("operator"), "{why}"),
            other => panic!("{origin:?}: {other:?}"),
        }
    }
}

#[test]
fn a_public_turn_runs_only_on_a_public_agent() {
    // A trusted agent keeps its writes under a public origin, and a write
    // there would park for an approval nobody gives: refused up front.
    let agents = Arc::clone(&booted().agents);
    for belt in [Belt::Chart, Belt::Operator, Belt::Drafter] {
        let agents = Arc::clone(&agents);
        let result = testkit::run(async move {
            OhRunner::new(agents)
                .run(turn(AgentKey::Belt(belt), Origin::Public, "hi", "probe"))
                .await
        });
        assert!(
            matches!(result, Err(RunError::Failed(_))),
            "{belt:?}: {result:?}"
        );
    }
    let room = testkit::run(async move {
        OhRunner::new(agents)
            .run(turn(
                AgentKey::Room {
                    persona: "skeptic".into(),
                    system_prompt: "You doubt everything.".into(),
                },
                Origin::Public,
                "hi",
                "probe",
            ))
            .await
    });
    assert!(matches!(room, Err(RunError::Failed(_))), "{room:?}");
}
