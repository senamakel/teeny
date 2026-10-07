//! `teeny`: the service and its operator subcommands.

mod cli;

use std::process::ExitCode;

use anyhow::bail;
use clap::Parser;
use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use teeny_core::Config;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = build_runtime()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| runtime.block_on(run(cli.command)));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("teeny: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The process's one tokio runtime, sized the way OpenHuman requires: an
/// agent turn is a deep async state machine and nests another for every
/// sub-agent, so tokio's default 2 MiB worker stack overflows and aborts.
fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()
}

async fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Serve => {
            // Refuse a bad environment the same way legacy did, before
            // anything starts; the service itself lands in later milestones.
            Config::from_env()?;
            bail!("serve: the configuration loads, but the service is not built yet")
        }
        Command::Healthcheck => bail!("healthcheck: there is no service to check yet"),
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
