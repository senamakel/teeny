//! The brain, as `/brain` serves it: the roster, the workflows and the latest
//! runs of the OpenCompany company in its checkout. The shapes are
//! `legacy/src/ledger/brain.ts`'s, which `web/src/lib/company.ts` reads.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainAgent {
    pub id: String,
    pub role: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainNode {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainEdge {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainWorkflow {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub nodes: Vec<BrainNode>,
    pub edges: Vec<BrainEdge>,
}

/// What a step did, cut to what the page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainStepBody {
    pub kind: String,
    pub status: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// One step of a run. The numbers are passed through as the host wrote them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainStep {
    pub run_id: String,
    pub step_seq: Value,
    pub at_millis: Value,
    pub step: BrainStepBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainCompany {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub slug: String,
}

/// The whole of `/brain`. A run is the host's own row, whole: the page reads
/// its fields by the host's names, so it is served as it was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSnapshot {
    pub source: String,
    pub generated_at: String,
    pub company: BrainCompany,
    pub agents: Vec<BrainAgent>,
    pub workflows: Vec<BrainWorkflow>,
    pub runs: Vec<Value>,
    pub steps: Vec<BrainStep>,
}

/// Whatever can produce the snapshot: the checkout, or a fake.
#[async_trait]
pub trait BrainSource: Send + Sync {
    /// The latest `runs` runs and at most `steps` of their steps.
    async fn snapshot(&self, runs: usize, steps: usize) -> anyhow::Result<BrainSnapshot>;
}
