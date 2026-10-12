//! The lore post: a page from the lab. Something comes through the wire, a
//! room of `LORE_BACKROOMS_TURNS` turns runs on it, the editor keeps one
//! thought, and the thought is written up as a text post. Nothing is drawn.
//!
//! Every stage can decline and the day is then quiet with a reason: no seed,
//! no thought, a post the filter holds. A quiet day is journaled; a wrong
//! post is not recoverable.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::json;
use teeny_agent::deps::Deps;
use teeny_cadence::clock::Clock;
use teeny_memory::Memory;
use teeny_safety::gate::{SafetyDecision, SafetyGate};
use teeny_safety::screen::Direction;
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::Outcome;

use crate::backrooms::distill::Thought;
use crate::backrooms::lore_post::LorePost;
use crate::backrooms::post::publish_own;
use crate::backrooms::seeds::SeedFinder;
use crate::backrooms::tokens::{Random, Rng};
use crate::backrooms::{RoomRunner, RunOptions, Seed};

/// Where the day's seed comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedSource {
    News,
    Forum,
    Memory,
}

impl SeedSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::News => "news",
            Self::Forum => "forum",
            Self::Memory => "memory",
        }
    }
}

/// The room a post came from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoreRoom {
    pub id: String,
    pub thought: Thought,
    pub saved: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoreResult {
    pub seed: Option<Seed>,
    /// Which source and topic the seed came from.
    pub from: Option<(SeedSource, String)>,
    pub room: Option<LoreRoom>,
    pub post: Option<LorePost>,
    /// What went out, or would have. Empty when nothing did.
    pub text: String,
    pub posted_id: Option<String>,
    pub outcome: Outcome,
    pub problems: Vec<String>,
}

impl LoreResult {
    fn quiet(problems: Vec<String>) -> Self {
        Self {
            seed: None,
            from: None,
            room: None,
            post: None,
            text: String::new(),
            posted_id: None,
            outcome: Outcome::Skipped,
            problems,
        }
    }
}

pub struct LorePipeline {
    pub rooms: Arc<dyn RoomRunner>,
    /// The finders, in the order they are tried today.
    pub seeds: Vec<(SeedSource, Arc<dyn SeedFinder>)>,
    /// What the wire is asked for: one is drawn a day.
    pub topics: Vec<String>,
    /// How many turns the room runs. Absent, the backrooms' own setting.
    pub turns: Option<u32>,
    pub deps: Arc<Deps>,
    /// The cadence kind, and the post ledger's: `lore`.
    pub kind: String,
    pub random: Mutex<Rng>,
    pub clock: Arc<dyn Clock>,
}

impl LorePipeline {
    fn memory(&self) -> &Arc<dyn Memory> {
        &self.deps.memory
    }

    fn safety(&self) -> &Arc<SafetyGate> {
        &self.deps.safety
    }

    /// One page. `rehearsal` posts nothing, whatever `DRY_RUN` says.
    pub async fn run(&self, rehearsal: bool) -> anyhow::Result<LoreResult> {
        let now = self.clock.now();
        let mut problems = Vec::new();

        // 1. The seed. One topic drawn for the day, the sources in today's order.
        let topic = {
            let mut random = self.random.lock().unwrap_or_else(|p| p.into_inner());
            let at = random.index(self.topics.len());
            self.topics
                .get(at)
                .cloned()
                .unwrap_or_else(|| "ai agents".into())
        };
        let mut found = None;
        for (source, finder) in &self.seeds {
            match finder.find(&topic).await {
                Err(error) => {
                    problems.push(format!("{} could not be read: {error}", source.as_str()))
                }
                Ok(Some(seed)) => {
                    found = Some((seed, *source));
                    break;
                }
                Ok(None) => {
                    problems.push(format!("{} had nothing on \"{topic}\"", source.as_str()))
                }
            }
        }
        let Some((seed, source)) = found else {
            problems.push("nothing came through the wire; no room was opened".into());
            return Ok(LoreResult::quiet(problems));
        };
        tracing::info!(source = source.as_str(), topic, kind = seed.kind.as_str(), title = ?seed.title, "lore: seed");
        let mut result = LoreResult {
            seed: Some(seed.clone()),
            from: Some((source, topic)),
            ..LoreResult::quiet(Vec::new())
        };

        // 2. The room, and the thought it leaves.
        let options = RunOptions {
            turns: self.turns,
            ..RunOptions::default()
        };
        let room = match self.rooms.open(&seed, &options).await {
            Ok(room) => room,
            Err(error) => {
                problems.push(format!("the room failed: {error}"));
                result.problems = problems;
                return Ok(result);
            }
        };
        problems.extend(
            room.problems
                .iter()
                .map(|problem| format!("the room: {problem}")),
        );
        let Some(thought) = room.thought.clone() else {
            problems.push("the room ran but left no thought".into());
            result.problems = problems;
            return Ok(result);
        };
        tracing::info!(room = %room.run.id, premise = %thought.premise, "lore: thought");
        result.room = Some(LoreRoom {
            id: room.run.id.clone(),
            thought: thought.clone(),
            saved: room.saved.as_ref().map(|saved| saved.markdown.clone()),
        });

        // 3. The post, from the thought.
        let post = match self
            .rooms
            .write_lore(&room.run, &thought, &mut problems)
            .await
        {
            Ok(post) => post,
            Err(error) => {
                problems.push(format!("the thought could not be written up: {error}"));
                result.problems = problems;
                return Ok(result);
            }
        };
        result.post = Some(post.clone());
        let context = HoldContext {
            kind: Some(HoldKind::Draft),
            tool: Some("lore_post".into()),
            ..HoldContext::new(HoldSurface::Lore)
        };
        let outgoing = self.safety().outbound(&post.text, Some(&context)).await;
        if !outgoing.fixes.is_empty() {
            problems.push(format!(
                "the content filter edited the post: {}",
                outgoing.fixes.join("; ")
            ));
        }
        if let SafetyDecision::Held(held) = &outgoing.decision {
            tracing::info!(category = %held.category, by = ?held.by, "lore: post held");
            problems.push(format!(
                "content filter withheld the post: {}",
                held.category
            ));
            result.outcome = Outcome::Held;
            result.problems = problems;
            return Ok(result);
        }
        result.text = outgoing.text;

        // 4. Post, or say what would have been posted.
        if rehearsal || self.deps.settings.dry_run {
            result.outcome = Outcome::DryRun;
            result.problems = problems;
            self.capture(now.timestamp_millis(), &mut result).await;
            return Ok(result);
        }
        let published = publish_own(
            &self.deps,
            &self.kind,
            Direction::Outbound,
            &result.text,
            None,
        )
        .await?;
        tracing::info!(id = %published.id, room = %room.run.id, "lore: posted");
        result.posted_id = Some(published.id.to_string());
        result.outcome = Outcome::Posted;
        result.problems = problems;
        self.capture(now.timestamp_millis(), &mut result).await;
        Ok(result)
    }

    /// What went out, with the room's premise, which is what the next run
    /// reads so a page is not told twice.
    async fn capture(&self, now_ms: i64, result: &mut LoreResult) {
        let id = result
            .posted_id
            .clone()
            .unwrap_or_else(|| format!("dry-run:{now_ms}"));
        let payload = json!({
            "text": result.text,
            "premise": result.room.as_ref().map(|room| room.thought.premise.clone()),
            "objects": result.room.as_ref().map(|room| room.thought.objects.clone()),
            "seed": result.seed.as_ref().map(|seed| json!({ "kind": seed.kind, "title": seed.title, "source": seed.source })),
            "room": result.room.as_ref().map(|room| room.id.clone()),
            "published": result.outcome == Outcome::Posted,
        });
        if let Err(error) = self
            .memory()
            .capture_activity("lore_post", &id, payload, None)
            .await
        {
            result
                .problems
                .push(format!("could not remember the post: {error}"));
        }
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
pub(crate) mod tests;
