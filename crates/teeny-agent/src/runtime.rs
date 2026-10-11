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
//! the autonomy policy on (access tiers are inert without it), and, only
//! with `BROWSER_RESEARCH` on, sets up the research browser on the data
//! volume and limits it to `BROWSER_ALLOWED_HOSTS`. OpenHuman's browser
//! checks every URL it opens, and every URL a page lands on, against
//! `http_request.allowed_domains` (`modules::browser::BrowserClient::
//! check_url`), and refuses everything with an empty list. A scheduled turn
//! may only click and press on its own; typing, choosing and the
//! autonomous task step, what a login or a post is made of, are never
//! unattended ([`READ_ONLY_ACTIONS`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use openhuman_core::config::Config as OhConfig;
use openhuman_core::config::schema::ModuleOverride;
use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use openhuman_embed::{
    Access, DomainSet, Runtime, RuntimeError, ServiceSet, ToolGroups, Workspace,
};
use teeny_core::config::OpenHumanConfig;
use tokio::sync::OnceCell;

/// The gated browser actions a scheduled turn may take without an approval:
/// the ones that read. Anything listed beyond these is dropped.
pub const READ_ONLY_ACTIONS: [&str; 3] = ["click", "double_click", "press"];

/// OpenHuman's switch that lets the browser open any host, the allow list
/// notwithstanding. Set, the experiment stays off.
const ALLOW_ALL: &str = "OPENHUMAN_BROWSER_ALLOW_ALL";

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
    /// `BROWSER_ALLOWED_HOSTS`, as configured; normalised when applied.
    pub allowed_domains: Vec<String>,
    /// `TINYCOMPUTER_MODULE_PATH`.
    pub tinycomputer_module: Option<PathBuf>,
    /// Where the core's incidental hosted-backend calls go. `None` keeps
    /// OpenHuman's default; a test points it at a stub so nothing leaves
    /// the machine.
    pub backend_url: Option<String>,
    /// `BROWSER_RESEARCH`, effective: see [`browser_research`].
    pub browser_research: bool,
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
            browser_research: browser_research(config),
        }
    }

    /// `<TEENY_DATA>/openhuman/workspace`: sessions, transcripts, the cron
    /// database and every agent's home. Its parent holds `config.toml` and
    /// the credential store.
    pub fn workspace_dir(&self) -> PathBuf {
        self.data_dir.join("openhuman").join("workspace")
    }
}

/// Whether the browser experiment is on: `BROWSER_RESEARCH`, unless
/// OpenHuman's allow-all switch is set in the environment, which would let
/// the browser past the host list.
pub fn browser_research(config: &OpenHumanConfig) -> bool {
    if config.browser_research && browser_allow_all_set() {
        tracing::error!(
            "[runtime] {ALLOW_ALL} is set, which voids BROWSER_ALLOWED_HOSTS; the browser stays off"
        );
        return false;
    }
    config.browser_research
}

/// Whether OpenHuman's allow-all browser switch is on in this process.
pub fn browser_allow_all_set() -> bool {
    std::env::var(ALLOW_ALL)
        .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
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
    // Teeny remembers through CortexDB, with its own tools.
    config.memory.conversations.enabled = false;
    config.memory.recall.enabled = false;
    // No local models on this host. (OpenHuman no longer ships an embedded
    // Python runtime; `runtimes` is off in [`domains`] besides.)
    config.local_ai.runtime_enabled = false;

    let data = &settings.data_dir;
    let browser = &mut config.browser;
    browser.enabled = settings.browser_research;
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
        .filter(|kind| READ_ONLY_ACTIONS.contains(kind))
        .map(str::to_owned)
        .collect();
    browser.unattended_actions = known;

    // Never OpenHuman's default `["*"]`: off, the list is empty, which the
    // browser reads as no host at all.
    config.http_request.allowed_domains = if settings.browser_research {
        browsable_domains(&settings.allowed_domains)
    } else {
        Vec::new()
    };

    if let Some(path) = &settings.tinycomputer_module {
        config.modules.overrides.push(ModuleOverride {
            id: TINYCOMPUTER.into(),
            path: path_text(path),
        });
    }
    config
}

/// `listed` as hosts: trimmed, lowercased, stripped of scheme, path, port
/// and `*.`, without duplicates, and less any entry that would open every
/// host or a whole top-level domain (`*`, `com`), since OpenHuman matches
/// an entry's subdomains too.
pub fn browsable_domains(listed: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for host in listed.iter().filter_map(|entry| host_of(entry)) {
        if host == "*" || !host.contains('.') {
            tracing::warn!(entry = %host, "[runtime] dropped a browse host that opens too much");
            continue;
        }
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
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
        .access(Access::readonly());
    if let Some(url) = &settings.backend_url {
        builder = builder.backend_url(url);
    }
    let runtime = builder.build().await?;
    tracing::info!(
        workspace = %settings.workspace_dir().display(),
        "[runtime] OpenHuman runtime built"
    );
    Ok(Arc::new(runtime))
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
