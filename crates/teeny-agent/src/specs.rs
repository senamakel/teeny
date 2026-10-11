//! One locked-down OpenHuman agent per belt, and one per backrooms persona.
//!
//! | Agent | Access | Belt | OpenHuman built-ins |
//! |---|---|---|---|
//! | `teeny-reply` | public (read-only) | Reply, its reads only | none |
//! | `teeny-chat` | public (read-only) | Chat, its reads only | none |
//! | `teeny-chart` | `full` | Chart | none |
//! | `teeny-operator` | `full` | Operator | none |
//! | `teeny-drafter` | `full` | Drafter | none |
//! | `teeny-learner` | `full` | Learner | none |
//! | `teeny-room-<persona>` | `full` | Room | `browser`, with `BROWSER_RESEARCH` |
//!
//! **Live prompts.** A fixed belt's system prompt is a file under
//! `<workspace>/agent/prompts/teeny/<instance>/` (OpenHuman's
//! `PromptSource::File`, read when each turn's session is built), and
//! [`TeenyAgents::belt`] renders the prompt again before handing the agent
//! out, rewriting the file when it changed. So the learnings brief written
//! after boot is in the next turn's system prompt with no re-registration;
//! a room re-registers on a new persona prompt, and the learner's file is
//! written with each call's instructions ([`TeenyAgents::learner`]). The
//! instance directory is per process, so an operator subcommand running
//! beside `teeny serve` never rewrites the server's prompts.
//!
//! The lockdown is built from OpenHuman's own mechanisms, each enough on its
//! own for what it covers:
//!
//! - **registration**: the agent's domain families
//!   ([`runtime::agent_domains`]) leave out automation (the cron tools) and,
//!   for every agent but the browsing ones, platform (shell, files, HTTP,
//!   installers, `browser`); every tool group is off;
//! - **scope**: a [`ToolScopeSpec::Named`] belt, the belt's tools by the
//!   names [`Belt::build`] gives them plus the built-ins above, over a
//!   definition with the orchestrator's sub-agents, extra and deferred tools
//!   taken out ([`base_definition`]), so nothing else the runtime registers
//!   is advertised and no delegation tool is synthesised. The browser is a
//!   deferred tool: a browsing agent is advertised the harness's
//!   `tool_search` bridge in its place (it finds the browser and nothing
//!   else) and calls `browser` by name;
//! - **policy**: OpenHuman's [`HostOnlyToolPolicy`] over that belt (and the
//!   [`harness_tools`] OpenHuman adds to every named belt), set as the
//!   session's whole gate through [`HostTurnTools::with_policy`]: a call to
//!   any other name is refused before anything runs;
//! - **deny rules**: [`DENY`] on the definition, its exact names as
//!   `disallow_tools` and the whole list, families included, as deny
//!   [`ToolRules`] on the catalogue, search and call surfaces.
//!
//! A public agent (Reply, Chat) runs `Access::readonly()` under the
//! `public` external-channel origin with the approval gate off, so a write
//! is refused at once, never parked, and it is handed only its belt's
//! reads: nothing that writes is registered on it at all.
//! `tests/posture.rs` proves all of it against the real runtime.
//!
//! The belt reaches OpenHuman as host tools ([`AgentSpec::tools`]), built
//! once and handed out per turn through [`SharedTool`]. A host tool is
//! spliced ahead of the config-derived registry and wins a collision on its
//! name (`openhuman-core` `agent/session_host/builder/host_tools.rs`,
//! `HostTurnTools::merge_into`; dedup keeps the first), so Teeny's
//! `web_search` and `web_answer` answer, never OpenHuman's.
//!
//! No `AgentRegistryEntry` is written: an embed agent's turn dispatches
//! `AgentChatTarget::Definition` with the definition in hand, so nothing is
//! resolved by id (`openhuman-embed` `turn.rs` `dispatch`).
//!
//! [`SharedTool`]: crate::shared_tool::SharedTool

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openhuman_core::agent::HostOnlyToolPolicy;
use openhuman_core::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, PromptSource, ToolScope,
};
use openhuman_core::agent::tool_policy::ToolPolicy;
use openhuman_core::inference::tokenjuice::{RECOVERY_TOOL_NAMES, REPL_TOOL_NAMES};
use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentError, AgentSpec, AgentTurnOrigin, DefinitionBase,
    HostTurnTools, MemoryBinding, Provider, Runtime, ToolGroups, ToolScopeSpec,
};
use teeny_core::Secret;
use tinytools::ToolRules;

use crate::deps::Deps;
use crate::posture::{Belt, DENY};
use crate::prompt::{self, Persona, PromptOptions};
use crate::runtime;
use crate::shared_tool::owned_belt;
use crate::tools::{PermissionLevel, Tool};

/// OpenHuman's research browser: the one built-in a Teeny agent may hold.
pub const BROWSER: &str = "browser";

/// The channel a public turn names in its origin.
pub const PUBLIC_CHANNEL: &str = "public";

/// The longest persona id a room agent takes, so its id fits OpenHuman's 64.
const MAX_PERSONA: usize = 48;

/// The agent id of `belt`. `teeny-room` is the prefix of each persona's id;
/// no agent is registered under it alone.
pub fn agent_id(belt: Belt) -> &'static str {
    match belt {
        Belt::Reply => "teeny-reply",
        Belt::Chat => "teeny-chat",
        Belt::Chart => "teeny-chart",
        Belt::Room => "teeny-room",
        Belt::Operator => "teeny-operator",
        Belt::Drafter => "teeny-drafter",
        Belt::Learner => "teeny-learner",
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

/// The OpenHuman built-ins `belt` holds beside its own tools: the browser
/// on a room (the backrooms personas and the research step), and only
/// while `BROWSER_RESEARCH` (`browse`) is on. Never a public belt, never the
/// learner, never the operator.
pub fn built_ins(belt: Belt, browse: bool) -> &'static [&'static str] {
    match belt {
        Belt::Room if browse => &[BROWSER],
        _ => &[],
    }
}

/// The tools OpenHuman itself adds to every named belt: the TinyJuice
/// recovery and REPL tools a compacted tool result points the model at.
/// They read back what a belt tool already returned, and nothing else.
pub fn harness_tools() -> impl Iterator<Item = &'static str> {
    RECOVERY_TOOL_NAMES.iter().chain(REPL_TOOL_NAMES).copied()
}

/// Whether `belt` takes untrusted input from the public (Reply, Chat).
pub fn is_public(belt: Belt) -> bool {
    matches!(belt, Belt::Reply | Belt::Chat)
}

/// What an agent of `belt` is handed of `tools`: all of them, or for a
/// public belt only its reads, so nothing that writes is registered on it.
pub fn handed(belt: Belt, tools: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
    tools
        .iter()
        .filter(|tool| !is_public(belt) || tool.permission_level() < PermissionLevel::Write)
        .cloned()
        .collect()
}

/// The named scope: `tools` by name, in order, then the built-ins.
pub fn tool_scope(belt: Belt, tools: &[Arc<dyn Tool>], browse: bool) -> Vec<String> {
    tools
        .iter()
        .map(|tool| tool.name().to_owned())
        .chain(
            built_ins(belt, browse)
                .iter()
                .map(|name| (*name).to_owned()),
        )
        .collect()
}

/// The origin a public turn runs under: an external channel named
/// [`PUBLIC_CHANNEL`], with no sender the core could grant anything to.
pub fn public_origin() -> AgentTurnOrigin {
    AgentTurnOrigin::ExternalChannel {
        channel: PUBLIC_CHANNEL.to_owned(),
        sender: None,
        sender_name: None,
        reply_target: String::new(),
        message_id: String::new(),
        history_key: None,
    }
}

/// Untrusted input (Reply, Chat) runs public: the read-only tier under the
/// public origin, with the approval gate off so a write is refused at once
/// rather than parked for an approval no one on a public channel can give.
/// Everything else is trusted automation.
pub fn access(belt: Belt) -> Access {
    if is_public(belt) {
        Access::readonly()
            .origin(public_origin())
            .approval_gate(false)
    } else {
        Access::full()
    }
}

/// The definition every Teeny agent starts from: OpenHuman's orchestrator
/// with every route to a tool the belt does not name taken out. The
/// orchestrator declares built-in sub-agents (image, video, vision, task
/// manager, skill setup), each of which would become a delegation tool on a
/// named belt as well, and extra and deferred tools of its own; a Teeny
/// agent delegates nothing, and discovers only what its belt names.
pub fn base_definition() -> AgentDefinition {
    let mut definition = AgentDefinitionRegistry::builtins_only()
        .get(ORCHESTRATOR)
        .cloned()
        .expect("OpenHuman ships the orchestrator definition");
    // Narrowed to the belt by the spec's named scope.
    definition.tools = ToolScope::Wildcard;
    definition.subagents.clear();
    definition.extra_tools.clear();
    definition.deferred_tools.clear();
    definition
}

/// The built-in definition [`base_definition`] narrows.
const ORCHESTRATOR: &str = "orchestrator";

/// The deny rules every agent carries: [`DENY`], families included, off the
/// catalogue, out of search and refused as a call.
pub fn deny_rules() -> ToolRules {
    ToolRules::from_allow_deny(Vec::<String>::new(), DENY.iter().copied())
}

/// The session gate for a belt whose named scope is `scope`: OpenHuman's
/// host-only policy over the scope and [`harness_tools`].
pub fn belt_policy(scope: &[String]) -> Arc<dyn ToolPolicy> {
    let allowed: HashSet<String> = scope
        .iter()
        .cloned()
        .chain(harness_tools().map(str::to_owned))
        .collect();
    Arc::new(HostOnlyToolPolicy::new(allowed, None))
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
    /// `TEENY_MEMORY_ROOT`: the memory root every agent is bound to.
    pub memory_root: String,
    /// `BROWSER_RESEARCH`, effective ([`runtime::browser_research`]).
    pub browser_research: bool,
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
            memory_root: config.memory.root.clone(),
            browser_research: runtime::browser_research(&config.openhuman),
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
/// with its persona's, and for the learner, which speaks with the brief's
/// instructions, handed over per call.
pub fn belt_prompt(belt: Belt, inputs: &PromptInputs) -> Option<String> {
    let options = inputs.options();
    Some(match belt {
        Belt::Reply => prompt::draft_prompt(&options),
        Belt::Chat => prompt::chat_prompt(&options, &inputs.chat_handle),
        Belt::Operator => prompt::system_prompt(&options),
        Belt::Chart => prompt::chart_prompt(&options, &inputs.token_symbol),
        // The calendar's task names the channels; X is the default.
        Belt::Drafter => prompt::content_prompt(&options, &["x"]),
        Belt::Room | Belt::Learner => return None,
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
    spec_with(
        id,
        belt,
        tools,
        PromptSource::Inline(system_prompt),
        context,
    )
}

/// The spec for `belt`, under its own id, speaking with the prompt file at
/// `prompt_file` under the workspace's `agent/prompts/` ([`LivePrompts`]).
pub fn live_spec(
    belt: Belt,
    tools: Arc<Vec<Arc<dyn Tool>>>,
    prompt_file: String,
    context: &SpecContext,
) -> AgentSpec {
    spec_with(
        agent_id(belt),
        belt,
        tools,
        PromptSource::File { path: prompt_file },
        context,
    )
}

fn spec_with(
    id: &str,
    belt: Belt,
    tools: Arc<Vec<Arc<dyn Tool>>>,
    prompt: PromptSource,
    context: &SpecContext,
) -> AgentSpec {
    let tools = handed(belt, &tools);
    let browse = context.browser_research;
    let scope = tool_scope(belt, &tools, browse);
    let policy = belt_policy(&scope);
    let mut base = base_definition();
    // Set on the base rather than through the spec, which only takes text:
    // a spec without a prompt of its own keeps the base's.
    base.system_prompt = prompt;
    let definition = AgentDefinitionSpec::from_base(DefinitionBase::Custom(Box::new(base)))
        .display_name(id)
        .tools(ToolScopeSpec::Named(scope))
        .disallow_tools(DENY.iter().filter(|name| !name.ends_with('*')).copied())
        .tool_rules(deny_rules())
        .max_iterations(context.max_iterations);
    AgentSpec::new(id)
        .definition(definition)
        .provider(context.provider())
        .access(access(belt))
        .domains(runtime::agent_domains(!built_ins(belt, browse).is_empty()))
        .tool_groups(ToolGroups::none())
        .skills_dir(&context.skills_dir)
        .include_user_skills(false)
        .memory(memory_binding(id, context))
        .tools(move |_turn| {
            HostTurnTools::advertised(owned_belt(&tools)).with_policy(Arc::clone(&policy))
        })
}

/// Whose memory an agent reads and writes: its own id's, under the shared
/// root. The learnings and beliefs at the root are every agent's; the turns
/// OpenHuman logs are kept at the agent's own node below it.
pub fn memory_binding(id: &str, context: &SpecContext) -> MemoryBinding {
    MemoryBinding::new(id).root(&context.memory_root)
}

/// How long another process's prompt directory may sit untouched before a
/// new process prunes it. A live one that was pruned is written again on
/// its next turn ([`LivePrompts::write`]).
const STALE_PROMPTS: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The fixed belts' system prompts as files OpenHuman reads per turn.
#[derive(Debug)]
pub struct LivePrompts {
    /// `<workspace>/agent/prompts`: OpenHuman's root for a file prompt.
    root: PathBuf,
    /// This process's directory under it: `teeny/<instance>`.
    dir: String,
    /// What each file was last written with.
    written: Mutex<HashMap<Belt, String>>,
}

impl LivePrompts {
    /// Under `<workspace>/agent/prompts/<dir>`.
    pub fn new(workspace: &Path, dir: &str) -> Self {
        Self {
            root: workspace.join("agent").join("prompts"),
            dir: dir.to_owned(),
            written: Mutex::new(HashMap::new()),
        }
    }

    /// `teeny/<pid>-<millis>`: one directory per process.
    pub fn instance_dir() -> String {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();
        format!("teeny/{}-{millis}", std::process::id())
    }

    /// The path a definition names, relative to the prompts root.
    pub fn relative(&self, belt: Belt) -> String {
        format!("{}/{}.md", self.dir, agent_id(belt))
    }

    /// The file itself.
    pub fn path(&self, belt: Belt) -> PathBuf {
        self.root.join(self.relative(belt))
    }

    /// Writes `text` as `belt`'s prompt when it differs from what was last
    /// written or the file has gone; atomically, so a turn never reads half.
    pub fn write(&self, belt: Belt, text: &str) -> std::io::Result<()> {
        self.refresh(belt, || text.to_owned())
    }

    /// [`write`](Self::write) with the prompt rendered under the same lock,
    /// so a render that read an older brief can never land after a newer
    /// one: renders and writes are ordered, and a turn reads what the last
    /// render saw.
    pub fn refresh(&self, belt: Belt, render: impl FnOnce() -> String) -> std::io::Result<()> {
        let path = self.path(belt);
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        let text = render();
        let text = text.as_str();
        if written.get(&belt).is_some_and(|last| last == text) && path.is_file() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let staging = path.with_extension("md.tmp");
        std::fs::write(&staging, text)?;
        std::fs::rename(&staging, &path)?;
        written.insert(belt, text.to_owned());
        tracing::debug!(agent = agent_id(belt), "[specs] system prompt written");
        Ok(())
    }

    /// Removes other processes' directories untouched for `stale`.
    pub fn prune(&self, stale: Duration) {
        let mine = self.root.join(&self.dir);
        let Some(parent) = mine.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == mine || !path.is_dir() {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age >= stale);
            if old && std::fs::remove_dir_all(&path).is_ok() {
                tracing::debug!(dir = %path.display(), "[specs] pruned stale prompts");
            }
        }
    }
}

/// The learner's prompt until its first call writes the brief's
/// instructions over it.
const LEARNER_PROMPT: &str =
    "You write this account's learnings brief from the record you are given.";

/// Registering an agent failed.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("persona id {0:?} must be 1 to 48 of [a-z0-9_-]")]
    Persona(String),
    #[error("could not write a system prompt: {0}")]
    Prompt(std::io::Error),
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
    inputs: PromptInputs,
    prompts: LivePrompts,
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
        let dir = LivePrompts::instance_dir();
        let mut prompts: Option<LivePrompts> = None;
        let mut belts = HashMap::new();
        for belt in Belt::ALL {
            let text = match (belt, belt_prompt(belt, &inputs)) {
                (_, Some(text)) => text,
                (Belt::Learner, None) => LEARNER_PROMPT.to_owned(),
                _ => continue,
            };
            let tools = Arc::new(belt.build(deps));
            let relative = format!("{dir}/{}.md", agent_id(belt));
            let agent = runtime.agent(live_spec(belt, tools, relative, &context))?;
            let live = prompts.get_or_insert_with(|| {
                let live = LivePrompts::new(agent.workspace_dir(), &dir);
                live.prune(STALE_PROMPTS);
                live
            });
            live.write(belt, &text).map_err(SpecError::Prompt)?;
            belts.insert(belt, agent);
        }
        Ok(Self {
            runtime,
            room_tools: Arc::new(Belt::Room.build(deps)),
            prompts: prompts.expect("Belt::ALL has fixed belts"),
            context,
            inputs,
            belts,
            rooms: Mutex::new(HashMap::new()),
        })
    }

    /// The agent for a fixed belt; `None` for [`Belt::Room`]. Its prompt is
    /// rendered again first, so a turn on it speaks with the persona as it
    /// is now (the learnings brief included). A prompt that cannot be
    /// written leaves the last one in place.
    pub fn belt(&self, belt: Belt) -> Option<&Agent> {
        let agent = self.belts.get(&belt)?;
        if matches!(belt, Belt::Room | Belt::Learner) {
            return Some(agent);
        }
        let refreshed = self.prompts.refresh(belt, || {
            belt_prompt(belt, &self.inputs).expect("a fixed belt has a prompt")
        });
        if let Err(error) = refreshed {
            tracing::warn!(%error, agent = agent_id(belt), "[specs] could not refresh the system prompt");
        }
        Some(agent)
    }

    /// The learner, speaking with `system_prompt` for its next turn. One
    /// writer (the morning role's learnings job), so turns do not race.
    pub fn learner(&self, system_prompt: &str) -> Result<Agent, SpecError> {
        self.prompts
            .write(Belt::Learner, system_prompt)
            .map_err(SpecError::Prompt)?;
        Ok(self
            .belts
            .get(&Belt::Learner)
            .cloned()
            .expect("the learner is registered with the belts"))
    }

    /// Where `belt`'s live prompt file is.
    pub fn prompt_path(&self, belt: Belt) -> PathBuf {
        self.prompts.path(belt)
    }

    /// Whether the browser experiment is on for these agents.
    pub fn browses(&self) -> bool {
        self.context.browser_research
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
