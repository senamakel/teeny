//! The service side of the configuration: the process, X, memory, the model,
//! the agent, the content filter, replies, cadence, storage and the brain.
//!
//! Every field names the environment variable it is read from and its default;
//! the comments on why each default is what it is live in `legacy/src/config.ts`
//! until cutover, and move here field by field as each consumer is ported.

use super::kinds::{AgentMode, ReplyMode, ServiceRole};
use crate::Secret;

/// The process itself.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// `PORT`, default 3001.
    pub port: u16,
    /// `LOG_LEVEL`, default `info`.
    pub log_level: String,
    /// `SERVICE_ROLE`, default `all`.
    pub service_role: ServiceRole,
}

/// The X account and the read cache in front of it.
#[derive(Debug, Clone, PartialEq)]
pub struct XConfig {
    /// `X_BOT_USERNAME`, required.
    pub bot_username: String,
    /// `X_API_KEY`, required.
    pub api_key: Secret,
    /// `X_API_SECRET`, required.
    pub api_secret: Secret,
    /// `X_ACCESS_TOKEN`, required.
    pub access_token: Secret,
    /// `X_ACCESS_TOKEN_SECRET`, required.
    pub access_token_secret: Secret,
    /// `X_BEARER_TOKEN`, optional; the activity stream needs it.
    pub bearer_token: Option<Secret>,
    /// `X_CACHE_ENABLED`, default on. Off, every read goes to X.
    pub cache_enabled: bool,
    /// `X_CACHE_POST_TTL_DAYS`, default 30.
    pub cache_post_ttl_days: f64,
    /// `X_CACHE_SEARCH_TTL_MINUTES`, default 10.
    pub cache_search_ttl_minutes: f64,
    /// `MENTION_LOOKBACK_MINUTES`, 1..=10080, default 15. A cost dial.
    pub mention_lookback_minutes: u32,
    /// `X_SEARCH_MAX_RESULTS`, 10..=100, default 10. A cost dial.
    pub search_max_results: u32,
    /// `ACTIVITY_STREAM_ENABLED`, default on.
    pub activity_stream_enabled: bool,
    /// `THREAD_CONTEXT_MAX_POSTS`, 0..=100, default 10. Zero reads no thread.
    pub thread_context_max_posts: u32,
}

/// Forwarding verified X deliveries to another service.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookConfig {
    /// `WEBHOOK_FORWARD_URL`, optional. Unset, nothing is forwarded.
    pub forward_url: Option<String>,
    /// `WEBHOOK_FORWARD_TOKEN`, optional, at least 16 characters.
    pub forward_token: Option<Secret>,
}

/// CortexDB, the account's long-term memory.
#[derive(Debug, Clone, PartialEq)]
pub struct CortexConfig {
    /// `CORTEX_API_URL`, default `https://api-v1.cortexdb.ai`.
    pub api_url: String,
    /// `CORTEX_ACTOR`, default `agent:teeny`.
    pub actor: String,
    /// `CORTEX_SCOPE`, default `agent:teeny`.
    pub scope: String,
    /// `CORTEX_TOKEN`, required.
    pub token: Secret,
    /// `CORTEX_RECALL_TIMEOUT_MS`, 1000..=120000, default 15000. Recall is
    /// on the turn's critical path, so it gets seconds: a recall that takes
    /// longer is one the reply should go out without.
    pub recall_timeout_ms: u64,
    /// `CORTEX_CAPTURE_TIMEOUT_MS`, 1000..=120000, default 30000. Captures
    /// run after the turn and can wait.
    pub capture_timeout_ms: u64,
    /// `CORTEX_PERSON_RECALL`, default on: the second recall pack, about the
    /// person being answered, filtered on their `@handle` label.
    pub person_recall: bool,
    /// `CORTEX_RECALL_COOLDOWN_MS`, 0..=3600000, default 300000: how long
    /// recall stays off after one times out, so a stalled store is not handed
    /// one more abandoned query per reply.
    pub recall_cooldown_ms: u64,
    /// `CORTEX_HEALTH_CRON`, default `17 * * * *`: when the store's own
    /// readiness report is read. A store that cannot embed still answers, by
    /// keyword alone, so nothing else notices.
    pub health_cron: String,
}

/// What a turn is told before it is drafted, read exactly rather than
/// searched for. Zero turns either source off.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryConfig {
    /// `MEMORY_EXCHANGES`, 0..=50, default 6: the last answered mentions from
    /// the person, from the mention queue.
    pub exchanges: u32,
    /// `MEMORY_OWN_POSTS_HOURS`, 0..=336, default 72.
    pub own_posts_hours: u32,
    /// `MEMORY_OWN_POSTS_LIMIT`, 0..=50, default 8: the account's own posts,
    /// from the post ledger.
    pub own_posts_limit: u32,
}

/// The learnings brief: a short document the account rewrites about itself
/// once a day, and which every prompt then carries next to SOUL.md and
/// CONTEXT.md.
#[derive(Debug, Clone, PartialEq)]
pub struct LearningsConfig {
    /// `LEARNINGS_ENABLED`, default on: one model call a day.
    pub enabled: bool,
    /// `LEARNINGS_PATH`, default `data/memory/LEARNINGS.md`; earlier briefs
    /// are kept under `history/` beside it.
    pub path: String,
    /// `LEARNINGS_CRON`, default `0 5 * * *`, before the morning post.
    pub cron: String,
    /// `LEARNINGS_MAX_CHARS`, 1000..=40000, default 10000.
    pub max_chars: u32,
    /// `LEARNINGS_DAYS`, 1..=60, default 7: how far back the record is read.
    pub days: u32,
}

/// The chat model, through the ladder gateway.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    /// `LLM_BASE_URL`, default `https://api.openai.com/v1`.
    pub base_url: String,
    /// `LLM_API_KEY`, required.
    pub api_key: Secret,
    /// `LLM_MODEL`, required.
    pub model: String,
}

/// The agent, its persona and who it is.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    /// `AGENT_MODE`, default `langgraph`.
    pub mode: AgentMode,
    /// `AGENT_MAX_STEPS`, 1..=100, default 24.
    pub max_steps: u32,
    /// `AGENT_API_TOKEN`, optional, at least 16 characters: the bearer for
    /// `/agent/run`. Unset, the endpoint is closed.
    pub api_token: Option<Secret>,
    /// `EXA_API_KEY`, optional: web and news search.
    pub exa_api_key: Option<Secret>,
    /// `SOUL_PATH`, default `SOUL.md`.
    pub soul_path: String,
    /// `CONTEXT_PATH`, default `CONTEXT.md`.
    pub context_path: String,
    /// `SKILLS_DIR`, default `skills`.
    pub skills_dir: String,
    /// `MAX_EMOJI_PER_POST`, 0..=10, default 1.
    pub max_emoji_per_post: u32,
    /// `OWN_TOKEN_ADDRESSES`, comma separated, default none.
    pub own_token_addresses: Vec<String>,
    /// `TRUSTED_HANDLES`, comma separated, default
    /// `TinyHumansAISol,senamakel,enamakel,tinyhumansai`.
    pub trusted_handles: Vec<String>,
    /// `SELF_NAMES`, comma separated, default `Teeny`.
    pub self_names: Vec<String>,
}

/// The content filter and the ranker's reader.
#[derive(Debug, Clone, PartialEq)]
pub struct SafetyConfig {
    /// `SAFETY_JUDGE`, default on.
    pub judge: bool,
    /// `SAFETY_JUDGE_MODEL`, optional; unset rides on the chat model.
    pub judge_model: Option<String>,
    /// `ALGO_CRITIC`, default on.
    pub algo_critic: bool,
    /// `ALGO_CRITIC_MODEL`, default `flash`; blank means the default.
    pub algo_critic_model: String,
    /// `LINK_ALLOWLIST`, comma separated hosts beyond the built-in list.
    pub link_allowlist: Vec<String>,
}

/// Answering mentions: ingest, the queue worker and the reply budget.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyConfig {
    /// `REPLY_CRON`, default `*/5 * * * *`.
    pub cron: String,
    /// `INGEST_CRON`, default `*/5 * * * *`.
    pub ingest_cron: String,
    /// `MAX_REPLIES_PER_RUN`, 1..=100, default 5.
    pub max_per_run: u32,
    /// `REPLY_MODE`, default `worker`.
    pub mode: ReplyMode,
    /// `REPLY_IDLE_MS`, 250..=60000, default 2000.
    pub idle_ms: u64,
    /// `REPLY_MIN_INTERVAL_MS`, 0..=3600000, default 30000. Zero disables pacing.
    pub min_interval_ms: u64,
    /// `REPLY_MAX_INTERVAL_MS`, 0..=3600000, default 60000.
    pub max_interval_ms: u64,
    /// `REPLY_AUTO_PER_POST`, 0..=100, default 10.
    pub auto_per_post: u32,
    /// `REPLY_SMART_PER_POST`, 0..=100, default 10.
    pub smart_per_post: u32,
    /// `REPLY_LIKE_MENTIONS`, default on.
    pub like_mentions: bool,
}

/// The gap between the account's own posts, and the heat job.
#[derive(Debug, Clone, PartialEq)]
pub struct CadenceConfig {
    /// `POST_MIN_GAP_HOURS`, 0..=48, default 7: three posts a day, eight
    /// hours apart, less an hour so a slow job that posts late does not cost
    /// the next slot its turn.
    pub post_min_gap_hours: f64,
    /// `POST_GAP_TOLERANCE_MINUTES`, 0..=120, default 20.
    pub post_gap_tolerance_minutes: f64,
    /// `HEAT_ENABLED`, default on.
    pub heat_enabled: bool,
    /// `HEAT_CRON`, default `*/20 * * * *`.
    pub heat_cron: String,
    /// `HOT_IMPRESSIONS_PER_HOUR`, at least 1, default 500.
    pub hot_impressions_per_hour: f64,
    /// `HOT_BASELINE_MULTIPLE`, at least 1, default 3.
    pub hot_baseline_multiple: f64,
    /// `HOT_MAX_HOLD_SLOTS`, 1..=6, default 2.
    pub hot_max_hold_slots: u32,
}

/// Mongo.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreConfig {
    /// `MONGO_URI`, default `mongodb://mongo:27017`.
    pub mongo_uri: String,
    /// `MONGO_DATABASE`, default `teeny`.
    pub mongo_database: String,
    /// `MENTIONS_COLLECTION`, default `mentions`.
    pub mentions_collection: String,
    /// `HOLDS_COLLECTION`, default `holds`.
    pub holds_collection: String,
    /// `TRANSCRIPT_COLLECTION`, default `telegram_transcript`.
    pub transcript_collection: String,
}

/// The brain's checkout, read-only.
#[derive(Debug, Clone, PartialEq)]
pub struct BrainConfig {
    /// `BRAIN_DATA_DIR`, default `brain`; blank serves neither the usage fold
    /// nor `/brain`.
    pub data_dir: Option<String>,
    /// `BRAIN_COMPANY`, default `teeny`.
    pub company: String,
    /// `BRAIN_USAGE_CRON`, default `* * * * *`.
    pub usage_cron: String,
}

/// The embedded OpenHuman runtime: where it keeps its state, and its
/// research browser. None of these are legacy variables.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenHumanConfig {
    /// `TEENY_DATA`, default `data`: the volume root. OpenHuman's workspace,
    /// the Chrome profile and the downloads live below it.
    pub data_dir: String,
    /// `BROWSER_CHROME_PATH`, optional; unset, the browser finds Chrome itself.
    pub chrome_path: Option<String>,
    /// `BROWSE_UNATTENDED_ACTIONS`, comma separated, default `click,press`:
    /// the gated browser actions a trusted scheduled turn may take without an
    /// approval it cannot get. Blank allows none.
    pub unattended_actions: Vec<String>,
    /// `BROWSE_ALLOWED_DOMAINS`, comma separated, default none: the hosts the
    /// browser may open. X is never among them, whatever is listed: the
    /// account reaches X through its own API only.
    pub allowed_domains: Vec<String>,
    /// `TINYCOMPUTER_MODULE_PATH`, optional: a locally built
    /// `libtinycomputer.so` loaded in place of the pinned release.
    pub tinycomputer_module_path: Option<String>,
}
