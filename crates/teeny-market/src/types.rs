//! Market data, kept deliberately small.
//!
//! Every field here is something a chart or a piece of technical analysis
//! actually consumes. The upstream APIs return a great deal more; carrying it
//! through would mean the difference between two providers showing up in the
//! renderer rather than in the adapter that knows about it.
//!
//! Serialised in the TypeScript's camelCase, with absent values left out, so
//! the JSON is the shape the legacy service produced.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// One OHLCV bar. `time` is unix seconds at the bar's open.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A candle size the tools accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "12h")]
    H12,
    #[serde(rename = "1d")]
    D1,
    #[serde(rename = "1w")]
    W1,
}

/// How a timeframe is spelled for GeckoTerminal, which splits it in two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeframeSpec {
    /// `minute`, `hour` or `day`.
    pub unit: &'static str,
    pub aggregate: u32,
    pub seconds: i64,
    /// Fetch this timeframe instead and fold `factor` bars into one.
    ///
    /// Weekly candles do not exist upstream: GeckoTerminal's daily endpoint
    /// accepts `aggregate=1` and nothing else. Folding seven daily bars into
    /// one is the same arithmetic the exchange would do, so the weekly is real
    /// rather than approximate, built from whatever daily history exists.
    pub fold_from: Option<FoldFrom>,
}

/// See [`TimeframeSpec::fold_from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldFrom {
    pub timeframe: Timeframe,
    pub factor: u32,
}

impl Timeframe {
    /// Every timeframe, shortest first.
    pub const ALL: [Timeframe; 7] = [
        Self::M5,
        Self::M15,
        Self::H1,
        Self::H4,
        Self::H12,
        Self::D1,
        Self::W1,
    ];

    /// `5m`, `1h`, `1w`...
    pub fn as_str(self) -> &'static str {
        match self {
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H4 => "4h",
            Self::H12 => "12h",
            Self::D1 => "1d",
            Self::W1 => "1w",
        }
    }

    /// The TypeScript's `TIMEFRAMES[timeframe]`.
    pub fn spec(self) -> TimeframeSpec {
        let (unit, aggregate, seconds) = match self {
            Self::M5 => ("minute", 5, 300),
            Self::M15 => ("minute", 15, 900),
            Self::H1 => ("hour", 1, 3_600),
            Self::H4 => ("hour", 4, 14_400),
            Self::H12 => ("hour", 12, 43_200),
            Self::D1 => ("day", 1, 86_400),
            Self::W1 => ("day", 1, 604_800),
        };
        TimeframeSpec {
            unit,
            aggregate,
            seconds,
            fold_from: (self == Self::W1).then_some(FoldFrom {
                timeframe: Self::D1,
                factor: 7,
            }),
        }
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Timeframe {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|timeframe| timeframe.as_str() == value)
            .ok_or_else(|| format!("unknown timeframe {value:?}"))
    }
}

impl From<teeny_core::config::ChartTimeframe> for Timeframe {
    fn from(value: teeny_core::config::ChartTimeframe) -> Self {
        // The config's spellings are this enum's, one for one.
        value.as_str().parse().unwrap_or(Self::H1)
    }
}

/// Folds a series into longer bars: open of the first, close of the last, the
/// extremes of the whole bucket, and volume summed.
///
/// Bucketed on absolute time rather than by counting back from the newest bar,
/// so a weekly candle covers the same seven days whenever it is asked for.
pub fn fold(candles: &[Candle], seconds: i64) -> Vec<Candle> {
    let mut buckets: BTreeMap<i64, Vec<Candle>> = BTreeMap::new();
    for candle in candles {
        let key = candle.time.div_euclid(seconds) * seconds;
        buckets.entry(key).or_default().push(*candle);
    }
    buckets
        .into_iter()
        .filter_map(|(time, group)| {
            let first = group.first()?;
            let last = group.last()?;
            Some(Candle {
                time,
                open: first.open,
                close: last.close,
                high: group
                    .iter()
                    .map(|c| c.high)
                    .fold(f64::NEG_INFINITY, f64::max),
                low: group.iter().map(|c| c.low).fold(f64::INFINITY, f64::min),
                volume: group.iter().fold(0.0, |total, c| total + c.volume),
            })
        })
        .collect()
}

/// Price change by window, in percent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PriceChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m5: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h1: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h6: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h24: Option<f64>,
}

impl PriceChange {
    /// The windows that have a value, shortest first.
    pub fn entries(&self) -> Vec<(&'static str, f64)> {
        [
            ("m5", self.m5),
            ("h1", self.h1),
            ("h6", self.h6),
            ("h24", self.h24),
        ]
        .into_iter()
        .filter_map(|(window, value)| value.map(|value| (window, value)))
        .collect()
    }
}

/// Buys and sells over a day.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Txns {
    pub buys: f64,
    pub sells: f64,
}

/// A token as the market sees it right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenOverview {
    pub address: String,
    pub chain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub price_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_cap_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fdv_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidity_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume24h_usd: Option<f64>,
    #[serde(default)]
    pub price_change: PriceChange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txns24h: Option<Txns>,
    /// The pool worth charting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair_created_at: Option<String>,
    /// Which provider answered, so a post can never cite a number without one.
    pub source: String,
}

/// Which way a swap went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeKind {
    Buy,
    Sell,
}

/// A single swap, with the wallet that made it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    pub tx_hash: String,
    pub timestamp: String,
    pub kind: TradeKind,
    pub wallet: String,
    pub volume_usd: f64,
    pub token_amount: f64,
    pub price_usd: f64,
}

/// One token account and its share of supply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HolderAccount {
    pub address: String,
    pub amount: f64,
    /// Share of circulating supply, 0-1.
    pub share: f64,
}

/// Supply and concentration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HolderSnapshot {
    pub total_supply: f64,
    pub decimals: u32,
    /// Largest token accounts, biggest first. These are token accounts, not owners.
    pub top: Vec<HolderAccount>,
    pub top10_share: f64,
    pub source: String,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
