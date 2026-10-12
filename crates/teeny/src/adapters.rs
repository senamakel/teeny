//! The HTTP server's seams over the agents: the operator's `/agent/run` on
//! `teeny-operator`, and the browser's state for `/health`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_agent::posture::Belt;
use teeny_agent::runtime::{RuntimeSettings, openhuman_config};
use teeny_jobs::runner::{AgentKey, AgentRun, Origin};

use crate::http::operator::{RunResult, RunStep};
use crate::http::{AgentRunner, BrowserStatus};

/// `/agent/run`: one turn of `teeny-operator` with the operator's origin
/// (trusted, behind the bearer token), continuing `threadId` when given.
/// The content filter has already read the task (`http::operator::run`).
pub struct OperatorAgent(pub Arc<dyn teeny_jobs::runner::AgentRunner>);

#[async_trait]
impl AgentRunner for OperatorAgent {
    async fn run(&self, task: &str, thread_id: Option<&str>) -> anyhow::Result<RunResult> {
        let outcome = self
            .0
            .run(AgentRun {
                agent: AgentKey::Belt(Belt::Operator),
                origin: Origin::Operator,
                prompt: task.to_owned(),
                session: thread_id.map(str::to_owned),
                model: None,
                label: "agent.run".into(),
            })
            .await?;
        Ok(RunResult {
            output: outcome.reply,
            steps: outcome
                .tool_calls
                .into_iter()
                .map(|call| RunStep {
                    tool: call.name,
                    input: call.arguments,
                    result: Some(call.result),
                })
                .collect(),
        })
    }
}

/// The research browser as the runtime was configured with it, read once at
/// boot. Nothing here launches Chrome: the browser is a module that starts
/// on a turn's first browse. It counts as configured when the module that
/// carries it (`TINYCOMPUTER_MODULE_PATH`) is set and `BROWSER_RESEARCH` is
/// on.
pub struct BrowserState(Value);

impl BrowserState {
    pub fn from_settings(settings: &RuntimeSettings) -> Self {
        let config = openhuman_config(settings);
        let browser = config.browser;
        let module = settings
            .tinycomputer_module
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        Self(json!({
            "configured": browser.enabled && module.is_some(),
            "research": settings.browser_research,
            "allowedHosts": config.http_request.allowed_domains,
            "launched": false,
            "module": module,
            "chromePath": browser.chrome_path,
            "headless": browser.headless,
            "profile": browser.profile_mode,
            "profilePath": browser.profile_path,
            "unattendedActions": browser.unattended_actions,
        }))
    }
}

#[async_trait]
impl BrowserStatus for BrowserState {
    async fn status(&self) -> Value {
        self.0.clone()
    }
}

#[cfg(test)]
#[path = "adapters_tests.rs"]
mod tests;
