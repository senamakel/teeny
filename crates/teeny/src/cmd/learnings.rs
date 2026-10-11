//! `teeny learnings [--dry-run]`: rewrites the learnings brief now, rather
//! than at `LEARNINGS_CRON`, and prints it. A port of
//! `legacy/src/scripts/learnings.ts`.
//!
//! The running service reads the file live, so the next turn after this
//! finishes already carries the new brief; nothing needs restarting.
//! `--dry-run` drafts the brief from today's record and prints it, and
//! writes nothing. The brief is written whether or not the daily job is on:
//! the operator asked for it.

use std::io::Write;
use std::sync::Arc;

use teeny_core::Config;
use teeny_jobs::ctx::JobCtx;
use teeny_store::journal::Outcome;

use super::forced_dry_run;
use crate::app::App;

/// `teeny learnings [--dry-run]`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::Args)]
pub struct LearningsArgs {
    /// Draft the brief and print it; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `config` as the command runs it: the brief on, and a rehearsal in
/// `DRY_RUN`.
pub fn for_command(mut config: Config, dry_run: bool) -> Config {
    config.learnings.enabled = true;
    if dry_run {
        forced_dry_run(config)
    } else {
        config
    }
}

/// Reflects on `ctx`'s record and prints what became of the brief.
pub async fn run(ctx: &JobCtx, dry_run: bool, out: &mut dyn Write) -> anyhow::Result<()> {
    let report = if dry_run {
        teeny_jobs::learnings::dry_run(ctx).await?
    } else {
        teeny_jobs::learnings::run(ctx).await?
    };
    let path = &ctx.config.learnings.path;
    match report.outcome {
        Outcome::Posted => writeln!(out, "The brief was written to {path}:")?,
        Outcome::DryRun => writeln!(out, "The brief, drafted, not written:")?,
        _ => writeln!(
            out,
            "The brief at {path} stands: kept the previous brief: {}",
            report.note.as_deref().unwrap_or("nothing was written")
        )?,
    }
    for draft in &report.drafts {
        writeln!(out, "\n{draft}")?;
    }
    for problem in &report.problems {
        writeln!(out, "(could not read {problem})")?;
    }
    Ok(())
}

/// The command, from the environment, on the service's own wiring.
pub async fn execute(args: &LearningsArgs) -> anyhow::Result<()> {
    let config = for_command(Config::from_env()?, args.dry_run);
    let ctx: Arc<JobCtx> = App::build(config).await?.ctx;
    run(&ctx, args.dry_run, &mut std::io::stdout()).await
}

#[cfg(test)]
#[path = "learnings_tests.rs"]
mod tests;
