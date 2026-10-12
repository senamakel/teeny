//! `teeny memory import [--dry-run] [--since <ts>] [--cursor <file>]`:
//! copies what the TypeScript service remembered in the legacy CortexDB
//! (`CORTEX_API_URL`, scope `CORTEX_SCOPE`, acting as `CORTEX_ACTOR`) into
//! the shared memory every agent reads (`TEENY_MEMORY_URL`, root
//! `TEENY_MEMORY_ROOT`).
//!
//! Each event keeps its text, labels, time, person and idempotency key, and
//! is stored exactly as a live capture would be, so a rerun, or a mention
//! the Rust bot already captured itself, is stored once. The run keeps its
//! place in a cursor file (by default `<TEENY_DATA>/state/
//! memory-import-<root>.json`): an interrupted run goes on where it
//! stopped, and the next one reads only what arrived since. See
//! `teeny_memory::import` for how the legacy store is read.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use teeny_agent::runtime::RuntimeSettings;
use teeny_agent::shared_memory::shared_memory;
use teeny_core::Config;
use teeny_memory::import::{ImportOptions, Importer, LegacyEvents, cursor_path};
use teeny_memory::{CortexMemory, CortexOptions, TinyMemory};

/// `teeny memory <command>`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum MemoryCommand {
    /// Copy the legacy CortexDB scope into the shared memory; safe to rerun.
    Import(ImportArgs),
}

/// `teeny memory import`'s options.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ImportArgs {
    /// Read and count everything, write nothing, keep no cursor.
    #[arg(long)]
    pub dry_run: bool,
    /// Leave out what was observed before this: a date (`2026-09-01`) or an
    /// RFC 3339 timestamp.
    #[arg(long, value_name = "TS", value_parser = parse_since)]
    pub since: Option<DateTime<Utc>>,
    /// The cursor file; defaults to one per root under `TEENY_DATA/state`.
    #[arg(long, value_name = "FILE")]
    pub cursor: Option<PathBuf>,
}

/// `--since`: a date, read as its UTC midnight, or a timestamp.
pub fn parse_since(value: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(at) = DateTime::parse_from_rfc3339(value) {
        return Ok(at.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc())
        .ok_or_else(|| format!("{value:?} is neither a date (2026-09-01) nor an RFC 3339 time"))
}

/// Where the import reads from and writes to, for the header it prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub scope: String,
    pub legacy_url: String,
    pub root: String,
    pub url: String,
}

/// Runs the import and prints its report.
pub async fn run(
    legacy: &dyn LegacyEvents,
    memory: &TinyMemory,
    place: &Place,
    args: &ImportArgs,
    cursor: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    writeln!(
        out,
        "{} {} at {} into {} at {}{}",
        if args.dry_run { "Reading" } else { "Importing" },
        place.scope,
        place.legacy_url,
        place.root,
        place.url,
        if args.dry_run {
            String::new()
        } else {
            format!(" (cursor {})", cursor.display())
        },
    )?;
    let options = ImportOptions {
        dry_run: args.dry_run,
        since: args.since,
        ..ImportOptions::default()
    };
    let report = Importer::new(legacy, memory, &place.root, &place.scope, options)
        .run(Some(cursor))
        .await;
    match report {
        Ok(report) => {
            write!(out, "{}", report.render())?;
            Ok(())
        }
        Err(error) => Err(error).context(
            "the import stopped; what it stored is kept, and running it again goes on from \
             where it stopped",
        ),
    }
}

/// The command, from the environment.
pub async fn execute(args: &MemoryArgs) -> anyhow::Result<()> {
    let MemoryCommand::Import(args) = &args.command;
    let config = Config::from_env()?;
    if config.memory.token.is_none() {
        anyhow::bail!("TEENY_MEMORY_TOKEN is not set: there is no shared memory to import into");
    }
    let legacy = CortexMemory::from_config(&config.cortex)?;
    let memory = shared_memory(
        RuntimeSettings::from_teeny(&config),
        CortexOptions::from_config(&config.cortex),
    );
    let place = Place {
        scope: config.cortex.scope.clone(),
        legacy_url: config.cortex.api_url.clone(),
        root: config.memory.root.clone(),
        url: config.memory.url.clone(),
    };
    let cursor = args
        .cursor
        .clone()
        .unwrap_or_else(|| cursor_path(Path::new(&config.openhuman.data_dir), &config.memory.root));
    run(
        &legacy,
        &memory,
        &place,
        args,
        &cursor,
        &mut std::io::stdout(),
    )
    .await
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
