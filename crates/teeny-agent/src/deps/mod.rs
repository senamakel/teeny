//! The concrete clients every belt is built from, wired from
//! [`Config`]: the store, memory, X, the content filter and the critic, the
//! cadence and the reply budget, the market, media, the ladder and Exa.
//! [`Deps::fake`] is the same set in process, for tests.
//!
//! An optional part is `None` when its configuration is absent (`EXA_API_KEY`,
//! `GIPHY_API_KEY`, `CHART_TOKEN_ADDRESS`, the media switches), and the belts
//! leave its tools off rather than offering a tool that cannot work.

mod fake;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use teeny_cadence::budget::{BudgetRules, QueueCounter, ReplyBudget};
use teeny_cadence::cadence::{Cadence, CadenceRules};
use teeny_cadence::clock::SystemClock;
use teeny_cadence::media_budget::{DailyMediaBudget, MediaLimits};
use teeny_core::Config;
use teeny_market::http::JsonClient;
use teeny_market::market::MarketData;
use teeny_market::{GlobalMarketClient, GlobalSource, MarketReports, TokenMarket};
use teeny_media::giphy::{GifSource, GiphyClient};
use teeny_media::store::MediaBudget;
use teeny_media::wiring::{MediaAccess, build_media};
use teeny_memory::{CortexMemory, Memory, MemoryError};
use teeny_safety::critic::{AlgoCritic, Critic};
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_safety::judge::{Judge, ModelJudge};
use teeny_store::mentions::{MongoMentionQueue, QueuedPost};
use teeny_store::{
    InferenceLedger, MongoHoldLog, MongoLedger, MongoPostLedger, MongoXCache, StoreError,
    XCacheEntry, XCacheKind, XCacheStore,
};
use teeny_x::XApi;
use teeny_x::cache::{CacheEntry, CacheError, CacheKind, ReadCache, XReads, read_through};
use teeny_x::model::XPost;

use crate::exa::ExaClient;
use crate::llm::LlmClient;
use crate::prompt::Persona;
use crate::tools::media::CadenceMediaBudget;

/// Why the clients could not be built.
#[derive(Debug, thiserror::Error)]
pub enum DepsError {
    #[error("the store: {0}")]
    Store(#[from] StoreError),
    #[error("memory: {0}")]
    Memory(#[from] MemoryError),
    #[error("the persona files: {0}")]
    Persona(#[from] std::io::Error),
}

/// The plain settings the tools and prompts name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `X_BOT_USERNAME`, without the `@`.
    pub bot_username: String,
    /// `DRY_RUN`.
    pub dry_run: bool,
    /// `MAX_EMOJI_PER_POST`.
    pub max_emoji: usize,
    /// `X_SEARCH_MAX_RESULTS`: posts per X search.
    pub search_limit: u32,
    /// `GIPHY_CHANNEL`.
    pub gif_channel: String,
    /// `CHART_TOKEN_SYMBOL`.
    pub token_symbol: String,
    /// `CHART_COMMUNITY_QUERY`.
    pub community_query: String,
    /// `CHART_WATERMARK`, or the handle.
    pub chart_watermark: Option<String>,
    /// `CHART_OUTPUT_DIR`.
    pub chart_output_dir: Option<PathBuf>,
}

impl Settings {
    pub fn from_config(config: &Config) -> Self {
        let bot = config.x.bot_username.trim_start_matches('@').to_owned();
        Self {
            dry_run: config.dry_run,
            max_emoji: config.agent.max_emoji_per_post as usize,
            search_limit: config.x.search_max_results,
            gif_channel: config.media.giphy_channel.clone(),
            token_symbol: config.chart.token_symbol.clone(),
            community_query: config.chart.community_query.clone(),
            chart_watermark: Some(
                config
                    .chart
                    .watermark
                    .clone()
                    .unwrap_or_else(|| format!("@{bot}")),
            ),
            chart_output_dir: (!config.chart.output_dir.is_empty())
                .then(|| PathBuf::from(&config.chart.output_dir)),
            bot_username: bot,
        }
    }
}

/// The account's own token, for the market and chart tools.
#[derive(Clone)]
pub struct TokenFeed {
    pub data: Arc<dyn MarketData>,
    /// `CHART_TOKEN_ADDRESS`: bound here, never named by the model.
    pub address: String,
    /// CoinGecko, for `market_context`.
    pub global: Option<Arc<dyn GlobalSource>>,
}

impl TokenFeed {
    /// The ten market tools' data for this token.
    pub fn reports(&self, symbol: &str) -> MarketReports {
        let reports = MarketReports::new(Arc::clone(&self.data), &self.address).with_symbol(symbol);
        match &self.global {
            Some(global) => reports.with_global(Arc::clone(global)),
            None => reports,
        }
    }
}

/// Everything a belt is built from. Every field is public so a test can
/// swap one part of [`Deps::fake`].
pub struct Deps {
    pub settings: Settings,
    /// SOUL.md and CONTEXT.md.
    pub persona: Persona,
    /// The ladder, writing to the inference ledger.
    pub llm: LlmClient,
    pub memory: Arc<dyn Memory>,
    /// Writes and uploads; posting goes through an `XPublisher` built on it.
    pub x: Arc<XApi>,
    /// Reads, through the read cache when it is on.
    pub x_reads: Arc<dyn XReads>,
    pub safety: Arc<SafetyGate>,
    /// `ALGO_CRITIC`.
    pub critic: Option<Arc<dyn Critic>>,
    pub cadence: Arc<Cadence>,
    pub reply_budget: Option<Arc<ReplyBudget>>,
    pub media: Option<MediaAccess>,
    pub media_budget: Arc<dyn MediaBudget>,
    pub gifs: Option<Arc<dyn GifSource>>,
    pub market: Option<TokenFeed>,
    pub exa: Option<Arc<ExaClient>>,
}

/// An X post as the mention queue stores it, so the reply budget can count
/// the queue. The reply worker owns the queue; the budget only reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueuedXPost(pub XPost);

impl QueuedPost for QueuedXPost {
    fn post_id(&self) -> &str {
        self.0.id.as_str()
    }

    fn author_id(&self) -> Option<&str> {
        self.0.author_id.as_ref().map(|id| id.as_str())
    }

    fn text(&self) -> &str {
        &self.0.text
    }

    fn conversation_id(&self) -> Option<&str> {
        self.0.conversation_id.as_ref().map(|id| id.as_str())
    }
}

impl Deps {
    /// The real clients. Mongo connects lazily, so this does not wait for
    /// any server.
    pub async fn from_config(config: Config) -> Result<Self, DepsError> {
        let db = teeny_store::connect(&config.store).await?;
        let ledger: Arc<dyn InferenceLedger> = Arc::new(MongoLedger::new(&db));
        let llm = LlmClient::from_config(&config.llm).with_ledger(ledger);

        let x = Arc::new(XApi::from_config(&config.x));
        let cache: Arc<dyn ReadCache> = Arc::new(StoreReadCache(Arc::new(MongoXCache::new(&db))));
        let x_reads = read_through(Arc::clone(&x), Some(cache), &config.x);

        let judge: Option<Arc<dyn Judge>> = config.safety.judge.then(|| {
            let model = config
                .safety
                .judge_model
                .as_deref()
                .unwrap_or(&config.llm.model);
            Arc::new(ModelJudge::new(Arc::new(llm.with_model(model)))) as Arc<dyn Judge>
        });
        let safety = Arc::new(SafetyGate::new(SafetyGateOptions {
            judge,
            extra_allowed_links: config.safety.link_allowlist.clone(),
            holds: Some(Arc::new(MongoHoldLog::with_collection(
                &db,
                &config.store.holds_collection,
            ))),
            ..SafetyGateOptions::default()
        }));
        let critic = config.safety.algo_critic.then(|| {
            Arc::new(AlgoCritic::new(Arc::new(
                llm.with_model(&config.safety.algo_critic_model),
            ))) as Arc<dyn Critic>
        });

        let posts = Arc::new(MongoPostLedger::new(&db));
        let cadence = Arc::new(Cadence::new(
            posts.clone(),
            CadenceRules::from_config(&config.cadence),
            Arc::new(SystemClock),
        ));
        let queue = Arc::new(MongoMentionQueue::<QueuedXPost>::with_collection(
            &db,
            &config.store.mentions_collection,
        ));
        let reply_budget = Arc::new(ReplyBudget::new(
            posts,
            Arc::new(QueueCounter::<QueuedXPost, _>::new(queue)),
            BudgetRules::from_config(&config),
            Arc::new(SystemClock),
        ));

        let market = config.chart.token_address.clone().map(|address| TokenFeed {
            data: Arc::new(TokenMarket::from_config(&config.chart)),
            address,
            global: Some(Arc::new(GlobalMarketClient::new(JsonClient::new()))),
        });
        let gifs = config.media.giphy_api_key.as_ref().map(|key| {
            Arc::new(GiphyClient::new(key.expose(), &config.media.giphy_channel))
                as Arc<dyn GifSource>
        });

        Ok(Self {
            settings: Settings::from_config(&config),
            persona: Persona::from_settings(&config.agent, &config.learnings)?,
            memory: Arc::new(CortexMemory::from_config(&config.cortex)?),
            x,
            x_reads,
            safety,
            critic,
            cadence,
            reply_budget: Some(reply_budget),
            media: build_media(&config),
            media_budget: Arc::new(CadenceMediaBudget(Arc::new(DailyMediaBudget::new(
                MediaLimits::from_config(&config.media),
                Arc::new(SystemClock),
            )))),
            gifs,
            market,
            exa: config
                .agent
                .exa_api_key
                .clone()
                .map(|key| Arc::new(ExaClient::new(key))),
            llm,
        })
    }
}

/// teeny-store's x-cache collection as the read cache in front of X, so
/// the rows the TypeScript service wrote are hits here.
pub struct StoreReadCache(pub Arc<dyn XCacheStore>);

fn to_store(kind: CacheKind) -> XCacheKind {
    match kind {
        CacheKind::Post => XCacheKind::Post,
        CacheKind::Missing => XCacheKind::Missing,
        CacheKind::Search => XCacheKind::Search,
        CacheKind::Conversation => XCacheKind::Conversation,
        CacheKind::Trends => XCacheKind::Trends,
        CacheKind::OwnCounters => XCacheKind::Metrics,
        CacheKind::Follow => XCacheKind::Follows,
    }
}

fn from_store(kind: XCacheKind) -> CacheKind {
    match kind {
        XCacheKind::Post => CacheKind::Post,
        XCacheKind::Missing => CacheKind::Missing,
        XCacheKind::Search => CacheKind::Search,
        XCacheKind::Conversation => CacheKind::Conversation,
        XCacheKind::Trends => CacheKind::Trends,
        XCacheKind::Metrics => CacheKind::OwnCounters,
        XCacheKind::Follows => CacheKind::Follow,
    }
}

#[async_trait]
impl ReadCache for StoreReadCache {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CacheError> {
        Ok(self.0.get(key).await?.map(|row| CacheEntry {
            key: row.key,
            kind: from_store(row.kind),
            value: row.value,
            stored_at: DateTime::parse_from_rfc3339(&row.stored_at)
                .map(|at| at.with_timezone(&Utc))
                .unwrap_or_default(),
            expires_at: row.expires_at,
        }))
    }

    async fn put(&self, entry: CacheEntry) -> Result<(), CacheError> {
        let row = XCacheEntry {
            key: entry.key,
            kind: to_store(entry.kind),
            value: entry.value,
            stored_at: teeny_store::iso(entry.stored_at),
            expires_at: entry.expires_at,
        };
        Ok(self.0.set(&row).await?)
    }
}

/// The variables a configuration needs to load, with placeholder values and
/// `DRY_RUN` on: what [`Deps::fake`] runs on, and what a test reads a
/// [`teeny_core::Config`] from.
pub fn fake_vars() -> Vec<(&'static str, &'static str)> {
    vec![
        ("X_BOT_USERNAME", "teenydroid"),
        ("X_API_KEY", "fake"),
        ("X_API_SECRET", "fake"),
        ("X_ACCESS_TOKEN", "fake"),
        ("X_ACCESS_TOKEN_SECRET", "fake"),
        ("CORTEX_TOKEN", "fake"),
        ("LLM_API_KEY", "fake"),
        ("LLM_MODEL", "chat"),
        ("DRY_RUN", "true"),
    ]
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
