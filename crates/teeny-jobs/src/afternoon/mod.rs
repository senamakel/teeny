//! The afternoon slot (`0 13 * * *`): one cron, and the rota says who posts
//! in it. The moon on its slot, the chart on its slot, and the lore page on
//! every day nobody else has; switching an owner off hands its days to the
//! lore page rather than leaving them quiet. Each owner keeps its own guards
//! (the twenty hours, the cadence). A port of `legacy/src/jobs/slot.ts` as
//! `legacy/src/index.ts` wired the afternoon.

use anyhow::Result;
use chrono::{DateTime, Utc};
use teeny_cadence::rota::Rota;
use teeny_core::Config;

use crate::ctx::{JobCtx, JobReport};

const JOB: &str = "afternoon";

/// Who posts in the afternoon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Afternoon {
    Moon,
    Chart,
    /// The lore page, the fallback (owned by [`crate::lore`]).
    Lore,
}

impl Afternoon {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Moon => "moon",
            Self::Chart => "chart",
            Self::Lore => "lore",
        }
    }
}

fn owner_of(config: &Config, slot: u32) -> Option<Afternoon> {
    let rota = &config.rota;
    if config.moon.enabled && slot == rota.moon_slot {
        Some(Afternoon::Moon)
    } else if config.chart.enabled && slot == rota.chart_slot {
        Some(Afternoon::Chart)
    } else if config.lore.enabled {
        Some(Afternoon::Lore)
    } else {
        None
    }
}

/// The day's owner of the slot, or `None` for a quiet day.
pub fn owner(config: &Config, now: DateTime<Utc>) -> Result<Option<Afternoon>> {
    let rota = Rota::from_config(&config.rota)?;
    Ok(owner_of(config, rota.slot(now)))
}

/// Who owns each slot of the rota, for the startup log: `0:moon 1:lore ...`.
pub fn describe(config: &Config) -> Result<String> {
    let rota = Rota::from_config(&config.rota)?;
    Ok((0..rota.days())
        .map(|slot| {
            format!(
                "{slot}:{}",
                owner_of(config, slot).map_or("-", Afternoon::as_str)
            )
        })
        .collect::<Vec<_>>()
        .join(" "))
}

/// Runs the day's owner.
pub async fn run(ctx: &JobCtx) -> Result<JobReport> {
    let Some(owner) = owner(&ctx.config, ctx.now())? else {
        tracing::info!("afternoon: nobody has today; quiet");
        return Ok(JobReport::declined(JOB, "slot: nobody has today; quiet"));
    };
    tracing::info!(job = owner.as_str(), "afternoon: running the day's job");
    match owner {
        Afternoon::Moon => crate::moon::run(ctx).await,
        Afternoon::Chart => crate::chart::run(ctx).await,
        Afternoon::Lore => crate::lore::run_afternoon(ctx).await,
    }
}

/// The rehearsal of the day's owner.
pub async fn dry_run(ctx: &JobCtx) -> Result<JobReport> {
    match owner(&ctx.config, ctx.now())? {
        None => Ok(JobReport::declined(JOB, "slot: nobody has today; quiet")),
        Some(Afternoon::Moon) => crate::moon::dry_run(ctx).await,
        Some(Afternoon::Chart) => crate::chart::dry_run(ctx).await,
        Some(Afternoon::Lore) => crate::lore::dry_run(ctx).await,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
