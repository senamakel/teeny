//! The brain, as `/brain` serves it: the roster, the workflows and the latest
//! runs of the OpenCompany company in its checkout. The shapes are
//! `legacy/src/ledger/brain.ts`'s, which `web/src/lib/company.ts` reads.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// The brain's checkout: the definition under `company/<slug>/`, the
/// runtime record under `companies/<slug>/`. Read-only; a file mid-edit or a
/// torn line at the tail is skipped, never an error.
pub struct BrainReader {
    dir: PathBuf,
    slug: String,
}

/// How much of each record file is read, from the end.
const RUNS_TAIL: u64 = 512 * 1024;
const STEPS_TAIL: u64 = 2 * 1024 * 1024;
/// A step's detail and result are cut to this many characters.
const STEP_TEXT: usize = 600;

impl BrainReader {
    pub fn new(dir: impl Into<PathBuf>, slug: &str) -> Self {
        Self {
            dir: dir.into(),
            slug: slug.to_owned(),
        }
    }

    fn definition(&self) -> PathBuf {
        self.dir.join("company").join(&self.slug)
    }

    fn record(&self) -> PathBuf {
        self.dir.join("companies").join(&self.slug)
    }

    /// Whether there is a brain at the path at all. The usage fold asks
    /// first; it arrives with the schedule.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn exists(&self) -> bool {
        tokio::fs::metadata(self.definition().join("company.toml"))
            .await
            .is_ok_and(|meta| meta.is_file())
    }

    async fn company(&self) -> BrainCompany {
        let company = read_toml(&self.definition().join("company.toml"))
            .await
            .and_then(|doc| doc.get("company").and_then(toml::Value::as_table).cloned());
        BrainCompany {
            name: company
                .as_ref()
                .and_then(|company| text(company, "name"))
                .unwrap_or_else(|| self.slug.clone()),
            output: company.as_ref().and_then(|company| text(company, "output")),
            slug: self.slug.clone(),
        }
    }

    async fn agents(&self) -> Vec<BrainAgent> {
        let mut agents = Vec::new();
        for (id, doc) in read_tomls(&self.definition().join("agents")).await {
            agents.push(BrainAgent {
                role: text(&doc, "role").unwrap_or_else(|| id.clone()),
                description: text(&doc, "description").unwrap_or_default(),
                tier: text(&doc, "tier"),
                id,
            });
        }
        agents
    }

    async fn workflows(&self) -> Vec<BrainWorkflow> {
        let mut workflows = Vec::new();
        for (stem, doc) in read_tomls(&self.definition().join("workflows")).await {
            let id = text(&doc, "id").unwrap_or(stem);
            let nodes = tables(&doc, "node")
                .map(|node| {
                    let id = text(&node, "id").unwrap_or_default();
                    BrainNode {
                        kind: text(&node, "kind").unwrap_or_else(|| "agent".into()),
                        name: text(&node, "name").unwrap_or_else(|| id.clone()),
                        summary: text(&node, "summary"),
                        agent: text(&node, "agent"),
                        schedule: text(&node, "schedule"),
                        id,
                    }
                })
                .collect();
            let edges = tables(&doc, "edge")
                .map(|edge| BrainEdge {
                    from: text(&edge, "from").unwrap_or_default(),
                    to: text(&edge, "to").unwrap_or_default(),
                    label: text(&edge, "label"),
                })
                .collect();
            workflows.push(BrainWorkflow {
                name: text(&doc, "name").unwrap_or_else(|| id.clone()),
                description: text(&doc, "description"),
                id,
                nodes,
                edges,
            });
        }
        workflows
    }

    /// The latest runs, newest first, one row per run. The host rewrites a
    /// run's row as it moves; the last occurrence is the current one, kept in
    /// the place of the first.
    async fn runs(&self, limit: usize) -> Vec<Value> {
        let mut order: Vec<String> = Vec::new();
        let mut by_id: HashMap<String, Value> = HashMap::new();
        for row in json_lines(&tail_lines(&self.record().join("runs.jsonl"), RUNS_TAIL).await) {
            let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            if by_id.insert(id.clone(), row).is_none() {
                order.push(id);
            }
        }
        let mut runs: Vec<Value> = order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect();
        let created = |run: &Value| {
            run.get("createdAtMillis")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        runs.sort_by(|a, b| created(b).total_cmp(&created(a)));
        runs.truncate(limit);
        runs
    }

    async fn steps(&self, run_ids: &HashSet<&str>, limit: usize) -> Vec<BrainStep> {
        let rows =
            json_lines(&tail_lines(&self.record().join("run-steps.jsonl"), STEPS_TAIL).await);
        let mut steps: Vec<BrainStep> = rows
            .into_iter()
            .filter_map(|row| {
                let run_id = row.get("runId")?.as_str()?;
                if !run_ids.contains(run_id) {
                    return None;
                }
                let step = row.get("step").filter(|step| truthy(step))?;
                let field = |name: &str| step.get(name).and_then(Value::as_str);
                let cut =
                    |name: &str| field(name).map(|text| text.chars().take(STEP_TEXT).collect());
                Some(BrainStep {
                    run_id: run_id.to_owned(),
                    step_seq: row.get("stepSeq").cloned().unwrap_or(Value::Null),
                    at_millis: row.get("atMillis").cloned().unwrap_or(Value::Null),
                    step: BrainStepBody {
                        kind: field("kind").unwrap_or("note").to_owned(),
                        status: field("status").unwrap_or("ok").to_owned(),
                        label: field("label").unwrap_or_default().to_owned(),
                        detail: cut("detail"),
                        result: cut("result"),
                    },
                })
            })
            .collect();
        let at = |step: &BrainStep| step.at_millis.as_f64().unwrap_or(0.0);
        steps.sort_by(|a, b| at(a).total_cmp(&at(b)));
        let skip = steps.len().saturating_sub(limit);
        steps.split_off(skip)
    }
}

#[async_trait]
impl BrainSource for BrainReader {
    async fn snapshot(&self, runs: usize, steps: usize) -> anyhow::Result<BrainSnapshot> {
        let (company, agents, workflows, runs) = tokio::join!(
            self.company(),
            self.agents(),
            self.workflows(),
            self.runs(runs)
        );
        let run_ids: HashSet<&str> = runs
            .iter()
            .filter_map(|run| run.get("id").and_then(Value::as_str))
            .collect();
        let steps = self.steps(&run_ids, steps).await;
        Ok(BrainSnapshot {
            source: "live".into(),
            generated_at: teeny_store::iso_now(),
            company,
            agents,
            workflows,
            runs,
            steps,
        })
    }
}

/// JavaScript truthiness, for the one place the port needs it.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn text(table: &toml::Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}

/// The array of tables at `key`; anything in it that is not a table reads
/// as an empty one.
fn tables(doc: &toml::Table, key: &str) -> impl Iterator<Item = toml::Table> {
    doc.get(key)
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| match item {
            toml::Value::Table(table) => table,
            _ => toml::Table::new(),
        })
}

async fn read_toml(path: &Path) -> Option<toml::Table> {
    let source = tokio::fs::read_to_string(path).await.ok()?;
    toml::from_str(&source).ok()
}

/// Every `*.toml` in a directory, by file name, with its stem.
async fn read_tomls(dir: &Path) -> Vec<(String, toml::Table)> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return Vec::new();
    };
    let mut names = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(name) = entry.file_name().to_str()
            && name.ends_with(".toml")
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    let mut docs = Vec::new();
    for name in names {
        if let Some(doc) = read_toml(&dir.join(&name)).await {
            docs.push((name.trim_end_matches(".toml").to_owned(), doc));
        }
    }
    docs
}

fn json_lines(lines: &[String]) -> Vec<Value> {
    lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The last `bytes` of a file, as whole, non-blank lines. A missing file is
/// no lines.
pub async fn tail_lines(path: &Path, bytes: u64) -> Vec<String> {
    use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return Vec::new();
    };
    let Ok(size) = file.metadata().await.map(|meta| meta.len()) else {
        return Vec::new();
    };
    let start = size.saturating_sub(bytes);
    let mut buffer = Vec::new();
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err()
        || file.read_to_end(&mut buffer).await.is_err()
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buffer);
    let mut lines = text.split('\n');
    // A cut that landed mid-line leaves a fragment at the front; drop it.
    if start > 0 {
        lines.next();
    }
    lines
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[path = "brain_tests.rs"]
mod tests;
