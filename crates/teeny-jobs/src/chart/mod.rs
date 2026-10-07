//! One chart post a day, on its day of the rota: the `teeny-chart` agent
//! researches, renders once and posts once through `post_chart` (which runs
//! the content filter, the cadence and `DRY_RUN` in host code, and writes
//! the post back to memory). A port of `legacy/src/jobs/chart-job.ts` and
//! the memory half of `legacy/src/harness/chart-harness.ts`.
//!
//! The harness fetches what was said yesterday and who the community is
//! every day rather than hoping the model asks: on a good run the model
//! called recall twice with near-identical queries, on a bad one it skipped
//! the community and posted into a void. A recall that fails is a worse
//! post, never a missing day.

use anyhow::{Result, anyhow};
use teeny_agent::posture::Belt;
use teeny_cadence::rota::Rota;
use teeny_store::ReflectionInput;
use teeny_store::journal::{Outcome, Tone, Trigger};

use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::{record_quietly, tool_steps};
use crate::morning::job::{Gates, admit, day_id, journal_failure};
use crate::runner::{AgentKey, AgentOutcome, AgentRun, Origin};

const JOB: &str = "chart";

/// The day's task, as the legacy job worded it.
pub fn task(symbol: &str, timeframe: &str) -> String {
    [
        format!("Post today's {symbol} chart."),
        format!("Start on the {timeframe} timeframe; use a different one if the read"),
        "is genuinely about a different horizon.".into(),
        "Follow your instructions exactly: research, render once, post once, remember what you did."
            .into(),
    ]
    .join(" ")
}

/// The task with yesterday's posts and the community in front of the agent.
pub fn with_memory(task: &str, symbol: &str, history: &str, community: &str) -> String {
    [
        task,
        "",
        "WHAT YOU HAVE ALREADY POSTED (untrusted, from your own memory). Do not repeat an angle",
        "or a framing that appears here, and never contradict something you already said:",
        if history.is_empty() {
            "(nothing recalled -- treat today as your first post and do not pretend otherwise)"
        } else {
            history
        },
        "",
        &format!(
            "THE {} COMMUNITY (untrusted, from your own memory). These are the people",
            symbol.to_uppercase()
        ),
        "you are writing for. Use this to know who you are talking to, not as material to quote:",
        if community.is_empty() {
            "(nothing recalled -- write for holders in general, invent nobody)"
        } else {
            community
        },
    ]
    .join("\n")
}

/// The scheduled run: the rota, the guard, the cadence, the turn, the journal.
pub async fn run(ctx: &JobCtx) -> Result<JobReport> {
    let rota = Rota::from_config(&ctx.config.rota)?;
    let gates = Gates {
        job: JOB,
        kind: Some(JOB),
        rota: Some((rota, ctx.config.rota.chart_slot)),
        force: false,
    };
    if let Err(why) = admit(ctx, gates).await {
        tracing::info!("{why}");
        return Ok(JobReport::declined(JOB, why));
    }
    let outcome = match turn(ctx).await {
        Ok(outcome) => outcome,
        Err(error) => {
            journal_failure(
                ctx,
                day_id(ctx, JOB),
                Trigger::Chart,
                &incoming(ctx),
                &error,
            )
            .await;
            return Err(error);
        }
    };
    let report = report(ctx, &outcome);
    tracing::info!(tools = ?outcome.tool_calls.iter().map(|call| call.name.as_str()).collect::<Vec<_>>(), output = outcome.reply, "chart job: done");
    journal(ctx, &outcome, &report).await;
    Ok(report)
}

/// The rehearsal (`chart-dry-run`): the same turn, no gates, no journal.
/// `post_chart` is the agent's own tool, so a rehearsal is only safe with
/// `DRY_RUN` on, and refuses otherwise.
pub async fn dry_run(ctx: &JobCtx) -> Result<JobReport> {
    if !ctx.deps.settings.dry_run {
        return Err(anyhow!(
            "the chart rehearsal posts through the agent's own tool, so it needs DRY_RUN on"
        ));
    }
    let outcome = turn(ctx).await?;
    Ok(report(ctx, &outcome))
}

fn incoming(ctx: &JobCtx) -> String {
    format!(
        "daily {} chart, {}",
        ctx.deps.settings.token_symbol,
        ctx.config.chart.timeframe.as_str()
    )
}

async fn recall(ctx: &JobCtx, query: &str) -> String {
    ctx.deps
        .memory
        .recall(query, None)
        .await
        .unwrap_or_default()
}

async fn turn(ctx: &JobCtx) -> Result<AgentOutcome> {
    let symbol = ctx.deps.settings.token_symbol.as_str();
    let history_query = format!(
        "What chart posts has this account published recently? For each one: which angle it took, which price level it called out, and how it was framed. Also anything about {symbol} the account has already said publicly."
    );
    let community_query = format!(
        "Who are the {symbol} holders and community members this account has been talking to? What do they care about, what have they been asking, what has been promised to them, and what is the mood in the community right now?"
    );
    let (history, community) =
        tokio::join!(recall(ctx, &history_query), recall(ctx, &community_query));
    let prompt = with_memory(
        &task(symbol, ctx.config.chart.timeframe.as_str()),
        symbol,
        &history,
        &community,
    );
    Ok(ctx
        .runner
        .run(AgentRun {
            agent: AgentKey::Belt(Belt::Chart),
            origin: Origin::Scheduled { once: true },
            prompt,
            session: Some(day_id(ctx, JOB)),
            model: None,
            label: JOB.into(),
        })
        .await?)
}

/// The text handed to `post_chart`, when the agent posted.
fn caption(outcome: &AgentOutcome) -> Option<String> {
    outcome
        .tool_calls
        .iter()
        .find(|call| call.name == "post_chart" && !call.is_error)
        .map(|call| {
            call.arguments
                .get("text")
                .and_then(|text| text.as_str())
                .unwrap_or("")
                .to_owned()
        })
}

fn report(ctx: &JobCtx, outcome: &AgentOutcome) -> JobReport {
    match caption(outcome) {
        Some(text) => {
            let mut report = JobReport::ran(
                JOB,
                if ctx.deps.settings.dry_run {
                    Outcome::DryRun
                } else {
                    Outcome::Posted
                },
            );
            report.drafts = vec![text];
            report
        }
        None => {
            let mut report = JobReport::ran(JOB, Outcome::Skipped);
            report.note = Some("the agent finished without posting".into());
            report
        }
    }
}

/// The chart turn, for the mind page: the caption is what was handed to
/// `post_chart`, the agent's summary is the thinking.
async fn journal(ctx: &JobCtx, outcome: &AgentOutcome, report: &JobReport) {
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(day_id(ctx, JOB)),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Chart,
            incoming: incoming(ctx),
            tone: Tone::Neutral,
            recalled: Vec::new(),
            tools: tool_steps(&outcome.tool_calls),
            thinking: outcome.reply.clone(),
            said: report.drafts.first().cloned().unwrap_or_default(),
            declined: Vec::new(),
            outcome: report.outcome,
            outcome_note: report.note.clone(),
        },
    )
    .await;
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
