//! `teeny remember`: one durable note in CortexDB, stored the way the
//! agent's own `remember` tool stores it, for facts the operator wants in
//! memory without waiting for a turn to store them.
//!
//! Memory is recalled on every turn, so a note written here is read back in
//! the account's public voice: write it as a fact about the account, not as
//! an operational aside (see `teeny forget` for what happens otherwise).

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::bail;
use serde_json::json;
use teeny_core::Config;
use teeny_memory::{CortexMemory, Memory};

/// `teeny remember "<note>" [--label <label>]...`
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct RememberArgs {
    /// The note, as a fact about the account. Several words are joined.
    #[arg(required = true, num_args = 1..)]
    pub note: Vec<String>,
    /// A label to store it under; repeat for more.
    #[arg(long = "label", value_name = "LABEL")]
    pub labels: Vec<String>,
}

/// Stores the note and says where. `place` is `<scope> at <api url>`.
pub async fn run(
    memory: &dyn Memory,
    args: &RememberArgs,
    place: &str,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let note = args.note.join(" ").trim().to_owned();
    if note.is_empty() {
        bail!("usage: teeny remember \"<note>\" [--label <label>]...");
    }
    let id = note_id();
    memory
        .capture_activity(
            "agent_note",
            &id,
            json!({ "note": note, "labels": args.labels }),
            None,
        )
        .await?;
    writeln!(out, "Stored as {id} in {place}")?;
    Ok(())
}

/// The command, from the environment.
pub async fn execute(args: &RememberArgs) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let memory = CortexMemory::from_config(&config.cortex)?;
    let place = format!("{} at {}", config.cortex.scope, config.cortex.api_url);
    run(&memory, args, &place, &mut std::io::stdout()).await
}

/// `note:<millis>:<8 hex>`, as the legacy script minted them.
pub fn note_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let salt = RandomState::new().hash_one(now.as_nanos()) as u32;
    format!("note:{}:{salt:08x}", now.as_millis())
}

#[cfg(test)]
#[path = "remember_tests.rs"]
mod tests;
