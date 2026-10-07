//! The quotes the account has already made: a JSON file on the data volume,
//! because "never the same post again" is exact and a recall over prose
//! cannot promise it. Written atomically, so a restart mid-write leaves the
//! old list. A port of `legacy/src/quote/store.ts`, same file format.

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const MAX_KEPT: usize = 2_000;

/// One post the account has quoted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotedPost {
    pub post_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_username: Option<String>,
    /// ISO 8601, when the quote went out (or would have, on a dry run).
    pub quoted_at: String,
    /// The account's own post that carried it; absent on a dry run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_post_id: Option<String>,
}

/// Where the quotes are kept.
#[async_trait]
pub trait QuoteLog: Send + Sync {
    async fn list(&self) -> anyhow::Result<Vec<QuotedPost>>;
    async fn record(&self, entry: QuotedPost) -> anyhow::Result<()>;
}

#[derive(Serialize, Deserialize)]
struct LogFile {
    #[serde(default)]
    quoted: Vec<serde_json::Value>,
}

/// `QUOTE_LOG_PATH`.
pub struct FileQuoteLog {
    path: PathBuf,
}

impl FileQuoteLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl QuoteLog for FileQuoteLog {
    async fn list(&self) -> anyhow::Result<Vec<QuotedPost>> {
        let text = match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let file: LogFile = serde_json::from_str(&text)?;
        Ok(file
            .quoted
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok())
            .collect())
    }

    async fn record(&self, entry: QuotedPost) -> anyhow::Result<()> {
        let mut quoted: Vec<QuotedPost> = self
            .list()
            .await?
            .into_iter()
            .filter(|kept| kept.post_id != entry.post_id)
            .collect();
        quoted.push(entry);
        let start = quoted.len().saturating_sub(MAX_KEPT);
        let file = serde_json::json!({ "quoted": &quoted[start..] });
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let temporary = self.path.with_extension("json.tmp");
        tokio::fs::write(
            &temporary,
            format!("{}\n", serde_json::to_string_pretty(&file)?),
        )
        .await?;
        tokio::fs::rename(&temporary, &self.path).await?;
        Ok(())
    }
}

/// For tests and rehearsals: the same contract, nothing on disk.
#[derive(Debug, Default)]
pub struct MemoryQuoteLog {
    quoted: Mutex<Vec<QuotedPost>>,
}

impl MemoryQuoteLog {
    pub fn new(quoted: Vec<QuotedPost>) -> Self {
        Self {
            quoted: Mutex::new(quoted),
        }
    }
}

#[async_trait]
impl QuoteLog for MemoryQuoteLog {
    async fn list(&self) -> anyhow::Result<Vec<QuotedPost>> {
        Ok(self
            .quoted
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone())
    }

    async fn record(&self, entry: QuotedPost) -> anyhow::Result<()> {
        let mut quoted = self.quoted.lock().unwrap_or_else(|p| p.into_inner());
        quoted.retain(|kept| kept.post_id != entry.post_id);
        quoted.push(entry);
        Ok(())
    }
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
