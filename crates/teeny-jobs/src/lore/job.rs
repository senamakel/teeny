//! The lore job: the guards (twenty hours, the cadence), the pipeline, the
//! journal. The rota is the slot's to apply ([`crate::afternoon`], whose
//! fallback this is), not this job's.

use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use teeny_core::config::LoreSeed;
use teeny_memory::Memory;
use teeny_store::journal::{Outcome, ReflectionInput, Tone, ToolCall, Trigger};

use super::pipeline::{LorePipeline, LoreResult, SeedSource};
use crate::backrooms::Backrooms;
use crate::backrooms::seeds::{ForumSeeds, MemorySeeds, NewsSeeds, SeedFinder};
use crate::backrooms::sources::{NewsFeed, WebSearch};
use crate::backrooms::tokens::{Random, Rng};
use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::record_quietly;
use crate::morning::job::{Gates, admit, day_id, journal_failure};
use crate::research::{BrowserResearch, BrowsingSeeds};

/// The page's name: its journal kind, its guard, its ledger kind.
pub const AFTERNOON: &str = "lore";

/// The scheduled run: the afternoon page, the afternoon slot's fallback.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    run_named(ctx, AFTERNOON).await
}

/// The rehearsal (`lore-dry-run`): the whole pipeline with no gates and
/// nothing posted, journaled as a dry run; the draft comes back in the report.
pub async fn dry_run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let pipeline = build(ctx, AFTERNOON);
    run_pipeline(ctx, AFTERNOON, &pipeline, true).await
}

async fn run_named(ctx: &JobCtx, name: &'static str) -> anyhow::Result<JobReport> {
    if let Err(why) = admit(
        ctx,
        Gates {
            job: name,
            kind: Some(name),
            rota: None,
            force: false,
        },
    )
    .await
    {
        tracing::info!("{why}");
        return Ok(JobReport::declined(name, why));
    }
    let pipeline = build(ctx, name);
    run_pipeline(ctx, name, &pipeline, false).await
}

/// The production pipeline: the backrooms, today's seed order (a rotation
/// of `LORE_SEEDS`, so the first source is not always the one that answers),
/// `LORE_TOPICS`, a room of `LORE_BACKROOMS_TURNS` turns.
pub fn build(ctx: &JobCtx, name: &str) -> LorePipeline {
    let config = &ctx.config;
    let backrooms = Arc::new(Backrooms::from_ctx(ctx));
    let mut random = Rng::new(Rng::seed_from_time());
    let start = random.index(config.lore.seeds.len());
    let mut order = config.lore.seeds.clone();
    order.rotate_left(start);
    let web: Option<Arc<dyn WebSearch>> = ctx.deps.exa.clone().map(|exa| exa as Arc<dyn WebSearch>);
    let seeds = order
        .into_iter()
        .map(|seed| -> (SeedSource, Arc<dyn SeedFinder>) {
            let draws = Mutex::new(Rng::new(Rng::seed_from_time()));
            match seed {
                LoreSeed::News => (
                    SeedSource::News,
                    Arc::new(NewsSeeds {
                        web: web.clone(),
                        feed: NewsFeed::new(),
                        random: draws,
                    }),
                ),
                LoreSeed::Forum => (
                    SeedSource::Forum,
                    Arc::new(ForumSeeds {
                        forums: backrooms.forums().to_vec(),
                        random: draws,
                    }),
                ),
                LoreSeed::Memory => (
                    SeedSource::Memory,
                    Arc::new(MemorySeeds {
                        memory: Arc::clone(&ctx.deps.memory) as Arc<dyn Memory>,
                    }),
                ),
            }
        })
        .map(|(source, finder)| -> (SeedSource, Arc<dyn SeedFinder>) {
            // The browser experiment: a seed from a page carries the page.
            if config.openhuman.browser_research {
                let research = BrowserResearch::new(Arc::clone(&ctx.runner), "lore.research");
                (
                    source,
                    Arc::new(BrowsingSeeds {
                        inner: finder,
                        research,
                    }),
                )
            } else {
                (source, finder)
            }
        })
        .collect();
    LorePipeline {
        rooms: backrooms,
        seeds,
        topics: config.lore.topics.clone(),
        turns: Some(config.lore.backrooms_turns),
        deps: Arc::clone(&ctx.deps),
        kind: name.to_owned(),
        random: Mutex::new(random),
        clock: Arc::clone(&ctx.clock),
    }
}

/// Runs an admitted (or rehearsed) page and journals it, a failure included.
pub async fn run_pipeline(
    ctx: &JobCtx,
    name: &str,
    pipeline: &LorePipeline,
    rehearsal: bool,
) -> anyhow::Result<JobReport> {
    let id = if rehearsal {
        format!("{}:dry-run", day_id(ctx, name))
    } else {
        day_id(ctx, name)
    };
    let result = match pipeline.run(rehearsal).await {
        Ok(result) => result,
        Err(error) => {
            journal_failure(ctx, id, Trigger::Lore, "the lore post", &error).await;
            return Err(error).context("the lore post");
        }
    };
    tracing::info!(job = name, outcome = ?result.outcome, room = ?result.room.as_ref().map(|r| &r.id), problems = ?result.problems, posted = ?result.posted_id, "lore job: done");
    journal(ctx, id, &result).await;
    Ok(report(name, &result))
}

fn report(name: &str, result: &LoreResult) -> JobReport {
    let mut report = JobReport::ran(name, result.outcome);
    report.posted_id = result.posted_id.clone();
    report.drafts = result.post.iter().map(|post| post.text.clone()).collect();
    report.problems = result.problems.clone();
    report.note = note(result);
    report
}

fn note(result: &LoreResult) -> Option<String> {
    match result.outcome {
        Outcome::Held => Some(
            result
                .problems
                .iter()
                .find(|problem| problem.starts_with("content filter withheld"))
                .cloned()
                .unwrap_or_else(|| "content filter withheld the post".into()),
        ),
        Outcome::Skipped => Some(
            result
                .problems
                .last()
                .cloned()
                .unwrap_or_else(|| "nothing to post".into()),
        ),
        _ => None,
    }
}

async fn journal(ctx: &JobCtx, id: String, result: &LoreResult) {
    let incoming = match &result.seed {
        Some(seed) => format!(
            "{}{}: {}",
            seed.kind.as_str(),
            seed.source
                .as_deref()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default(),
            seed.title
                .clone()
                .unwrap_or_else(|| crate::backrooms::text::head(&seed.text, 200))
        ),
        None => "nothing came through the wire".into(),
    };
    let mut thinking = Vec::new();
    if let Some(room) = &result.room {
        thinking.push(room.thought.thought.clone());
    }
    if let Some(post) = &result.post {
        thinking.push(post.reasons.clone());
        if !post.continues.is_empty() {
            thinking.push(format!("continues: {}", post.continues));
        }
    }
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(id),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Lore,
            incoming,
            tone: Tone::Neutral,
            recalled: Vec::new(),
            tools: result
                .room
                .iter()
                .map(|room| ToolCall {
                    name: "backrooms".into(),
                    result: format!("{}: {}", room.id, room.thought.premise),
                })
                .collect(),
            thinking: thinking
                .into_iter()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            said: if result.outcome == Outcome::Held {
                String::new()
            } else {
                result.text.clone()
            },
            declined: result.problems.clone(),
            outcome: result.outcome,
            outcome_note: note(result),
        },
    )
    .await;
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
