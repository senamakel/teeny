//! The article job: one article per run on its evening days, with the
//! serial's guards and one more: the last published article's time is read
//! from the store, so a restart near the cron minute cannot publish twice
//! inside `ARTICLE_MIN_INTERVAL_HOURS`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use chrono::TimeDelta;
use teeny_cadence::clock::parse_iso;
use teeny_store::journal::{Outcome, ReflectionInput, Tone, ToolCall, Trigger};

use super::pipeline::{ArticlePipeline, ArticleResult};
use super::proofread::load_humanizer;
use super::stages::render_structure;
use super::store::FileArticleStore;
use crate::backrooms::Backrooms;
use crate::backrooms::seeds::{ForumSeeds, NewsSeeds, SeedFinder};
use crate::backrooms::sources::{NewsFeed, WebSearch};
use crate::backrooms::tokens::Rng;
use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::record_quietly;
use crate::morning::job::{Gates, admit, day_id, journal_failure};
use crate::narrative::state::FileNarrativeState;
use crate::narrative::voice::parse_lore;

pub const JOB: &str = "article";

/// The scheduled run: the interval (in memory and against the store), the
/// cadence, the pipeline, the journal.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let now = ctx.now();
    let minimum =
        TimeDelta::milliseconds((ctx.config.article.min_interval_hours * 3_600_000.0) as i64);
    let articles = FileArticleStore::new(&ctx.config.article.dir);
    if let Some(published) = articles
        .latest_published_at()
        .await
        .ok()
        .flatten()
        .and_then(|at| parse_iso(&at))
        && now - published < minimum
    {
        return Ok(JobReport::declined(
            JOB,
            format!("{JOB} job: skipping, an article was published recently"),
        ));
    }
    if let Err(why) = admit(
        ctx,
        Gates {
            job: JOB,
            kind: Some(JOB),
            rota: None,
            force: false,
        },
    )
    .await
    {
        tracing::info!("{why}");
        return Ok(JobReport::declined(JOB, why));
    }
    let pipeline = match build(ctx) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            journal_failure(
                ctx,
                day_id(ctx, JOB),
                Trigger::Article,
                "an article",
                &error,
            )
            .await;
            return Err(error);
        }
    };
    run_pipeline(ctx, &pipeline, false).await
}

/// The rehearsal (`article-dry-run`): no gates, nothing to X, a `dry-run`
/// record in the store; journaled; the title and dek in the report.
pub async fn dry_run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let pipeline = build(ctx)?;
    run_pipeline(ctx, &pipeline, true).await
}

/// The production pipeline. Pictures are optional: an article without them
/// is still an article, and the run says so.
pub fn build(ctx: &JobCtx) -> anyhow::Result<ArticlePipeline> {
    let config = &ctx.config;
    let deps = &ctx.deps;
    let backrooms = Arc::new(Backrooms::from_ctx(ctx));
    let lore_text =
        std::fs::read_to_string(&config.narrative.lore_path).context("the serial's LORE.md")?;
    let humanizer = load_humanizer(&PathBuf::from(&config.article.humanizer_path))
        .context("the humanizer skill")?;
    let web: Option<Arc<dyn WebSearch>> = deps.exa.clone().map(|exa| exa as Arc<dyn WebSearch>);
    let seed: Arc<dyn SeedFinder> = Arc::new(NewsThenForum {
        news: NewsSeeds {
            web,
            feed: NewsFeed::new(),
            random: Mutex::new(Rng::new(Rng::seed_from_time())),
        },
        forum: ForumSeeds {
            forums: backrooms.forums().to_vec(),
            random: Mutex::new(Rng::new(Rng::seed_from_time())),
        },
    });
    // The editor reads the whole piece and the flow pass rewrites all of it
    // on a reasoning model: at a caption's deadline both were cut off and the
    // article went out unchecked and unsmoothed.
    let timeout = std::time::Duration::from_secs(u64::from(config.article.model_timeout_seconds));
    let model = |source: &str, temperature: f32| {
        Arc::new(
            deps.llm
                .clone()
                .with_source(source)
                .with_temperature(temperature)
                .with_timeout(timeout),
        )
    };
    Ok(ArticlePipeline {
        rooms: backrooms,
        seed,
        topics: config.article.news_topics.clone(),
        lore: parse_lore(&lore_text),
        lore_text,
        state: Arc::new(FileNarrativeState {
            path: PathBuf::from(&config.narrative.state_path),
        }),
        writer: model("article:writer", config.article.writer_temperature as f32),
        editor: model("article:editor", 0.2),
        fixer: model("article:proof-fix", 0.4),
        humanizer,
        deps: Arc::clone(deps),
        media: deps.media.clone(),
        articles: FileArticleStore::new(&config.article.dir),
        room_count: config.article.rooms,
        sections_max: config.article.sections_max,
        words: config.article.words,
        publish: config.article.publish,
        structure_only: false,
        no_images: false,
        no_proof: false,
        no_flow: false,
        from: None,
        topic: None,
        engine: None,
        seed_text: None,
        brief: None,
        random: Mutex::new(Rng::new(Rng::seed_from_time())),
        clock: Arc::clone(&ctx.clock),
    })
}

/// The news first, then the boards.
struct NewsThenForum {
    news: NewsSeeds,
    forum: ForumSeeds,
}

#[async_trait::async_trait]
impl SeedFinder for NewsThenForum {
    async fn find(&self, topic: &str) -> anyhow::Result<Option<crate::backrooms::Seed>> {
        if let Some(seed) = self.news.find(topic).await? {
            return Ok(Some(seed));
        }
        self.forum.find(topic).await
    }
}

pub async fn run_pipeline(
    ctx: &JobCtx,
    pipeline: &ArticlePipeline,
    rehearsal: bool,
) -> anyhow::Result<JobReport> {
    let id = if rehearsal {
        format!("{}:dry-run", day_id(ctx, JOB))
    } else {
        day_id(ctx, JOB)
    };
    let result = match pipeline.run(rehearsal).await {
        Ok(result) => result,
        Err(error) => {
            journal_failure(ctx, id, Trigger::Article, "an article", &error).await;
            return Err(error).context("an article");
        }
    };
    tracing::info!(outcome = ?result.outcome, slug = ?result.slug, words = result.word_count, problems = ?result.problems, post = ?result.post_id, "article job: done");
    journal(ctx, id, &result).await;
    let mut report = JobReport::ran(JOB, result.outcome);
    report.posted_id = result.post_id.clone();
    report.drafts = result
        .structure
        .iter()
        .map(|s| format!("{}\n{}", s.title, s.dek))
        .collect();
    report.problems = result.problems.clone();
    report.note = note(&result);
    Ok(report)
}

fn note(result: &ArticleResult) -> Option<String> {
    match result.outcome {
        Outcome::Skipped => Some(
            result
                .problems
                .last()
                .cloned()
                .unwrap_or_else(|| "nothing to publish".into()),
        ),
        Outcome::Held => Some(
            result
                .problems
                .iter()
                .find(|p| p.starts_with("content filter withheld"))
                .cloned()
                .unwrap_or_else(|| "content filter withheld the article".into()),
        ),
        Outcome::DryRun => Some(format!(
            "rehearsal saved as {}",
            result.slug.as_deref().unwrap_or("?")
        )),
        _ => None,
    }
}

async fn journal(ctx: &JobCtx, id: String, result: &ArticleResult) {
    let mut thinking: Vec<String> = result
        .rooms
        .iter()
        .map(|room| {
            format!(
                "room {}{}: {}",
                room.index + 1,
                room.run_id
                    .as_deref()
                    .map(|id| format!(" {id}"))
                    .unwrap_or_default(),
                room.rejected.clone().unwrap_or_else(|| format!(
                    "scored {}",
                    room.total.map_or("?".into(), |t| t.to_string())
                ))
            )
        })
        .collect();
    if let Some(thought) = &result.thought {
        thinking.extend([String::new(), format!("premise: {}", thought.premise)]);
    }
    if let Some(structure) = &result.structure {
        thinking.extend([
            String::new(),
            "the plan:".into(),
            render_structure(structure),
        ]);
    }
    if let Some(check) = &result.structure_check {
        thinking.extend([
            String::new(),
            format!(
                "the editor: {}",
                if check.ok {
                    "ok".into()
                } else {
                    check.problems.join("; ")
                }
            ),
        ]);
    }
    if let Some(proof) = &result.proof {
        thinking.extend([
            String::new(),
            format!(
                "the proof-reader: {:?}, {} finding(s), fix {}",
                proof.review.verdict,
                proof.review.findings.len(),
                if proof.applied {
                    "applied"
                } else {
                    "not applied"
                }
            )
            .to_lowercase(),
        ]);
    }
    let mut tools = Vec::new();
    if let Some(cover) = &result.cover {
        tools.push(ToolCall {
            name: "generate_image".into(),
            result: cover.id.clone(),
        });
    }
    tools.extend(result.images.iter().map(|(_, media)| ToolCall {
        name: "generate_image".into(),
        result: media.id.clone(),
    }));
    if let Some(post) = &result.post_id {
        tools.push(ToolCall {
            name: "article_publish".into(),
            result: post.clone(),
        });
    }
    record_quietly(
        ctx.journal.as_ref(),
        ReflectionInput {
            id: Some(id),
            at: Some(teeny_store::iso(ctx.now())),
            trigger: Trigger::Article,
            incoming: format!(
                "an article: {} room(s){}{}",
                result.rooms.len(),
                result
                    .engine
                    .as_deref()
                    .map(|e| format!(" about {}", e.split(':').next().unwrap_or(e)))
                    .unwrap_or_default(),
                result
                    .seed
                    .as_ref()
                    .and_then(|s| s.title.as_deref())
                    .map(|t| format!(", seeded by \"{t}\""))
                    .unwrap_or_default()
            ),
            tone: Tone::Neutral,
            recalled: result.thought.iter().map(|t| t.thought.clone()).collect(),
            tools,
            thinking: thinking.join("\n"),
            said: result
                .structure
                .as_ref()
                .map(|s| format!("{}\n{}", s.title, s.dek))
                .unwrap_or_default(),
            declined: result.problems.clone(),
            outcome: result.outcome,
            outcome_note: note(result),
        },
    )
    .await;
}
