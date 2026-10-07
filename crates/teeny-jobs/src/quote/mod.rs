//! One quote post a day: the job around [`pipeline::QuotePipeline`]. A port
//! of `legacy/src/jobs/quote-job.ts` and `legacy/src/quote/wiring.ts`.
//!
//! The same twenty-hour guard as the other daily jobs: the pipeline's own log
//! only stops it quoting the same post, not quoting a second one an hour
//! after the first.

pub mod candidates;
pub mod log;
pub mod pipeline;
pub mod prompt;

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::TimeDelta;
use regex::Regex;
use teeny_agent::prompt::render_persona;
use teeny_store::ReflectionInput;
use teeny_store::holds::HoldSurface;
use teeny_store::journal::{Outcome, Tone, ToolCall, Trigger};

use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::record_quietly;
use crate::morning::job::{Gates, admit, day_id, journal_failure};
use crate::morning::pipeline::Mode;
use crate::morning::post::XOwnPoster;
use crate::morning::prompts::with_skills;
use log::FileQuoteLog;
use pipeline::{QuotePipeline, QuoteResult};

const JOB: &str = "quote";

/// The pipeline the service runs, from the clients and the configuration.
pub fn build(ctx: &JobCtx) -> QuotePipeline {
    let deps = &ctx.deps;
    let config = &ctx.config;
    let quote = &config.quote;
    let mut own = config.agent.own_token_addresses.clone();
    if let Some(address) = &config.chart.token_address
        && !own.contains(address)
    {
        own.push(address.clone());
    }
    QuotePipeline {
        x: Arc::clone(&deps.x_reads),
        text: Arc::new(deps.llm.clone().with_source("quote")),
        memory: Arc::clone(&deps.memory),
        log: Arc::new(FileQuoteLog::new(&quote.log_path)),
        poster: Arc::new(XOwnPoster::from_deps(deps, HoldSurface::Quote, "quote_post")),
        safety: Some(Arc::clone(&deps.safety)),
        persona: with_skills(&render_persona(&deps.persona), &config.agent.skills_dir),
        bot_username: deps.settings.bot_username.clone(),
        trusted_handles: config.agent.trusted_handles.clone(),
        own_token_addresses: own,
        own_symbol: deps.settings.token_symbol.clone(),
        query: quote.query.clone(),
        max_age: TimeDelta::hours(i64::from(quote.hours)),
        author_cooldown: TimeDelta::days(i64::from(quote.author_cooldown_days)),
        max_results: quote.posts_per_search,
        keep: 20,
        max_emoji: deps.settings.max_emoji,
        now: ctx.now(),
    }
}

fn gates() -> Gates {
    Gates {
        job: JOB,
        kind: Some(JOB),
        rota: None,
        force: false,
    }
}

/// The scheduled run: the guard, the cadence, the pipeline, the journal.
pub async fn run(ctx: &JobCtx) -> Result<JobReport> {
    if let Err(why) = admit(ctx, gates()).await {
        tracing::info!("{why}");
        return Ok(JobReport::declined(JOB, why));
    }
    run_pipeline(ctx, &build(ctx)).await
}

/// The rehearsal (`quote-dry-run`): nothing posted, no gates, no journal.
/// The pick is still logged, as the legacy rehearsal logged it.
pub async fn dry_run(ctx: &JobCtx) -> Result<JobReport> {
    let result = build(ctx).run(Mode::Rehearsal).await?;
    Ok(report(&result))
}

/// Runs an admitted quote and journals it, a failure included.
pub async fn run_pipeline(ctx: &JobCtx, pipeline: &QuotePipeline) -> Result<JobReport> {
    let result = match pipeline.run(Mode::Live).await {
        Ok(result) => result,
        Err(error) => {
            journal_failure(ctx, day_id(ctx, JOB), Trigger::Quote, "the quote post", &error).await;
            return Err(error).context("the quote post");
        }
    };
    tracing::info!(outcome = ?result.outcome, quoted = ?result.chosen.as_ref().map(|post| post.id.to_string()), candidates = result.candidates.len(), problems = ?result.problems, posted = ?result.posted_id, "quote job: done");
    journal(ctx, &result).await;
    Ok(report(&result))
}

fn note(result: &QuoteResult) -> Option<String> {
    match result.outcome {
        Outcome::Held => Some(
            result
                .problems
                .iter()
                .find(|problem| problem.starts_with("content filter withheld"))
                .cloned()
                .unwrap_or_else(|| "content filter withheld the text".into()),
        ),
        Outcome::Skipped => Some(result.why.clone()),
        _ => None,
    }
}

fn report(result: &QuoteResult) -> JobReport {
    let mut report = JobReport::ran(JOB, result.outcome);
    report.posted_id = result.posted_id.clone();
    report.drafts = (!result.text.is_empty())
        .then(|| result.text.clone())
        .into_iter()
        .collect();
    report.problems = result.problems.clone();
    report.note = note(result);
    report
}

/// "already quoted ×3, names another token ×2": the reasons, counted.
pub fn summarise(reasons: &[String]) -> String {
    let author = Regex::new(r"@\S+ was quoted on \S+").expect("valid");
    let token = Regex::new(r"\(\$\w+\)").expect("valid");
    let mut counts: Vec<(String, usize)> = Vec::new();
    for reason in reasons {
        let key = author.replace(reason, "author quoted lately");
        let key = token.replace(&key, "").trim().to_owned();
        match counts.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, count)) => *count += 1,
            None => counts.push((key, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(reason, count)| {
            if count > 1 {
                format!("{reason} ×{count}")
            } else {
                reason
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn journal(ctx: &JobCtx, result: &QuoteResult) {
    let mut thinking = vec![result.why.clone()];
    if !result.skipped.is_empty() {
        let reasons: Vec<String> = result.skipped.iter().map(|entry| entry.reason.clone()).collect();
        thinking.push(format!("skipped: {}", summarise(&reasons)));
    }
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(day_id(ctx, JOB)),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Quote,
            incoming: match &result.chosen {
                Some(post) => format!(
                    "@{}: {}",
                    post.author_username.as_deref().unwrap_or("someone"),
                    post.text.split_whitespace().collect::<Vec<_>>().join(" ")
                ),
                None => format!(
                    "{} posts read, {} candidates",
                    result.searched,
                    result.candidates.len()
                ),
            },
            tone: Tone::Friendly,
            recalled: (!result.recalled.is_empty())
                .then(|| result.recalled.clone())
                .into_iter()
                .collect(),
            tools: vec![ToolCall {
                name: "search".into(),
                result: format!(
                    "{} posts, {} kept, {} skipped",
                    result.searched,
                    result.candidates.len(),
                    result.skipped.len()
                ),
            }],
            thinking: thinking.join("\n"),
            said: if result.outcome == Outcome::Held {
                String::new()
            } else {
                result.text.clone()
            },
            declined: result.problems.clone(),
            outcome: result.outcome,
            outcome_note: note(result),
        },
    )
    .await;
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
