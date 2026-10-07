//! The read-only routes: `/health`, `/queue`, and the site's feeds `/mind`,
//! `/ledger` and `/brain`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value, json};
use teeny_store::inference::{Lane, RecentFilter, redact_inference};
use teeny_store::journal::redact_reflection;
use teeny_store::{Journal, iso_now};

use super::State;
use super::reply::{bounded, error, internal};

/// The journal, served at `/mind`, and the line of status above it.
pub struct Mind {
    pub journal: Arc<dyn Journal>,
    pub status: MindStatus,
}

/// What the page says the account is running as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MindStatus {
    /// `AGENT_MODE`: `langgraph` or `simple`.
    pub mode: String,
    pub model: String,
    pub dry_run: bool,
}

/// The browser's state, for `/health`.
#[async_trait]
pub trait BrowserStatus: Send + Sync {
    async fn status(&self) -> Value;
}

type Params = Query<HashMap<String, String>>;

fn param<'a>(params: &'a Params, name: &str) -> Option<&'a str> {
    params.get(name).map(String::as_str)
}

pub async fn health(state: State) -> Response {
    let browser = match &state.browser {
        Some(browser) => browser.status().await,
        None => Value::Null,
    };
    Json(json!({ "ok": true, "browser": browser })).into_response()
}

pub async fn queue(state: State) -> Response {
    let Some(queue) = &state.queue else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no mention queue is configured");
    };
    match queue.counts().await {
        Ok(counts) => Json(json!({ "counts": counts })).into_response(),
        Err(failure) => internal(failure),
    }
}

/// The journal, newest first and redacted, with the status line and, when
/// there is a ledger, the running token count. The queue and the ledger are
/// garnish: either failing leaves the feed up.
pub async fn mind(state: State, params: Params) -> Response {
    let Some(mind) = &state.mind else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no journal is configured");
    };
    let limit = bounded(param(&params, "limit"), 40, 1, 200);
    let counts = async {
        match &state.queue {
            Some(queue) => queue.counts().await.ok(),
            None => None,
        }
    };
    let totals = async {
        match &state.ledger {
            Some(ledger) => ledger.totals().await.ok(),
            None => None,
        }
    };
    let (reflections, counts, totals) = tokio::join!(mind.journal.recent(limit), counts, totals);
    let reflections = match reflections {
        Ok(reflections) => reflections,
        Err(failure) => return internal(failure),
    };
    let counts = counts.unwrap_or_default();

    let mut status = match serde_json::to_value(&mind.status) {
        Ok(Value::Object(status)) => status,
        _ => Map::new(),
    };
    status.insert(
        "queue".into(),
        json!({ "pending": counts.pending, "done": counts.replied, "failed": counts.failed }),
    );
    let mut body = Map::new();
    body.insert("source".into(), json!("live"));
    body.insert("generatedAt".into(), json!(iso_now()));
    body.insert("status".into(), Value::Object(status));
    if let Some(totals) = totals {
        body.insert("totals".into(), json!(totals));
    }
    let reflections: Vec<_> = reflections.iter().map(redact_reflection).collect();
    body.insert("reflections".into(), json!(reflections));
    Json(Value::Object(body)).into_response()
}

/// Every model call, newest first and redacted, with the totals.
pub async fn ledger(state: State, params: Params) -> Response {
    let Some(ledger) = &state.ledger else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no inference ledger is configured");
    };
    let limit = bounded(param(&params, "limit"), 50, 1, 200);
    let filter = RecentFilter {
        lane: match param(&params, "lane") {
            Some("mascot") => Some(Lane::Mascot),
            Some("brain") => Some(Lane::Brain),
            _ => None,
        },
        before: param(&params, "before")
            .filter(|before| !before.is_empty())
            .map(str::to_owned),
    };
    let (totals, entries) = tokio::join!(ledger.totals(), ledger.recent(limit, &filter));
    let (totals, entries) = match (totals, entries) {
        (Ok(totals), Ok(entries)) => (totals, entries),
        (Err(failure), _) | (_, Err(failure)) => return internal(failure),
    };
    let entries: Vec<_> = entries.iter().map(redact_inference).collect();
    Json(json!({
        "source": "live",
        "generatedAt": iso_now(),
        "totals": totals,
        "entries": entries,
    }))
    .into_response()
}

pub async fn brain(state: State, params: Params) -> Response {
    let Some(brain) = &state.brain else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "no brain is mounted");
    };
    let runs = bounded(param(&params, "runs"), 40, 1, 200);
    let steps = bounded(param(&params, "steps"), 300, 1, 1000);
    match brain.snapshot(runs, steps).await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(failure) => internal(failure),
    }
}

#[cfg(test)]
#[path = "feeds_tests.rs"]
mod tests;
