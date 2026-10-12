//! The rest of the market, for context.
//!
//! A number on its own says very little: 14% is a quiet day in a bull run and
//! a remarkable one on a day the whole market is red. This is what lets a post
//! say the second thing.

use serde::{Deserialize, Serialize};

use crate::error::{MarketError, Result};
use crate::format::{js_number, round_to};
use crate::http::{JsonClient, num};

/// One major, by symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub symbol: String,
    pub price_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change24h_pct: Option<f64>,
}

/// The whole market, last 24 hours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_market_cap_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_change24h_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub btc_dominance_pct: Option<f64>,
    pub assets: Vec<Asset>,
    pub source: String,
}

/// One comparison, in percentage points. Positive is beating it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub what: String,
    pub points: f64,
    pub their_change_pct: f64,
}

/// Which way the whole market went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketDirection {
    Up,
    Down,
    Flat,
}

impl MarketDirection {
    /// `up`, `down`, `flat`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Flat => "flat",
        }
    }
}

/// How the token did against the rest of the market over the same 24 hours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelativeStrength {
    pub against: Vec<Comparison>,
    /// The comparisons that flatter, best first. Empty when the token lagged
    /// everything.
    pub beating: Vec<String>,
    pub market_direction: MarketDirection,
}

/// The token against the market, in percentage points.
///
/// Points rather than ratios: "+15 points on SOL" is a sentence a person can
/// read. Only comparisons the token won are offered as sentences; there is no
/// reason to hand the model the ones where it lost.
pub fn relative_strength(token_change24h_pct: f64, snapshot: &GlobalSnapshot) -> RelativeStrength {
    let compare = |what: &str, theirs: f64| Comparison {
        what: what.to_owned(),
        points: round_to(token_change24h_pct - theirs, 1),
        their_change_pct: round_to(theirs, 1),
    };
    let mut against: Vec<Comparison> = snapshot
        .assets
        .iter()
        .filter_map(|asset| {
            asset
                .change24h_pct
                .map(|theirs| compare(&asset.symbol, theirs))
        })
        .collect();
    if let Some(total) = snapshot.total_change24h_pct {
        against.push(compare("the whole crypto market", total));
    }

    let mut winning: Vec<&Comparison> = against.iter().filter(|entry| entry.points > 0.0).collect();
    winning.sort_by(|a, b| b.points.total_cmp(&a.points));
    let beating = winning
        .into_iter()
        .map(|entry| {
            format!(
                "outperforming {what} by {points} points over 24h ({what} {sign}{theirs}%)",
                what = entry.what,
                points = js_number(entry.points),
                sign = if entry.their_change_pct >= 0.0 {
                    "+"
                } else {
                    ""
                },
                theirs = js_number(entry.their_change_pct),
            )
        })
        .collect();

    let direction = snapshot
        .total_change24h_pct
        .or_else(|| {
            snapshot
                .assets
                .first()
                .and_then(|asset| asset.change24h_pct)
        })
        .unwrap_or(0.0);
    RelativeStrength {
        against,
        beating,
        market_direction: if direction > 0.5 {
            MarketDirection::Up
        } else if direction < -0.5 {
            MarketDirection::Down
        } else {
            MarketDirection::Flat
        },
    }
}

/// Where the rest of the market comes from. [`GlobalMarketClient`] is the real
/// one; a test or another source can stand in for it.
#[async_trait::async_trait]
pub trait GlobalSource: Send + Sync {
    /// The majors and the whole market, last 24 hours.
    async fn snapshot(&self) -> Result<GlobalSnapshot>;
}

#[async_trait::async_trait]
impl GlobalSource for GlobalMarketClient {
    async fn snapshot(&self) -> Result<GlobalSnapshot> {
        GlobalMarketClient::snapshot(self).await
    }
}

const BASE: &str = "https://api.coingecko.com/api/v3";

/// CoinGecko's global and simple-price endpoints.
#[derive(Debug, Clone)]
pub struct GlobalMarketClient {
    http: JsonClient,
    base: String,
}

impl GlobalMarketClient {
    /// The public API.
    pub fn new(http: JsonClient) -> Self {
        Self {
            http,
            base: BASE.to_owned(),
        }
    }

    /// Somewhere else, such as a test server.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// BTC, ETH and SOL, total market cap and dominance. Either endpoint may
    /// fail; only both failing is an error.
    pub async fn snapshot(&self) -> Result<GlobalSnapshot> {
        let global_url = format!("{}/global", self.base);
        let prices_url = format!(
            "{}/simple/price?ids=bitcoin,solana,ethereum&vs_currencies=usd&include_24hr_change=true",
            self.base
        );
        let (global, prices) = tokio::join!(
            self.http.get(&global_url, &[]),
            self.http.get(&prices_url, &[])
        );
        let (global, prices) = (global.ok(), prices.ok());
        if global.is_none() && prices.is_none() {
            return Err(MarketError::data(
                "No global market data is available right now",
            ));
        }

        let data = global.as_ref().and_then(|global| global.get("data"));
        let field = |pointer: &str| num(data.and_then(|data| data.pointer(pointer)));
        let assets = [("bitcoin", "BTC"), ("ethereum", "ETH"), ("solana", "SOL")]
            .into_iter()
            .filter_map(|(id, symbol)| {
                let entry = prices.as_ref()?.get(id)?;
                Some(Asset {
                    symbol: symbol.to_owned(),
                    price_usd: num(entry.get("usd"))?,
                    change24h_pct: num(entry.get("usd_24h_change")),
                })
            })
            .collect();

        Ok(GlobalSnapshot {
            total_market_cap_usd: field("/total_market_cap/usd"),
            total_change24h_pct: field("/market_cap_change_percentage_24h_usd"),
            btc_dominance_pct: field("/market_cap_percentage/btc"),
            assets,
            source: "coingecko".into(),
        })
    }
}

#[cfg(test)]
#[path = "global_tests.rs"]
mod tests;
