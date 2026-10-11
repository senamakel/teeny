//! A room, opened and read: the contexts built, one [`RunnerAgent`] per
//! persona, the turns, the counts, the judge, the editor's thought, and the
//! post written from it. Built from a [`JobCtx`] in production; every part
//! can be swapped in a test.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use teeny_agent::prompt::render_persona;
use teeny_cadence::clock::{Clock, iso};
use teeny_media::reference::TextModel;

use super::agents::{agent_presets, pick_agents};
use super::context::{ContextBuilder, ContextSources};
use super::distill::{Thought, distill};
use super::evaluate::{Craziness, RunStats, judge, stats};
use super::gm::{GmAgain, GmPost, GmSources, gather_gm_material, gm};
use super::lore_post::{LORE_HISTORY_QUERY, LorePost, write_lore};
use super::room::{RunnerAgent, Seat, run_backrooms};
use super::sources::{Forum, FourchanForum, Pacer, RedditForum, WebSearch};
use super::store::{Saved, SavedExtras, run_id, save};
use super::text::head;
use super::tokens::{Random, Rng};
use super::{AgentContext, BackroomsRun, BackroomsSettings, Seed};
use crate::ctx::JobCtx;
use crate::runner::AgentRunner;

/// What a room is opened with, when the caller does not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomDefaults {
    /// `BACKROOMS_AGENTS`.
    pub agents: Vec<String>,
    /// `BACKROOMS_TURNS`.
    pub turns: u32,
    /// `BACKROOMS_BUDGET_TOKENS`.
    pub budget: usize,
    /// `BACKROOMS_TOOL_CALLS`: zero takes the tools away.
    pub max_tool_calls: u32,
}

/// How one room is opened.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    pub agents: Option<Vec<String>>,
    pub turns: Option<u32>,
    pub rng: Option<u32>,
    pub budget: Option<usize>,
    pub drift_every: Option<u32>,
    /// What tonight is about, one of the lore's engines; drawn when absent.
    pub engine: Option<String>,
    /// Whether the cold judge reads the room.
    pub score: bool,
    /// Whether the editor reads it down to one thought. On by default.
    pub distill: bool,
    /// Whether the thought is written up as the good-morning post.
    pub gm: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            agents: None,
            turns: None,
            rng: None,
            budget: None,
            drift_every: None,
            engine: None,
            score: false,
            distill: true,
            gm: false,
        }
    }
}

/// What a room left.
#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub run: BackroomsRun,
    pub stats: RunStats,
    pub judge: Option<Craziness>,
    pub thought: Option<Thought>,
    pub gm: Option<GmPost>,
    /// Sources the post could not read; it was written without them.
    pub problems: Vec<String>,
    /// Where the notebook was written, when there is one.
    pub saved: Option<Saved>,
}

/// What the lore and article pipelines need of the backrooms.
#[async_trait]
pub trait RoomRunner: Send + Sync {
    async fn open(&self, seed: &Seed, options: &RunOptions) -> anyhow::Result<RunResult>;
    async fn write_lore(
        &self,
        run: &BackroomsRun,
        thought: &Thought,
        problems: &mut Vec<String>,
    ) -> anyhow::Result<LorePost>;
}

pub struct Backrooms {
    runner: Arc<dyn AgentRunner>,
    builder: ContextBuilder,
    presets: BTreeMap<String, AgentSpec>,
    editor: Arc<dyn TextModel>,
    judge: Arc<dyn TextModel>,
    editor_prompt: Option<String>,
    store_dir: Option<PathBuf>,
    defaults: RoomDefaults,
    gm_sources: GmSources,
    clock: Arc<dyn Clock>,
}

use super::AgentSpec;

/// How the personas are told of the browser; the name the belt gives it.
const BROWSER_TOOL: &str =
    "browser (open a page on an allowed site and read it: never log in, type or post)";

impl Backrooms {
    pub fn new(
        runner: Arc<dyn AgentRunner>,
        sources: ContextSources,
        editor: Arc<dyn TextModel>,
        judge: Arc<dyn TextModel>,
        defaults: RoomDefaults,
        gm_sources: GmSources,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            presets: agent_presets(&sources.bot_username),
            builder: ContextBuilder::new(sources),
            runner,
            editor,
            judge,
            editor_prompt: None,
            store_dir: None,
            defaults,
            gm_sources,
            clock,
        }
    }

    /// The editor's standing instructions, from `narrative/EDITOR.md`.
    pub fn with_editor_prompt(mut self, prompt: Option<String>) -> Self {
        self.editor_prompt = prompt;
        self
    }

    /// Whether the personas hold OpenHuman's browser (`BROWSER_RESEARCH`):
    /// each one with tools is told it has it, so it is used.
    pub fn with_browser(mut self, browse: bool) -> Self {
        if browse {
            for spec in self.presets.values_mut() {
                if !spec.tools.is_empty() && !spec.tools.iter().any(|tool| tool == BROWSER_TOOL) {
                    spec.tools.push(BROWSER_TOOL.to_owned());
                }
            }
        }
        self
    }

    /// Where runs are written (`BACKROOMS_DIR`).
    pub fn with_store_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.store_dir = dir;
        self
    }

    /// The production rooms: the clients in `ctx.deps`, the files and
    /// settings in `ctx.config`, the personas behind `ctx.runner`.
    pub fn from_ctx(ctx: &JobCtx) -> Self {
        let config = &ctx.config;
        let deps = &ctx.deps;
        let read = |path: &str| {
            std::fs::read_to_string(path).unwrap_or_else(|error| {
                tracing::warn!(%error, path, "backrooms: could not read a lore file; going without it");
                String::new()
            })
        };
        let voice = read(&config.narrative.voice_path);
        let lore = read(&config.narrative.lore_path);
        let web: Option<Arc<dyn WebSearch>> = deps.exa.clone().map(|exa| exa as Arc<dyn WebSearch>);
        let pacer = Arc::new(Pacer::new(Duration::from_millis(1_100)));
        let forums: Vec<Arc<dyn Forum>> = vec![
            Arc::new(RedditForum::new(
                config.backrooms.reddit_subs.clone(),
                web.clone(),
                Arc::clone(&pacer),
            )),
            Arc::new(FourchanForum::new(
                config.backrooms.fourchan_boards.clone(),
                pacer,
            )),
        ];
        let persona = render_persona(&deps.persona);
        let gm_sources = GmSources {
            memory: Some(Arc::clone(&deps.memory)),
            x: Some(Arc::clone(&deps.x_reads)),
            persona: if persona.is_empty() {
                voice.clone()
            } else {
                persona
            },
            lore: lore.clone(),
            bot_username: deps.settings.bot_username.clone(),
            max_emoji: deps.settings.max_emoji,
            history_query: None,
        };
        let editor_prompt = std::fs::read_to_string(&config.backrooms.editor_path)
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        Self::new(
            Arc::clone(&ctx.runner),
            ContextSources {
                memory: Some(Arc::clone(&deps.memory)),
                x: Some(Arc::clone(&deps.x_reads)),
                web,
                forums,
                voice,
                lore,
                bot_username: deps.settings.bot_username.clone(),
            },
            // Warmer than the judge: the thought is written, not classified.
            Arc::new(
                deps.llm
                    .clone()
                    .with_source("backrooms:editor")
                    .with_temperature(0.5),
            ),
            Arc::new(
                deps.llm
                    .clone()
                    .with_source("backrooms:judge")
                    .with_temperature(0.2),
            ),
            RoomDefaults {
                agents: config.backrooms.agents.clone(),
                turns: config.backrooms.turns,
                budget: config.backrooms.budget_tokens as usize,
                max_tool_calls: config.backrooms.tool_calls,
            },
            gm_sources,
            Arc::clone(&ctx.clock),
        )
        .with_editor_prompt(editor_prompt)
        .with_browser(config.openhuman.browser_research)
        .with_store_dir(Some(PathBuf::from(&config.backrooms.dir)))
    }

    pub fn forums(&self) -> &[Arc<dyn Forum>] {
        &self.builder.sources().forums
    }

    pub fn sources(&self) -> &ContextSources {
        self.builder.sources()
    }

    /// Opens a room on `seed` and reads it.
    pub async fn run(&self, seed: &Seed, options: &RunOptions) -> anyhow::Result<RunResult> {
        let names = options
            .agents
            .clone()
            .unwrap_or_else(|| self.defaults.agents.clone());
        let specs = pick_agents(&names, &self.presets)?;
        let settings = BackroomsSettings {
            rng: options.rng.unwrap_or_else(Rng::seed_from_time),
            turns: options.turns.unwrap_or(self.defaults.turns),
            temps: specs
                .iter()
                .map(|spec| (spec.id.clone(), spec.temperature))
                .collect(),
            budget: options.budget.unwrap_or(self.defaults.budget),
            drift_every: options.drift_every.unwrap_or(4),
            max_tool_calls: self.defaults.max_tool_calls,
        };
        let now = self.clock.now();
        // The rng goes into the id: two rooms on one seed in one minute must not share a name.
        let slug = seed.title.clone().unwrap_or_else(|| head(&seed.text, 40));
        let id = format!("{}-{}", run_id(now, &slug), base36(settings.rng));
        let mut random = Rng::new(settings.rng);
        let mut contexts: Vec<AgentContext> = Vec::new();
        for spec in &specs {
            // One draw per agent, in order, so the same rng gives the same contexts.
            let mut draw = Rng::new((random.next_f64() * 2f64.powi(31)) as u32);
            let tools: &[String] = if settings.max_tool_calls > 0 {
                &spec.tools
            } else {
                &[]
            };
            let context = self
                .builder
                .build(
                    spec,
                    seed,
                    &mut draw,
                    settings.budget,
                    tools,
                    options.engine.as_deref(),
                )
                .await;
            tracing::info!(agent = %spec.id, tokens = context.tokens, problems = ?context.problems, "backrooms: context");
            contexts.push(context);
        }
        let agents: Vec<RunnerAgent> = contexts
            .iter()
            .map(|context| RunnerAgent::new(Arc::clone(&self.runner), &id, context))
            .collect();
        let seats: Vec<Seat<'_>> = specs
            .iter()
            .zip(&agents)
            .zip(&contexts)
            .map(|((spec, agent), context)| Seat {
                spec,
                agent,
                context,
            })
            .collect();
        let turns = run_backrooms(
            &seats,
            seed,
            settings.turns,
            &mut random,
            settings.drift_every,
        )
        .await?;
        let mut run = BackroomsRun {
            id,
            started_at: iso(now),
            finished_at: None,
            seed: seed.clone(),
            agents: contexts.clone(),
            turns,
            settings,
        };
        run.finished_at = Some(iso(self.clock.now()));
        let counted = stats(&run);
        let mut verdict = None;
        if options.score {
            match judge(&run, self.judge.as_ref()).await {
                Ok(read) => verdict = Some(read),
                Err(error) => tracing::info!(%error, "backrooms: the judge failed"),
            }
        }
        let mut thought = None;
        if options.distill {
            match self.distill(&run).await {
                Ok(kept) => thought = Some(kept),
                Err(error) => tracing::info!(%error, "backrooms: distill failed"),
            }
        }
        let mut problems = Vec::new();
        let mut morning = None;
        if options.gm
            && let Some(thought) = &thought
        {
            match self
                .gm(&run, thought, &mut problems, &GmAgain::default())
                .await
            {
                Ok(post) => morning = Some(post),
                Err(error) => tracing::info!(%error, "backrooms: gm failed"),
            }
        }
        let extras = SavedExtras {
            stats: Some(counted.clone()),
            judge: verdict.clone(),
            thought: thought.clone(),
            gm: morning.clone(),
        };
        let saved = self
            .store_dir
            .as_ref()
            .and_then(|dir| match save(dir, &run, &extras) {
                Ok(saved) => Some(saved),
                Err(error) => {
                    tracing::warn!(%error, "backrooms: could not write the run");
                    None
                }
            });
        Ok(RunResult {
            run,
            stats: counted,
            judge: verdict,
            thought,
            gm: morning,
            problems,
            saved,
        })
    }

    /// The editor's one thought, for a run already on disk.
    pub async fn distill(&self, run: &BackroomsRun) -> anyhow::Result<Thought> {
        distill(run, self.editor.as_ref(), self.editor_prompt.as_deref()).await
    }

    /// The good-morning post from a thought.
    pub async fn gm(
        &self,
        run: &BackroomsRun,
        thought: &Thought,
        problems: &mut Vec<String>,
        again: &GmAgain,
    ) -> anyhow::Result<GmPost> {
        let material =
            gather_gm_material(&self.gm_sources, self.clock.now().date_naive(), problems).await;
        gm(run, thought, &material, self.editor.as_ref(), again).await
    }

    /// The afternoon lore post from a thought.
    pub async fn lore(
        &self,
        run: &BackroomsRun,
        thought: &Thought,
        problems: &mut Vec<String>,
    ) -> anyhow::Result<LorePost> {
        let sources = GmSources {
            history_query: Some(LORE_HISTORY_QUERY.into()),
            ..self.gm_sources.clone()
        };
        let material = gather_gm_material(&sources, self.clock.now().date_naive(), problems).await;
        write_lore(run, thought, &material, self.editor.as_ref()).await
    }
}

#[async_trait]
impl RoomRunner for Backrooms {
    async fn open(&self, seed: &Seed, options: &RunOptions) -> anyhow::Result<RunResult> {
        self.run(seed, options).await
    }

    async fn write_lore(
        &self,
        run: &BackroomsRun,
        thought: &Thought,
        problems: &mut Vec<String>,
    ) -> anyhow::Result<LorePost> {
        self.lore(run, thought, problems).await
    }
}

fn base36(mut value: u32) -> String {
    if value == 0 {
        return "0".into();
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push(char::from_digit(value % 36, 36).unwrap_or('0'));
        value /= 36;
    }
    digits.iter().rev().collect()
}

#[cfg(test)]
#[path = "wiring_tests.rs"]
mod tests;
