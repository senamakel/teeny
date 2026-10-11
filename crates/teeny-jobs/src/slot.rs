//! The jobs of a rota slot, as [`teeny_cadence::slot::SlotJob`] runs them:
//! a job's `run` behind [`SlotRun`], posted when its report says `posted`.
//!
//! Each slot is built per firing around the run's [`JobCtx`], so its jobs
//! borrow the context rather than own it. A slot's jobs are plain function
//! pointers ([`JobFn`]) so a test can hand the dispatcher its own.

use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;
use teeny_cadence::rota::Rota;
use teeny_cadence::slot::{SlotJob, SlotRun, SlotRunError};
use teeny_store::journal::Outcome;

use crate::ctx::{JobCtx, JobReport};

/// One job's scheduled run.
pub type JobFn = for<'c> fn(&'c JobCtx) -> BoxFuture<'c, anyhow::Result<JobReport>>;

/// A job in a slot: its run against the slot's context. It posted when its
/// report's outcome is `posted`; a rehearsal, a skip, a hold or a decline is
/// a slot the fallback may still take, as `legacy/src/index.ts` read
/// `outcome === 'posted'`.
struct JobOwner<'a> {
    ctx: &'a JobCtx,
    run: JobFn,
}

#[async_trait]
impl SlotRun for JobOwner<'_> {
    async fn run(&self) -> Result<bool, SlotRunError> {
        match (self.run)(self.ctx).await {
            Ok(report) => Ok(report.outcome == Outcome::Posted),
            Err(error) => Err(format!("{error:#}").into()),
        }
    }
}

/// An empty slot named `label` on the configured rota and the context's clock.
pub fn slot<'a>(ctx: &'a JobCtx, label: &str) -> anyhow::Result<SlotJob<'a>> {
    let rota = Rota::from_config(&ctx.config.rota)?;
    Ok(SlotJob::new(label, rota, Arc::clone(&ctx.clock)))
}

/// `run` as a job of a slot over `ctx`.
pub fn job(ctx: &JobCtx, run: JobFn) -> Arc<dyn SlotRun + '_> {
    Arc::new(JobOwner { ctx, run })
}
