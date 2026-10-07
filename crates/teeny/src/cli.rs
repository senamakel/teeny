//! The command line. More subcommands arrive with the milestones that need
//! them (`auth-x`, `webhook`, `forget`, `remember`, `unsay`, the dry runs).

use clap::{Parser, Subcommand};

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
    Serve,
    /// Exit zero if the running service answers its health check.
    Healthcheck,
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
