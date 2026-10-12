//! The data behind each read-only market tool, and the text it hands the model.
//!
//! One method per tool, named after it. Each returns a serialisable report;
//! [`Report::text`] renders the exact text the TypeScript tool returned, which
//! is what the prompts were tuned against. The token is bound when the
//! [`MarketReports`] is built rather than passed per call: a chart agent that
//! can be told which contract to analyse can be told to shill a stranger's
//! contract by anything that lands in its context, and this one reads posts.

mod chart;
mod context;
mod onchain;
mod token;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use chart::{
    ChartTimeframesReport, HORIZONS, Horizon, HorizonRead, ProgressReport, TechnicalAnalysisReport,
};
pub use context::MarketContextReport;
pub use onchain::{FlowReport, HoldersReport, RankedTradersReport, TraderActivityReport};
pub use token::{CandleWindow, CandlesReport, OverviewReport};

use crate::error::Result;
use crate::global::{GlobalSource, relative_strength};
use crate::market::MarketData;
use crate::ta::analysis::analyse;
use crate::ta::progress::read_progress;
use crate::types::Timeframe;

/// What a tool hands the model.
pub trait Report: Serialize {
    /// The tool's text, as the TypeScript wrote it.
    fn text(&self) -> String;
}

/// `token_candles` and `technical_analysis` input: `timeframe` (default `1h`)
/// and `bars`, 20..=500 (default 120).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandlesInput {
    pub timeframe: Timeframe,
    pub bars: u32,
}

impl Default for CandlesInput {
    fn default() -> Self {
        Self {
            timeframe: Timeframe::H1,
            bars: 120,
        }
    }
}

/// `token_progress` input: `days`, 7..=365 (default 120).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgressInput {
    pub days: u32,
}

impl Default for ProgressInput {
    fn default() -> Self {
        Self { days: 120 }
    }
}

/// `token_flow` input: ignore trades under `minVolumeUsd`, 0..=1,000,000
/// (default 100, which filters out dust).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FlowInput {
    pub min_volume_usd: f64,
}

impl Default for FlowInput {
    fn default() -> Self {
        Self {
            min_volume_usd: 100.0,
        }
    }
}

/// `trader_activity` input: a wallet (26 to 64 characters) and a floor
/// (default 0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraderActivityInput {
    pub wallet: String,
    #[serde(default)]
    pub min_volume_usd: f64,
}

/// Why a tool input was refused, in the schema's terms.
pub fn validate_bars(bars: u32) -> std::result::Result<(), String> {
    if (20..=500).contains(&bars) {
        Ok(())
    } else {
        Err(format!("bars must be between 20 and 500, not {bars}"))
    }
}

/// See [`validate_bars`].
pub fn validate_days(days: u32) -> std::result::Result<(), String> {
    if (7..=365).contains(&days) {
        Ok(())
    } else {
        Err(format!("days must be between 7 and 365, not {days}"))
    }
}

/// See [`validate_bars`].
pub fn validate_min_volume(min_volume_usd: f64) -> std::result::Result<(), String> {
    if (0.0..=1_000_000.0).contains(&min_volume_usd) {
        Ok(())
    } else {
        Err("minVolumeUsd must be between 0 and 1,000,000".into())
    }
}

/// See [`validate_bars`].
pub fn validate_wallet(wallet: &str) -> std::result::Result<(), String> {
    let length = wallet.chars().count();
    if (26..=64).contains(&length) {
        Ok(())
    } else {
        Err("wallet must be 26 to 64 characters".into())
    }
}

/// The market tools' data, bound to one token.
#[derive(Clone)]
pub struct MarketReports {
    market: Arc<dyn MarketData>,
    global: Option<Arc<dyn GlobalSource>>,
    address: String,
    symbol: Option<String>,
}

impl std::fmt::Debug for MarketReports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketReports")
            .field("address", &self.address)
            .field("symbol", &self.symbol)
            .finish_non_exhaustive()
    }
}

impl MarketReports {
    /// Reports on `address`, with no global source until one is given.
    pub fn new(market: Arc<dyn MarketData>, address: impl Into<String>) -> Self {
        Self {
            market,
            global: None,
            address: address.into(),
            symbol: None,
        }
    }

    /// The ticker the account calls its token.
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// The rest of the market, for `market_context`.
    pub fn with_global(mut self, global: Arc<dyn GlobalSource>) -> Self {
        self.global = Some(global);
        self
    }

    /// `token_overview`: the live snapshot.
    pub async fn token_overview(&self) -> Result<OverviewReport> {
        Ok(OverviewReport(self.market.overview(&self.address).await?))
    }

    /// `token_candles`: raw OHLCV.
    pub async fn token_candles(&self, input: CandlesInput) -> Result<CandlesReport> {
        let series = self
            .market
            .candles(&self.address, input.timeframe, input.bars)
            .await?;
        Ok(CandlesReport::from_series(input.timeframe, &series))
    }

    /// `technical_analysis`: the read of one timeframe.
    pub async fn technical_analysis(&self, input: CandlesInput) -> Result<TechnicalAnalysisReport> {
        let series = self
            .market
            .candles(&self.address, input.timeframe, input.bars)
            .await?;
        Ok(TechnicalAnalysisReport {
            timeframe: input.timeframe,
            bars: series.len(),
            read: read_of(&series, input.timeframe),
        })
    }

    /// `market_context`: the token against BTC, ETH, SOL and everything.
    pub async fn market_context(&self) -> Result<MarketContextReport> {
        let Some(global) = &self.global else {
            return Ok(MarketContextReport {
                market: None,
                symbol: self.symbol.clone(),
                token_change24h_pct: None,
                strength: None,
            });
        };
        let (snapshot, token) =
            futures::join!(global.snapshot(), self.market.overview(&self.address));
        let snapshot = snapshot?;
        let change = token.ok().and_then(|token| token.price_change.h24);
        Ok(MarketContextReport {
            strength: change.map(|change| relative_strength(change, &snapshot)),
            market: Some(snapshot),
            symbol: self.symbol.clone(),
            token_change24h_pct: change,
        })
    }

    /// `chart_timeframes`: the chart at four zoom levels.
    pub async fn chart_timeframes(&self) -> ChartTimeframesReport {
        let reads = HORIZONS.iter().map(|horizon| async move {
            let series = self
                .market
                .candles(&self.address, horizon.timeframe, horizon.bars)
                .await
                .unwrap_or_default();
            HorizonRead {
                timeframe: horizon.timeframe,
                label: horizon.label.to_owned(),
                bars: series.len(),
                read: read_of(&series, horizon.timeframe),
            }
        });
        ChartTimeframesReport {
            horizons: futures::future::join_all(reads).await,
        }
    }

    /// `token_progress`: the long horizon, off daily candles.
    pub async fn token_progress(&self, input: ProgressInput) -> Result<ProgressReport> {
        let daily = self
            .market
            .candles(&self.address, Timeframe::D1, input.days)
            .await?;
        Ok(ProgressReport {
            days_read: daily.len(),
            progress: read_progress(&daily).ok(),
        })
    }

    /// `token_holders`: supply and concentration, or why there is none.
    pub async fn token_holders(&self) -> HoldersReport {
        match self.market.holders(&self.address).await {
            Ok(snapshot) => HoldersReport {
                snapshot: Some(snapshot),
                unavailable: None,
            },
            Err(error) => HoldersReport {
                snapshot: None,
                unavailable: Some(error.to_string()),
            },
        }
    }

    /// `token_flow`: who is buying and selling in the recent trades window.
    pub async fn token_flow(&self, input: FlowInput) -> Result<FlowReport> {
        let summary = self
            .market
            .flow(&self.address, input.min_volume_usd)
            .await?;
        Ok(FlowReport {
            min_volume_usd: input.min_volume_usd,
            summary,
        })
    }

    /// `trader_activity`: one wallet in the same window.
    pub async fn trader_activity(
        &self,
        input: &TraderActivityInput,
    ) -> Result<TraderActivityReport> {
        let activity = self
            .market
            .wallet(&self.address, &input.wallet, input.min_volume_usd)
            .await?;
        Ok(TraderActivityReport {
            wallet: input.wallet.clone(),
            activity,
        })
    }

    /// `ranked_traders`: KOLs and smart money, when a source for them exists.
    pub async fn ranked_traders(&self) -> RankedTradersReport {
        RankedTradersReport {
            traders: self.market.ranked_traders(&self.address).await,
        }
    }
}

/// The read of a series, when it has the 20 bars a read needs.
fn read_of(
    series: &[crate::types::Candle],
    timeframe: Timeframe,
) -> Option<crate::ta::analysis::TechnicalRead> {
    if series.len() < 20 {
        None
    } else {
        analyse(series, timeframe).ok()
    }
}

/// Wallets are shown truncated: they are for the agent's reasoning, not a post.
pub(crate) fn short(wallet: &str) -> String {
    let chars: Vec<char> = wallet.chars().collect();
    if chars.len() <= 12 {
        wallet.to_owned()
    } else {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
