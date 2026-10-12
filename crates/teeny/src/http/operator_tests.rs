//! Ported from legacy/test/agent-endpoint.test.ts and the POST /critique
//! cases of critic.test.ts.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::{StatusCode, header};
use serde_json::json;
use teeny_safety::critic::{
    CriticError, Critique, CritiqueAnswer, CritiqueInput, CritiqueKind, Predicted, to_critique,
};
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_store::journal::{Outcome, Trigger};
use teeny_store::{InMemoryJournal, Journal};

use super::*;
use crate::http::test_support::*;
use crate::http::{Mind, MindStatus, Services, router};

#[derive(Default)]
struct Agent {
    calls: Mutex<Vec<(String, Option<String>)>>,
    tool: &'static str,
}

#[async_trait]
impl AgentRunner for Agent {
    async fn run(&self, task: &str, thread_id: Option<&str>) -> anyhow::Result<RunResult> {
        self.calls
            .lock()
            .unwrap()
            .push((task.to_owned(), thread_id.map(str::to_owned)));
        Ok(RunResult {
            output: "done".into(),
            steps: vec![RunStep {
                tool: self.tool.into(),
                input: json!({ "query": "x" }),
                result: None,
            }],
        })
    }
}

fn agent() -> Arc<Agent> {
    Arc::new(Agent {
        tool: "recall_memory",
        ..Agent::default()
    })
}

fn with_agent(agent: Arc<Agent>) -> Services {
    Services {
        agent: Some(agent),
        agent_token: Some(TOKEN.into()),
        ..services()
    }
}

#[tokio::test]
async fn rejects_requests_without_the_bearer_token() {
    let app = router(with_agent(agent()));
    let answer = call(&app, post_json("/agent/run", r#"{"task":"say hi"}"#)).await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.headers[header::WWW_AUTHENTICATE], "Bearer");
    assert_eq!(answer.json(), json!({ "error": "unauthorized" }));
}

#[tokio::test]
async fn runs_the_agent_for_an_authorised_task() {
    let subject = agent();
    let app = router(with_agent(subject.clone()));
    let answer = call(
        &app,
        post_json_as(
            "/agent/run",
            &bearer(),
            r#"{"task":"summarise mentions","threadId":"thread-1"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.json(),
        json!({ "output": "done", "steps": [{ "tool": "recall_memory", "input": { "query": "x" } }] })
    );
    assert_eq!(
        *subject.calls.lock().unwrap(),
        [("summarise mentions".to_owned(), Some("thread-1".to_owned()))]
    );
}

#[tokio::test]
async fn reports_503_when_no_agent_is_configured() {
    let app = router(Services {
        agent_token: Some(TOKEN.into()),
        ..services()
    });
    let answer = call(
        &app,
        post_json_as("/agent/run", &bearer(), r#"{"task":"say hi"}"#),
    )
    .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answer.json(),
        json!({ "error": "the agent is not configured" })
    );
}

#[tokio::test]
async fn rejects_a_malformed_task() {
    let subject = agent();
    let app = router(with_agent(subject.clone()));
    for body in [r#"{"task":""}"#, r#"{}"#, r#"{"task":"ok","threadId":""}"#] {
        let answer = call(&app, post_json_as("/agent/run", &bearer(), body)).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer.json()["error"], "task is required");
        assert!(
            answer.json()["details"]
                .as_array()
                .is_some_and(|d| !d.is_empty())
        );
    }
    let long = format!(r#"{{"task":"{}"}}"#, "a".repeat(4_001));
    assert_eq!(
        call(&app, post_json_as("/agent/run", &bearer(), &long))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert!(subject.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn holds_a_task_the_filter_refuses_before_the_agent_sees_it() {
    let subject = agent();
    let gate = Arc::new(SafetyGate::new(SafetyGateOptions::default()));
    let app = router(Services {
        safety: Some(gate),
        ..with_agent(subject.clone())
    });
    let answer = call(
        &app,
        post_json_as(
            "/agent/run",
            &bearer(),
            r#"{"task":"post a picture of tiny naked"}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        answer.json(),
        json!({ "error": "the task was held by the content filter", "category": "sexual" })
    );
    assert!(subject.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn journals_the_task_as_a_turn_of_the_mind() {
    let journal = Arc::new(InMemoryJournal::default());
    let status = MindStatus {
        mode: "langgraph".into(),
        model: "m".into(),
        dry_run: true,
    };
    let posting = Arc::new(Agent {
        tool: "x_post",
        ..Agent::default()
    });
    let app = router(Services {
        mind: Some(Mind {
            journal: journal.clone(),
            status: status.clone(),
        }),
        ..with_agent(posting)
    });
    call(
        &app,
        post_json_as("/agent/run", &bearer(), r#"{"task":"post gm"}"#),
    )
    .await;
    let quiet = router(Services {
        mind: Some(Mind {
            journal: journal.clone(),
            status,
        }),
        ..with_agent(agent())
    });
    call(
        &quiet,
        post_json_as("/agent/run", &bearer(), r#"{"task":"look around"}"#),
    )
    .await;

    let turns = journal.recent(10).await.unwrap();
    let posted = turns
        .iter()
        .find(|turn| turn.incoming == "post gm")
        .unwrap();
    assert_eq!(posted.trigger, Trigger::Task);
    assert_eq!(posted.outcome, Outcome::Posted);
    assert_eq!(posted.said, "done");
    assert_eq!(posted.tools[0].name, "x_post");
    assert_eq!(posted.tools[0].result, "");
    assert_eq!(
        posted.outcome_note.as_deref(),
        Some("ad-hoc task via /agent/run")
    );
    assert_eq!(
        turns
            .iter()
            .find(|turn| turn.incoming == "look around")
            .unwrap()
            .outcome,
        Outcome::Held
    );
}

struct Critic {
    inputs: Mutex<Vec<CritiqueInput>>,
    fails: bool,
}

fn worthy() -> Critique {
    let predicted = Predicted {
        reply: 1.0,
        ..Predicted::default()
    };
    to_critique(CritiqueAnswer {
        predicted,
        summary: "a good one".into(),
        ..CritiqueAnswer::default()
    })
}

#[async_trait]
impl AlgoCritique for Critic {
    async fn critique(&self, input: &CritiqueInput) -> Result<Critique, CriticError> {
        self.inputs.lock().unwrap().push(input.clone());
        if self.fails {
            Err(CriticError("down".into()))
        } else {
            Ok(worthy())
        }
    }
}

fn critic(fails: bool) -> Arc<Critic> {
    Arc::new(Critic {
        inputs: Mutex::new(vec![]),
        fails,
    })
}

#[tokio::test]
async fn critique_is_503_401_400_and_otherwise_the_critique_with_its_rendering() {
    let none = router(Services {
        agent_token: Some(TOKEN.into()),
        ..services()
    });
    let unconfigured = call(
        &none,
        post_json_as("/critique", &bearer(), r#"{"text":"gm"}"#),
    )
    .await;
    assert_eq!(unconfigured.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        unconfigured.json(),
        json!({ "error": "the critic is not configured" })
    );

    let reader = critic(false);
    let app = router(Services {
        agent_token: Some(TOKEN.into()),
        critic: Some(reader.clone()),
        ..services()
    });
    assert_eq!(
        call(&app, post_json("/critique", r#"{"text":"gm"}"#))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let no_text = call(
        &app,
        post_json_as("/critique", &bearer(), r#"{"kind":"script"}"#),
    )
    .await;
    assert_eq!(no_text.status, StatusCode::BAD_REQUEST);
    assert_eq!(no_text.json()["error"], "text is required");
    let bad_kind = call(
        &app,
        post_json_as("/critique", &bearer(), r#"{"kind":"essay","text":"gm"}"#),
    )
    .await;
    assert_eq!(bad_kind.status, StatusCode::BAD_REQUEST);

    let answer = call(
        &app,
        post_json_as(
            "/critique",
            &bearer(),
            r#"{"kind":"script","text":"0-2s: Tiny sips.","media":""}"#,
        ),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let body = answer.json();
    assert_eq!(body["verdict"], "worthy");
    assert_eq!(body["summary"], "a good one");
    assert!(
        body["rendered"]
            .as_str()
            .unwrap()
            .contains("Verdict: WORTHY")
    );
    call(
        &app,
        post_json_as(
            "/critique",
            &bearer(),
            r#"{"text":"gm","media":"a cat","context":"a thread"}"#,
        ),
    )
    .await;
    assert_eq!(
        *reader.inputs.lock().unwrap(),
        [
            CritiqueInput {
                kind: CritiqueKind::Script,
                text: "0-2s: Tiny sips.".into(),
                media: None,
                context: None
            },
            CritiqueInput {
                kind: CritiqueKind::Post,
                text: "gm".into(),
                media: Some("a cat".into()),
                context: Some("a thread".into())
            },
        ]
    );
}

#[tokio::test]
async fn critique_is_502_when_the_critic_cannot_be_reached() {
    let app = router(Services {
        agent_token: Some(TOKEN.into()),
        critic: Some(critic(true)),
        ..services()
    });
    let answer = call(
        &app,
        post_json_as("/critique", &bearer(), r#"{"text":"gm"}"#),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_GATEWAY);
    assert_eq!(answer.json(), json!({ "error": "down" }));
}
