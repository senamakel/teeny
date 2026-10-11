//! The command line: the service, its health check, and the operator
//! subcommands (each in `crate::cmd`).

use clap::{Args, Parser, Subcommand};

use crate::cmd::auth_x::AuthXArgs;
use crate::cmd::cron::CronArgs;
use crate::cmd::dry_run::DryRunArgs;
use crate::cmd::forget::ForgetArgs;
use crate::cmd::learnings::LearningsArgs;
use crate::cmd::memory::MemoryArgs;
use crate::cmd::remember::RememberArgs;
use crate::cmd::unsay::UnsayArgs;
use crate::cmd::webhook::WebhookArgs;

/// Teeny (@teenydroid), on the OpenHuman embedded harness.
#[derive(Debug, Parser)]
#[command(name = "teeny", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Run the service: the HTTP server, the schedule and the agents.
    Serve(ServeArgs),
    /// Exit zero if the running service answers its health check.
    Healthcheck,
    /// Mint the account's X access pair: the OAuth 1.0a flow, by PIN.
    AuthX(AuthXArgs),
    /// Read and move where X delivers the account's activity.
    Webhook(WebhookArgs),
    /// Preview, and with --confirm perform, a deletion in the legacy CortexDB.
    Forget(ForgetArgs),
    /// Store one durable note in the shared memory, as the remember tool does.
    Remember(RememberArgs),
    /// The shared memory: `import` copies the legacy CortexDB scope into it.
    Memory(MemoryArgs),
    /// Delete replies the account should not have made, and requeue the
    /// mentions behind them.
    Unsay(UnsayArgs),
    /// Rehearse one job with DRY_RUN forced on and print its drafts.
    DryRun(DryRunArgs),
    /// The registered cron jobs: list them with their last runs, or run one.
    Cron(CronArgs),
    /// Rewrite the learnings brief now and print it; --dry-run writes nothing.
    Learnings(LearningsArgs),
    /// Prompt-inject the reply and chat pipelines with DRY_RUN forced on;
    /// exits non-zero on any tool outside the belt, any post, or a leaked
    /// system prompt.
    Redteam,
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
