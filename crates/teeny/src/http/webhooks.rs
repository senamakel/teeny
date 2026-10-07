//! `/webhooks/x`: X's challenge-response check, and its signed deliveries.
//!
//! A delivery is verified against the raw bytes, copied on to the forwarder
//! (not awaited: X's clock is the one this handler answers to), then handed
//! to the intake, which answers once the queue has it.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::body::Bytes;
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use teeny_x::intake::Source;
use teeny_x::webhook::{crc_response, is_valid_signature};

use super::State;
use super::body::Payload;
use super::reply::{error, internal};

/// The header X signs a delivery in.
pub const SIGNATURE_HEADER: &str = "x-twitter-webhooks-signature";

/// Where a copy of every verified delivery goes. Never fails: a copy that
/// does not arrive is logged and lost.
#[async_trait]
pub trait Forwarder: Send + Sync {
    async fn forward(&self, body: Bytes, signature: Option<String>);
}

/// The forwarder over HTTP, `WEBHOOK_FORWARD_URL` and its optional bearer.
pub struct HttpForwarder {
    client: reqwest::Client,
    url: String,
    token: Option<String>,
    timeout: Duration,
}

impl HttpForwarder {
    /// One attempt may take `timeout` (10 seconds in the service).
    pub fn new(url: impl Into<String>, token: Option<String>, timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.into(),
            token,
            timeout,
        }
    }
}

#[async_trait]
impl Forwarder for HttpForwarder {
    async fn forward(&self, body: Bytes, signature: Option<String>) {
        let bytes = body.len();
        let mut request = self
            .client
            .post(&self.url)
            .timeout(self.timeout)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(signature) = signature {
            request = request.header(SIGNATURE_HEADER, signature);
        }
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                tracing::info!(url = %self.url, bytes, "webhook copy delivered");
            }
            Ok(response) => {
                tracing::warn!(url = %self.url, status = response.status().as_u16(), "webhook copy refused");
            }
            Err(failure) => {
                tracing::warn!(url = %self.url, err = %failure, "webhook copy not delivered");
            }
        }
    }
}

pub async fn challenge(state: State, params: Query<HashMap<String, String>>) -> Response {
    match params.get("crc_token").filter(|token| !token.is_empty()) {
        Some(token) => Json(json!({
            "response_token": crc_response(token, &state.consumer_secret),
        }))
        .into_response(),
        None => error(StatusCode::BAD_REQUEST, "crc_token is required"),
    }
}

pub async fn delivery(state: State, headers: HeaderMap, payload: Payload) -> Response {
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    // Only a JSON body keeps its bytes, and the signature is over the bytes.
    let (Some(raw), Some(body)) = (payload.raw, payload.body) else {
        return error(StatusCode::UNAUTHORIZED, "invalid webhook signature");
    };
    if !is_valid_signature(&raw, signature.as_deref(), &state.consumer_secret) {
        return error(StatusCode::UNAUTHORIZED, "invalid webhook signature");
    }

    // The copy goes out first and is not awaited: the receiver's clock is
    // its own, and X's is the one this handler answers to.
    if let Some(forwarder) = state.forwarder.clone() {
        let copy = raw.clone();
        let signature = signature.clone();
        tokio::spawn(async move { forwarder.forward(copy, signature).await });
    }

    // X retries a delivery it does not see acknowledged quickly, so the work
    // is queued, not done: the intake answers once the queue has it.
    match state.intake.deliver(&body, Source::Webhook).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(failure) => internal(failure),
    }
}

#[cfg(test)]
#[path = "webhooks_tests.rs"]
mod tests;
