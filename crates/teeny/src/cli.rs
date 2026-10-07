//! The command line. More subcommands arrive with the milestones that need
//! them (`auth-x`, `webhook`, `forget`, `remember`, `unsay`, the dry runs).

use clap::{Args, Parser, Subcommand};

/// Teeny (@teenydroid), on the OpenHuman embedded harness.
#[derive(Debug, Parser)]
#[command(name = "teeny", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Run the service: the HTTP server, the schedule and the agents.
    Serve(ServeArgs),
    /// Exit zero if the running service answers its health check.
    Healthcheck,
}

/// `teeny serve`'s options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct ServeArgs {
    /// Rehearse: force DRY_RUN and TELEGRAM_DRY_RUN on, whatever the
    /// environment says. Nothing is posted to X or sent on Telegram.
    #[arg(long)]
    pub dry_run: bool,
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
