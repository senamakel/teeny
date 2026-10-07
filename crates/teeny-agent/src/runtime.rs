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
//! narrows further with `AgentSpec::lockdown()` (see [`crate::specs`]):
//!
//! - **domains**: the embedded preset with skills and modules (the browser is
//!   a module) on, and every family Teeny has no use for off: MCP, web3,
//!   desktop, hosted, flows, integrations, runtimes, exec, filesystem,
//!   system, voice, media, channels;
//! - **tool groups**: every group off. No belt names a packed tool, and a
//!   group that is off is not registered at all;
//! - **services**: the cron scheduler, and nothing that listens. Building
//!   does not start them: `teeny serve` installs every cron handler and job
//!   first, then calls [`start_services`]; an operator subcommand never
//!   starts them, so it cannot run a second scheduler beside the server.
//!
//! The config overlay ([`openhuman_config`]) pins native tool calling, turns
//! the autonomy policy on (access tiers are inert without it), sets up the
//! research browser on the data volume, and limits browsing to
//! `BROWSE_ALLOWED_DOMAINS` less X: the account reaches X through its own
//! API, never a browser.

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
        skills: true,
        modules: true,
        mcp: false,
        web3: false,
        desktop: false,
        hosted: false,
        flows: false,
        integrations: false,
        runtimes: false,
        exec: false,
        filesystem: false,
        system: false,
        voice: false,
        media: false,
        channels: false,
        ..DomainSet::embedded()
    }
}

/// Every tool group off: no Teeny belt names a packed tool.
pub fn tool_groups() -> ToolGroups {
    ToolGroups::none()
}

/// The cron scheduler on top of the one-shot harness init; nothing listens.
/// Selected here, started by [`start_services`].
pub fn services() -> ServiceSet {
    ServiceSet {
        harness_init: true,
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
    // No local models and no Python runtime on this host.
    config.local_ai.runtime_enabled = false;
    config.runtime_python.enabled = false;

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
