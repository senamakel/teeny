//! The backrooms: a room where hot, tool-using agents talk for a while about
//! whatever came through the wire, each from a big and different context.
//!
//! Nothing here posts. A run is a record -- what the seed was, what each
//! agent was told, what was said, what was looked up -- kept so the material
//! can be read, scored, and compared before any of it is cut into content.
//! Each persona is its own agent behind the [`AgentRunner`](crate::runner::AgentRunner)
//! (`AgentKey::Room`, a scheduled turn that runs once), with the `Room` belt:
//! research tools, never a posting one.
//!
//! What came through the wire (news, reddit, 4chan, the Google News feed) is
//! untrusted. It reaches a persona only as quoted material under the
//! legacy framing ("this came through the wire just now:", "the boards, this
//! week. anonymous people saying things. none of it is true and all of it is
//! material."), screened first, and with anything that poses as the room's
//! own system line defused ([`screen::quote_untrusted`]).
//!
//! A port of `legacy/src/backrooms/*`.

pub mod agents;
pub mod ask;
pub mod context;
pub mod distill;
pub mod evaluate;
pub mod gm;
pub mod lore_post;
pub mod post;
pub mod room;
pub mod screen;
pub mod seeds;
pub mod sources;
pub mod store;
pub mod text;
pub mod tokens;
pub mod tones;
pub mod variants;
pub mod wiring;

#[cfg(test)]
pub(crate) mod testing;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use wiring::{Backrooms, RoomDefaults, RoomRunner, RunOptions, RunResult};

/// What kind of thing came through the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeedKind {
    News,
    Memory,
    Mention,
    Forum,
    Text,
}

impl SeedKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::News => "news",
            Self::Memory => "memory",
            Self::Mention => "mention",
            Self::Forum => "forum",
            Self::Text => "text",
        }
    }
}

/// What a room is opened on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seed {
    pub kind: SeedKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
    /// Where it came from: a url, a handle, a board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Seed {
    pub fn text(kind: SeedKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            title: None,
            text: text.into(),
            source: None,
        }
    }

    /// `kind, source` as the prompts name it.
    pub fn provenance(&self) -> String {
        match &self.source {
            Some(source) => format!("{}, {source}", self.kind.as_str()),
            None => self.kind.as_str().to_owned(),
        }
    }

    /// `title — text`, the text cut to `max` characters.
    pub fn headline(&self, max: usize) -> String {
        let text = text::head(&self.text, max);
        match &self.title {
            Some(title) => format!("{title} — {text}"),
            None => text,
        }
    }
}

/// One voice in the room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub id: String,
    /// The prompt marker its lines carry in the transcript, e.g. `swarm>`.
    pub marker: String,
    /// Its standing instructions; the assembled context is appended after.
    pub system: String,
    pub temperature: f64,
    /// The tools it is told it has: the `Room` belt's names.
    pub tools: Vec<String>,
    /// Whether VOICE.md is part of its context. Only teeny speaks in teeny's voice.
    pub voice: bool,
}

/// What a context block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockKind {
    Seed,
    Tone,
    Engine,
    Voice,
    Lore,
    Past,
    Memory,
    Live,
    Forum,
    Tools,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBlock {
    pub kind: BlockKind,
    pub label: String,
    pub text: String,
    pub tokens: usize,
}

/// What one agent was told before the room opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentContext {
    pub agent: AgentSpec,
    pub blocks: Vec<ContextBlock>,
    pub tokens: usize,
    pub tone: String,
    /// Sources that gave nothing, and why. Nothing is invented in their place.
    pub problems: Vec<String>,
}

/// A tool an agent used mid-turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUse {
    pub name: String,
    pub input: serde_json::Value,
    pub summary: String,
}

/// One line of the room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackroomsTurn {
    pub who: String,
    pub text: String,
    /// Set on the turn where the mood shifted, to what.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Set on the turn where the wire was put back in front of the speaker.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wire: bool,
    #[serde(default)]
    pub tools: Vec<ToolUse>,
}

impl BackroomsTurn {
    pub fn said(who: &str, text: &str) -> Self {
        Self {
            who: who.to_owned(),
            text: text.to_owned(),
            tone: None,
            wire: false,
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackroomsSettings {
    pub rng: u32,
    pub turns: u32,
    pub temps: BTreeMap<String, f64>,
    pub budget: usize,
    pub drift_every: u32,
    pub max_tool_calls: u32,
}

/// One room, start to finish.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackroomsRun {
    pub id: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub seed: Seed,
    pub agents: Vec<AgentContext>,
    pub turns: Vec<BackroomsTurn>,
    pub settings: BackroomsSettings,
}

/// Which board a forum post came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Site {
    #[serde(rename = "reddit")]
    Reddit,
    #[serde(rename = "4chan")]
    Fourchan,
}

/// A reddit or 4chan post, whichever board or sub it came from. Untrusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForumPost {
    pub site: Site,
    /// `r/singularity`, `/x/`.
    #[serde(rename = "where")]
    pub place: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
    pub url: String,
    /// Upvotes on reddit, replies on 4chan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}
