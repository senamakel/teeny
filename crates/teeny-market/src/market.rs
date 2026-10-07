//! One place to ask about a token.
//!
//! Several providers sit behind it because none of them answers everything:
//! DexScreener has the best snapshot and no candles, GeckoTerminal has candles
//! and per-trade wallets, and an RPC has supply and concentration. GMGN would
//! have all of it and is unreachable from a server, so it is tried first only
//! when it has been given a base URL that works.
//!
//! Every method degrades rather than failing where it sensibly can: a chart
//! post that loses the holder breakdown is still a chart post, and the agent is
//! told what is missing rather than handed a plausible-looking zero.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use teeny_core::config::{ChartConfig, KolWallet};

use crate::dexscreener::DexscreenerClient;
use crate::error::{MarketError, Result};
use crate::geckoterminal::GeckoTerminalClient;
use crate::gmgn::{GmgnClient, RankedTrader};
use crate::http::JsonClient;
use crate::solana::SolanaRpc;
use crate::types::{Candle, HolderSnapshot, Timeframe, TokenOverview, Trade, TradeKind, fold};

/// One wallet's footprint in the trades window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletActivity {
    pub wallet: String,
    /// The watchlist's name for it. For the agent's reasoning, never a post.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub buys: usize,
    pub sells: usize,
    pub bought_usd: f64,
    pub sold_usd: f64,
    /// Bought minus sold, in the window read. Positive is accumulation.
    pub net_usd: f64,
    pub first_seen: String,
    pub last_seen: String,
}

/// How far back the trades read actually go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowWindow {
    pub from: String,
    pub to: String,
    pub trades: usize,
}

/// Who is doing what in the recent trades window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowSummary {
    /// Everything here is about this window only.
    pub window: FlowWindow,
    pub buy_volume_usd: f64,
    pub sell_volume_usd: f64,
    pub net_usd: f64,
    pub unique_wallets: usize,
    pub buyers: usize,
    pub sellers: usize,
    pub top_buyers: Vec<WalletActivity>,
    pub top_sellers: Vec<WalletActivity>,
    /// Watchlist wallets seen trading in the window.
    pub known: Vec<WalletActivity>,
}

/// What the market tools need to be able to ask. [`TokenMarket`] is the real
/// one; a test or another source can stand in for it.
#[async_trait]
pub trait MarketData: Send + Sync {
    /// The live snapshot.
    async fn overview(&self, address: &str) -> Result<TokenOverview>;
    /// `limit` bars of `timeframe`, oldest first.
    async fn candles(&self, address: &str, timeframe: Timeframe, limit: u32)
    -> Result<Vec<Candle>>;
    /// Supply and concentration.
    async fn holders(&self, address: &str) -> Result<HolderSnapshot>;
    /// The recent trades over `min_volume_usd`, aggregated.
    async fn flow(&self, address: &str, min_volume_usd: f64) -> Result<FlowSummary>;
    /// One wallet's footprint in the same window; `None` when it did not trade.
    async fn wallet(
        &self,
        address: &str,
        wallet: &str,
        min_volume_usd: f64,
    ) -> Result<Option<WalletActivity>>;
    /// Ranked traders, when a source for them is configured. Empty when not.
    async fn ranked_traders(&self, address: &str) -> Vec<RankedTrader>;
}

/// The clients behind a [`TokenMarket`].
#[derive(Debug, Clone)]
pub struct Providers {
    pub gecko: GeckoTerminalClient,
    pub dexscreener: DexscreenerClient,
    /// Only on Solana.
    pub solana: Option<SolanaRpc>,
    /// Only when `GMGN_BASE_URL` names something that answers.
    pub gmgn: Option<GmgnClient>,
}

/// How to build a [`TokenMarket`] with the public endpoints.
#[derive(Debug, Clone)]
pub struct MarketOptions {
    /// `solana` unless told otherwise.
    pub network: String,
    pub solana_rpc_url: Option<String>,
    pub gmgn_base_url: Option<String>,
    /// Wallets worth naming when they show up in the flow.
    pub watchlist: Vec<KolWallet>,
    /// How long a snapshot or a series may be reused. A daily post does not
    /// need a live tick.
    pub cache_ttl: Duration,
    /// The deadline per provider call.
    pub timeout: Duration,
}

impl Default for MarketOptions {
    fn default() -> Self {
        Self {
            network: "solana".into(),
            solana_rpc_url: None,
            gmgn_base_url: None,
            watchlist: Vec::new(),
            cache_ttl: Duration::from_secs(60),
            timeout: Duration::from_secs(15),
        }
    }
}

struct Cached<T> {
    at: Instant,
    value: T,
}

/// The market for any token, with every provider behind it.
pub struct TokenMarket {
    providers: Providers,
    /// Lowercased wallet to label.
    watchlist: HashMap<String, String>,
    ttl: Duration,
    overviews: Mutex<HashMap<String, Cached<TokenOverview>>>,
    /// Candles by `address:timeframe:limit`. The agent reads four timeframes
    /// and then renders one of them, and GeckoTerminal's free tier answers 429
    /// well before that many calls.
    series: Mutex<HashMap<String, Cached<Vec<Candle>>>>,
}

impl std::fmt::Debug for TokenMarket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenMarket")
            .field("providers", &self.providers)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl TokenMarket {
    /// The public endpoints, as `options` describe.
    pub fn new(options: MarketOptions) -> Self {
        let http = JsonClient::new().with_timeout(options.timeout);
        let providers = Providers {
            gecko: GeckoTerminalClient::new(http.clone(), options.network.clone()),
            dexscreener: DexscreenerClient::new(http.clone()),
            solana: (options.network == "solana")
                .then(|| SolanaRpc::new(http.clone(), options.solana_rpc_url.clone())),
            gmgn: options
                .gmgn_base_url
                .as_ref()
                .map(|base| GmgnClient::new(http.clone(), base.clone())),
        };
        Self::with_providers(providers, &options.watchlist, options.cache_ttl)
    }

    /// The chart agent's market: `CHART_CHAIN`, `SOLANA_RPC_URL`,
    /// `GMGN_BASE_URL` and `KOL_WALLETS`.
    pub fn from_config(chart: &ChartConfig) -> Self {
        Self::new(MarketOptions {
            network: chart.chain.clone(),
            solana_rpc_url: chart.solana_rpc_url.clone(),
            gmgn_base_url: chart.gmgn_base_url.clone(),
            watchlist: chart.kol_wallets.clone(),
            ..MarketOptions::default()
        })
    }

    /// These clients, this watchlist, this cache lifetime.
    pub fn with_providers(providers: Providers, watchlist: &[KolWallet], ttl: Duration) -> Self {
        Self {
            providers,
            watchlist: watchlist
                .iter()
                .map(|entry| (entry.wallet.to_lowercase(), entry.label.clone()))
                .collect(),
            ttl,
            overviews: Mutex::new(HashMap::new()),
            series: Mutex::new(HashMap::new()),
        }
    }

    /// Which providers are wired up, so the agent never claims a source it lacks.
    pub fn sources(&self) -> Vec<&'static str> {
        let mut sources = Vec::new();
        if self.providers.gmgn.is_some() {
            sources.push("gmgn");
        }
        sources.extend(["dexscreener", "geckoterminal"]);
        if self.providers.solana.is_some() {
            sources.push("solana-rpc");
        }
        sources
    }

    /// The pool a chart should be drawn from.
    pub async fn pool(&self, address: &str) -> Result<String> {
        self.overview(address)
            .await?
            .pool_address
            .ok_or_else(|| MarketError::data(format!("No pool known for {address}")))
    }

    /// The recent trades over a size floor, from GeckoTerminal.
    pub async fn trades(&self, address: &str, min_volume_usd: f64) -> Result<Vec<Trade>> {
        let pool = self.pool(address).await?;
        self.providers.gecko.trades(&pool, min_volume_usd).await
    }

    async fn fetch_candles(
        &self,
        address: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        if let Some(gmgn) = &self.providers.gmgn
            && let Ok(candles) = gmgn.candles(address, timeframe, limit).await
            && !candles.is_empty()
        {
            return Ok(candles);
        }
        let pool = self.pool(address).await?;
        self.providers.gecko.candles(&pool, timeframe, limit).await
    }

    fn fresh<T: Clone>(&self, cache: &Mutex<HashMap<String, Cached<T>>>, key: &str) -> Option<T> {
        let cache = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .get(key)
            .filter(|entry| entry.at.elapsed() < self.ttl)
            .map(|entry| entry.value.clone())
    }

    fn remember<T>(cache: &Mutex<HashMap<String, Cached<T>>>, key: String, value: T) {
        let mut cache = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.insert(
            key,
            Cached {
                at: Instant::now(),
                value,
            },
        );
    }
}

#[async_trait]
impl MarketData for TokenMarket {
    /// The snapshot, from whichever provider answers, filled in from the
    /// others: merged rather than picked, because the gaps do not overlap.
    async fn overview(&self, address: &str) -> Result<TokenOverview> {
        if let Some(cached) = self.fresh(&self.overviews, address) {
            return Ok(cached);
        }
        let gmgn = async {
            match &self.providers.gmgn {
                Some(gmgn) => gmgn.overview(address).await,
                None => Err(MarketError::data("gmgn not configured")),
            }
        };
        let (gmgn, dexscreener, gecko) = futures::join!(
            gmgn,
            self.providers.dexscreener.overview(address),
            self.providers.gecko.overview(address)
        );
        let attempts = [gmgn, dexscreener, gecko];

        let mut answered = attempts.iter().filter_map(|attempt| attempt.as_ref().ok());
        let Some(primary) = answered.next() else {
            let why: Vec<String> = attempts
                .iter()
                .filter_map(|attempt| attempt.as_ref().err())
                .map(ToString::to_string)
                .collect();
            return Err(MarketError::data(format!(
                "No market source could price {address}: {}",
                why.join("; ")
            )));
        };
        let merged = answered.fold(primary.clone(), fill);
        Self::remember(&self.overviews, address.to_owned(), merged.clone());
        Ok(merged)
    }

    async fn candles(
        &self,
        address: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let key = format!("{address}:{timeframe}:{limit}");
        if let Some(cached) = self.fresh(&self.series, &key) {
            return Ok(cached);
        }
        // A timeframe the provider does not serve is built from one it does,
        // with `factor` source bars per output bar plus a little slack, so the
        // oldest bucket is not a fragment of a week presented as a week.
        let value = match timeframe.spec().fold_from {
            Some(from) => {
                let wanted = (limit * from.factor + from.factor).min(1_000);
                let source = Box::pin(self.candles(address, from.timeframe, wanted)).await?;
                let folded = fold(&source, timeframe.spec().seconds);
                folded[folded.len().saturating_sub(limit as usize)..].to_vec()
            }
            None => self.fetch_candles(address, timeframe, limit).await?,
        };
        Self::remember(&self.series, key, value.clone());
        Ok(value)
    }

    async fn holders(&self, address: &str) -> Result<HolderSnapshot> {
        match &self.providers.solana {
            Some(solana) => solana.holders(address).await,
            None => Err(MarketError::data("Holder data is only wired up for Solana")),
        }
    }

    /// A window, not a history: the provider returns the most recent few
    /// hundred swaps over the size floor and nothing older.
    async fn flow(&self, address: &str, min_volume_usd: f64) -> Result<FlowSummary> {
        let trades = self.trades(address, min_volume_usd).await?;
        Ok(summarise_flow(&trades, &self.watchlist))
    }

    async fn wallet(
        &self,
        address: &str,
        wallet: &str,
        min_volume_usd: f64,
    ) -> Result<Option<WalletActivity>> {
        let target = wallet.to_lowercase();
        let mine: Vec<Trade> = self
            .trades(address, min_volume_usd)
            .await?
            .into_iter()
            .filter(|trade| trade.wallet.to_lowercase() == target)
            .collect();
        Ok((!mine.is_empty()).then(|| wallet_activity(wallet, &mine, &self.watchlist)))
    }

    async fn ranked_traders(&self, address: &str) -> Vec<RankedTrader> {
        match &self.providers.gmgn {
            Some(gmgn) => gmgn.top_traders(address).await.unwrap_or_default(),
            None => Vec::new(),
        }
    }
}

/// The trades window, aggregated by wallet. `watchlist` maps a lowercased
/// wallet to its label.
pub fn summarise_flow(trades: &[Trade], watchlist: &HashMap<String, String>) -> FlowSummary {
    // Grouped in order of first appearance, as the TypeScript's Map was.
    let mut order: Vec<String> = Vec::new();
    let mut by_wallet: HashMap<String, Vec<Trade>> = HashMap::new();
    for trade in trades {
        let key = trade.wallet.to_lowercase();
        if !by_wallet.contains_key(&key) {
            order.push(key.clone());
        }
        by_wallet.entry(key).or_default().push(trade.clone());
    }
    let wallets: Vec<WalletActivity> = order
        .iter()
        .map(|key| {
            let group = &by_wallet[key];
            wallet_activity(&group[0].wallet, group, watchlist)
        })
        .collect();

    let (from, to) = span(trades);
    let buy_volume = volume(trades, TradeKind::Buy);
    let sell_volume = volume(trades, TradeKind::Sell);
    let ranked = |amount: fn(&WalletActivity) -> f64| -> Vec<WalletActivity> {
        let mut sorted = wallets.clone();
        sorted.sort_by(|a, b| amount(b).total_cmp(&amount(a)));
        sorted
            .into_iter()
            .filter(|wallet| amount(wallet) > 0.0)
            .take(5)
            .collect()
    };

    FlowSummary {
        window: FlowWindow {
            from,
            to,
            trades: trades.len(),
        },
        buy_volume_usd: buy_volume,
        sell_volume_usd: sell_volume,
        net_usd: buy_volume - sell_volume,
        unique_wallets: wallets.len(),
        buyers: wallets.iter().filter(|wallet| wallet.buys > 0).count(),
        sellers: wallets.iter().filter(|wallet| wallet.sells > 0).count(),
        top_buyers: ranked(|wallet| wallet.bought_usd),
        top_sellers: ranked(|wallet| wallet.sold_usd),
        known: wallets
            .iter()
            .filter(|wallet| wallet.label.is_some())
            .cloned()
            .collect(),
    }
}

/// One wallet's trades, totted up.
pub fn wallet_activity(
    wallet: &str,
    trades: &[Trade],
    watchlist: &HashMap<String, String>,
) -> WalletActivity {
    let (first_seen, last_seen) = span(trades);
    let bought = volume(trades, TradeKind::Buy);
    let sold = volume(trades, TradeKind::Sell);
    WalletActivity {
        wallet: wallet.to_owned(),
        label: watchlist
            .get(&wallet.to_lowercase())
            .filter(|label| !label.is_empty())
            .cloned(),
        buys: trades
            .iter()
            .filter(|trade| trade.kind == TradeKind::Buy)
            .count(),
        sells: trades
            .iter()
            .filter(|trade| trade.kind == TradeKind::Sell)
            .count(),
        bought_usd: bought,
        sold_usd: sold,
        net_usd: bought - sold,
        first_seen,
        last_seen,
    }
}

/// The earliest and latest timestamps, compared as strings as the TypeScript did.
fn span(trades: &[Trade]) -> (String, String) {
    let mut times: Vec<&str> = trades
        .iter()
        .map(|trade| trade.timestamp.as_str())
        .collect();
    times.sort_unstable();
    (
        times
            .first()
            .map(|time| (*time).to_owned())
            .unwrap_or_default(),
        times
            .last()
            .map(|time| (*time).to_owned())
            .unwrap_or_default(),
    )
}

fn volume(trades: &[Trade], kind: TradeKind) -> f64 {
    trades
        .iter()
        .filter(|trade| trade.kind == kind)
        .fold(0.0, |total, trade| total + trade.volume_usd)
}

/// Copies across only the fields the first provider left blank.
fn fill(into: TokenOverview, extra: &TokenOverview) -> TokenOverview {
    let change = &extra.price_change;
    TokenOverview {
        name: into.name.or_else(|| extra.name.clone()),
        symbol: into.symbol.or_else(|| extra.symbol.clone()),
        market_cap_usd: into.market_cap_usd.or(extra.market_cap_usd),
        fdv_usd: into.fdv_usd.or(extra.fdv_usd),
        liquidity_usd: into.liquidity_usd.or(extra.liquidity_usd),
        volume24h_usd: into.volume24h_usd.or(extra.volume24h_usd),
        price_change: crate::types::PriceChange {
            m5: into.price_change.m5.or(change.m5),
            h1: into.price_change.h1.or(change.h1),
            h6: into.price_change.h6.or(change.h6),
            h24: into.price_change.h24.or(change.h24),
        },
        txns24h: into.txns24h.or(extra.txns24h),
        pool_address: into.pool_address.or_else(|| extra.pool_address.clone()),
        pool_name: into.pool_name.or_else(|| extra.pool_name.clone()),
        pair_created_at: into
            .pair_created_at
            .or_else(|| extra.pair_created_at.clone()),
        source: format!("{}+{}", into.source, extra.source),
        ..into
    }
}

#[cfg(test)]
#[path = "market_tests.rs"]
mod tests;
