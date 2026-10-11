//! The learnings brief: a short document the account rewrites about itself
//! once a day (`LEARNINGS_CRON`, 05:00 UTC, before the morning post), from
//! what it posted and how that did, who it talked to, and what memory holds
//! that the record does not. Every prompt then carries it next to SOUL.md
//! and CONTEXT.md, read live from `LEARNINGS_PATH` (see
//! [`teeny_agent::prompt::Persona`]), so a brief written at five is in the
//! six o'clock reply without a restart. A port of
//! `legacy/src/learnings/reflect.ts` and `legacy/src/scripts/learnings.ts`.
//!
//! The model call goes through the runner like every other pipeline's: a
//! scheduled turn on its own agent, `teeny-learner` (`AgentKey::Learner`),
//! whose system prompt is the brief's instructions and whose belt is one
//! memory read: it cannot post, browse or reach X. The answer is a fixed shape, one list per section;
//! the markdown, the six headings and the length are host code's. One
//! writer, the morning role's: every process reads the file, and two
//! writers would race to rewrite it.

pub mod landed;
pub mod reflect;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use teeny_media::reference::TextModel;
use teeny_store::journal::Outcome;

pub use reflect::{LEARNINGS_SECTIONS, Mode, Reflect, ReflectOutcome, ReflectResult, fit, reflect};

use crate::ctx::{JobCtx, JobReport};
use crate::runner::{AgentKey, AgentRun, AgentRunner, Origin};

const JOB: &str = "learnings";

/// A [`TextModel`] over the runner: each completion is one scheduled turn
/// on the learner, with the call's system prompt as the agent's. Run once:
/// the stage around it decides on a second try.
pub struct RunnerText {
    pub runner: Arc<dyn AgentRunner>,
    pub label: &'static str,
}

#[async_trait]
impl TextModel for RunnerText {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> teeny_media::Result<String> {
        let outcome = self
            .runner
            .run(AgentRun {
                agent: AgentKey::Learner {
                    system_prompt: system.unwrap_or_default().to_owned(),
                },
                origin: Origin::Scheduled { once: true },
                prompt: prompt.to_owned(),
                session: None,
                model: None,
                label: self.label.to_owned(),
            })
            .await
            .map_err(|error| teeny_media::Error::failed(error.to_string()))?;
        Ok(outcome.reply)
    }
}

/// Whether the brief is on and has not been written yet: the first one
/// should not wait for tomorrow's five o'clock.
pub fn needs_first_brief(ctx: &JobCtx) -> bool {
    ctx.config.learnings.enabled && !Path::new(&ctx.config.learnings.path).exists()
}

/// The scheduled run: rewrites the brief.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    if !ctx.config.learnings.enabled {
        return Ok(JobReport::declined(JOB, "LEARNINGS_ENABLED is off"));
    }
    Ok(report(&reflect_with(ctx, Mode::Write).await?))
}

/// `teeny learnings --dry-run`: the brief drafted from today's record, in
/// the report's drafts; nothing is written.
pub async fn dry_run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    Ok(report(&reflect_with(ctx, Mode::Rehearsal).await?))
}

async fn reflect_with(ctx: &JobCtx, mode: Mode) -> anyhow::Result<ReflectResult> {
    let text = RunnerText {
        runner: Arc::clone(&ctx.runner),
        label: JOB,
    };
    let conversation = ctx.conversation.as_deref();
    let config = &ctx.config.learnings;
    let path = Path::new(&config.path);
    reflect(
        &Reflect {
            path,
            text: &text,
            ledger: conversation.map(|conversation| conversation.posts.as_ref()),
            journal: Some(ctx.journal.as_ref()),
            people: conversation.and_then(|conversation| conversation.people.as_deref()),
            memory: Some(ctx.deps.memory.as_ref()),
            bot_username: &ctx.config.x.bot_username,
            days: config.days,
            max_chars: config.max_chars as usize,
            now: ctx.now(),
        },
        mode,
    )
    .await
}

/// A written brief reads as `posted` (it went where it is read from), a kept
/// one as `skipped`, a rehearsal's as `dry-run`.
fn report(result: &ReflectResult) -> JobReport {
    let outcome = match result.outcome {
        ReflectOutcome::Written => Outcome::Posted,
        ReflectOutcome::Kept => Outcome::Skipped,
        ReflectOutcome::Drafted => Outcome::DryRun,
    };
    let mut report = JobReport::ran(JOB, outcome);
    report.note = result.reason.clone();
    report.drafts = result.brief.iter().cloned().collect();
    report.problems = result.problems.clone();
    report
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
