//! The bits every OpenAI-shaped client here shares.

use serde_json::Value;

/// `baseUrl.replace(/\/$/, '') + path`.
pub(crate) fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{path}", base_url.strip_suffix('/').unwrap_or(base_url))
}

/// The response body as JSON, or `{}` when it is not JSON: a provider's error
/// page should surface as its status, not as a parse error.
pub(crate) async fn json_body(response: reqwest::Response) -> Value {
    response.json::<Value>().await.unwrap_or_else(|_| Value::Object(Default::default()))
}

/// `body.error.message ?? 'no detail'`.
pub(crate) fn error_detail(body: &Value) -> &str {
    body.pointer("/error/message").and_then(Value::as_str).unwrap_or("no detail")
}
