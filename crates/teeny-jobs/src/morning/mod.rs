//! The good-morning post, once a day: the job around
//! [`pipeline::MorningPipeline`]. A port of `legacy/src/jobs/morning-job.ts`
//! and `legacy/src/morning/wiring.ts`.
//!
//! The same twenty-hour guard as the other daily jobs: a container restarted
//! near the cron minute would say good morning twice, and two morning posts
//! an hour apart is the repetitive-publishing pattern that cost this account
//! its reach once. The calendar's `gm` item stands down while this job runs
//! ([`teeny_cadence::calendar::MORNING_CALENDAR_SLOT`]).
//!
//! The submodules are the stages the other daily posts share: `stages`
//! (one JSON answer from a plain completion), `post` (an own post through
//! the cadence and the publisher) and `job` (the rota, the guard, the
//! cadence).

pub mod job;
pub mod news;
pub mod pipeline;
pub mod post;
pub mod prompts;
pub mod stages;
#[cfg(test)]
pub(crate) mod test_support;
pub mod timeline;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use teeny_agent::prompt::render_persona;
use teeny_store::ReflectionInput;
use teeny_store::holds::HoldSurface;
use teeny_store::journal::{Outcome, Tone, ToolCall, Trigger};

use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::record_quietly;
use job::{Gates, admit, day_id, journal_failure, roll};
use news::ExaNewsSource;
use pipeline::{Mode, MorningPipeline, MorningResult, Reference};
use post::XOwnPoster;
use prompts::with_skills;
use timeline::{TimelineSource, XTimelineSource, parse_timeline_queries};

const JOB: &str = "morning";

/// The pipeline the service runs, from the clients and the configuration.
/// It needs a news key and an image generator; missing either, it says
/// which, because a morning post without a picture is not the feature.
pub fn build(ctx: &JobCtx) -> Result<MorningPipeline> {
    let deps = &ctx.deps;
    let config = &ctx.config;
    let exa = deps
        .exa
        .clone()
        .ok_or_else(|| anyhow!("EXA_API_KEY is not set, so there is no news to read"))?;
    let media = deps.media.as_ref();
    let images = media.and_then(|media| media.images.clone()).ok_or_else(|| {
        anyhow!("IMAGE_GEN_ENABLED is off, so there is nothing to draw the morning post with")
    })?;
    let media = media.expect("images came from it");
    let morning = &config.morning;
    let timeline = (morning.x_posts_per_query > 0).then(|| {
        Arc::new(XTimelineSource::new(
            Arc::clone(&deps.x_reads),
            (morning.x_trends_woeid > 0).then_some(morning.x_trends_woeid),
            ctx.now(),
        )) as Arc<dyn TimelineSource>
    });
    Ok(MorningPipeline {
        news: Arc::new(ExaNewsSource::new(exa)),
        timeline,
        timeline_queries: parse_timeline_queries(&morning.x_queries),
        timeline_hours: morning.x_hours,
        posts_per_query: morning.x_posts_per_query,
        text: Arc::new(deps.llm.clone().with_source("morning")),
        images,
        videos: media.videos.clone(),
        reader: media.reader.clone(),
        reference: media
            .reference
            .clone()
            .map(|sheet| sheet as Arc<dyn Reference>),
        store: Arc::clone(&media.store),
        budget: Arc::clone(&deps.media_budget),
        memory: Arc::clone(&deps.memory),
        poster: Arc::new(XOwnPoster::from_deps(deps, HoldSurface::Morning, "morning_post")),
        safety: Some(Arc::clone(&deps.safety)),
        critic: deps.critic.clone(),
        persona: with_skills(&render_persona(&deps.persona), &config.agent.skills_dir),
        bot_username: deps.settings.bot_username.clone(),
        topics: morning.news_topics.clone(),
        news_days: morning.news_days,
        stories_per_topic: morning.stories_per_topic,
        video_seconds: morning.video_seconds,
        max_emoji: deps.settings.max_emoji,
        random: roll(ctx),
        now: ctx.now(),
        retry_delay: Duration::from_secs(30),
    })
}

/// The scheduled run: the guard, the cadence, the pipeline, the journal.
pub async fn run(ctx: &JobCtx) -> Result<JobReport> {
    if let Err(why) = admit(ctx, gates(false)).await {
        tracing::info!("{why}");
        return Ok(JobReport::declined(JOB, why));
    }
    let pipeline = match build(ctx) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            journal_failure(ctx, day_id(ctx, JOB), Trigger::Morning, "the morning post", &error).await;
            return Err(error);
        }
    };
    run_pipeline(ctx, &pipeline).await
}

/// The rehearsal (`morning-dry-run`): the whole pipeline, nothing posted,
/// no gates, nothing journaled; the drafts come back in the report.
pub async fn dry_run(ctx: &JobCtx) -> Result<JobReport> {
    let pipeline = build(ctx)?;
    let result = pipeline.run(Mode::Rehearsal).await?;
    Ok(report(&result))
}

fn gates(force: bool) -> Gates {
    Gates {
        job: JOB,
        kind: Some(JOB),
        rota: None,
        force,
    }
}

/// Runs an admitted morning and journals it, a failure included.
pub async fn run_pipeline(ctx: &JobCtx, pipeline: &MorningPipeline) -> Result<JobReport> {
    let result = match pipeline.run(Mode::Live).await {
        Ok(result) => result,
        Err(error) => {
            journal_failure(ctx, day_id(ctx, JOB), Trigger::Morning, "the morning post", &error).await;
            return Err(error).context("the morning post");
        }
    };
    tracing::info!(outcome = ?result.outcome, attached = result.attached.as_str(), story = result.pick.headline, problems = ?result.problems, posted = ?result.posted_id, "morning job: done");
    journal(ctx, &result).await;
    Ok(report(&result))
}

fn report(result: &MorningResult) -> JobReport {
    let mut report = JobReport::ran(JOB, result.outcome);
    report.posted_id = result.posted_id.clone();
    report.drafts = (!result.caption.is_empty())
        .then(|| result.caption.clone())
        .into_iter()
        .collect();
    report.problems = result.problems.clone();
    report.note = match result.outcome {
        Outcome::Held => Some(result.held_note()),
        Outcome::Skipped => Some("nothing could be attached".into()),
        _ => None,
    };
    report
}

/// The morning turn, for the mind page: the caption is what went out, the
/// rest is the thinking. A held caption is never on the public page.
async fn journal(ctx: &JobCtx, result: &MorningResult) {
    let mut tools = vec![ToolCall {
        name: "news".into(),
        result: match &result.story {
            Some(story) => format!("{} ({})", story.title, story.source),
            None => "no story chosen".into(),
        },
    }];
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
    let mut thinking = vec![
        format!("story: {}", result.pick.headline),
        format!("angle: {}", result.pick.angle),
    ];
    if let Some(seed) = &result.seed {
        thinking.push(format!("seed: {}", seed.url));
    }
    for (index, concept) in result.scene.concepts.iter().enumerate() {
        thinking.push(format!("concept {}: {concept}", index + 1));
    }
    thinking.push(format!("scene: {}", result.scene.scene));
    thinking.push(format!("cue: {}", result.scene.story_cue));
    if let Some(script) = &result.script {
        thinking.push(format!("script: {}", script.script));
    }
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(day_id(ctx, JOB)),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Morning,
            incoming: format!(
                "{} stories read, {} dropped as political",
                result.stories.len(),
                result.dropped_as_political
            ),
            tone: Tone::Neutral,
            recalled: [&result.history, &result.community]
                .into_iter()
                .filter(|recalled| !recalled.is_empty())
                .cloned()
                .collect(),
            tools,
            thinking: thinking.join("\n"),
            said: if result.outcome == Outcome::Held {
                String::new()
            } else {
                result.caption.clone()
            },
            declined: result.problems.clone(),
            outcome: result.outcome,
            outcome_note: report(result).note,
        },
    )
    .await;
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
