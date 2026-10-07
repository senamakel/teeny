//! What every daily posting job asks before it does anything, and how it
//! writes a failure down. The order is the legacy jobs': the rota (is it
//! this job's day?), the twenty-hour re-run guard, then the cadence (too
//! soon after the last post, or held for a hot one). A forced run (the
//! rehearsal scripts) skips all three. The guard is marked once the job has
//! decided to run, so a run that then fails still counts.

use teeny_cadence::cadence::Claim;
use teeny_cadence::rota::Rota;
use teeny_store::ReflectionInput;
use teeny_store::journal::{Outcome, Tone, Trigger};

use crate::ctx::JobCtx;
use crate::journal_glue::record_quietly;

/// One job's gates.
#[derive(Debug, Clone, Copy)]
pub struct Gates {
    /// The job's name, for the guard and the log.
    pub job: &'static str,
    /// The cadence's name for its post; `None` when it posts nowhere the
    /// cadence counts.
    pub kind: Option<&'static str>,
    /// The rota and this job's slot in it; `None` posts every day.
    pub rota: Option<(Rota, u32)>,
    /// Skip every gate.
    pub force: bool,
}

/// Whether the job may run now; `Err` says why not, which is not an error.
pub async fn admit(ctx: &JobCtx, gates: Gates) -> Result<(), String> {
    let now = ctx.now();
    if gates.force {
        ctx.guard(gates.job).mark(now);
        return Ok(());
    }
    if let Some((rota, slot)) = gates.rota
        && !rota.is_turn(slot, now)
    {
        return Err(format!(
            "{} job: skipping, not its day on the rota",
            gates.job
        ));
    }
    let guard = ctx.guard(gates.job);
    if guard.check(now).is_err() {
        return Err(format!(
            "{} job: skipping, it already ran recently",
            gates.job
        ));
    }
    if let Some(kind) = gates.kind {
        match ctx.deps.cadence.claim(kind).await {
            Ok(Claim::Granted) => {}
            Ok(Claim::Gap { .. }) => {
                return Err(format!(
                    "{} job: skipping, the cadence says too soon after the last post",
                    gates.job
                ));
            }
            Ok(Claim::Hot { .. }) => {
                return Err(format!(
                    "{} job: skipping, the cadence says the timeline is held for a hot post",
                    gates.job
                ));
            }
            Err(error) => {
                return Err(format!(
                    "{} job: skipping, the post ledger could not be read ({error})",
                    gates.job
                ));
            }
        }
    }
    guard.mark(now);
    Ok(())
}

/// The journal id of a daily job's turn: `morning:2026-09-14`.
pub fn day_id(ctx: &JobCtx, job: &str) -> String {
    format!("{job}:{}", ctx.now().format("%Y-%m-%d"))
}

/// A turn that failed, written down before the error is returned.
pub async fn journal_failure(
    ctx: &JobCtx,
    id: String,
    trigger: Trigger,
    incoming: &str,
    error: &anyhow::Error,
) {
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(id),
            at: Some(teeny_store::iso(ctx.now())),
            trigger,
            incoming: incoming.to_owned(),
            tone: Tone::Neutral,
            recalled: Vec::new(),
            tools: Vec::new(),
            thinking: String::new(),
            said: String::new(),
            declined: Vec::new(),
            outcome: Outcome::Failed,
            outcome_note: Some(error.to_string()),
        },
    )
    .await;
}

/// A random value in `[0, 1)` from the clock, for the picks that only need
/// to differ from day to day.
pub fn roll(ctx: &JobCtx) -> f64 {
    let nanos = ctx.now().timestamp_subsec_nanos() as u64 ^ ctx.now().timestamp() as u64;
    let mut x = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
