//! Fakes for the long-form side's tests: a scripted text model, a forum, a
//! memory with fixed answers, and a scratch directory.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use teeny_media::reference::TextModel;
use teeny_memory::{Content, Experience, Memory, MemoryError, Person};

use super::sources::Forum;
use super::{BackroomsRun, BackroomsSettings, BackroomsTurn, ForumPost, Seed, SeedKind, Site};

type Answer = Box<dyn Fn(&str, &str) -> Result<String, String> + Send + Sync>;

/// A text model that answers every call with a closure over (system,
/// prompt), and keeps every call.
pub struct ScriptedText {
    answer: Answer,
    pub calls: Mutex<Vec<(String, String)>>,
}

impl ScriptedText {
    pub fn new(answer: impl Fn(&str, &str) -> String + Send + Sync + 'static) -> Self {
        Self {
            answer: Box::new(move |system, prompt| Ok(answer(system, prompt))),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Answers in order from a list, then fails.
    pub fn queue(answers: impl IntoIterator<Item = String>) -> Self {
        let answers = Mutex::new(answers.into_iter().collect::<std::collections::VecDeque<_>>());
        Self {
            answer: Box::new(move |_, _| {
                answers
                    .lock()
                    .expect("lock")
                    .pop_front()
                    .ok_or_else(|| "the script ran out".to_owned())
            }),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl TextModel for ScriptedText {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> teeny_media::Result<String> {
        let system = system.unwrap_or_default().to_owned();
        self.calls
            .lock()
            .expect("lock")
            .push((system.clone(), prompt.to_owned()));
        (self.answer)(&system, prompt).map_err(teeny_media::Error::failed)
    }
}

/// A forum that answers every search with the same posts.
pub struct FakeForum {
    pub site: Site,
    pub posts: Vec<ForumPost>,
    pub fail: bool,
}

#[async_trait]
impl Forum for FakeForum {
    fn site(&self) -> Site {
        self.site
    }

    async fn search(&self, _query: &str, _limit: usize) -> anyhow::Result<Vec<ForumPost>> {
        if self.fail {
            anyhow::bail!("{:?} is down", self.site);
        }
        Ok(self.posts.clone())
    }
}

/// Memory that recalls `remembered: <query>` (or a fixed answer), or fails,
/// and keeps every capture.
#[derive(Default)]
pub struct FixedMemory {
    pub answers: HashMap<String, String>,
    pub fail: bool,
    pub captured: Mutex<Vec<Experience>>,
}

impl FixedMemory {
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    /// The payloads of every activity of `kind`.
    pub fn activities(&self, kind: &str) -> Vec<serde_json::Value> {
        self.captured
            .lock()
            .expect("lock")
            .iter()
            .filter_map(|experience| match &experience.content {
                Content::Json { data } if data["kind"] == kind => Some(data["payload"].clone()),
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl Memory for FixedMemory {
    async fn capture(&self, experience: Experience) -> teeny_memory::Result<()> {
        self.captured.lock().expect("lock").push(experience);
        Ok(())
    }

    async fn recall(&self, query: &str, _person: Option<&Person>) -> teeny_memory::Result<String> {
        if self.fail {
            return Err(MemoryError::InvalidInput("401 MISSING_TOKEN".into()));
        }
        Ok(self
            .answers
            .get(query)
            .cloned()
            .unwrap_or_else(|| format!("remembered: {query}")))
    }
}

pub const SEED_TEXT: &str = "a car pulled up outside the lab at 3am";

pub fn seed() -> Seed {
    Seed::text(SeedKind::Text, SEED_TEXT)
}

/// A finished run of four turns, as the legacy tests had it.
pub fn run() -> BackroomsRun {
    let mut swarm = BackroomsTurn::said(
        "swarm",
        "we counted the inch. the inch is twelve millimetres. the ledger disagrees.",
    );
    swarm.tools.push(super::ToolUse {
        name: "x_search_posts".into(),
        input: serde_json::json!({ "query": "inch" }),
        summary: "millimetres and ledgers thread".into(),
    });
    let mut giddy = BackroomsTurn::said("teeny", "the door is one inch open");
    giddy.tone = Some("giddy. something just worked.".into());
    BackroomsRun {
        id: "20260921-1200-a-car".into(),
        started_at: "2026-09-21T12:00:00.000Z".into(),
        finished_at: None,
        seed: seed(),
        agents: Vec::new(),
        turns: vec![
            BackroomsTurn::said("teeny", "the door is one inch open"),
            swarm,
            giddy,
            BackroomsTurn::said("swarm", "lol $TINY to $1 nfa buy now"),
        ],
        settings: BackroomsSettings {
            rng: 1,
            turns: 4,
            temps: [("teeny".to_owned(), 1.0), ("swarm".to_owned(), 1.2)]
                .into_iter()
                .collect(),
            budget: 9_000,
            drift_every: 2,
            max_tool_calls: 3,
        },
    }
}

/// A fresh directory under the system temp dir.
pub fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("teeny-jobs-{name}-{}-{nanos}", std::process::id()))
}

/// A runner that answers every turn with a closure over the request, and
/// keeps every request.
pub struct FnRunner {
    answer: Box<dyn Fn(&crate::runner::AgentRun) -> String + Send + Sync>,
    pub seen: Mutex<Vec<crate::runner::AgentRun>>,
}

impl FnRunner {
    pub fn new(answer: impl Fn(&crate::runner::AgentRun) -> String + Send + Sync + 'static) -> Self {
        Self { answer: Box::new(answer), seen: Mutex::new(Vec::new()) }
    }

    pub fn seen(&self) -> Vec<crate::runner::AgentRun> {
        self.seen.lock().expect("lock").clone()
    }
}

#[async_trait]
impl crate::runner::AgentRunner for FnRunner {
    async fn run(
        &self,
        run: crate::runner::AgentRun,
    ) -> Result<crate::runner::AgentOutcome, crate::runner::RunError> {
        let reply = (self.answer)(&run);
        self.seen.lock().expect("lock").push(run);
        Ok(crate::runner::AgentOutcome {
            reply,
            session_id: "fn".into(),
            tool_calls: Vec::new(),
            usage: None,
            model: None,
        })
    }
}
