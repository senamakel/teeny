//! The operator's token: `Authorization: Bearer <token>` from a script, or
//! HTTP Basic with the token as the password from a browser opening the
//! review page. Compared in constant time.

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use base64::Engine as _;

use super::reply::error;

/// The token a client sent, from either shape.
pub fn token_from(header: Option<&str>) -> Option<String> {
    let header = header?;
    if let Some(token) = header.strip_prefix("Bearer ") {
        return Some(token.to_owned());
    }
    let encoded = header.strip_prefix("Basic ")?;
    // Node's base64 decoder is lenient; an undecodable header is a wrong token.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded.trim()))
        .unwrap_or_default();
    let decoded = String::from_utf8_lossy(&bytes);
    Some(match decoded.split_once(':') {
        Some((_user, password)) => password.to_owned(),
        None => decoded.into_owned(),
    })
}

/// Equal length and equal bytes, without an early exit.
pub fn matches_token(provided: Option<&str>, expected: &str) -> bool {
    let Some(provided) = provided.filter(|provided| !provided.is_empty()) else {
        return false;
    };
    let (a, b) = (provided.as_bytes(), expected.as_bytes());
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .fold(0u8, |difference, (x, y)| difference | (x ^ y))
            == 0
}

/// `Ok` when the request carries the operator's token; otherwise the answer
/// to send. `browser` asks for a Basic challenge rather than a Bearer one.
// A refusal is a whole response; it is built once, on the way out.
#[allow(clippy::result_large_err)]
pub fn authorise(token: Option<&str>, headers: &HeaderMap, browser: bool) -> Result<(), Response> {
    let Some(expected) = token else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "AGENT_API_TOKEN is not set",
        ));
    };
    let sent = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if matches_token(token_from(sent).as_deref(), expected) {
        return Ok(());
    }
    let challenge = if browser {
        "Basic realm=\"teeny\", charset=\"UTF-8\""
    } else {
        "Bearer"
    };
    let mut refused = error(StatusCode::UNAUTHORIZED, "unauthorized");
    refused.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(challenge),
    );
    Err(refused)
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
