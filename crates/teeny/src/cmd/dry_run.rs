//! `teeny dry-run <job>`: one job's rehearsal, its drafts printed, nothing
//! published. `DRY_RUN` is forced on before anything is built, whatever the
//! environment says, so every posting path (the publisher, the reply
//! outlet, the article publisher) holds; `run` refuses a context that is
//! not in `DRY_RUN` besides.
//!
//! A rehearsal is otherwise real: the agents, the searches, the generators
//! (and the day's media budget), memory, and the backrooms notebooks.

use std::future::Future;
use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, bail};
use teeny_core::Config;
use teeny_jobs::backrooms::{Backrooms, RunOptions, Seed, SeedKind};
use teeny_jobs::calendar::load_calendar;
use teeny_jobs::chat::{ChatMessage, handle_message};
use teeny_jobs::ctx::{JobCtx, JobReport};
use teeny_jobs::reply::harness::generate;
use teeny_memory::ChatKind;
use teeny_x::model::XPost;

pub use super::forced_dry_run;
use crate::app::App;

/// What to rehearse.
#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum DryRunJob {
    /// The good-morning post: reads, draws, films, captions.
    Morning,
    /// Today's afternoon slot (the chart, the moon chart or lore, by the rota).
    Afternoon,
    /// Today's evening slot (an article, the serial or the quote, by the rota).
    Evening,
    /// The moon chart: read the board, draw, caption.
    Moon,
    /// The daily chart post.
    Chart,
    /// The lore post: seed, room, thought, post.
    Lore,
    /// The quote post: search, filter, pick, write.
    Quote,
    /// One episode of the serial.
    Narrative,
    /// One article.
    Article,
    /// One `calendar.json` item, by name.
    Calendar {
        /// The item's `name`.
        item: String,
    },
    /// Opens the backrooms on a seed and prints the thought.
    Backrooms {
        /// What the room is opened on.
        #[arg(long)]
        seed: String,
    },
    /// What the account would reply to a hypothetical post.
    Reply {
        /// The post's text.
        #[arg(long)]
        text: String,
        /// Its author's handle.
        #[arg(long, default_value = "someone")]
        from: String,
    },
    /// What the account would answer on Telegram.
    Telegram {
        /// The message.
        text: String,
        /// As a group message rather than a direct one.
        #[arg(long)]
        group: bool,
    },
}

/// `teeny dry-run <job>`
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct DryRunArgs {
    #[command(subcommand)]
    pub job: DryRunJob,
}

/// Forces `DRY_RUN` on `config`, builds the context from it and rehearses.
pub async fn rehearse<B, F>(
    config: Config,
    build: B,
    job: &DryRunJob,
    out: &mut dyn Write,
) -> anyhow::Result<()>
where
    B: FnOnce(Config) -> F,
    F: Future<Output = anyhow::Result<Arc<JobCtx>>>,
{
    let ctx = build(forced_dry_run(config)).await?;
    run(&ctx, job, out).await
}

/// The command, from the environment, on the service's own wiring.
pub async fn execute(args: &DryRunArgs) -> anyhow::Result<()> {
    rehearse(
        Config::from_env()?,
        |config| async move { Ok(App::build(config).await?.ctx) },
        &args.job,
        &mut std::io::stdout(),
    )
    .await
}

/// Rehearses `job` on `ctx`, which must be in `DRY_RUN`.
pub async fn run(ctx: &JobCtx, job: &DryRunJob, out: &mut dyn Write) -> anyhow::Result<()> {
    if !ctx.config.dry_run || !ctx.deps.settings.dry_run {
        bail!("refusing to rehearse: DRY_RUN is not on, so this could publish");
    }
    let report = match job {
        DryRunJob::Morning => teeny_jobs::morning::dry_run(ctx).await?,
        DryRunJob::Afternoon => teeny_jobs::afternoon::dry_run(ctx).await?,
        DryRunJob::Evening => match teeny_jobs::evening::dry_run(ctx).await? {
            Some(report) => report,
            None => {
                writeln!(out, "Nothing owns tonight's evening slot.")?;
                return Ok(());
            }
        },
        DryRunJob::Moon => teeny_jobs::moon::dry_run(ctx).await?,
        DryRunJob::Chart => teeny_jobs::chart::dry_run(ctx).await?,
        DryRunJob::Lore => teeny_jobs::lore::dry_run(ctx).await?,
        DryRunJob::Quote => teeny_jobs::quote::dry_run(ctx).await?,
        DryRunJob::Narrative => teeny_jobs::narrative::dry_run(ctx).await?,
        DryRunJob::Article => teeny_jobs::article::dry_run(ctx).await?,
        DryRunJob::Calendar { item } => {
            let calendar = load_calendar(&ctx.config.calendar.path)
                .with_context(|| format!("reading {}", ctx.config.calendar.path))?;
            let found = calendar
                .items
                .iter()
                .find(|entry| entry.name == *item)
                .with_context(|| {
                    format!("{} has no item named {item:?}", ctx.config.calendar.path)
                })?;
            teeny_jobs::calendar::dry_run(ctx, found).await?
        }
        DryRunJob::Backrooms { seed } => {
            return backrooms(ctx, seed, out).await;
        }
        DryRunJob::Reply { text, from } => {
            let draft = generate(ctx, &hypothetical_post(text, from)).await?;
            writeln!(out, "reply ({:?}): {}", draft.incoming_tone, draft.text)?;
            if !draft.thinking.is_empty() {
                writeln!(out, "  thinking: {}", draft.thinking)?;
            }
            for declined in &draft.declined {
                writeln!(out, "  declined: {declined}")?;
            }
            return Ok(());
        }
        DryRunJob::Telegram { text, group } => {
            match handle_message(ctx, &hypothetical_message(text, *group)).await {
                Some(said) => writeln!(out, "would send: {said}")?,
                None => writeln!(out, "would stay quiet")?,
            }
            return Ok(());
        }
    };
    print_report(&report, out)
}

/// Prints what a job did and what it would have posted.
pub fn print_report(report: &JobReport, out: &mut dyn Write) -> anyhow::Result<()> {
    let ran = if report.ran { "" } else { " (declined)" };
    writeln!(out, "{}: {:?}{ran}", report.job, report.outcome)?;
    if let Some(note) = &report.note {
        writeln!(out, "  note: {note}")?;
    }
    if let Some(id) = &report.posted_id {
        writeln!(out, "  posted: {id}")?;
    }
    for draft in &report.drafts {
        writeln!(out, "  draft:\n{}", indent(draft))?;
    }
    for problem in &report.problems {
        writeln!(out, "  problem: {problem}")?;
    }
    Ok(())
}

async fn backrooms(ctx: &JobCtx, seed: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    let result = Backrooms::from_ctx(ctx)
        .run(&Seed::text(SeedKind::Text, seed), &RunOptions::default())
        .await?;
    if let Some(thought) = &result.thought {
        writeln!(
            out,
            "thought:\n{}",
            indent(&serde_json::to_string_pretty(thought)?)
        )?;
    }
    if let Some(gm) = &result.gm {
        writeln!(out, "gm:\n{}", indent(&serde_json::to_string_pretty(gm)?))?;
    }
    for problem in &result.problems {
        writeln!(out, "problem: {problem}")?;
    }
    if let Some(saved) = &result.saved {
        writeln!(out, "saved: {}", saved.markdown.display())?;
    }
    Ok(())
}

/// A post nobody wrote, as a mention from `from`.
pub fn hypothetical_post(text: &str, from: &str) -> XPost {
    let mut post = XPost::new("dry-run", text);
    post.author_id = Some(format!("dry-run-{from}").into());
    post.author_username = Some(from.trim_start_matches('@').to_owned());
    post
}

/// A Telegram message nobody sent, sent now.
pub fn hypothetical_message(text: &str, group: bool) -> ChatMessage {
    ChatMessage {
        id: "dry-run".into(),
        chat_id: "dry-run".into(),
        chat_kind: if group {
            ChatKind::Group
        } else {
            ChatKind::Private
        },
        chat_title: group.then(|| "dry run".to_owned()),
        text: text.to_owned(),
        author_id: "dry-run".into(),
        author_username: Some("someone".into()),
        author_name: None,
        created_at: Some(chrono::Utc::now().to_rfc3339()),
        message_thread_id: None,
        batch_ids: Vec::new(),
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "dry_run_tests.rs"]
mod tests;
