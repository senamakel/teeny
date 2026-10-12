//! DexScreener, used for the snapshot only.
//!
//! It has the better overview -- five-minute and one-hour price change, buy
//! and sell counts -- and no OHLCV endpoint at all, so it is paired with
//! GeckoTerminal rather than replacing it.

use serde_json::Value;

use crate::error::{MarketError, Result};
use crate::format::iso_from_millis;
use crate::http::{JsonClient, num, text};
use crate::types::{PriceChange, TokenOverview, Txns};

const BASE: &str = "https://api.dexscreener.com/latest/dex";

/// The DexScreener token endpoint.
#[derive(Debug, Clone)]
pub struct DexscreenerClient {
    http: JsonClient,
    base: String,
}

impl DexscreenerClient {
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

    /// The token's snapshot, from the pool that actually trades.
    pub async fn overview(&self, address: &str) -> Result<TokenOverview> {
        let url = format!("{}/tokens/{address}", self.base);
        let response = self.http.get(&url, &[]).await?;

        // A token trades in several pools and the quiet ones print prices
        // nobody could get filled at. Ranked by 24-hour volume rather than by
        // liquidity, because they disagree, and charting the deep dead pool
        // draws a flat line through a pool nothing happens in.
        let mut pairs: Vec<&Value> = response
            .get("pairs")
            .and_then(Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .filter(|pair| {
                        pair.pointer("/baseToken/address").and_then(Value::as_str) == Some(address)
                    })
                    .collect()
            })
            .unwrap_or_default();
        pairs.sort_by(|a, b| {
            volume24h(b)
                .total_cmp(&volume24h(a))
                .then(liquidity(b).total_cmp(&liquidity(a)))
        });
        let pair = pairs
            .first()
            .ok_or_else(|| MarketError::data(format!("Dexscreener knows no pair for {address}")))?;

        let price = num(pair.get("priceUsd")).ok_or_else(|| {
            MarketError::data(format!("Dexscreener returned no price for {address}"))
        })?;
        let change = |window: &str| num(pair.pointer(&format!("/priceChange/{window}")));
        let buys = num(pair.pointer("/txns/h24/buys"));
        let sells = num(pair.pointer("/txns/h24/sells"));

        Ok(TokenOverview {
            address: address.to_owned(),
            chain: text(pair.get("chainId")).unwrap_or_else(|| "unknown".into()),
            name: text(pair.pointer("/baseToken/name")),
            symbol: text(pair.pointer("/baseToken/symbol")),
            price_usd: price,
            market_cap_usd: num(pair.get("marketCap")),
            fdv_usd: num(pair.get("fdv")),
            liquidity_usd: num(pair.pointer("/liquidity/usd")),
            volume24h_usd: num(pair.pointer("/volume/h24")),
            price_change: PriceChange {
                m5: change("m5"),
                h1: change("h1"),
                h6: change("h6"),
                h24: change("h24"),
            },
            txns24h: buys.zip(sells).map(|(buys, sells)| Txns { buys, sells }),
            pool_address: text(pair.get("pairAddress")),
            pool_name: text(pair.get("dexId")),
            pair_created_at: num(pair.get("pairCreatedAt")).and_then(iso_from_millis),
            source: "dexscreener".into(),
        })
    }
}

fn liquidity(pair: &Value) -> f64 {
    num(pair.pointer("/liquidity/usd")).unwrap_or(0.0)
}

fn volume24h(pair: &Value) -> f64 {
    num(pair.pointer("/volume/h24")).unwrap_or(0.0)
}

#[cfg(test)]
#[path = "dexscreener_tests.rs"]
mod tests;
