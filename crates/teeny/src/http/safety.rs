//! The review side of the content filter: `/safety`, the page an operator
//! opens in a browser, and the JSON it calls. Behind the operator's token,
//! with a Basic challenge so a browser can get in.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use teeny_safety::review::{Release, Requeue, ReviewDependencies, review_hold};
use teeny_store::holds::HoldDecision;
use teeny_store::{HoldLog, HoldStatus};

use super::State;
use super::auth::authorise;
use super::body::Payload;
use super::reply::{bounded, error, internal};
use super::validate::{Issues, object, one_of, string};
use crate::mentions::QueueAdapter;

/// The page, as the TypeScript service rendered it.
pub const DASHBOARD_HTML: &str = include_str!("dashboard.html");

/// What a review reaches.
pub struct Review {
    pub holds: Arc<dyn HoldLog>,
    /// Publishes an approved draft. Absent, approving a draft only records it.
    pub release: Option<Arc<dyn Release>>,
}

/// The review, once the operator is through; otherwise the answer to send.
#[allow(clippy::result_large_err)]
fn review<'a>(state: &'a State, headers: &HeaderMap) -> Result<&'a Review, Response> {
    let Some(review) = &state.review else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "no hold log is configured",
        ));
    };
    authorise(state.agent_token.as_deref(), headers, true)?;
    Ok(review)
}

pub async fn dashboard(state: State, headers: HeaderMap) -> Response {
    if let Err(refused) = review(&state, &headers) {
        return refused;
    }
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        DASHBOARD_HTML,
    )
        .into_response()
}

pub async fn holds(
    state: State,
    headers: HeaderMap,
    params: Query<HashMap<String, String>>,
) -> Response {
    let review = match review(&state, &headers) {
        Ok(review) => review,
        Err(refused) => return refused,
    };
    let status = match params.get("status").map(String::as_str) {
        Some("pending") => Some(HoldStatus::Pending),
        Some("approved") => Some(HoldStatus::Approved),
        Some("rejected") => Some(HoldStatus::Rejected),
        _ => None,
    };
    let limit = bounded(params.get("limit").map(String::as_str), 100, 1, 500);
    let (holds, counts) = tokio::join!(
        review.holds.list(status, Some(limit)),
        review.holds.counts()
    );
    match (holds, counts) {
        (Ok(holds), Ok(counts)) => {
            Json(json!({ "counts": counts, "holds": holds })).into_response()
        }
        (Err(failure), _) | (_, Err(failure)) => internal(failure),
    }
}

pub async fn decide(
    state: State,
    Path(id): Path<String>,
    headers: HeaderMap,
    payload: Payload,
) -> Response {
    let review = match review(&state, &headers) {
        Ok(review) => review,
        Err(refused) => return refused,
    };
    let mut issues = Issues::new();
    let (decision, note) = match object(payload.body.as_ref(), &mut issues) {
        Some(fields) => (
            one_of(
                fields,
                "decision",
                &["approved", "rejected"],
                None,
                &mut issues,
            ),
            string(fields, "note", 0, 500, false, &mut issues),
        ),
        None => (None, None),
    };
    let decision = match decision.as_deref() {
        Some("approved") if issues.is_empty() => HoldDecision::Approved,
        Some("rejected") if issues.is_empty() => HoldDecision::Rejected,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "error": "decision must be approved or rejected", "details": issues }),
                ),
            )
                .into_response();
        }
    };

    let requeue = state.queue.clone().map(QueueAdapter);
    let gate = state.safety.clone();
    let refresh = move || {
        if let Some(gate) = &gate {
            gate.refresh_approved_links();
        }
    };
    let deps = ReviewDependencies {
        holds: review.holds.as_ref(),
        requeue: requeue.as_ref().map(|queue| queue as &dyn Requeue),
        release: review.release.as_deref(),
        refresh_links: state
            .safety
            .is_some()
            .then_some(&refresh as &(dyn Fn() + Sync)),
    };
    match review_hold(&id, decision, &deps, note.as_deref()).await {
        Ok(Some(hold)) => {
            tracing::info!(hold_id = %id, ?decision, effect = ?hold.effect, "reviewed a hold");
            Json(hold).into_response()
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "no such hold"),
        Err(failure) => {
            tracing::error!(hold_id = %id, err = %failure, "could not act on a hold decision");
            error(StatusCode::BAD_GATEWAY, failure.to_string())
        }
    }
}

#[cfg(test)]
#[path = "safety_tests.rs"]
mod tests;
