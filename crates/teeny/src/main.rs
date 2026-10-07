//! `teeny`: the service and its operator subcommands.

mod adapters;
mod app;
mod cli;
mod cmd;
mod healthcheck;
mod http;
mod mentions;
mod release;
mod schedule;
mod serve;
mod sinks;
mod stream;
mod telegram;

use std::process::ExitCode;

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
        Command::Serve(args) => {
            // Refuse a bad environment the same way legacy did, before
            // anything starts.
            let config = Config::from_env()?;
            init_tracing(&config.server.log_level);
            serve::serve(config, args.dry_run).await
        }
        Command::Healthcheck => {
            let port = std::env::var("PORT").ok();
            healthcheck::check(&healthcheck::health_url(port.as_deref())).await
        }
    }
}

/// Logs to stderr at `LOG_LEVEL`, overridable with `RUST_LOG`.
fn init_tracing(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
