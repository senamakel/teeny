//! The answers every route shares: the `{ error }` body, Fastify's own error
//! shape for what Fastify answered itself, and the clamped numeric params.

use axum::Json;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// `{ "error": message }` with a status, the shape every handler refused with.
pub fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// What Fastify sends for an error it raised itself.
pub fn fastify(status: StatusCode, code: Option<&str>, message: impl Into<String>) -> Response {
    let mut body = json!({
        "statusCode": status.as_u16(),
        "error": status.canonical_reason().unwrap_or(""),
        "message": message.into(),
    });
    if let Some(code) = code {
        body["code"] = json!(code);
    }
    (status, Json(body)).into_response()
}

/// A handler that threw: Fastify's 500.
pub fn internal(error: impl std::fmt::Display) -> Response {
    fastify(StatusCode::INTERNAL_SERVER_ERROR, None, error.to_string())
}

/// Fastify's 404 for a route it does not have, whatever the method.
pub async fn not_found(method: Method, uri: Uri) -> Response {
    fastify(
        StatusCode::NOT_FOUND,
        None,
        format!("Route {method}:{} not found", uri.path()),
    )
}

/// `Math.min(Math.max(Number(raw) || default, min), max)`.
pub fn bounded(raw: Option<&str>, default: usize, min: usize, max: usize) -> usize {
    let number = raw
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|number| *number != 0.0 && !number.is_nan())
        .unwrap_or(default as f64);
    // Truncated, as a fractional limit reached the stores as their floor.
    number.clamp(min as f64, max as f64) as usize
}

#[cfg(test)]
#[path = "reply_tests.rs"]
mod tests;
