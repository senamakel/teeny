//! CortexDB over HTTP: the `/v1/experience` and `/v1/recall` calls the
//! TypeScript service made through the `cortexdbai` SDK, with the same
//! headers and bodies.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use teeny_core::Secret;
use teeny_core::config::CortexConfig;

use crate::memory::{ABOUT_QUERY, format_recall};
use crate::{Experience, Memory, MemoryError, Person, Result};

#[path = "cortex_admin.rs"]
mod admin;
pub use admin::ForgetSelector;

/// The SDK's request timeout.
const TIMEOUT: Duration = Duration::from_secs(60);
/// What a recall pass reads.
const INCLUDE: [&str; 4] = ["events", "facts", "beliefs", "episodes"];
/// Token budgets: the person pass is half the general one.
const ABOUT_TOKENS: u32 = 600;
const GENERAL_TOKENS: u32 = 1_200;

/// The account's memory in CortexDB.
#[derive(Debug, Clone)]
pub struct CortexMemory {
    http: reqwest::Client,
    api_url: String,
    actor: String,
    bearer: Secret,
    scope: String,
}

impl CortexMemory {
    /// A client for `api_url`, acting as `actor`, storing under `scope`.
    pub fn new(api_url: &str, actor: &str, bearer: Secret, scope: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("teeny/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            api_url: api_url.trim_end_matches('/').to_owned(),
            actor: actor.to_owned(),
            bearer,
            scope: scope.to_owned(),
        })
    }

    /// The client the configuration describes.
    pub fn from_config(config: &CortexConfig) -> Result<Self> {
        Self::new(
            &config.api_url,
            &config.actor,
            config.token.clone(),
            &config.scope,
        )
    }

    /// POSTs `body` and returns the answer's text, or the error envelope.
    async fn post(&self, path: &str, body: &Value) -> Result<String> {
        let response = self
            .http
            .post(format!("{}{path}", self.api_url))
            .header("X-Cortex-Actor", &self.actor)
            .bearer_auth(self.bearer.expose())
            .json(body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            Ok(text)
        } else {
            Err(api_error(status.as_u16(), &text))
        }
    }

    /// One recall pass. The person pass is filtered to memories observed
    /// about them, on half the general pass's budget.
    async fn recall_pack(&self, query: &str, about: Option<&Person>) -> Result<String> {
        let mut body = json!({
            "scope": self.scope,
            "view": "holistic",
            "query": query,
            "include": INCLUDE,
            "diagnostics": "none",
            "budgets": { "max_tokens": if about.is_some() { ABOUT_TOKENS } else { GENERAL_TOKENS } },
        });
        if let Some(person) = about {
            body["filters"] = json!({ "metadata": { "observed_actor": person.id } });
        }
        let pack: Pack = serde_json::from_str(&self.post("/v1/recall", &body).await?)?;
        Ok(pack.context_block)
    }
}

/// The part of a recall's stratified pack the account reads.
#[derive(Deserialize)]
struct Pack {
    #[serde(default)]
    context_block: String,
}

/// CortexDB's error body.
#[derive(Deserialize, Default)]
struct Envelope {
    error_code: Option<String>,
    message: Option<String>,
}

/// The error a refused call reads as, as the SDK builds it.
fn api_error(status: u16, body: &str) -> MemoryError {
    let envelope = if body.is_empty() {
        Envelope::default()
    } else {
        serde_json::from_str(body).unwrap_or_else(|_| Envelope {
            error_code: Some("UNKNOWN".to_owned()),
            message: Some(body.chars().take(500).collect()),
        })
    };
    MemoryError::Api {
        status,
        code: envelope.error_code.unwrap_or_else(|| "UNKNOWN".to_owned()),
        message: envelope
            .message
            .unwrap_or_else(|| "(no message)".to_owned()),
    }
}

#[async_trait]
impl Memory for CortexMemory {
    async fn capture(&self, experience: Experience) -> Result<()> {
        // `captured` returns at the durable accept point, ~10ms, with
        // indexing continuing in the background.
        match self
            .post(
                "/v1/experience?wait=captured",
                &experience.envelope(&self.scope),
            )
            .await
        {
            Err(error) if error.is_idempotency_conflict() => Ok(()),
            other => other.map(|_| ()),
        }
    }

    async fn recall(&self, query: &str, person: Option<&Person>) -> Result<String> {
        let about = async {
            match person {
                Some(person) => self.recall_pack(ABOUT_QUERY, Some(person)).await,
                None => Ok(String::new()),
            }
        };
        let (about, general) = futures::try_join!(about, self.recall_pack(query, None))?;
        Ok(format_recall(person, &about, &general))
    }
}

#[cfg(test)]
#[path = "cortex_tests.rs"]
mod tests;
