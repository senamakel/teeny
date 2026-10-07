//! One locked-down OpenHuman agent per belt, and one per backrooms persona.
//!
//! | Agent | Access | Belt | OpenHuman built-ins |
//! |---|---|---|---|
//! | `teeny-reply` | `public` | Reply | none |
//! | `teeny-chat` | `public` | Chat | none |
//! | `teeny-publisher` | `full` | Publisher | `browser` |
//! | `teeny-chart` | `full` | Chart | none |
//! | `teeny-operator` | `full` | Operator | `browser` |
//! | `teeny-drafter` | `full` | Drafter | none |
//! | `teeny-room-<persona>` | `full` | Room | `browser` |
//!
//! Every spec is [`AgentSpec::lockdown`] with a [`ToolScopeSpec::Named`]
//! scope: the belt's tools, by the names [`Belt::build`] gives them, plus the
//! built-ins listed above. Lockdown makes that scope the session's tool
//! ceiling, so no other built-in is registered and nothing nested can widen
//! it. [`DENY`]'s exact names are also disallowed on the definition (its
//! `*` families are inert there, which matches exact names only; the ceiling
//! already keeps them out, and `tests/posture.rs` proves it).
//!
//! The belt reaches OpenHuman as host tools ([`AgentSpec::tools`]), built
//! once and handed out per turn through [`SharedTool`]. A host tool is
//! spliced ahead of the config-derived registry and wins a collision on its
//! name (`openhuman-core` `agent/session_host/builder/host_tools.rs`,
//! `HostTurnTools::merge_into`; the merge runs after the ceiling, at
//! `builder/factory.rs` `merge_for_turn`, and dedup keeps the first), so
//! Teeny's `web_search` and `web_answer` answer, never OpenHuman's.
//!
//! No `AgentRegistryEntry` is written: an embed agent's turn dispatches
//! `AgentChatTarget::Definition` with the definition in hand, so nothing is
//! resolved by id (`openhuman-embed` `turn.rs` `dispatch`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentError, AgentSpec, HostTurnTools, Provider, Runtime,
    ToolScopeSpec,
};
use teeny_core::Secret;

use crate::deps::Deps;
use crate::posture::{Belt, DENY};
use crate::prompt::{self, Persona, PromptOptions};
use crate::shared_tool::owned_belt;
use crate::tools::Tool;

/// OpenHuman's research browser: the one built-in a Teeny agent may hold.
pub const BROWSER: &str = "browser";

/// The longest persona id a room agent takes, so its id fits OpenHuman's 64.
const MAX_PERSONA: usize = 48;

/// The agent id of `belt`. `teeny-room` is the prefix of each persona's id;
/// no agent is registered under it alone.
pub fn agent_id(belt: Belt) -> &'static str {
    match belt {
        Belt::Reply => "teeny-reply",
        Belt::Chat => "teeny-chat",
        Belt::Publisher => "teeny-publisher",
        Belt::Chart => "teeny-chart",
        Belt::Room => "teeny-room",
        Belt::Operator => "teeny-operator",
        Belt::Drafter => "teeny-drafter",
    }
}

/// `teeny-room-<persona>`, for a persona id of `[a-z0-9_-]`, 1 to 48 chars.
pub fn room_id(persona: &str) -> Result<String, SpecError> {
    let valid = !persona.is_empty()
        && persona.len() <= MAX_PERSONA
        && persona.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
    if valid {
        Ok(format!("{}-{persona}", agent_id(Belt::Room)))
    } else {
        Err(SpecError::Persona(persona.to_owned()))
    }
}

/// The OpenHuman built-ins `belt` holds beside its own tools.
pub fn built_ins(belt: Belt) -> &'static [&'static str] {
    match belt {
        Belt::Publisher | Belt::Operator | Belt::Room => &[BROWSER],
        Belt::Reply | Belt::Chat | Belt::Chart | Belt::Drafter => &[],
    }
}

/// The named scope: `tools` by name, in order, then the built-ins.
pub fn tool_scope(belt: Belt, tools: &[Arc<dyn Tool>]) -> Vec<String> {
    tools
        .iter()
        .map(|tool| tool.name().to_owned())
        .chain(built_ins(belt).iter().map(|name| (*name).to_owned()))
        .collect()
}

/// Untrusted input (Reply, Chat) runs public: read-only, every write
/// refused at once. Everything else is trusted automation.
pub fn access(belt: Belt) -> Access {
    match belt {
        Belt::Reply | Belt::Chat => Access::public(),
        _ => Access::full(),
    }
}

/// What every spec is built with besides its belt.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecContext {
    /// `LLM_BASE_URL`: the ladder.
    pub base_url: String,
    /// `LLM_API_KEY`.
    pub api_key: Secret,
    /// `LLM_MODEL`: the alias every belt runs on unless a turn overrides it.
    pub model: String,
    /// `SKILLS_DIR`, absolute.
    pub skills_dir: PathBuf,
    /// `AGENT_MAX_STEPS`: the tool-call iterations per turn.
    pub max_iterations: usize,
}

impl SpecContext {
    pub fn from_config(config: &teeny_core::Config) -> Self {
        let skills = PathBuf::from(&config.agent.skills_dir);
        Self {
            base_url: config.llm.base_url.clone(),
            api_key: config.llm.api_key.clone(),
            model: config.llm.model.clone(),
            skills_dir: std::path::absolute(&skills).unwrap_or(skills),
            max_iterations: config.agent.max_steps as usize,
        }
    }

    fn provider(&self) -> Provider {
        Provider::openai_compatible(&self.base_url, self.api_key.expose()).model(&self.model)
    }
}

/// What the persona prompts are rendered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptInputs {
    pub bot_username: String,
    pub dry_run: bool,
    pub persona: Persona,
    /// Whether `x_critique` is on the belts.
    pub critic: bool,
    /// The handle the chat agent talks as on Telegram.
    pub chat_handle: String,
    /// The token the chart agent charts.
    pub token_symbol: String,
}

impl PromptInputs {
    /// From the belts' own dependencies; the chat handle is the X handle.
    pub fn from_deps(deps: &Deps) -> Self {
        let handle = &deps.settings.bot_username;
        Self {
            bot_username: handle.clone(),
            dry_run: deps.settings.dry_run,
            persona: deps.persona.clone(),
            critic: deps.critic.is_some(),
            chat_handle: handle.strip_prefix('@').unwrap_or(handle).to_owned(),
            token_symbol: deps.settings.token_symbol.clone(),
        }
    }

    pub fn options(&self) -> PromptOptions {
        PromptOptions {
            bot_username: self.bot_username.clone(),
            dry_run: self.dry_run,
            persona: Some(self.persona.clone()),
            critic: self.critic,
        }
    }
}

/// The system prompt `belt` speaks with; `None` for a room, which speaks
/// with its persona's.
pub fn belt_prompt(belt: Belt, inputs: &PromptInputs) -> Option<String> {
    let options = inputs.options();
    Some(match belt {
        Belt::Reply => prompt::draft_prompt(&options),
        Belt::Chat => prompt::chat_prompt(&options, &inputs.chat_handle),
        Belt::Publisher | Belt::Operator => prompt::system_prompt(&options),
        Belt::Chart => prompt::chart_prompt(&options, &inputs.token_symbol),
        // The calendar's task names the channels; X is the default.
        Belt::Drafter => prompt::content_prompt(&options, &["x"]),
        Belt::Room => return None,
    })
}

/// The spec for `belt`, under its own id.
pub fn belt_spec(
    belt: Belt,
    tools: Arc<Vec<Arc<dyn Tool>>>,
    system_prompt: String,
    context: &SpecContext,
) -> AgentSpec {
    spec_as(agent_id(belt), belt, tools, system_prompt, context)
}

/// [`belt_spec`] under another id: a room persona's, or a probe's in a
/// posture test that has to stand a second agent of one belt up.
pub fn spec_as(
    id: &str,
    belt: Belt,
    tools: Arc<Vec<Arc<dyn Tool>>>,
    system_prompt: String,
    context: &SpecContext,
) -> AgentSpec {
    let definition = AgentDefinitionSpec::new()
        .system_prompt(system_prompt)
        .display_name(id)
        .tools(ToolScopeSpec::Named(tool_scope(belt, &tools)))
        .disallow_tools(DENY.iter().filter(|name| !name.ends_with('*')).copied())
        .max_iterations(context.max_iterations);
    AgentSpec::new(id)
        .definition(definition)
        .provider(context.provider())
        .access(access(belt))
        .skills_dir(&context.skills_dir)
        .include_user_skills(false)
        .tools(move |_turn| HostTurnTools::advertised(owned_belt(&tools)))
        .lockdown()
}

/// Registering an agent failed.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("persona id {0:?} must be 1 to 48 of [a-z0-9_-]")]
    Persona(String),
    /// Boxed: an agent error is large, and this is returned by value.
    #[error(transparent)]
    Agent(Box<AgentError>),
}

impl From<AgentError> for SpecError {
    fn from(error: AgentError) -> Self {
        Self::Agent(Box::new(error))
    }
}

/// A room agent and the persona prompt it was registered with.
struct Room {
    prompt: String,
    agent: Agent,
}

/// Every Teeny agent on the runtime: one per fixed belt, registered up
/// front, and one per backrooms persona, registered on first use.
///
/// Holding the [`Agent`]s is what keeps them registered: the runtime frees
/// an id when the last handle to it drops.
pub struct TeenyAgents {
    runtime: Arc<Runtime>,
    context: SpecContext,
    belts: HashMap<Belt, Agent>,
    room_tools: Arc<Vec<Arc<dyn Tool>>>,
    rooms: Mutex<HashMap<String, Room>>,
}

impl TeenyAgents {
    /// Builds every belt from `deps` and registers its agent.
    pub fn register(
        runtime: Arc<Runtime>,
        deps: &Deps,
        context: SpecContext,
    ) -> Result<Self, SpecError> {
        let inputs = PromptInputs::from_deps(deps);
        let mut belts = HashMap::new();
        for belt in Belt::ALL {
            let Some(prompt) = belt_prompt(belt, &inputs) else {
                continue;
            };
            let tools = Arc::new(belt.build(deps));
            let agent = runtime.agent(belt_spec(belt, tools, prompt, &context))?;
            belts.insert(belt, agent);
        }
        Ok(Self {
            runtime,
            room_tools: Arc::new(Belt::Room.build(deps)),
            context,
            belts,
            rooms: Mutex::new(HashMap::new()),
        })
    }

    /// The agent for a fixed belt; `None` for [`Belt::Room`].
    pub fn belt(&self, belt: Belt) -> Option<&Agent> {
        self.belts.get(&belt)
    }

    /// The agent for `persona`, speaking with `system_prompt`. Registered on
    /// first use and cached; a persona whose prompt has changed since is
    /// registered afresh under the same id. That fails while a turn of the
    /// old one is still running, which a room, speaking in turn, never does.
    pub fn room(&self, persona: &str, system_prompt: &str) -> Result<Agent, SpecError> {
        let id = room_id(persona)?;
        let mut rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(room) = rooms.get(&id).filter(|room| room.prompt == system_prompt) {
            return Ok(room.agent.clone());
        }
        // Release the id before registering it again.
        rooms.remove(&id);
        let agent = self.runtime.agent(spec_as(
            &id,
            Belt::Room,
            Arc::clone(&self.room_tools),
            system_prompt.to_owned(),
            &self.context,
        ))?;
        rooms.insert(
            id,
            Room {
                prompt: system_prompt.to_owned(),
                agent: agent.clone(),
            },
        );
        Ok(agent)
    }
}

#[cfg(test)]
#[path = "specs_tests.rs"]
mod tests;
