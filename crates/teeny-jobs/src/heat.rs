//! The heat job's wiring: [`HeatJob`] over the account's own posts (the post
//! ledger) and their counters from X, as `legacy/src/index.ts` wired the
//! legacy `HeatJob`. The rules themselves live in teeny-cadence.

use std::sync::Arc;

use async_trait::async_trait;
use teeny_cadence::heat::{HeatError, HeatJob, HeatRules, MetricsSource, PostMetrics};
use teeny_core::types::PostId;
use teeny_store::journal::Outcome;
use teeny_x::cache::XReads;

use crate::ctx::{JobCtx, JobReport};

/// The job's name in reports.
pub const JOB: &str = "heat";

/// The counters of the account's own posts, read through X.
pub struct XMetrics(pub Arc<dyn XReads>);

fn signed(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

#[async_trait]
impl MetricsSource for XMetrics {
    async fn post_metrics(&self, ids: &[String]) -> Result<Vec<PostMetrics>, HeatError> {
        let ids: Vec<PostId> = ids.iter().map(|id| PostId::from(id.as_str())).collect();
        let metrics = self
            .0
            .post_metrics(&ids)
            .await
            .map_err(|error| HeatError::Metrics(error.to_string()))?;
        Ok(metrics
            .into_iter()
            .map(|counts| PostMetrics {
                id: counts.id.to_string(),
                impressions: signed(counts.impressions),
                likes: signed(counts.likes),
                replies: signed(counts.replies),
                reposts: signed(counts.reposts),
                quotes: signed(counts.quotes),
            })
            .collect())
    }
}

/// One reading of every own post from the last day; a hot one holds the
/// timeline. Off with `HEAT_ENABLED=false`.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    if !ctx.config.cadence.heat_enabled {
        return Ok(JobReport::declined(JOB, "heat is off"));
    }
    let conversation = ctx.conversation()?;
    let job = HeatJob::new(
        Arc::clone(&conversation.posts),
        Arc::new(XMetrics(Arc::clone(&ctx.deps.x_reads))),
        HeatRules::from_config(&ctx.config.cadence),
        Arc::clone(&ctx.clock),
    );
    let readings = job.run().await?;
    let hot = readings.iter().filter(|reading| reading.hot).count();
    Ok(JobReport {
        note: Some(format!("read {} posts, {hot} hot", readings.len())),
        ..JobReport::ran(JOB, Outcome::Skipped)
    })
}

#[cfg(test)]
#[path = "heat_tests.rs"]
mod tests;
