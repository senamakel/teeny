//! The process-wide OpenHuman runtime every Teeny agent lives on.
//!
//! `openhuman_embed` allows one [`Runtime`] per process (the keyring, the
//! event bus and the domain subscribers are process-scoped), so it lives in a
//! cell: [`global`] builds it on first use and hands out the same `Arc` after.
//! It is built on its own tokio runtime ([`executor`]), sized the way the
//! embed docs require, so the core's background work (the cron scheduler, bus
//! subscribers) outlives whichever caller happened to build it.
//!
//! What the runtime registers is the outer wall of the posture; each agent
//! narrows further (see [`crate::specs`]):
//!
//! - **domains**: the agent core (agent, memory, threads, config, security),
//!   inference, skills, modules, automation (the cron store and scheduler
//!   Teeny's own jobs run on) and platform (OpenHuman files its `browser`
//!   under platform, beside shell and files). Every other family is off:
//!   MCP, web3, desktop, hosted, flows, integrations, runtimes, voice, media,
//!   channels. An agent narrows these again with [`agent_domains`]: no
//!   agent has automation, and only a browsing agent keeps platform;
//! - **tool groups**: every group off. No belt names a packed tool, and a
//!   group that is off is not registered at all;
//! - **services**: the cron scheduler, and nothing that listens. Building
//!   does not start them (`defer_services`): `teeny serve` installs every
//!   cron handler and job first, then calls [`start_services`]; an operator
//!   subcommand never starts them, so it cannot run a second scheduler
//!   beside the server.
//!
//! The config overlay ([`openhuman_config`]) pins native tool calling, turns
//! the autonomy policy on (access tiers are inert without it), sets up the
//! research browser on the data volume, and limits browsing to
//! `BROWSE_ALLOWED_DOMAINS` less X: the account reaches X through its own
//! API, never a browser.
//!
//! **Memory.** Every agent shares one TinyMemory store: the dedicated
//! CortexDB at `TEENY_MEMORY_URL`, installed as the runtime's memory engine
//! ([`memory_engine`]; OpenHuman's `RuntimeBuilder::memory_engine`, the
//! embed path for a host that owns its store, so no key goes through the
//! keychain). Every agent is bound to the root `TEENY_MEMORY_ROOT`
//! ([`crate::specs`]): the learnings and beliefs there are shared, each
//! agent's own turns are logged at its node below, and the account's own
//! captures go to the root through [`crate::shared_memory`]. Without
//! `TEENY_MEMORY_TOKEN` memory is off (`[memory] engine = "none"`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use openhuman_core::config::Config as OhConfig;
use openhuman_core::config::schema::ModuleOverride;
use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use openhuman_embed::memory::api::MemoryEngine;
use openhuman_embed::{
    Access, DomainSet, LearningSettings, Runtime, RuntimeError, ServiceSet, ToolGroups, Workspace,
};
use teeny_core::Secret;
use teeny_core::config::{MemoryConfig, OpenHumanConfig};
use tinymemory_integrations::cortex::{CortexCredential, CortexEngine};
use tokio::sync::OnceCell;

/// Hosts no browser turn may open, with their subdomains: X, and the
/// shortener that redirects into it.
pub const NEVER_BROWSED: [&str; 3] = ["x.com", "twitter.com", "t.co"];

/// The module id the browser loads from.
const TINYCOMPUTER: &str = "tinycomputer";

/// What the runtime is booted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettings {
    /// `TEENY_DATA`, absolute.
    pub data_dir: PathBuf,
    /// `BROWSER_CHROME_PATH`.
    pub chrome_path: Option<String>,
    /// `BROWSE_UNATTENDED_ACTIONS`, as configured.
    pub unattended_actions: Vec<String>,
    /// `BROWSE_ALLOWED_DOMAINS`, as configured; X is removed when applied.
    pub allowed_domains: Vec<String>,
    /// `TINYCOMPUTER_MODULE_PATH`.
    pub tinycomputer_module: Option<PathBuf>,
    /// Where the core's incidental hosted-backend calls go. `None` keeps
    /// OpenHuman's default; a test points it at a stub so nothing leaves
    /// the machine.
    pub backend_url: Option<String>,
    /// The shared memory.
    pub memory: MemorySettings,
}

/// The shared memory every agent reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySettings {
    /// `TEENY_MEMORY_URL` and `TEENY_MEMORY_TOKEN`; `None` turns memory off.
    pub store: Option<MemoryStore>,
    /// `TEENY_MEMORY_ROOT`: the layout root every agent is bound to.
    pub root: String,
    /// `MEMORY_BUILD_BELIEFS_EVERY`.
    pub build_beliefs_every: u32,
    /// `MEMORY_LEARNINGS_LIMIT`.
    pub learnings_limit: u32,
    /// `MEMORY_TURN_PACK`: OpenHuman's pre-turn pack and turn log.
    pub turn_pack: bool,
    /// An engine in place of the store: a test's reference engine.
    pub engine: Option<HostEngine>,
}

impl Default for MemorySettings {
    /// Off, on the default root.
    fn default() -> Self {
        Self {
            store: None,
            root: teeny_memory::DEFAULT_ROOT.to_owned(),
            build_beliefs_every: LearningSettings::default().build_beliefs_every,
            learnings_limit: LearningSettings::default().learnings_limit,
            turn_pack: true,
            engine: None,
        }
    }
}

impl MemorySettings {
    pub fn from_config(config: &MemoryConfig) -> Self {
        Self {
            store: config.token.clone().map(|token| MemoryStore {
                url: config.url.clone(),
                token,
            }),
            root: config.root.clone(),
            build_beliefs_every: config.build_beliefs_every,
            learnings_limit: config.learnings_limit,
            turn_pack: config.turn_pack,
            engine: None,
        }
    }

    /// Whether an engine will be installed.
    pub fn is_on(&self) -> bool {
        self.store.is_some() || self.engine.is_some()
    }

    /// The learning policy every agent inherits.
    pub fn learning(&self) -> LearningSettings {
        LearningSettings {
            build_beliefs_every: self.build_beliefs_every,
            learnings_limit: self.learnings_limit,
        }
    }
}

/// The dedicated CortexDB the shared memory lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStore {
    pub url: String,
    pub token: Secret,
}

/// An engine handed to the runtime as is; equal only to itself.
#[derive(Clone)]
pub struct HostEngine(pub Arc<dyn MemoryEngine>);

impl PartialEq for HostEngine {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HostEngine {}

impl std::fmt::Debug for HostEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("HostEngine")
            .field(&self.0.descriptor().id)
            .finish()
    }
}

/// The engine the runtime installs: the test's, else the dedicated
/// CortexDB, attributing each item to the person it is about (the
/// `observed_actor` the legacy store was written with). `None` leaves
/// memory off.
///
/// A store on the stack's private network is reached through a loopback
/// relay ([`crate::memory_relay`]): the engine refuses cleartext to any
/// other host.
pub async fn memory_engine(
    memory: &MemorySettings,
) -> Result<Option<Arc<dyn MemoryEngine>>, BootError> {
    if let Some(engine) = &memory.engine {
        return Ok(Some(Arc::clone(&engine.0)));
    }
    let Some(store) = &memory.store else {
        return Ok(None);
    };
    let endpoint = crate::memory_relay::engine_endpoint(&store.url)
        .await
        .map_err(|error| BootError::Memory(error.to_string()))?;
    let engine = CortexEngine::direct(&endpoint, CortexCredential::api_key(store.token.expose()))
        .map_err(|error| BootError::Memory(error.to_string()))?
        .with_observed_actor(true);
    Ok(Some(Arc::new(engine)))
}

impl RuntimeSettings {
    /// From the service's configuration. A relative `TEENY_DATA` is resolved
    /// against the working directory, once, here.
    pub fn from_config(config: &OpenHumanConfig) -> Self {
        let data_dir = PathBuf::from(&config.data_dir);
        Self {
            data_dir: std::path::absolute(&data_dir).unwrap_or(data_dir),
            chrome_path: config.chrome_path.clone(),
            unattended_actions: config.unattended_actions.clone(),
            allowed_domains: config.allowed_domains.clone(),
            tinycomputer_module: config.tinycomputer_module_path.as_ref().map(PathBuf::from),
            backend_url: None,
            memory: MemorySettings::default(),
        }
    }

    /// From the whole configuration: [`Self::from_config`] with the shared
    /// memory's settings.
    pub fn from_teeny(config: &teeny_core::Config) -> Self {
        Self {
            memory: MemorySettings::from_config(&config.memory),
            ..Self::from_config(&config.openhuman)
        }
    }

    /// `<TEENY_DATA>/openhuman/workspace`: sessions, transcripts, the cron
    /// database and every agent's home. Its parent holds `config.toml` and
    /// the credential store.
    pub fn workspace_dir(&self) -> PathBuf {
        self.data_dir.join("openhuman").join("workspace")
    }
}

/// The domain families the runtime registers. Agents can only narrow these.
pub fn domains() -> DomainSet {
    DomainSet {
        inference: true,
        skills: true,
        modules: true,
        automation: true,
        platform: true,
        ..DomainSet::harness()
    }
}

/// The families an agent runs with: the runtime's, less automation (no
/// agent schedules anything: the cron tools are automation's), and less
/// platform unless the agent browses. Platform is where OpenHuman files
/// every tool without a family of its own (`browser`, and also shell,
/// files, HTTP, installers), so an agent without the browser does not even
/// have them registered; one with it is held to its named belt by the
/// scope, the host-only policy and the deny rules (see [`crate::specs`]).
pub fn agent_domains(browses: bool) -> DomainSet {
    DomainSet {
        automation: false,
        platform: browses,
        ..domains()
    }
}

/// Every tool group off: no Teeny belt names a packed tool.
pub fn tool_groups() -> ToolGroups {
    ToolGroups::none()
}

/// The cron scheduler, and nothing that listens. Selected here, started by
/// [`start_services`].
pub fn services() -> ServiceSet {
    ServiceSet {
        cron: true,
        ..ServiceSet::none()
    }
}

/// The base OpenHuman config every agent starts from.
pub fn openhuman_config(settings: &RuntimeSettings) -> OhConfig {
    let mut config = OhConfig::default();
    // The schema default is `python`, which hands the model its tools as
    // prose; every Teeny tool is a typed schema.
    config.agent.tool_dispatcher = "native".into();
    config.autonomy.enabled = true;
    config.cron.enabled = true;
    // One shared store, installed as the host engine (`memory_engine`).
    // Without it memory is off outright rather than reaching for the
    // TinyHumans backend. The pre-turn pack and the turn log run only with
    // a store, and only when `MEMORY_TURN_PACK` asks for them.
    let memory = &settings.memory;
    if !memory.is_on() {
        config.memory.engine = "none".into();
    }
    let pack = memory.is_on() && memory.turn_pack;
    config.memory.conversations.enabled = pack;
    config.memory.recall.enabled = pack;
    config.memory.root = Some(memory.root.clone());
    // No local models on this host. (OpenHuman no longer ships an embedded
    // Python runtime; `runtimes` is off in [`domains`] besides.)
    config.local_ai.runtime_enabled = false;

    let data = &settings.data_dir;
    let browser = &mut config.browser;
    browser.enabled = true;
    browser.headless = true;
    browser.profile_mode = "persistent".into();
    browser.profile_path = Some(path_text(&data.join("chrome-profile")));
    browser.download_dir = Some(path_text(&data.join("media").join("downloads")));
    browser.chrome_path = settings.chrome_path.clone();
    browser.unattended_actions = settings.unattended_actions.clone();
    let known: Vec<String> = settings
        .unattended_actions
        .iter()
        .filter_map(|action| browser.unattended_kind(action))
        .map(str::to_owned)
        .collect();
    browser.unattended_actions = known;

    config.http_request.allowed_domains = browsable_domains(&settings.allowed_domains);

    if let Some(path) = &settings.tinycomputer_module {
        config.modules.overrides.push(ModuleOverride {
            id: TINYCOMPUTER.into(),
            path: path_text(path),
        });
    }
    config
}

/// `listed`, less anything that would open X: an entry naming X or one of
/// its subdomains, and an entry broad enough to cover X (`*`, `com`), since
/// OpenHuman matches an entry's subdomains too. Entries are trimmed,
/// lowercased and stripped to their host.
pub fn browsable_domains(listed: &[String]) -> Vec<String> {
    listed
        .iter()
        .filter_map(|entry| host_of(entry))
        .filter(|host| {
            let opens_x = host == "*"
                || NEVER_BROWSED
                    .iter()
                    .any(|never| within(never, host) || within(host, never));
            if opens_x {
                tracing::warn!(entry = %host, "[runtime] dropped a browse domain that opens X");
            }
            !opens_x
        })
        .collect()
}

/// The host an allow-list entry names: no scheme, path, port or `*.`.
fn host_of(entry: &str) -> Option<String> {
    let entry = entry.trim().to_ascii_lowercase();
    let entry = entry
        .split_once("://")
        .map_or(entry.as_str(), |(_, rest)| rest);
    let host = entry.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let host = host.trim_start_matches("*.").trim_matches('.');
    (!host.is_empty()).then(|| host.to_owned())
}

/// Whether `host` is `domain` or one of its subdomains.
fn within(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Building the runtime failed.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("the OpenHuman boot task failed: {0}")]
    Task(String),
    #[error("the shared memory store: {0}")]
    Memory(String),
}

static RUNTIME: OnceCell<Arc<Runtime>> = OnceCell::const_new();
static EXECUTOR: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The tokio runtime the OpenHuman core lives on: process-long, with the
/// worker stack and blocking-thread cap OpenHuman documents for a turn. Turn
/// futures themselves run wherever their caller awaits them.
pub fn executor() -> &'static tokio::runtime::Handle {
    EXECUTOR
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("openhuman-core")
                .thread_stack_size(AGENT_WORKER_STACK_BYTES)
                .max_blocking_threads(MAX_BLOCKING_THREADS)
                .build()
                .expect("the OpenHuman executor builds")
        })
        .handle()
}

/// The process-wide runtime, built on the first call with `settings`. Later
/// calls return the same runtime and ignore theirs: the workspace is a
/// process fact.
pub async fn global(settings: RuntimeSettings) -> Result<Arc<Runtime>, BootError> {
    RUNTIME
        .get_or_try_init(|| async move {
            executor()
                .spawn(build(settings))
                .await
                .map_err(|error| BootError::Task(error.to_string()))?
        })
        .await
        .cloned()
}

/// The runtime, if it has been built.
pub fn existing() -> Option<Arc<Runtime>> {
    RUNTIME.get().cloned()
}

/// Starts the services [`services`] selects, the cron scheduler among them,
/// on the [`executor`], so they outlive the caller's runtime. Idempotent
/// while they run.
pub async fn start_services(runtime: Arc<Runtime>) {
    let started = executor()
        .spawn(async move { runtime.start_services().await })
        .await;
    if let Err(error) = started {
        tracing::error!(%error, "[runtime] starting the services failed");
    }
}

async fn build(settings: RuntimeSettings) -> Result<Arc<Runtime>, BootError> {
    // For the context-less paths that read tool groups (agent builds outside
    // a core context): the same answer as the runtime's.
    openhuman_core::tools::toolpacks::set_process_default(tool_groups());
    let mut builder = Runtime::builder()
        .workspace(Workspace::dir(settings.workspace_dir()))
        .config(openhuman_config(&settings))
        .domains(domains())
        .tool_groups(tool_groups())
        .services(services())
        // Started by the host once every cron handler is in place
        // ([`start_services`]): a stored job due at boot would otherwise
        // fire before its handler exists and be recorded "ok" unrun.
        .defer_services()
        // Every Teeny agent names its own access; one that forgot would get
        // the narrowest tier rather than the supervised default.
        .access(Access::readonly())
        .learning(settings.memory.learning());
    if let Some(engine) = memory_engine(&settings.memory).await? {
        builder = builder.memory_engine(engine);
    }
    if let Some(url) = &settings.backend_url {
        builder = builder.backend_url(url);
    }
    let runtime = builder.build().await?;
    tracing::info!(
        workspace = %settings.workspace_dir().display(),
        memory = settings.memory.is_on(),
        root = %settings.memory.root,
        "[runtime] OpenHuman runtime built"
    );
    Ok(Arc::new(runtime))
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
