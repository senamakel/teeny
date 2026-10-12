//! Clients pointed at a mock server, for the command tests.

use std::sync::Arc;

use teeny_core::Secret;
use teeny_x::oauth::OAuth1;
use teeny_x::{NoopGuard, XApi, XPublisher};
use wiremock::MockServer;

/// The account's client, with an app bearer, as production runs.
pub fn x(server: &MockServer) -> XApi {
    XApi::new(
        "teenydroid",
        OAuth1::new("ck", Secret::new("cs"), "at", Secret::new("ats")),
        Some(Secret::new("app-bearer")),
    )
    .with_base_url(server.uri())
}

pub fn publisher(server: &MockServer, dry_run: bool) -> XPublisher {
    XPublisher::new(Arc::new(x(server)), Arc::new(NoopGuard), dry_run)
}

pub fn printed(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("utf-8 output")
}

/// A configuration as the environment would give it, `DRY_RUN` as stated.
pub fn config(dry_run: bool) -> teeny_core::Config {
    let mut vars = teeny_agent::deps::fake_vars();
    vars.retain(|(key, _)| *key != "DRY_RUN");
    vars.push(("DRY_RUN", if dry_run { "true" } else { "false" }));
    teeny_core::Config::from_vars(vars).expect("the test configuration loads")
}

/// The answer a `teeny-reply` turn gives, as JSON.
pub fn reply_answer(text: &str) -> String {
    serde_json::json!({ "reply": text, "incomingTone": "neutral" }).to_string()
}

/// Every request that would have changed something on X.
pub async fn x_writes(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request.method.as_str() != "GET")
        .map(|request| format!("{} {}", request.method, request.url.path()))
        .collect()
}
