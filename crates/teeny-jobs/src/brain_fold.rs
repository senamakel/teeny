//! The brain's token spend, folded into the inference ledger so the counter
//! on the page counts both minds: a port of `foldUsage` in
//! `legacy/src/ledger/brain.ts` and of its one-minute timer in
//! `legacy/src/index.ts`.
//!
//! The brain is an OpenCompany company whose state is a git working tree,
//! mounted read-only. Only `companies/<slug>/usage.jsonl` is read here: token
//! counts and nothing else. The snapshot the page shows is the server's.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

use chrono::DateTime;
use serde::Deserialize;
use teeny_cadence::clock::iso;
use teeny_store::InferenceLedger;
use teeny_store::inference::{InferenceInput, Lane, Tokens, record_quietly, sha256};
use teeny_store::journal::Outcome;

use crate::ctx::{JobCtx, JobReport};

/// The job's name in reports.
pub const JOB: &str = "brain_usage";

/// How much of the usage file's tail is read per fold.
const USAGE_TAIL: u64 = 4 * 1024 * 1024;

/// One line of `usage.jsonl`: a metered model call, camelCase as the host
/// writes it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageSample {
    at_millis: Option<f64>,
    agent: Option<serde_json::Value>,
    provider: Option<serde_json::Value>,
    model: Option<serde_json::Value>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    kind: Option<String>,
}

/// A string field, or `fallback` when it is absent or not a string.
fn text(value: Option<&serde_json::Value>, fallback: &str) -> String {
    value
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback)
        .to_owned()
}

/// The last `bytes` of a file, as whole lines. A missing file is no lines.
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

/// The brain's checkout and how far its usage has been folded.
pub struct BrainFold {
    dir: PathBuf,
    slug: String,
    /// The newest sample folded, unix ms. In process: the ledger dedupes by
    /// id, so a restart re-reads harmlessly.
    folded_to: AtomicI64,
}

impl BrainFold {
    pub fn new(dir: impl AsRef<Path>, slug: &str) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
            slug: slug.to_owned(),
            folded_to: AtomicI64::new(0),
        }
    }

    /// Whether there is a brain at the path at all.
    pub async fn exists(&self) -> bool {
        let definition = self
            .dir
            .join("company")
            .join(&self.slug)
            .join("company.toml");
        tokio::fs::metadata(definition)
            .await
            .is_ok_and(|meta| meta.is_file())
    }

    /// Offers every usage sample from the last fold on to the ledger; how
    /// many were offered. The ledger drops the ones it already has.
    pub async fn fold_usage(&self, ledger: &dyn InferenceLedger) -> usize {
        let path = self
            .dir
            .join("companies")
            .join(&self.slug)
            .join("usage.jsonl");
        let folded_to = self.folded_to.load(Ordering::SeqCst);
        let mut newest = folded_to;
        let mut offered = 0;
        for line in tail_lines(&path, USAGE_TAIL).await {
            let Ok(sample) = serde_json::from_str::<UsageSample>(&line) else {
                continue;
            };
            let Some(at) = sample.at_millis.map(|at| at as i64) else {
                continue;
            };
            if at < folded_to {
                continue;
            }
            let Some(when) = DateTime::from_timestamp_millis(at) else {
                continue;
            };
            let input = sample.input_tokens.unwrap_or(0);
            let output = sample.output_tokens.unwrap_or(0);
            let model = match text(sample.model.as_ref(), "") {
                model if !model.is_empty() => model,
                _ => text(sample.provider.as_ref(), "unknown"),
            };
            let entry = InferenceInput {
                id: Some(format!("b-{at}-{}", &sha256(&[&line])[..12])),
                at: Some(iso(when)),
                kind: sample.kind.clone(),
                ..InferenceInput::new(
                    Lane::Brain,
                    text(sample.agent.as_ref(), "company"),
                    model,
                    Tokens {
                        input,
                        output,
                        reasoning: 0,
                        cached: sample.cached_input_tokens.unwrap_or(0),
                        total: input + output,
                    },
                )
            };
            record_quietly(Some(ledger), entry).await;
            offered += 1;
            newest = newest.max(at);
        }
        // Samples in the same millisecond as the newest could still arrive,
        // so the cursor stays on it; the ledger's id check makes the overlap free.
        self.folded_to.store(newest, Ordering::SeqCst);
        offered
    }
}

/// One fold, when there is a brain and a ledger to fold into.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let conversation = ctx.conversation()?;
    let (Some(brain), Some(ledger)) = (&conversation.brain, &conversation.inferences) else {
        return Ok(JobReport::declined(JOB, "no brain to fold"));
    };
    if !brain.exists().await {
        return Ok(JobReport::declined(JOB, "no brain at the path"));
    }
    let offered = brain.fold_usage(ledger.as_ref()).await;
    if offered > 0 {
        tracing::debug!(offered, "brain usage folded");
    }
    Ok(JobReport {
        note: Some(format!("offered {offered}")),
        ..JobReport::ran(JOB, Outcome::Skipped)
    })
}

#[cfg(test)]
#[path = "brain_fold_tests.rs"]
mod tests;
