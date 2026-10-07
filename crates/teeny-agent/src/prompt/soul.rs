//! Who the account is: `SOUL.md` and `CONTEXT.md`, a port of
//! `legacy/src/agent/soul.ts`.
//!
//! Skills are no longer read here. The legacy pasted every `skills/` document
//! into the system prompt; OpenHuman loads them itself through `skills_dir`,
//! so the persona is the soul and the facts and nothing else.

use std::io::ErrorKind;
use std::path::Path;

use teeny_core::config::AgentConfig;

/// The persona documents, each absent when its file is missing or blank.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Persona {
    /// `SOUL.md`: who the account is and how it talks.
    pub soul: Option<String>,
    /// `CONTEXT.md`: the facts the account may treat as known.
    pub context: Option<String>,
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
        })
    }

    /// `SOUL_PATH` and `CONTEXT_PATH`.
    pub fn from_config(agent: &AgentConfig) -> std::io::Result<Self> {
        Self::load(&agent.soul_path, &agent.context_path)
    }
}

/// The persona as the prompt opens with it; empty for an empty persona.
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
    sections.join("\n")
}
