//! The poll cursor, kept in a JSON file: where the mention search last got
//! to, and when it last ran.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;

/// The reply job's cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotState {
    /// The newest mention already read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_id: Option<String>,
    /// When the mention search last ran, saved whether or not anything
    /// matched. Without it a quiet period re-reads the whole 7-day window
    /// every cycle, and X bills per post retrieved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_polled_at: Option<String>,
    #[serde(default)]
    pub replied_post_ids: Vec<String>,
}

/// Where the cursor is kept.
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn load(&self) -> Result<BotState>;
    async fn save(&self, state: &BotState) -> Result<()>;
}

/// The cursor in a file, written atomically.
#[derive(Debug, Clone)]
pub struct FileStateStore {
    path: PathBuf,
}

impl FileStateStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

#[async_trait]
impl StateStore for FileStateStore {
    async fn load(&self) -> Result<BotState> {
        let contents = match tokio::fs::read_to_string(&self.path).await {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BotState::default());
            }
            Err(error) => return Err(error.into()),
        };
        let parsed: BotState = serde_json::from_str(&contents)?;
        // A blank cursor is no cursor, as `parsed.sinceId ? ... : {}` reads it.
        Ok(BotState {
            since_id: parsed.since_id.filter(|id| !id.is_empty()),
            last_polled_at: parsed.last_polled_at.filter(|at| !at.is_empty()),
            replied_post_ids: parsed.replied_post_ids,
        })
    }

    async fn save(&self, state: &BotState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Written beside the real file and renamed over it, so a crash
        // mid-write leaves the old cursor rather than half a new one.
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let mut contents = serde_json::to_string_pretty(state)?;
        contents.push('\n');
        tokio::fs::write(&temporary, contents).await?;
        tokio::fs::rename(&temporary, &self.path).await?;
        Ok(())
    }
}

/// The cursor in process, for tests and dry runs.
#[derive(Debug, Default)]
pub struct InMemoryStateStore {
    state: Mutex<BotState>,
}

#[async_trait]
impl StateStore for InMemoryStateStore {
    async fn load(&self) -> Result<BotState> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone())
    }

    async fn save(&self, state: &BotState) -> Result<()> {
        *self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = state.clone();
        Ok(())
    }
}

#[cfg(test)]
#[path = "file_state_tests.rs"]
mod tests;
