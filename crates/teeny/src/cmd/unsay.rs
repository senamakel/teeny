//! `teeny unsay`: deletes replies the account should not have made, and
//! queues the mentions behind them to be answered again.
//!
//! Deleting is only half of it: the mention a reply answered is marked
//! `replied`, so nothing would ever answer it again. This puts it back to
//! pending, and the reply worker picks it up with whatever the prompt says
//! now. A deleted post is gone for good; the mention row is not. Nothing
//! happens without `--confirm`, and nothing reaches X while `DRY_RUN` is on.

use std::io::Write;
use std::sync::Arc;

use anyhow::bail;
use teeny_agent::deps::QueuedXPost;
use teeny_core::Config;
use teeny_core::types::PostId;
use teeny_store::MentionQueue;
use teeny_store::mentions::MongoMentionQueue;
use teeny_x::{NoopGuard, XApi, XPublisher};

const USAGE: &str = "usage: teeny unsay <reply-id> [<reply-id> ...] [--confirm]";

/// `teeny unsay <reply-id>... [--confirm]`
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct UnsayArgs {
    /// The account's replies to delete, by post id.
    #[arg(required = true, value_name = "REPLY_ID")]
    pub ids: Vec<String>,
    /// Delete and requeue; without it, only what would happen is shown.
    #[arg(long)]
    pub confirm: bool,
}

pub async fn run(
    x: &XApi,
    publisher: &XPublisher,
    queue: &dyn MentionQueue<QueuedXPost>,
    args: &UnsayArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let ids: Vec<&String> = args
        .ids
        .iter()
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
        .collect();
    if ids.is_empty() {
        bail!(USAGE);
    }
    if args.confirm && publisher.is_dry_run() {
        writeln!(
            out,
            "DRY_RUN is on, so nothing is deleted or requeued. Set DRY_RUN=false to unsay."
        )?;
    }

    for id in ids {
        let reply = PostId::from(id.as_str());
        let answered = x.replied_to(&reply).await?;
        let status = match &answered {
            Some(mention) => queue.status_of(mention.as_str()).await?,
            None => None,
        };

        if !args.confirm || publisher.is_dry_run() {
            match (&answered, status) {
                (Some(mention), Some(status)) => writeln!(
                    out,
                    "would delete {id} and requeue {mention} ({})",
                    format!("{status:?}").to_lowercase()
                )?,
                _ => writeln!(out, "would delete {id} (no mention found)")?,
            }
            continue;
        }

        match publisher.unsay(&reply).await {
            Ok(true) => writeln!(out, "deleted {id}")?,
            Ok(false) => {
                writeln!(out, "  could not delete {id}: X did not confirm it")?;
                continue;
            }
            Err(error) => {
                writeln!(out, "  could not delete {id}: {error}")?;
                continue;
            }
        }
        if let Some(mention) = answered {
            if queue.requeue_fresh(mention.as_str()).await? {
                writeln!(out, "  requeued {mention} for a fresh reply")?;
            } else {
                writeln!(out, "  no mention row for {mention}; nothing to requeue")?;
            }
        }
    }

    if !args.confirm {
        writeln!(out, "\nNothing was changed. Re-run with --confirm.")?;
    }
    Ok(())
}

/// The command, from the environment.
pub async fn execute(args: &UnsayArgs) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let x = Arc::new(XApi::from_config(&config.x));
    let publisher = XPublisher::new(Arc::clone(&x), Arc::new(NoopGuard), config.dry_run);
    let db = teeny_store::connect(&config.store).await?;
    let queue =
        MongoMentionQueue::<QueuedXPost>::with_collection(&db, &config.store.mentions_collection);
    run(&x, &publisher, &queue, args, &mut std::io::stdout()).await
}

#[cfg(test)]
#[path = "unsay_tests.rs"]
mod tests;
