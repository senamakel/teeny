//! A request body, read the way the TypeScript service's Fastify read it:
//! at most 1 MiB, JSON kept as raw bytes beside the parse (the webhook's
//! signature is over the bytes), `text/plain` as a string, and anything else
//! refused with Fastify's own errors.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{StatusCode, header};
use axum::response::Response;
use serde_json::Value;

use super::reply::fastify;

/// Fastify's default `bodyLimit`.
pub const BODY_LIMIT: usize = 1024 * 1024;

/// A parsed body.
#[derive(Debug, Default)]
pub struct Payload {
    /// The exact bytes, for a JSON body only.
    pub raw: Option<Bytes>,
    /// The parse: the JSON value, or the text of a `text/plain` body.
    pub body: Option<Value>,
}

impl<S: Send + Sync> FromRequest<S> for Payload {
    type Rejection = Response;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Response> {
        let headers = request.headers();
        let media_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            });
        let declared = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        if declared.is_some_and(|length| length > BODY_LIMIT) {
            return Err(too_large());
        }
        let bytes = axum::body::to_bytes(request.into_body(), BODY_LIMIT)
            .await
            .map_err(|_| too_large())?;

        match media_type.as_deref() {
            None | Some("") if bytes.is_empty() => Ok(Self::default()),
            Some("application/json") => {
                if bytes.is_empty() {
                    return Err(fastify(
                        StatusCode::BAD_REQUEST,
                        Some("FST_ERR_CTP_EMPTY_JSON_BODY"),
                        "Body cannot be empty when content-type is set to 'application/json'",
                    ));
                }
                let body = serde_json::from_slice(&bytes).map_err(|_| {
                    fastify(
                        StatusCode::BAD_REQUEST,
                        Some("FST_ERR_CTP_INVALID_JSON_BODY"),
                        "Body is not valid JSON but content-type is set to 'application/json'",
                    )
                })?;
                Ok(Self {
                    raw: Some(bytes),
                    body: Some(body),
                })
            }
            Some("text/plain") => Ok(Self {
                raw: None,
                body: Some(Value::String(String::from_utf8_lossy(&bytes).into_owned())),
            }),
            _ => Err(fastify(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Some("FST_ERR_CTP_INVALID_MEDIA_TYPE"),
                "Unsupported Media Type",
            )),
        }
    }
}

fn too_large() -> Response {
    fastify(
        StatusCode::PAYLOAD_TOO_LARGE,
        Some("FST_ERR_CTP_BODY_TOO_LARGE"),
        "Request body is too large",
    )
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
