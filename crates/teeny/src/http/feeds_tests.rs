//! Ported from legacy/test/journal.test.ts (GET /mind) and ledger.test.ts
//! (GET /ledger and GET /brain), plus /health and /queue.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use serde_json::{Value, json};
use teeny_store::inference::{LedgerMessage, Role};
use teeny_store::journal::{Outcome, Tone, Trigger};
use teeny_store::{
    InMemoryJournal, InMemoryLedger, InMemoryMentionQueue, InferenceInput, InferenceLedger,
    Journal, MentionQueue, ReflectionInput, Reply, Tokens,
};
use teeny_x::model::XPost;

use super::*;
use crate::http::brain::{BrainCompany, BrainSnapshot, BrainSource};
use crate::http::test_support::*;
use crate::http::{Services, router};
use crate::mentions::QueuedXPost;

fn reflection(incoming: &str, said: &str) -> ReflectionInput {
    ReflectionInput {
        id: None,
        at: None,
        trigger: Trigger::Mention,
        incoming: incoming.into(),
        tone: Tone::Friendly,
        recalled: vec![],
        tools: vec![],
        thinking: String::new(),
        said: said.into(),
        declined: vec![],
        outcome: Outcome::Posted,
        outcome_note: None,
    }
}

fn tokens(input: i64, output: i64) -> Tokens {
    Tokens {
        input,
        output,
        reasoning: 0,
        cached: 0,
        total: input + output,
    }
}

fn status() -> MindStatus {
    MindStatus {
        mode: "langgraph".into(),
        model: "flash".into(),
        dry_run: false,
    }
}

async fn queue_with(pending: usize, replied: usize) -> Arc<InMemoryMentionQueue<QueuedXPost>> {
    let queue = Arc::new(InMemoryMentionQueue::default());
    for n in 0..pending + replied {
        queue
            .cache(&[QueuedXPost(XPost::new(n.to_string(), "hi"))])
            .await
            .unwrap();
        if n >= pending {
            let reply = Reply {
                id: Some("r".into()),
                text: "hey".into(),
                ..Reply::default()
            };
            queue.mark_replied(&n.to_string(), &reply).await.unwrap();
        }
    }
    queue
}

#[tokio::test]
async fn health_is_ok_with_a_null_browser_until_one_is_wired() {
    let answer = call(&router(services()), get("/health")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.json(), json!({ "ok": true, "browser": null }));
}

struct Browser;

#[async_trait]
impl BrowserStatus for Browser {
    async fn status(&self) -> Value {
        json!({ "ready": true })
    }
}

#[tokio::test]
async fn health_carries_the_browser_state_once_wired() {
    let app = router(Services {
        browser: Some(Arc::new(Browser)),
        ..services()
    });
    assert_eq!(
        call(&app, get("/health")).await.json(),
        json!({ "ok": true, "browser": { "ready": true } })
    );
}

#[tokio::test]
async fn queue_serves_the_counts_or_says_there_is_no_queue() {
    let none = call(&router(services()), get("/queue")).await;
    assert_eq!(none.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        none.json(),
        json!({ "error": "no mention queue is configured" })
    );

    let app = router(Services {
        queue: Some(queue_with(2, 1).await),
        ..services()
    });
    let answer = call(&app, get("/queue")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.json(),
        json!({ "counts": { "pending": 2, "replied": 1, "failed": 0, "skipped": 0 } })
    );
}

#[tokio::test]
async fn mind_serves_the_journal_redacted_with_the_queue_counts() {
    let journal = Arc::new(InMemoryJournal::default());
    journal
        .record(reflection("@fren gm", "gm @fren"))
        .await
        .unwrap();
    let app = router(Services {
        queue: Some(queue_with(2, 40).await),
        mind: Some(Mind {
            journal,
            status: status(),
        }),
        ..services()
    });

    let answer = call(&app, get("/mind")).await;
    assert_eq!(answer.status, StatusCode::OK);
    let body = answer.json();
    assert_eq!(body["source"], "live");
    assert!(body["generatedAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        body["status"],
        json!({ "mode": "langgraph", "model": "flash", "dryRun": false, "queue": { "pending": 2, "done": 40, "failed": 0 } })
    );
    assert_eq!(body["reflections"][0]["incoming"], "@… gm");
    assert_eq!(body["reflections"][0]["said"], "gm @…");
    // No ledger, no totals.
    assert!(body.get("totals").is_none());
}

#[tokio::test]
async fn mind_reads_as_many_reflections_as_asked_and_zeroes_a_missing_queue() {
    let journal = Arc::new(InMemoryJournal::default());
    for n in 0..3 {
        journal
            .record(reflection(&format!("m{n}"), ""))
            .await
            .unwrap();
    }
    let app = router(Services {
        mind: Some(Mind {
            journal,
            status: status(),
        }),
        ..services()
    });

    let body = call(&app, get("/mind?limit=2")).await.json();
    assert_eq!(body["reflections"].as_array().unwrap().len(), 2);
    assert_eq!(
        body["status"]["queue"],
        json!({ "pending": 0, "done": 0, "failed": 0 })
    );
}

#[tokio::test]
async fn mind_is_503_without_a_journal() {
    let answer = call(&router(services()), get("/mind")).await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answer.json(),
        json!({ "error": "no journal is configured" })
    );
}

#[tokio::test]
async fn ledger_serves_the_rows_redacted_with_the_totals_which_ride_along_on_mind() {
    let ledger = Arc::new(InMemoryLedger::default());
    let mut reply = InferenceInput::new(
        teeny_store::inference::Lane::Mascot,
        "reply",
        "m",
        tokens(10, 2),
    );
    reply.input = Some(vec![LedgerMessage {
        role: Role::User,
        tool: None,
        text: "hi @fren".into(),
    }]);
    reply.output = Some("hey @fren".into());
    ledger.record(reply).await.unwrap();
    ledger
        .record(InferenceInput::new(
            teeny_store::inference::Lane::Brain,
            "scout",
            "m",
            tokens(5, 1),
        ))
        .await
        .unwrap();
    let app = router(Services {
        ledger: Some(ledger),
        mind: Some(Mind {
            journal: Arc::new(InMemoryJournal::default()),
            status: status(),
        }),
        ..services()
    });

    let all = call(&app, get("/ledger")).await.json();
    assert_eq!(all["source"], "live");
    assert_eq!(all["totals"]["lanes"]["mascot"]["tokens"]["total"], 12);
    assert_eq!(all["totals"]["lanes"]["brain"]["tokens"]["total"], 6);
    assert_eq!(all["entries"].as_array().unwrap().len(), 2);
    assert!(!all.to_string().contains("@fren"));

    let brain_only = call(&app, get("/ledger?lane=brain&limit=1")).await.json();
    let sources: Vec<&str> = brain_only["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["scout"]);

    // An unknown lane is no filter at all.
    assert_eq!(
        call(&app, get("/ledger?lane=nope")).await.json()["entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let mind = call(&app, get("/mind")).await.json();
    assert_eq!(mind["totals"]["lanes"]["mascot"]["calls"], 1);
}

#[tokio::test]
async fn ledger_and_brain_are_503_without_one() {
    let app = router(services());
    let ledger = call(&app, get("/ledger")).await;
    assert_eq!(ledger.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        ledger.json(),
        json!({ "error": "no inference ledger is configured" })
    );
    let brain = call(&app, get("/brain")).await;
    assert_eq!(brain.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(brain.json(), json!({ "error": "no brain is mounted" }));
}

#[derive(Default)]
struct FakeBrain(Mutex<Vec<(usize, usize)>>);

#[async_trait]
impl BrainSource for FakeBrain {
    async fn snapshot(&self, runs: usize, steps: usize) -> anyhow::Result<BrainSnapshot> {
        self.0.lock().unwrap().push((runs, steps));
        Ok(BrainSnapshot {
            source: "live".into(),
            generated_at: "now".into(),
            company: BrainCompany {
                name: "Teeny".into(),
                output: None,
                slug: "teeny".into(),
            },
            agents: vec![],
            workflows: vec![],
            runs: vec![],
            steps: vec![],
        })
    }
}

#[tokio::test]
async fn brain_serves_the_snapshot_with_clamped_bounds() {
    let brain = Arc::new(FakeBrain::default());
    let app = router(Services {
        brain: Some(brain.clone()),
        ..services()
    });

    let answer = call(&app, get("/brain?runs=5")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.json(),
        json!({
            "source": "live", "generatedAt": "now", "company": { "name": "Teeny", "slug": "teeny" },
            "agents": [], "workflows": [], "runs": [], "steps": [],
        })
    );
    call(&app, get("/brain")).await;
    call(&app, get("/brain?runs=900&steps=5000")).await;
    assert_eq!(*brain.0.lock().unwrap(), [(5, 300), (40, 300), (200, 1000)]);
}
