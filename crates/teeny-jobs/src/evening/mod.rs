//! The evening slot, one cron (`EVENING_CRON`): the serial on its days
//! (`NARRATIVE_ROTA_SLOTS`, default 1 and 4), the article on its
//! (`ARTICLE_ROTA_SLOTS`, default 2 and 5), and the evening lore page on
//! every day nobody has. A job switched off hands its days to the lore; with
//! the lore off too, those days are quiet. Each job keeps its own guards.
//! A port of the evening dispatch in `legacy/src/jobs/slot.ts`.

use teeny_cadence::rota::Rota;
use teeny_cadence::slot::SlotOutcome;

use crate::ctx::{JobCtx, JobReport};

/// Who posts in the evening slot on a given day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EveningJob {
    Narrative,
    Article,
    Lore,
}

impl EveningJob {
    pub fn name(self) -> &'static str {
        match self {
            Self::Narrative => "narrative",
            Self::Article => "article",
            Self::Lore => "lore",
        }
    }
}

/// The owner of rota day `day`, by the enabled jobs and their slots, the
/// first listed winning; the lore takes the rest.
pub fn owner_of(ctx: &JobCtx, day: u32) -> Option<EveningJob> {
    let config = &ctx.config;
    if config.narrative.enabled && config.rota.narrative_slots.contains(&day) {
        return Some(EveningJob::Narrative);
    }
    if config.article.enabled && config.rota.article_slots.contains(&day) {
        return Some(EveningJob::Article);
    }
    config.lore.enabled.then_some(EveningJob::Lore)
}

/// Who owns each day of the rota, for the startup log: `0:lore 1:narrative ...`.
pub fn describe(ctx: &JobCtx) -> anyhow::Result<String> {
    let rota =
        Rota::from_config(&ctx.config.rota).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok((0..rota.days())
        .map(|day| format!("{day}:{}", owner_of(ctx, day).map_or("-", EveningJob::name)))
        .collect::<Vec<_>>()
        .join(" "))
}

/// Fires the slot: runs the day's owner, or nobody. An owner's failure is
/// logged and reported, never raised: the slot carries on.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<SlotOutcome> {
    let rota =
        Rota::from_config(&ctx.config.rota).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let day = rota.slot(ctx.now());
    let Some(owner) = owner_of(ctx, day) else {
        tracing::info!(slot = "evening", day, "slot: nobody has today; quiet");
        return Ok(SlotOutcome::Quiet { day });
    };
    tracing::info!(
        slot = "evening",
        day,
        job = owner.name(),
        "slot: running the day's job"
    );
    let ran: anyhow::Result<JobReport> = match owner {
        EveningJob::Narrative => crate::narrative::run(ctx).await,
        EveningJob::Article => crate::article::run(ctx).await,
        EveningJob::Lore => crate::lore::run_evening(ctx).await,
    };
    Ok(match ran {
        Ok(_) => SlotOutcome::Ran {
            day,
            job: owner.name().into(),
        },
        Err(error) => {
            tracing::error!(%error, slot = "evening", job = owner.name(), "slot: the day's job failed");
            SlotOutcome::Failed {
                day,
                job: owner.name().into(),
                error: format!("{error:#}"),
            }
        }
    })
}

/// The evening rehearsal: the day's owner's `dry_run`.
pub async fn dry_run(ctx: &JobCtx) -> anyhow::Result<Option<JobReport>> {
    let rota =
        Rota::from_config(&ctx.config.rota).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(match owner_of(ctx, rota.slot(ctx.now())) {
        None => None,
        Some(EveningJob::Narrative) => Some(crate::narrative::dry_run(ctx).await?),
        Some(EveningJob::Article) => Some(crate::article::dry_run(ctx).await?),
        Some(EveningJob::Lore) => Some(crate::lore::dry_run(ctx).await?),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{TimeZone, Utc};
    use teeny_agent::deps::{Deps, fake_vars};
    use teeny_cadence::clock::ManualClock;
    use teeny_store::InMemoryJournal;

    use super::*;
    use crate::runner::ScriptedRunner;

    fn ctx(extra: &[(&'static str, &'static str)]) -> JobCtx {
        let mut vars = fake_vars();
        vars.push(("POST_ROTA_DAYS", "7"));
        vars.push(("POST_ROTA_ANCHOR", "2026-09-14"));
        vars.extend_from_slice(extra);
        let config = teeny_core::Config::from_vars(vars).unwrap();
        JobCtx::new(
            Arc::new(Deps::fake()),
            Arc::new(ScriptedRunner::new()),
            Arc::new(config),
            Arc::new(InMemoryJournal::default()),
        )
    }

    #[test]
    fn runs_the_days_owner_and_the_lore_on_days_nobody_has() {
        let all = ctx(&[
            ("NARRATIVE_ENABLED", "true"),
            ("ARTICLE_ENABLED", "true"),
            ("LORE_ENABLED", "true"),
        ]);
        assert_eq!(
            describe(&all).unwrap(),
            "0:lore 1:narrative 2:article 3:lore 4:narrative 5:article 6:lore"
        );
        let article_only = ctx(&[("ARTICLE_ENABLED", "true")]);
        assert_eq!(
            describe(&article_only).unwrap(),
            "0:- 1:- 2:article 3:- 4:- 5:article 6:-"
        );
        // A job switched off hands its days to the lore.
        let no_serial = ctx(&[("ARTICLE_ENABLED", "true"), ("LORE_ENABLED", "true")]);
        assert_eq!(owner_of(&no_serial, 1), Some(EveningJob::Lore));
    }

    #[tokio::test]
    async fn is_quiet_with_no_owner() {
        let quiet = ctx(&[("ARTICLE_ENABLED", "true")]).with_clock(Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 14, 19, 0, 0).unwrap(),
        )));
        assert_eq!(run(&quiet).await.unwrap(), SlotOutcome::Quiet { day: 0 });
    }

    #[tokio::test]
    async fn survives_an_owner_failing() {
        // The serial cannot be built without its lore files on this box; the slot reports it and carries on.
        let failing = ctx(&[
            ("NARRATIVE_ENABLED", "true"),
            ("NARRATIVE_LORE_PATH", "/nonexistent/LORE.md"),
        ])
        .with_clock(Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 15, 19, 0, 0).unwrap(),
        )));
        match run(&failing).await.unwrap() {
            SlotOutcome::Failed { day: 1, job, .. } => assert_eq!(job, "narrative"),
            other => panic!("{other:?}"),
        }
    }
}
