//! The afternoon slot (`AFTERNOON_CRON`, `0 15 * * *`): one cron, and the
//! rota says who posts in it. The chart on its day (`CHART_ROTA_SLOT`, 0),
//! the moon chart on its (`MOON_ROTA_SLOT`, 1), and the lore page, the
//! fallback, on every other day and on an owner's day when the owner did not
//! post. Switching an owner off hands its days to the lore; with the lore
//! off too, those days are quiet. Each job keeps its own guards (the twenty
//! hours, the cadence). A port of `legacy/src/jobs/slot.ts` as
//! `legacy/src/index.ts` wired the afternoon.

use anyhow::Result;
use chrono::{DateTime, Utc};
use teeny_cadence::rota::Rota;
use teeny_cadence::slot::{SlotJob, SlotOutcome};
use teeny_core::Config;

use crate::ctx::{JobCtx, JobReport};
use crate::slot::{JobFn, job, slot};

/// Who posts in the afternoon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Afternoon {
    Chart,
    Moon,
    /// The lore page, the fallback (owned by [`crate::lore`]).
    Lore,
}

impl Afternoon {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chart => "chart",
            Self::Moon => "moon",
            Self::Lore => "lore",
        }
    }
}

/// The afternoon's jobs, so a test can stand in for them.
pub struct AfternoonJobs {
    pub chart: JobFn,
    pub moon: JobFn,
    pub lore: JobFn,
}

/// The production jobs.
pub const JOBS: AfternoonJobs = AfternoonJobs {
    chart: |ctx| Box::pin(crate::chart::run(ctx)),
    moon: |ctx| Box::pin(crate::moon::run(ctx)),
    lore: |ctx| Box::pin(crate::lore::run(ctx)),
};

/// The slot over `ctx`: the enabled owners on their days, the lore behind them.
pub fn build<'a>(ctx: &'a JobCtx, jobs: &AfternoonJobs) -> Result<SlotJob<'a>> {
    let config = &ctx.config;
    let mut afternoon = slot(ctx, "afternoon")?;
    if config.chart.enabled {
        afternoon = afternoon.owner("chart", [config.rota.chart_slot], job(ctx, jobs.chart));
    }
    if config.moon.enabled {
        afternoon = afternoon.owner("moon", [config.rota.moon_slot], job(ctx, jobs.moon));
    }
    if config.lore.enabled {
        afternoon = afternoon.fallback("lore", job(ctx, jobs.lore));
    }
    Ok(afternoon)
}

/// Who runs first on rota day `slot`: the day's owner, else the lore.
fn owner_of(config: &Config, slot: u32) -> Option<Afternoon> {
    let rota = &config.rota;
    if config.chart.enabled && slot == rota.chart_slot {
        Some(Afternoon::Chart)
    } else if config.moon.enabled && slot == rota.moon_slot {
        Some(Afternoon::Moon)
    } else if config.lore.enabled {
        Some(Afternoon::Lore)
    } else {
        None
    }
}

/// Who runs first in the slot at `now`, or `None` for a quiet day.
pub fn owner(config: &Config, now: DateTime<Utc>) -> Result<Option<Afternoon>> {
    let rota = Rota::from_config(&config.rota)?;
    Ok(owner_of(config, rota.slot(now)))
}

/// Who owns each day of the rota, for the startup log: `0:chart 1:moon 2:lore`.
pub fn describe(ctx: &JobCtx) -> Result<String> {
    Ok(build(ctx, &JOBS)?.describe())
}

/// Fires the slot: the day's owner, then the lore when it did not post.
pub async fn run(ctx: &JobCtx) -> Result<SlotOutcome> {
    run_with(ctx, &JOBS).await
}

/// [`run`] with these jobs.
pub async fn run_with(ctx: &JobCtx, jobs: &AfternoonJobs) -> Result<SlotOutcome> {
    Ok(build(ctx, jobs)?.run().await)
}

/// The rehearsal of whoever runs first today.
pub async fn dry_run(ctx: &JobCtx) -> Result<JobReport> {
    match owner(&ctx.config, ctx.now())? {
        None => Ok(JobReport::declined(
            "afternoon",
            "slot: nobody has today; quiet",
        )),
        Some(Afternoon::Chart) => crate::chart::dry_run(ctx).await,
        Some(Afternoon::Moon) => crate::moon::dry_run(ctx).await,
        Some(Afternoon::Lore) => crate::lore::dry_run(ctx).await,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
