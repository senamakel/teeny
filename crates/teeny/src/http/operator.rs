//! The operator's two endpoints behind the token: `/critique`, the ranker's
//! reader on a draft, and `/agent/run`, an ad-hoc task for the agent.
//!
//! Both reach models the agent crate owns, so they meet the server through
//! seams: [`AgentRunner`] here, and the critic's own trait from teeny-safety
//! under the name [`AlgoCritique`]. Fakes stand in until they are wired.

use async_trait::async_trait;
use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use teeny_safety::critic::{CritiqueInput, CritiqueKind, render_critique};
use teeny_safety::gate::SafetyDecision;
use teeny_store::ReflectionInput;
use teeny_store::holds::{HoldContext, HoldSurface};
use teeny_store::journal::{Outcome, Tone, ToolCall, Trigger};

pub use teeny_safety::critic::Critic as AlgoCritique;

use super::State;
use super::auth::authorise;
use super::body::Payload;
use super::reply::{error, internal};
use super::validate::{Issues, object, one_of, string};

const KINDS: [(&str, CritiqueKind); 7] = [
    ("post", CritiqueKind::Post),
    ("quote", CritiqueKind::Quote),
    ("reply", CritiqueKind::Reply),
    ("caption", CritiqueKind::Caption),
    ("script", CritiqueKind::Script),
    ("thread", CritiqueKind::Thread),
    ("other", CritiqueKind::Other),
];

fn invalid(message: &str, issues: Issues) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message, "details": issues })),
    )
        .into_response()
}

/// One tool call the agent made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStep {
    pub tool: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// What a task produced, served as it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub output: String,
    pub steps: Vec<RunStep>,
}

/// Runs one ad-hoc task. The agent crate implements it.
#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn run(&self, task: &str, thread_id: Option<&str>) -> anyhow::Result<RunResult>;
}

/// Feedback on a draft from the reader that knows the ranker's weights.
/// The text is the operator's and nothing is published, so it is not put
/// through the content filter; the critic's own brief treats it as data.
pub async fn critique(state: State, headers: HeaderMap, payload: Payload) -> Response {
    let Some(critic) = &state.critic else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "the critic is not configured",
        );
    };
    if let Err(refused) = authorise(state.agent_token.as_deref(), &headers, false) {
        return refused;
    }
    let mut issues = Issues::new();
    let Some(fields) = object(payload.body.as_ref(), &mut issues) else {
        return invalid("text is required", issues);
    };
    let names: Vec<&str> = KINDS.iter().map(|(name, _)| *name).collect();
    let kind = one_of(fields, "kind", &names, Some("post"), &mut issues);
    let text = string(fields, "text", 1, 8_000, true, &mut issues);
    let media = string(fields, "media", 0, 600, false, &mut issues);
    let context = string(fields, "context", 0, 2_000, false, &mut issues);
    let (Some(kind), Some(text), true) = (kind, text, issues.is_empty()) else {
        return invalid("text is required", issues);
    };
    let input = CritiqueInput {
        kind: KINDS
            .iter()
            .find(|(name, _)| *name == kind)
            .map_or(CritiqueKind::Post, |(_, kind)| *kind),
        text,
        media: media.filter(|media| !media.is_empty()),
        context: context.filter(|context| !context.is_empty()),
    };
    match critic.critique(&input).await {
        Ok(critique) => {
            let mut body = json!(critique);
            body["rendered"] = json!(render_critique(&critique));
            Json(body).into_response()
        }
        Err(failure) => error(StatusCode::BAD_GATEWAY, failure.to_string()),
    }
}

/// An ad-hoc task for the agent, after the content filter: the operator is
/// trusted with the token, not with the filter.
pub async fn run(state: State, headers: HeaderMap, payload: Payload) -> Response {
    let Some(agent) = &state.agent else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "the agent is not configured",
        );
    };
    if let Err(refused) = authorise(state.agent_token.as_deref(), &headers, false) {
        return refused;
    }
    let mut issues = Issues::new();
    let Some(fields) = object(payload.body.as_ref(), &mut issues) else {
        return invalid("task is required", issues);
    };
    let task = string(fields, "task", 1, 4_000, true, &mut issues);
    let thread_id = string(fields, "threadId", 1, 200, false, &mut issues);
    let (Some(task), true) = (task, issues.is_empty()) else {
        return invalid("task is required", issues);
    };

    if let Some(gate) = &state.safety
        && let SafetyDecision::Held(held) = gate
            .inbound(&task, Some(&HoldContext::new(HoldSurface::Task)))
            .await
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "the task was held by the content filter", "category": held.category })),
        )
            .into_response();
    }

    let result = match agent.run(&task, thread_id.as_deref()).await {
        Ok(result) => result,
        Err(failure) => return internal(failure),
    };
    // An ad-hoc task is a turn of the mind like any other; the task text is
    // the operator's, so it is journaled as what came in.
    if let Some(mind) = &state.mind {
        let posted = result.steps.iter().any(|step| step.tool == "x_post");
        let turn = ReflectionInput {
            id: None,
            at: None,
            trigger: Trigger::Task,
            incoming: task,
            tone: Tone::Neutral,
            recalled: vec![],
            tools: result
                .steps
                .iter()
                .map(|step| ToolCall {
                    name: step.tool.clone(),
                    result: step.result.clone().unwrap_or_default(),
                })
                .collect(),
            thinking: String::new(),
            said: result.output.clone(),
            declined: vec![],
            outcome: if posted {
                Outcome::Posted
            } else {
                Outcome::Held
            },
            outcome_note: Some("ad-hoc task via /agent/run".into()),
        };
        if let Err(failure) = mind.journal.record(turn).await {
            tracing::warn!(err = %failure, "could not journal the task");
        }
    }
    Json(result).into_response()
}

#[cfg(test)]
#[path = "operator_tests.rs"]
mod tests;
