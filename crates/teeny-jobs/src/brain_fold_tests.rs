//! A port of the usage-fold parts of `legacy/test/ledger.test.ts` ("the
//! brain, read from its checkout").

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use teeny_store::inference::{Lane, RecentFilter};
use teeny_store::{InMemoryLedger, InferenceLedger};

use super::*;
use crate::ctx::{Conversation, JobCtx};
use crate::runner::ScriptedRunner;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh directory for one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "teeny-jobs-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn checkout() -> Scratch {
    let scratch = Scratch::new("brain");
    std::fs::create_dir_all(scratch.0.join("company/teeny")).unwrap();
    std::fs::create_dir_all(scratch.0.join("companies/teeny")).unwrap();
    std::fs::write(
        scratch.0.join("company/teeny/company.toml"),
        "[company]\nname = \"Teeny\"\n",
    )
    .unwrap();
    scratch
}

fn sample(at: i64, agent: &str, input: i64, output: i64, kind: &str) -> String {
    json!({
        "atMillis": at, "agent": agent, "provider": "managed", "model": "flash",
        "inputTokens": input, "outputTokens": output, "cachedInputTokens": 0,
        "costUsd": 0, "kind": kind
    })
    .to_string()
}

fn usage(dir: &Scratch, lines: &[String]) {
    std::fs::write(
        dir.0.join("companies/teeny/usage.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();
}

#[tokio::test]
async fn folds_usage_samples_into_the_brain_lane_by_agent_and_only_once() {
    let dir = checkout();
    usage(
        &dir,
        &[
            sample(1_000, "scout", 100, 10, "inference"),
            sample(2_000, "scribe", 50, 5, "planningCall"),
        ],
    );
    let ledger = InMemoryLedger::default();
    let brain = BrainFold::new(&dir.0, "teeny");
    assert!(brain.exists().await);
    assert_eq!(brain.fold_usage(&ledger).await, 2);
    // The same file again, plus one new line: the old ones are offered again
    // at most and counted zero more times.
    usage(
        &dir,
        &[
            sample(1_000, "scout", 100, 10, "inference"),
            sample(2_000, "scribe", 50, 5, "planningCall"),
            sample(3_000, "scout", 1, 1, "inference"),
        ],
    );
    brain.fold_usage(&ledger).await;

    let totals = ledger.totals().await.unwrap();
    assert_eq!(totals.lanes.brain.calls, 3);
    assert_eq!(
        (
            totals.lanes.brain.tokens.input,
            totals.lanes.brain.tokens.output,
            totals.lanes.brain.tokens.total
        ),
        (151, 16, 167)
    );
    assert_eq!(totals.lanes.mascot.calls, 0);
    assert_eq!(totals.sources["brain/scout"].calls, 2);
    assert_eq!(totals.sources["brain/scribe"].calls, 1);
    let newest = &ledger.recent(1, &RecentFilter::default()).await.unwrap()[0];
    assert_eq!(newest.lane, Lane::Brain);
    assert_eq!(newest.source, "scout");
    assert_eq!(newest.model, "flash");
    assert_eq!(newest.at, "1970-01-01T00:00:03.000Z");
    let brain_rows = ledger
        .recent(
            10,
            &RecentFilter {
                lane: Some(Lane::Brain),
                ..RecentFilter::default()
            },
        )
        .await
        .unwrap();
    let scribe = brain_rows
        .iter()
        .find(|row| row.source == "scribe")
        .unwrap();
    assert_eq!(scribe.kind.as_deref(), Some("planningCall"));
}

#[tokio::test]
async fn is_empty_not_broken_when_there_is_no_brain_and_no_record() {
    let empty = Scratch::new("nobrain");
    let brain = BrainFold::new(&empty.0, "teeny");
    assert!(!brain.exists().await);
    assert_eq!(brain.fold_usage(&InMemoryLedger::default()).await, 0);
    assert!(
        tail_lines(&empty.0.join("missing.jsonl"), 1024)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn reads_only_the_tail_of_a_long_file_whole_lines_only() {
    let dir = Scratch::new("tail");
    let path = dir.0.join("big.jsonl");
    let lines: Vec<String> = (0..1_000).map(|n| format!("{{\"n\":{n}}}")).collect();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    let tail = tail_lines(&path, 100).await;
    assert!(!tail.is_empty() && tail.len() < 20);
    for line in &tail {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    assert_eq!(tail.last().unwrap(), "{\"n\":999}");
}

#[tokio::test]
async fn the_job_folds_into_the_conversation_ledger_and_declines_without_a_brain() {
    let dir = checkout();
    usage(&dir, &[sample(1_000, "scout", 1, 1, "inference")]);
    let ledger = Arc::new(InMemoryLedger::default());
    let ctx = JobCtx::fake(Arc::new(ScriptedRunner::new())).with_conversation(Conversation {
        inferences: Some(ledger.clone()),
        brain: Some(Arc::new(BrainFold::new(&dir.0, "teeny"))),
        ..Conversation::fake()
    });
    let report = run(&ctx).await.unwrap();
    assert!(report.ran);
    assert_eq!(report.note.as_deref(), Some("offered 1"));
    assert_eq!(ledger.totals().await.unwrap().lanes.brain.calls, 1);

    let without = JobCtx::fake(Arc::new(ScriptedRunner::new()));
    assert!(!run(&without).await.unwrap().ran);
}
