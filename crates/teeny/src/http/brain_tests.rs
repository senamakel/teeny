//! Ported from legacy/test/ledger.test.ts, "the brain, read from its
//! checkout" (the usage fold belongs to the schedule and is not here).

use serde_json::json;

use super::*;
use crate::http::test_support::Scratch;

fn checkout() -> Scratch {
    let dir = Scratch::new("brain");
    dir.write("company/teeny/company.toml", "[company]\nname = \"Teeny\"\noutput = \"a mind\"\n");
    dir.write("company/teeny/agents/scout.toml", "role = \"Scout\"\ndescription = \"watches\"\n");
    dir.write(
        "company/teeny/agents/curator.toml",
        "role = \"Curator\"\ndescription = \"edits\"\ntier = \"orchestrator\"\n",
    );
    dir.write("company/teeny/agents/broken.toml", "role = \n");
    dir.write("company/teeny/agents/notes.md", "not an agent");
    dir.write(
        "company/teeny/workflows/reflect.toml",
        [
            "id = \"reflect\"", "name = \"Reflect\"", "description = \"look, learn, revise\"",
            "[[node]]", "id = \"daily\"", "kind = \"trigger\"", "name = \"Every morning\"", "schedule = \"0 6 * * *\"",
            "[[node]]", "id = \"look\"", "kind = \"agent\"", "name = \"Look\"", "agent = \"scout\"",
            "[[edge]]", "from = \"daily\"", "to = \"look\"", "label = \"go\"",
        ]
        .join("\n"),
    );
    dir
}

fn run(id: &str, status: &str, created: u64) -> String {
    json!({ "id": id, "company": "teeny", "agentId": "scout", "attempt": 1, "status": status, "createdAtMillis": created, "usage": { "input": 10, "output": 2 } })
        .to_string()
}

#[tokio::test]
async fn serves_the_roster_the_workflow_graph_and_the_latest_runs_one_row_per_run() {
    let dir = checkout();
    dir.write(
        "companies/teeny/runs.jsonl",
        [run("r1", "running", 1), run("r2", "pending", 2), run("r1", "succeeded", 1), String::new()].join("\n"),
    );
    dir.write(
        "companies/teeny/run-steps.jsonl",
        [
            json!({ "runId": "r1", "stepSeq": 1, "atMillis": 5, "step": { "kind": "thinking", "status": "ok", "label": "reading the record" } }).to_string(),
            json!({ "runId": "zzz", "stepSeq": 1, "atMillis": 6, "step": { "kind": "note", "status": "ok", "label": "not ours" } }).to_string(),
            "{torn".to_owned(),
        ]
        .join("\n"),
    );
    let brain = BrainReader::new(&dir.0, "teeny");
    assert!(brain.exists().await);

    let snapshot = serde_json::to_value(brain.snapshot(40, 300).await.unwrap()).unwrap();
    assert_eq!(snapshot["source"], "live");
    assert_eq!(snapshot["company"], json!({ "name": "Teeny", "output": "a mind", "slug": "teeny" }));
    assert_eq!(
        snapshot["agents"],
        json!([
            { "id": "curator", "role": "Curator", "description": "edits", "tier": "orchestrator" },
            { "id": "scout", "role": "Scout", "description": "watches" },
        ])
    );
    assert_eq!(
        snapshot["workflows"],
        json!([{
            "id": "reflect", "name": "Reflect", "description": "look, learn, revise",
            "nodes": [
                { "id": "daily", "kind": "trigger", "name": "Every morning", "schedule": "0 6 * * *" },
                { "id": "look", "kind": "agent", "name": "Look", "agent": "scout" },
            ],
            "edges": [{ "from": "daily", "to": "look", "label": "go" }],
        }])
    );
    // The host's own row, whole, the last write of each run winning.
    assert_eq!(snapshot["runs"][0], serde_json::from_str::<serde_json::Value>(&run("r2", "pending", 2)).unwrap());
    let runs: Vec<(&str, &str)> = snapshot["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap(), r["status"].as_str().unwrap()))
        .collect();
    assert_eq!(runs, [("r2", "pending"), ("r1", "succeeded")]);
    assert_eq!(
        snapshot["steps"],
        json!([{ "runId": "r1", "stepSeq": 1, "atMillis": 5, "step": { "kind": "thinking", "status": "ok", "label": "reading the record" } }])
    );
}

#[tokio::test]
async fn bounds_the_runs_and_keeps_the_latest_steps_cut_to_length() {
    let dir = checkout();
    dir.write("companies/teeny/runs.jsonl", [run("a", "ok", 1), run("b", "ok", 2), run("c", "ok", 3)].join("\n"));
    let long = "x".repeat(700);
    dir.write(
        "companies/teeny/run-steps.jsonl",
        (1..=4)
            .map(|n| json!({ "runId": "c", "stepSeq": n, "atMillis": 10 - n, "step": { "detail": long, "result": "r" } }).to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let snapshot = BrainReader::new(&dir.0, "teeny").snapshot(1, 2).await.unwrap();

    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0]["id"], "c");
    // Oldest first, then the last two kept.
    let seqs: Vec<_> = snapshot.steps.iter().map(|step| step.step_seq.clone()).collect();
    assert_eq!(seqs, [json!(2), json!(1)]);
    let step = &snapshot.steps[0].step;
    assert_eq!((step.kind.as_str(), step.status.as_str(), step.label.as_str()), ("note", "ok", ""));
    assert_eq!(step.detail.as_deref().map(str::len), Some(600));
    assert_eq!(step.result.as_deref(), Some("r"));
}

#[tokio::test]
async fn is_empty_not_broken_when_there_is_no_brain_and_no_record() {
    let empty = Scratch::new("nobrain");
    let brain = BrainReader::new(&empty.0, "teeny");
    assert!(!brain.exists().await);
    let snapshot = brain.snapshot(40, 300).await.unwrap();
    assert_eq!(snapshot.company, BrainCompany { name: "teeny".into(), output: None, slug: "teeny".into() });
    assert!(snapshot.agents.is_empty() && snapshot.workflows.is_empty());
    assert!(snapshot.runs.is_empty() && snapshot.steps.is_empty());
    assert!(tail_lines(&empty.0.join("missing.jsonl"), 512 * 1024).await.is_empty());
}

#[tokio::test]
async fn reads_only_the_tail_of_a_long_file_whole_lines_only() {
    let dir = Scratch::new("tail");
    let lines: Vec<String> = (0..1_000).map(|i| format!("{{\"n\":{i}}}")).collect();
    let path = dir.write("big.jsonl", lines.join("\n") + "\n");

    let tail = tail_lines(&path, 100).await;
    assert!(!tail.is_empty() && tail.len() < 20);
    for line in &tail {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    assert_eq!(tail.last().unwrap(), "{\"n\":999}");
}
