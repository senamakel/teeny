//! What the store says about itself, reduced to what decides recall
//! quality. A port of `legacy/src/memory/health.ts`.
//!
//! Nothing on the turn's path notices a store that cannot embed: recall
//! still answers, from the keyword index alone, and the answers are merely
//! worse. From 2026-10-04 the store embedded nothing for a day because the
//! router had dropped the embedding ladder, and the only sign was replies
//! that seemed to have forgotten everyone. This is the sign, on a schedule,
//! in the log.

use std::time::Duration;

use serde::Deserialize;

/// How long the readiness report may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How much of the store's last embedding error is kept.
const LAST_ERROR_CHARS: usize = 300;

/// Whether embeddings work. When they do not, recall is keyword-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Embeddings {
    pub ok: bool,
    pub ok_count: Option<u64>,
    pub fail_count: Option<u64>,
    pub last_error: Option<String>,
}

/// The store's readiness, as the account reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CortexHealth {
    pub reachable: bool,
    /// The store's own verdict: true when any check is failing.
    pub degraded: bool,
    pub embeddings: Embeddings,
    pub provider: Option<String>,
    /// Why the store could not be read, when it could not.
    pub error: Option<String>,
}

#[derive(Deserialize, Default)]
struct ReadyBody {
    degraded: Option<bool>,
    embedding_provider: Option<String>,
    checks: Option<Checks>,
}

#[derive(Deserialize, Default)]
struct Checks {
    embeddings: Option<EmbeddingCheck>,
}

#[derive(Deserialize, Default)]
struct EmbeddingCheck {
    ok: Option<bool>,
    ok_count: Option<u64>,
    fail_count: Option<u64>,
    last_error: Option<String>,
}

/// Reads `GET /v1/admin/ready` on the store at `api_url` (any path on it is
/// replaced). Never fails: an unreachable store is a report saying so.
pub async fn check_cortex(api_url: &str) -> CortexHealth {
    match read(api_url).await {
        Ok(health) => health,
        Err(error) => CortexHealth {
            reachable: false,
            degraded: true,
            embeddings: Embeddings::default(),
            provider: None,
            error: Some(error),
        },
    }
}

async fn read(api_url: &str) -> Result<CortexHealth, String> {
    let url = reqwest::Url::parse(api_url)
        .and_then(|base| base.join("/v1/admin/ready"))
        .map_err(|error| format!("the store URL {api_url:?}: {error}"))?;
    let response = reqwest::Client::new()
        .get(url)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status_ok = response.status().is_success();
    let body: ReadyBody = response.json().await.unwrap_or_default();
    let check = body
        .checks
        .and_then(|checks| checks.embeddings)
        .unwrap_or_default();
    Ok(CortexHealth {
        reachable: true,
        degraded: body.degraded.unwrap_or(!status_ok),
        embeddings: Embeddings {
            ok: check.ok.unwrap_or(status_ok),
            ok_count: check.ok_count,
            fail_count: check.fail_count,
            last_error: check
                .last_error
                .filter(|error| !error.is_empty())
                .map(|error| error.chars().take(LAST_ERROR_CHARS).collect()),
        },
        provider: check_provider(body.embedding_provider),
        error: None,
    })
}

fn check_provider(provider: Option<String>) -> Option<String> {
    provider.filter(|provider| !provider.is_empty())
}

/// Whether `health` deserves a warning, and the line it is logged under.
pub fn verdict(health: &CortexHealth) -> (bool, &'static str) {
    if !health.reachable {
        (
            true,
            "memory: the store is unreachable; every recall this hour will come back empty",
        )
    } else if !health.embeddings.ok {
        (
            true,
            "memory: the store cannot embed; recall is keyword-only until it can",
        )
    } else if health.degraded {
        (true, "memory: the store reports itself degraded")
    } else {
        (false, "memory: the store is healthy")
    }
}

/// Reads the report and logs it at the level it deserves; returns it.
pub async fn report_cortex_health(api_url: &str) -> CortexHealth {
    let health = check_cortex(api_url).await;
    let (warn, message) = verdict(&health);
    if warn {
        tracing::warn!(cortex = ?health, "{message}");
    } else {
        tracing::info!(provider = ?health.provider, embeddings = ?health.embeddings, "{message}");
    }
    health
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
