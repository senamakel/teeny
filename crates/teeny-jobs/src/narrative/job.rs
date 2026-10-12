//! The serial's job: one episode per run on its evening days, with the
//! morning's guards (twenty hours, the cadence), journaled.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use teeny_store::journal::{Outcome, ReflectionInput, Tone, ToolCall, Trigger};

use super::likeness::{LikenessFinder, LikenessSource};
use super::pipeline::{NarrativePipeline, NarrativeResult, TelegramTarget};
use super::stages::render_transcript;
use super::state::FileNarrativeState;
use super::telegram::BotApi;
use super::voice::load_narrative_voice;
use super::world_state::XWorld;
use crate::backrooms::sources::WebSearch;
use crate::backrooms::tokens::Rng;
use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::record_quietly;
use crate::morning::job::{Gates, admit, day_id, journal_failure};

pub const JOB: &str = "narrative";

/// The scheduled run: the guards, the pipeline, the journal.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    if let Err(why) = admit(
        ctx,
        Gates {
            job: JOB,
            kind: Some(JOB),
            rota: None,
            force: false,
        },
    )
    .await
    {
        tracing::info!("{why}");
        return Ok(JobReport::declined(JOB, why));
    }
    let pipeline = match build(ctx) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            journal_failure(
                ctx,
                day_id(ctx, JOB),
                Trigger::Narrative,
                "an episode of the serial",
                &error,
            )
            .await;
            return Err(error);
        }
    };
    run_pipeline(ctx, &pipeline, false).await
}

/// The rehearsal (`narrative-dry-run`): no gates, nothing posted, the
/// ledger untouched; journaled as a dry run, the caption in the report.
pub async fn dry_run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let pipeline = build(ctx)?;
    run_pipeline(ctx, &pipeline, true).await
}

/// The production pipeline. The serial needs a picture: with image
/// generation off there is nothing to draw it with.
pub fn build(ctx: &JobCtx) -> anyhow::Result<NarrativePipeline> {
    let config = &ctx.config;
    let deps = &ctx.deps;
    let media = deps
        .media
        .clone()
        .filter(|media| media.images.is_some())
        .context("IMAGE_GEN_ENABLED is off, so there is nothing to draw the serial with")?;
    let voice = load_narrative_voice(
        &PathBuf::from(&config.narrative.voice_path),
        &PathBuf::from(&config.narrative.lore_path),
    )
    .context("the serial's VOICE.md and LORE.md")?;
    let telegram = config.telegram.bot_token.clone().and_then(|token| {
        let chat_ids: Vec<i64> = config
            .narrative
            .telegram_chat_ids
            .iter()
            .filter_map(|id| id.trim().parse().ok())
            .collect();
        (!chat_ids.is_empty()).then(|| TelegramTarget {
            channel: Arc::new(BotApi::new(token)),
            chat_ids,
        })
    });
    let likeness = match (&deps.exa, &media.reader) {
        (Some(exa), Some(reader)) => Some(Arc::new(LikenessFinder {
            web: Arc::clone(exa) as Arc<dyn WebSearch>,
            reader: Arc::clone(reader),
            cache_dir: Some(PathBuf::from(&config.narrative.likeness_dir)),
        }) as Arc<dyn LikenessSource>),
        _ => None,
    };
    Ok(NarrativePipeline {
        voice,
        state: Arc::new(FileNarrativeState {
            path: PathBuf::from(&config.narrative.state_path),
        }),
        world: Arc::new(XWorld {
            x: Arc::clone(&deps.x),
            reads: Arc::clone(&deps.x_reads),
            bot_username: deps.settings.bot_username.clone(),
            token_symbol: deps.settings.token_symbol.clone(),
        }),
        runner: Arc::clone(&ctx.runner),
        text: Arc::new(
            deps.llm
                .clone()
                .with_source("narrative")
                .with_temperature(0.2),
        ),
        deps: Arc::clone(deps),
        media,
        telegram,
        likeness,
        bot_username: deps.settings.bot_username.clone(),
        branches: config.narrative.branches,
        turns: config.narrative.turns,
        video_seconds: config.narrative.video_seconds,
        room_only: false,
        cut: None,
        visitor: None,
        random: Mutex::new(Rng::new(Rng::seed_from_time())),
        clock: Arc::clone(&ctx.clock),
        retry_delay: Duration::from_secs(30),
    })
}

/// Runs an episode and journals it, a failure included.
pub async fn run_pipeline(
    ctx: &JobCtx,
    pipeline: &NarrativePipeline,
    rehearsal: bool,
) -> anyhow::Result<JobReport> {
    let id = if rehearsal {
        format!("{}:dry-run", day_id(ctx, JOB))
    } else {
        day_id(ctx, JOB)
    };
    let result = match pipeline.run(rehearsal).await {
        Ok(result) => result,
        Err(error) => {
            journal_failure(
                ctx,
                id,
                Trigger::Narrative,
                "an episode of the serial",
                &error,
            )
            .await;
            return Err(error).context("an episode of the serial");
        }
    };
    tracing::info!(outcome = ?result.outcome, episode = result.episode_number, attached = result.attached.as_str(), problems = ?result.problems, posted = ?result.posted_id, "narrative job: done");
    journal(ctx, id, &result).await;
    let mut report = JobReport::ran(JOB, result.outcome);
    report.posted_id = result.posted_id.clone();
    report.drafts = result
        .chosen
        .as_ref()
        .and_then(|branch| branch.episode.as_ref())
        .map(|episode| episode.caption.clone())
        .into_iter()
        .collect();
    report.problems = result.problems.clone();
    report.note = note(&result);
    Ok(report)
}

fn note(result: &NarrativeResult) -> Option<String> {
    match result.outcome {
        Outcome::Skipped => Some("nothing could be attached".into()),
        Outcome::Held => Some(
            result
                .problems
                .iter()
                .find(|p| p.starts_with("content filter withheld"))
                .cloned()
                .unwrap_or_else(|| "content filter withheld the caption".into()),
        ),
        _ => None,
    }
}

async fn journal(ctx: &JobCtx, id: String, result: &NarrativeResult) {
    let episode = result
        .chosen
        .as_ref()
        .and_then(|branch| branch.episode.as_ref());
    let mut thinking = Vec::new();
    if let Some(episode) = episode {
        thinking.extend([
            format!("title: {}", episode.title),
            format!("logline: {}", episode.logline),
            format!("state after: {}", episode.state_after),
        ]);
    }
    if !result.live_lore.is_empty() {
        thinking.push(format!("live lore: {}", result.live_lore.join(", ")));
    }
    for branch in &result.branches {
        thinking.push(format!(
            "branch {}{}: {}",
            branch.index + 1,
            branch
                .episode
                .as_ref()
                .map(|e| format!(" \"{}\"", e.title))
                .unwrap_or_default(),
            branch
                .rejected
                .clone()
                .unwrap_or_else(|| format!("composite {}", branch.composite.unwrap_or(0.0)))
        ));
    }
    if let Some(chosen) = &result.chosen {
        thinking.extend([
            String::new(),
            "the room:".into(),
            render_transcript(&chosen.transcript),
        ]);
    }
    if let Some(episode) = episode {
        thinking.extend([String::new(), format!("shot list: {}", episode.shot_list)]);
    }
    let mut tools = Vec::new();
    if let Some(image) = &result.image {
        tools.push(ToolCall {
            name: "generate_image".into(),
            result: image.id.clone(),
        });
    }
    if let Some(video) = &result.video {
        tools.push(ToolCall {
            name: "generate_video".into(),
            result: video.id.clone(),
        });
    }
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(id),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Narrative,
            incoming: format!(
                "episode {}: {} rooms written, {} cuts survived",
                result.episode_number,
                result.branches.len(),
                result
                    .branches
                    .iter()
                    .filter(|b| b.episode.is_some() && b.rejected.is_none())
                    .count()
            ),
            tone: Tone::Neutral,
            recalled: vec![result.briefing.clone()],
            tools,
            thinking: thinking.join("\n"),
            said: if result.outcome == Outcome::Held {
                String::new()
            } else {
                result.caption.clone()
            },
            declined: result.problems.clone(),
            outcome: result.outcome,
            outcome_note: note(result),
        },
    )
    .await;
}
