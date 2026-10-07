//! `teeny healthcheck`: asks the running service's `/health` on the
//! loopback, for the container's health check.

use std::time::Duration;

use anyhow::{Context as _, bail};

/// How long the check waits.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// `/health` on the loopback at `PORT`, 3001 when unset or unreadable.
pub fn health_url(port: Option<&str>) -> String {
    let port = port
        .and_then(|port| port.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(3001);
    format!("http://127.0.0.1:{port}/health")
}

/// Ok when the service answers 2xx with `"ok": true`.
pub async fn check(url: &str) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .get(url)
        .timeout(TIMEOUT)
        .send()
        .await
        .with_context(|| format!("no answer from {url}"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("{url} answered {status}");
    }
    let body: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("{url} did not answer JSON"))?;
    if body.get("ok") != Some(&serde_json::Value::Bool(true)) {
        bail!("{url} is not ok: {body}");
    }
    Ok(())
}

#[cfg(test)]
#[path = "healthcheck_tests.rs"]
mod tests;
