//! One moon chart a day, on its day of the rota: the job around
//! [`pipeline::MoonPipeline`]. A port of `legacy/src/jobs/moon-job.ts` and
//! `legacy/src/moon/wiring.ts`.

pub mod pick;
pub mod pipeline;
pub mod prompt;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use teeny_agent::prompt::render_persona;
use teeny_cadence::rota::Rota;
use teeny_market::http::JsonClient;
use teeny_market::leaderboard::CoinGeckoLeaderboard;
use teeny_media::moon::multiple;
use teeny_store::ReflectionInput;
use teeny_store::holds::HoldSurface;
use teeny_store::journal::{Outcome, Tone, ToolCall, Trigger};

use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::record_quietly;
use crate::morning::job::{Gates, admit, day_id, journal_failure, roll};
use crate::morning::pipeline::Mode;
use crate::morning::post::XOwnPoster;
use crate::morning::prompts::with_skills;
use crate::morning::timeline::thousands;
use pipeline::{MoonPipeline, MoonResult};

const JOB: &str = "moon";

/// The pipeline the service runs, from the clients and the configuration.
pub fn build(ctx: &JobCtx) -> MoonPipeline {
    let deps = &ctx.deps;
    let config = &ctx.config;
    let moon = &config.moon;
    MoonPipeline {
        source: Arc::new(CoinGeckoLeaderboard::new(JsonClient::new())),
        text: Arc::new(deps.llm.clone().with_source("moon")),
        memory: Arc::clone(&deps.memory),
        poster: Arc::new(XOwnPoster::from_deps(deps, HoldSurface::Moon, "moon_post")),
        safety: Some(Arc::clone(&deps.safety)),
        persona: with_skills(&render_persona(&deps.persona), &config.agent.skills_dir),
        bot_username: deps.settings.bot_username.clone(),
        coin_id: moon.coin_id.clone(),
        category: moon.category.clone(),
        board_name: moon.board_name.clone(),
        board_size: moon.board_size as usize,
        rungs: moon.rungs as usize,
        exclude_ids: moon.exclude_ids.clone(),
        min_multiple: moon.min_multiple,
        max_multiple: moon.max_multiple,
        max_emoji: deps.settings.max_emoji,
        output_dir: (!moon.output_dir.is_empty()).then(|| PathBuf::from(&moon.output_dir)),
        watermark: deps.settings.chart_watermark.clone(),
        random: roll(ctx),
        now: ctx.now(),
    }
}

/// The scheduled run: the rota, the guard, the cadence, the pipeline, the journal.
pub async fn run(ctx: &JobCtx) -> Result<JobReport> {
    let rota = Rota::from_config(&ctx.config.rota)?;
    let gates = Gates {
        job: JOB,
        kind: Some(JOB),
        rota: Some((rota, ctx.config.rota.moon_slot)),
        force: false,
    };
    if let Err(why) = admit(ctx, gates).await {
        tracing::info!("{why}");
        return Ok(JobReport::declined(JOB, why));
    }
    run_pipeline(ctx, &build(ctx)).await
}

/// The rehearsal (`moon-dry-run`): nothing posted, no gates, no journal.
pub async fn dry_run(ctx: &JobCtx) -> Result<JobReport> {
    let result = build(ctx).run(Mode::Rehearsal).await?;
    Ok(report(&result))
}

/// Runs an admitted moon and journals it, a failure included.
pub async fn run_pipeline(ctx: &JobCtx, pipeline: &MoonPipeline) -> Result<JobReport> {
    let result = match pipeline.run(Mode::Live).await {
        Ok(result) => result,
        Err(error) => {
            journal_failure(
                ctx,
                day_id(ctx, JOB),
                Trigger::Moon,
                "the moon chart",
                &error,
            )
            .await;
            return Err(error).context("the moon chart");
        }
    };
    tracing::info!(outcome = ?result.outcome, yardstick = result.target.coin.symbol, multiple = result.target.multiple, problems = ?result.problems, posted = ?result.posted_id, "moon job: done");
    journal(ctx, &result).await;
    Ok(report(&result))
}

fn held_note(result: &MoonResult) -> Option<String> {
    (result.outcome == Outcome::Held).then(|| {
        result
            .problems
            .iter()
            .find(|problem| problem.starts_with("content filter withheld"))
            .cloned()
            .unwrap_or_else(|| "content filter withheld the caption".into())
    })
}

fn report(result: &MoonResult) -> JobReport {
    let mut report = JobReport::ran(JOB, result.outcome);
    report.posted_id = result.posted_id.clone();
    report.drafts = (!result.caption.is_empty())
        .then(|| result.caption.clone())
        .into_iter()
        .collect();
    report.problems = result.problems.clone();
    report.note = held_note(result);
    report
}

/// The moon turn, for the mind page.
async fn journal(ctx: &JobCtx, result: &MoonResult) {
    let mut thinking = vec![
        format!(
            "yardstick: ${} ({})",
            result.target.coin.symbol, result.target.coin.name
        ),
        format!("multiple: {}", multiple(result.target.multiple)),
        format!("implied: ${}", result.target.implied_price_usd),
    ];
    if !result.excluded.is_empty() {
        thinking.push(format!(
            "skipped as recently used: {}",
            result.excluded.join(", ")
        ));
    }
    thinking.push(format!(
        "ladder: {}",
        result
            .ladder
            .iter()
            .map(|entry| format!("${} {}", entry.coin.symbol, multiple(entry.multiple)))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(day_id(ctx, JOB)),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Moon,
            incoming: format!(
                "{} coins on the board, ${} at ${}",
                result.board.len(),
                result.token.symbol,
                thousands(result.token.market_cap_usd.round() as u64)
            ),
            tone: Tone::Neutral,
            recalled: [&result.history, &result.community]
                .into_iter()
                .filter(|recalled| !recalled.is_empty())
                .cloned()
                .collect(),
            tools: vec![
                ToolCall {
                    name: "leaderboard".into(),
                    result: result
                        .board
                        .iter()
                        .map(|coin| coin.symbol.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                },
                ToolCall {
                    name: "render_moon".into(),
                    result: result.path.as_ref().map_or_else(
                        || format!("{} bytes", result.png.len()),
                        |path| path.display().to_string(),
                    ),
                },
            ],
            thinking: thinking.join("\n"),
            said: if result.outcome == Outcome::Held {
                String::new()
            } else {
                result.caption.clone()
            },
            declined: result.problems.clone(),
            outcome: result.outcome,
            outcome_note: held_note(result),
        },
    )
    .await;
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
