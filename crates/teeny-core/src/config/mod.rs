//! The service's configuration, read from the environment.
//!
//! A typed port of `legacy/src/config.ts`: the same variable names, the same
//! defaults, the same ranges, so one `.env` drives either service during the
//! shadow run. Departures from the legacy schema are deliberate and each has a
//! test in `mod_tests.rs`:
//!
//! - the wallet (`AIRDROP_*`, `RECIPIENTS_COLLECTION`) is not ported, and its
//!   settings are ignored;
//! - the on-by-default flags (`X_CACHE_ENABLED`, `ACTIVITY_STREAM_ENABLED`,
//!   `HEAT_ENABLED`, `REPLY_LIKE_MENTIONS`) read `false` as off, which is what
//!   `.env.example` documents and what `z.coerce.boolean()` did not do;
//! - a blank optional value is absent everywhere, including the few legacy
//!   refused when blank (`EXA_API_KEY`, `AGENT_API_TOKEN`, ...);
//! - an empty entry in a rota slot list is skipped rather than read as slot 0;
//! - the per-job crons legacy dropped with the three-slot rota (`CHART_CRON`,
//!   `MOON_CRON`, `LORE_CRON`, `QUOTE_CRON`) are ignored, as zod ignored them.

mod content;
mod env;
mod kinds;
mod load;
mod service;

pub use content::{
    ArticleConfig, BackroomsConfig, CalendarConfig, ChartConfig, DEFAULT_REFERENCE_IMAGES,
    KolWallet, LoreConfig, MediaConfig, MoonConfig, MorningConfig, NarrativeConfig, QuoteConfig,
    RotaConfig, TelegramConfig,
};
pub use kinds::{
    AgentMode, ChartRenderer, ChartTimeframe, LoreSeed, ReplyMode, ServiceRole, TelegramTransport,
    VideoGenApi,
};
pub use service::{
    AgentConfig, BrainConfig, CadenceConfig, CortexConfig, LearningsConfig, LlmConfig,
    MemoryConfig, OpenHumanConfig, ReplyConfig, SafetyConfig, ServerConfig, StoreConfig,
    WebhookConfig, XConfig,
};

use crate::ConfigError;

/// Everything the service reads from its environment, grouped by concern.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `DRY_RUN`, default on: drafts are logged and nothing is published.
    pub dry_run: bool,
    pub server: ServerConfig,
    pub x: XConfig,
    pub webhook: WebhookConfig,
    pub cortex: CortexConfig,
    pub memory: MemoryConfig,
    pub learnings: LearningsConfig,
    pub llm: LlmConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
    pub reply: ReplyConfig,
    pub cadence: CadenceConfig,
    pub store: StoreConfig,
    pub brain: BrainConfig,
    pub rota: RotaConfig,
    pub chart: ChartConfig,
    pub morning: MorningConfig,
    pub moon: MoonConfig,
    pub lore: LoreConfig,
    pub quote: QuoteConfig,
    pub narrative: NarrativeConfig,
    pub backrooms: BackroomsConfig,
    pub article: ArticleConfig,
    pub calendar: CalendarConfig,
    pub telegram: TelegramConfig,
    pub media: MediaConfig,
    pub openhuman: OpenHumanConfig,
}

impl Config {
    /// Reads the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(
            std::env::vars_os().filter_map(|(key, value)| {
                Some((key.into_string().ok()?, value.into_string().ok()?))
            }),
        )
    }

    /// Reads a set of variables, as the process environment would hold them.
    pub fn from_vars<I, K, V>(vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut env = env::Env::new(
            vars.into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        );
        let config = load::load(&mut env);
        env.finish().map(|()| config)
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
