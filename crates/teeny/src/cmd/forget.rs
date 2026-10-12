//! `teeny forget`: previews, and only with `--confirm` performs, a
//! CortexDB deletion.
//!
//! Operational notes written during setup are recalled like anything else,
//! and a persona repeats them in public voice, so they have to be removed
//! rather than only discouraged in the prompt. List first (`--find`):
//! a selector that matches nothing looks exactly like one that matched and
//! found nothing to delete.

use std::io::Write;

use anyhow::bail;
use teeny_core::Config;
use teeny_memory::{CortexMemory, ForgetSelector};

/// Events `--find` searches.
const FIND_LIMIT: u32 = 500;
/// Characters of each matching event printed.
const FIND_SHOWN: usize = 400;
/// Why, as CortexDB records it.
const REASON: &str = "operational note leaking into public replies";
const USAGE: &str =
    "usage: teeny forget [--find <text> | --ids <id,id> | \"<subject>\"] [--confirm]";

/// `teeny forget --find "droplet"`, `--ids id1,id2 [--confirm]`, or
/// `"<subject>" [--confirm]`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ForgetArgs {
    /// Forget everything about this subject.
    pub subject: Option<String>,
    /// List the events whose JSON contains this text, with their ids.
    #[arg(long, value_name = "TEXT")]
    pub find: Option<String>,
    /// Forget these events, comma-separated.
    #[arg(long, value_delimiter = ',', value_name = "ID,ID")]
    pub ids: Vec<String>,
    /// Delete; without it, only the preview is shown.
    #[arg(long)]
    pub confirm: bool,
}

pub async fn run(
    memory: &CortexMemory,
    args: &ForgetArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if let Some(find) = &args.find {
        let needle = find.to_lowercase();
        for event in memory.events(FIND_LIMIT).await? {
            let blob = event.to_string();
            if blob.to_lowercase().contains(&needle) {
                let id = event["id"].as_str().unwrap_or("?");
                let shown: String = blob.chars().take(FIND_SHOWN).collect();
                writeln!(out, "{id}\t{shown}")?;
            }
        }
        return Ok(());
    }

    let ids: Vec<String> = args
        .ids
        .iter()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    let selector = match (ids.is_empty(), &args.subject) {
        (false, _) => ForgetSelector::MemoryIds(ids),
        (true, Some(subject)) if !subject.trim().is_empty() => {
            ForgetSelector::AboutSubject(subject.clone())
        }
        _ => bail!(USAGE),
    };

    let preview = memory.forget_preview(&selector).await?;
    writeln!(out, "{}", serde_json::to_string_pretty(&preview)?)?;
    if !args.confirm {
        writeln!(out, "\nPreview only. Re-run with --confirm to delete.")?;
        return Ok(());
    }
    let result = memory
        .forget(&selector, REASON, preview["preview_id"].as_str())
        .await?;
    writeln!(out, "{}", serde_json::to_string_pretty(&result)?)?;
    Ok(())
}

/// The command, from the environment.
pub async fn execute(args: &ForgetArgs) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let memory = CortexMemory::from_config(&config.cortex)?;
    run(&memory, args, &mut std::io::stdout()).await
}

#[cfg(test)]
#[path = "forget_tests.rs"]
mod tests;
