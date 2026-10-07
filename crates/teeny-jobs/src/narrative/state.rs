//! Where the serial is: which episode comes next, which lore has come out,
//! what was left hanging. Memory remembers the episodes as text; this file
//! is the ledger, because a serial that loses count of itself reveals the
//! same secret twice.

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeState {
    #[serde(default = "one")]
    pub season: u32,
    /// The number of the last episode published. The next one is this plus one.
    #[serde(default)]
    pub episode: u32,
    #[serde(default)]
    pub surfaced_lore: Vec<String>,
    #[serde(default)]
    pub open_threads: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_episode_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_logline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_after: Option<String>,
}

fn one() -> u32 {
    1
}

impl Default for NarrativeState {
    fn default() -> Self {
        Self {
            season: 1,
            episode: 0,
            surfaced_lore: Vec::new(),
            open_threads: Vec::new(),
            last_episode_id: None,
            last_title: None,
            last_logline: None,
            state_after: None,
        }
    }
}

#[async_trait]
pub trait NarrativeStateStore: Send + Sync {
    async fn load(&self) -> anyhow::Result<NarrativeState>;
    async fn save(&self, state: &NarrativeState) -> anyhow::Result<()>;
}

/// `NARRATIVE_STATE_PATH`, written atomically.
pub struct FileNarrativeState {
    pub path: PathBuf,
}

#[async_trait]
impl NarrativeStateStore for FileNarrativeState {
    async fn load(&self) -> anyhow::Result<NarrativeState> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(NarrativeState::default()),
            Err(error) => Err(error.into()),
        }
    }

    async fn save(&self, state: &NarrativeState) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let temporary = self.path.with_extension("json.tmp");
        tokio::fs::write(&temporary, format!("{}\n", serde_json::to_string_pretty(state)?)).await?;
        tokio::fs::rename(&temporary, &self.path).await?;
        Ok(())
    }
}

/// In process, for tests and rehearsals.
#[derive(Default)]
pub struct MemoryNarrativeState(pub Mutex<NarrativeState>);

#[async_trait]
impl NarrativeStateStore for MemoryNarrativeState {
    async fn load(&self) -> anyhow::Result<NarrativeState> {
        Ok(self.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    async fn save(&self, state: &NarrativeState) -> anyhow::Result<()> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = state.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backrooms::testing::scratch;

    #[tokio::test]
    async fn starts_at_episode_zero_and_round_trips_a_file() {
        let dir = scratch("narrative-state");
        let store = FileNarrativeState { path: dir.join("state.json") };
        assert_eq!(store.load().await.unwrap(), NarrativeState::default());
        let state = NarrativeState {
            episode: 4,
            surfaced_lore: vec!["goat-blinks".into()],
            open_threads: vec!["the door".into()],
            last_episode_id: Some("9".into()),
            state_after: Some("out".into()),
            ..NarrativeState::default()
        };
        store.save(&state).await.unwrap();
        assert_eq!(store.load().await.unwrap(), state);
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("state.json")).unwrap()).unwrap();
        assert_eq!(raw["surfacedLore"][0], "goat-blinks");
        std::fs::remove_dir_all(dir).ok();
    }
}
