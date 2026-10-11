//! The evening slot, one cron (`EVENING_CRON`, `0 23 * * *`): the article
//! on its days (`ARTICLE_ROTA_SLOTS`, default 1), the serial on its
//! (`NARRATIVE_ROTA_SLOTS`, default 2), and the quote post, the fallback, on
//! every other day and on an owner's day when the owner did not post (the
//! article rehearsed, the serial had no seed, either failed). A job switched
//! off hands its days to the quote; with the quote off too, those days are
//! quiet. Each job keeps its own guards. A port of the evening dispatch in
//! `legacy/src/jobs/slot.ts` and `legacy/src/index.ts`.

use teeny_cadence::rota::Rota;
use teeny_cadence::slot::{SlotJob, SlotOutcome};

use crate::ctx::{JobCtx, JobReport};
use crate::slot::{JobFn, job, slot};

/// Who posts in the evening slot on a given day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EveningJob {
    Narrative,
    Article,
    Quote,
}

impl EveningJob {
    pub fn name(self) -> &'static str {
        match self {
            Self::Narrative => "narrative",
            Self::Article => "article",
            Self::Quote => "quote",
        }
    }
}

/// The evening's jobs, so a test can stand in for them.
pub struct EveningJobs {
    pub narrative: JobFn,
    pub article: JobFn,
    pub quote: JobFn,
}

/// The production jobs.
pub const JOBS: EveningJobs = EveningJobs {
    narrative: |ctx| Box::pin(crate::narrative::run(ctx)),
    article: |ctx| Box::pin(crate::article::run(ctx)),
    quote: |ctx| Box::pin(crate::quote::run(ctx)),
};

/// The slot over `ctx`: the enabled owners on their days, the quote behind
/// them. The serial registers first, so on a day both claim it is the
/// serial's, as `legacy/src/index.ts` registered them.
pub fn build<'a>(ctx: &'a JobCtx, jobs: &EveningJobs) -> anyhow::Result<SlotJob<'a>> {
    let config = &ctx.config;
    let mut evening = slot(ctx, "evening")?;
    if config.narrative.enabled {
        evening = evening.owner(
            "narrative",
            config.rota.narrative_slots.iter().copied(),
            job(ctx, jobs.narrative),
        );
    }
    if config.article.enabled {
        evening = evening.owner(
            "article",
            config.rota.article_slots.iter().copied(),
            job(ctx, jobs.article),
        );
    }
    if config.quote.enabled {
        evening = evening.fallback("quote", job(ctx, jobs.quote));
    }
    Ok(evening)
}

/// Who runs first on rota day `day`: the day's owner, else the quote.
pub fn owner_of(ctx: &JobCtx, day: u32) -> Option<EveningJob> {
    let config = &ctx.config;
    if config.narrative.enabled && config.rota.narrative_slots.contains(&day) {
        return Some(EveningJob::Narrative);
    }
    if config.article.enabled && config.rota.article_slots.contains(&day) {
        return Some(EveningJob::Article);
    }
    config.quote.enabled.then_some(EveningJob::Quote)
}

/// Who owns each day of the rota, for the startup log: `0:quote 1:article 2:narrative`.
pub fn describe(ctx: &JobCtx) -> anyhow::Result<String> {
    Ok(build(ctx, &JOBS)?.describe())
}

/// Fires the slot: the day's owner, then the quote when it did not post. A
/// job's failure is logged and reported in the outcome, never raised.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<SlotOutcome> {
    run_with(ctx, &JOBS).await
}

/// [`run`] with these jobs.
pub async fn run_with(ctx: &JobCtx, jobs: &EveningJobs) -> anyhow::Result<SlotOutcome> {
    Ok(build(ctx, jobs)?.run().await)
}

/// The evening rehearsal: the `dry_run` of whoever runs first tonight.
pub async fn dry_run(ctx: &JobCtx) -> anyhow::Result<Option<JobReport>> {
    let rota = Rota::from_config(&ctx.config.rota)?;
    Ok(match owner_of(ctx, rota.slot(ctx.now())) {
        None => None,
        Some(EveningJob::Narrative) => Some(crate::narrative::dry_run(ctx).await?),
        Some(EveningJob::Article) => Some(crate::article::dry_run(ctx).await?),
        Some(EveningJob::Quote) => Some(crate::quote::dry_run(ctx).await?),
    })
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
