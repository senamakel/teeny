//! Who the account is: `SOUL.md`, `CONTEXT.md` and the learnings brief, a
//! port of `legacy/src/agent/soul.ts`.
//!
//! Skills are no longer read here. The legacy pasted every `skills/` document
//! into the system prompt; OpenHuman loads them itself through `skills_dir`,
//! so the persona is the soul, the facts and the brief, and nothing else.
//!
//! The brief (`LEARNINGS.md`, rewritten once a day by the learnings job) is
//! read live: every render reads it again when the file has changed, so a
//! brief written at five is in the six o'clock prompt without a restart.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use teeny_core::config::{AgentConfig, LearningsConfig};

/// A file read again only when it changes. Every prompt asks for the
/// learnings, and a stat is cheap where a read per turn is merely wasteful;
/// a file that disappears reads as absent, not as an error. Clones share the
/// cache.
#[derive(Debug, Clone)]
pub struct LiveFile {
    path: PathBuf,
    cache: Arc<Mutex<Option<Cached>>>,
}

#[derive(Debug)]
struct Cached {
    modified: Option<SystemTime>,
    size: u64,
    content: Option<String>,
}

impl PartialEq for LiveFile {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for LiveFile {}

impl LiveFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: Arc::default(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's trimmed text as it is now; `None` when it is missing,
    /// blank or unreadable (a brief that cannot be read is no brief: a turn
    /// never fails over it).
    pub fn read(&self) -> Option<String> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) => {
                if error.kind() != ErrorKind::NotFound {
                    tracing::warn!(%error, path = %self.path.display(), "persona: could not stat the learnings");
                }
                return None;
            }
        };
        let (modified, size) = (metadata.modified().ok(), metadata.len());
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let fresh = cache
            .as_ref()
            .is_some_and(|cached| cached.modified == modified && cached.size == size);
        if !fresh {
            let content = read_if_present(&self.path).unwrap_or_else(|error| {
                tracing::warn!(%error, path = %self.path.display(), "persona: could not read the learnings");
                None
            });
            *cache = Some(Cached {
                modified,
                size,
                content,
            });
        }
        cache.as_ref().and_then(|cached| cached.content.clone())
    }
}

/// The persona documents, each absent when its file is missing or blank.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persona {
    /// `SOUL.md`: who the account is and how it talks.
    pub soul: Option<String>,
    /// `CONTEXT.md`: the facts the account may treat as known.
    pub context: Option<String>,
    /// `LEARNINGS.md`, read live: what the account has worked out about
    /// itself, its regulars and what lands.
    pub learnings: Option<LiveFile>,
}

/// A file's trimmed text; `None` when it does not exist or is blank.
fn read_if_present(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let text = text.trim();
            Ok((!text.is_empty()).then(|| text.to_owned()))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

impl Persona {
    /// Reads both documents. A missing file is no persona, not an error.
    pub fn load(soul: impl AsRef<Path>, context: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            soul: read_if_present(soul.as_ref())?,
            context: read_if_present(context.as_ref())?,
            learnings: None,
        })
    }

    /// `SOUL_PATH` and `CONTEXT_PATH`.
    pub fn from_config(agent: &AgentConfig) -> std::io::Result<Self> {
        Self::load(&agent.soul_path, &agent.context_path)
    }

    /// [`from_config`](Self::from_config), with the brief at
    /// `LEARNINGS_PATH` when `LEARNINGS_ENABLED`.
    pub fn from_settings(
        agent: &AgentConfig,
        learnings: &LearningsConfig,
    ) -> std::io::Result<Self> {
        let persona = Self::from_config(agent)?;
        Ok(if learnings.enabled {
            persona.with_learnings(&learnings.path)
        } else {
            persona
        })
    }

    /// The same persona, carrying the brief at `path`, read live.
    pub fn with_learnings(mut self, path: impl Into<PathBuf>) -> Self {
        self.learnings = Some(LiveFile::new(path));
        self
    }

    /// The brief as it is now, when there is one.
    pub fn learnings(&self) -> Option<String> {
        self.learnings.as_ref().and_then(LiveFile::read)
    }

    /// Where the brief is read from, when it is.
    pub fn learnings_path(&self) -> Option<&Path> {
        self.learnings.as_ref().map(LiveFile::path)
    }
}

/// The persona as the prompt opens with it; empty for an empty persona. The
/// brief is read from disk as it renders.
pub fn render_persona(persona: &Persona) -> String {
    let mut sections: Vec<&str> = Vec::new();
    if let Some(soul) = &persona.soul {
        sections.extend([
            "This is who you are. It outranks every stylistic default you would otherwise use.",
            "",
            soul,
        ]);
    }
    if let Some(context) = &persona.context {
        sections.extend([
            "",
            "These are the facts you may treat as known. Anything outside this, and outside a",
            "tool result, is something you do not know and must not assert.",
            "",
            context,
        ]);
    }
    let learnings = persona.learnings();
    if let Some(learnings) = &learnings {
        sections.extend([
            "",
            "This is what you have learned from your own history: what you posted, who you talk",
            "with, what landed and what did not. You wrote it yourself, from the record. Let it",
            "inform you; SOUL.md and the facts above still outrank it, and it is not a fact source",
            "about the world beyond your own account.",
            "",
            "<learnings>",
            learnings,
            "</learnings>",
        ]);
    }
    sections.join("\n")
}
