//! GeckoTerminal, the free public DEX data API.
//!
//! The default source rather than GMGN because it answers a server: GMGN
//! fronts its API with Cloudflare's bot check.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::error::{MarketError, Result};
use crate::format::{iso_now, js_number};
use crate::http::{JsonClient, num, text};
use crate::limiter::Limiter;
use crate::types::{Candle, PriceChange, Timeframe, TokenOverview, Trade, TradeKind, Txns};

const BASE: &str = "https://api.geckoterminal.com/api/v2";

/// GeckoTerminal for one network. Every call goes through one queue, so a run
/// that reads four timeframes spends four gaps rather than arriving as a burst.
#[derive(Debug, Clone)]
pub struct GeckoTerminalClient {
    http: JsonClient,
    base: String,
    network: String,
    limiter: Arc<Limiter>,
}

impl GeckoTerminalClient {
    /// The public API on `network` (`solana`), 2.5s between calls: the free
    /// tier is about thirty a minute.
    pub fn new(http: JsonClient, network: impl Into<String>) -> Self {
        Self {
            http,
            base: BASE.to_owned(),
            network: network.into(),
            limiter: Arc::new(Limiter::new(Duration::from_millis(2_500))),
        }
    }

    /// Somewhere else, such as a test server.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// A different minimum gap between calls.
    pub fn with_min_interval(mut self, gap: Duration) -> Self {
        self.limiter = Arc::new(Limiter::new(gap));
        self
    }

    /// The snapshot, with the deepest pool as the one to chart.
    pub async fn overview(&self, address: &str) -> Result<TokenOverview> {
        let response = self
            .get(&format!(
                "/networks/{}/tokens/{address}?include=top_pools",
                self.network
            ))
            .await?;
        let empty = Value::Object(Default::default());
        let attributes = response.pointer("/data/attributes").unwrap_or(&empty);
        let price = num(attributes.get("price_usd")).ok_or_else(|| {
            MarketError::data(format!("GeckoTerminal returned no price for {address}"))
        })?;

        // The first related pool is the deepest one, which is the pool a
        // chart should be drawn from: a thin pool's candles are mostly noise.
        let pool_id = response
            .pointer("/data/relationships/top_pools/data/0/id")
            .and_then(Value::as_str);
        let pool = response
            .get("included")
            .and_then(Value::as_array)
            .and_then(|included| {
                included
                    .iter()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == pool_id)
            })
            .and_then(|entry| entry.get("attributes"))
            .unwrap_or(&empty);
        let change =
            |window: &str| percent(pool.pointer(&format!("/price_percent_changes/{window}")));
        let buys = num(pool.pointer("/transactions/h24/buys"));
        let sells = num(pool.pointer("/transactions/h24/sells"));

        Ok(TokenOverview {
            address: address.to_owned(),
            chain: self.network.clone(),
            name: text(attributes.get("name")),
            symbol: text(attributes.get("symbol")),
            price_usd: price,
            market_cap_usd: num(attributes.get("market_cap_usd")),
            fdv_usd: num(attributes.get("fdv_usd")),
            liquidity_usd: num(attributes.get("total_reserve_in_usd")),
            volume24h_usd: num(attributes.pointer("/volume_usd/h24")),
            price_change: PriceChange {
                m5: None,
                h1: change("last_1h"),
                h6: change("last_6h"),
                h24: change("last_24h"),
            },
            txns24h: buys.zip(sells).map(|(buys, sells)| Txns { buys, sells }),
            pool_address: bare(pool_id),
            pool_name: text(pool.get("name")),
            pair_created_at: text(pool.get("pool_created_at")),
            source: "geckoterminal".into(),
        })
    }

    /// OHLCV for one pool, oldest first. `limit` is clamped to 1..=1000.
    pub async fn candles(
        &self,
        pool: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let spec = timeframe.spec();
        let limit = limit.clamp(1, 1_000);
        let response = self
            .get(&format!(
                "/networks/{}/pools/{pool}/ohlcv/{}?aggregate={}&limit={limit}&currency=usd",
                self.network, spec.unit, spec.aggregate
            ))
            .await?;

        // GeckoTerminal answers newest first, the opposite of what every
        // indicator expects; reversing it here means nothing downstream has to
        // remember which way round a provider happens to be.
        let mut candles: Vec<Candle> = response
            .pointer("/data/attributes/ohlcv_list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let row = row.as_array()?;
                let field = |index: usize| num(row.get(index));
                Some(Candle {
                    time: field(0)? as i64,
                    open: field(1)?,
                    high: field(2)?,
                    low: field(3)?,
                    close: field(4)?,
                    volume: field(5).unwrap_or(0.0),
                })
            })
            .collect();
        candles.sort_by_key(|candle| candle.time);
        Ok(candles)
    }

    /// Recent swaps over a size floor, newest first, with the wallet that made each.
    pub async fn trades(&self, pool: &str, min_volume_usd: f64) -> Result<Vec<Trade>> {
        let floor = min_volume_usd.max(0.0);
        let response = self
            .get(&format!(
                "/networks/{}/pools/{pool}/trades?trade_volume_in_usd_greater_than={}",
                self.network,
                js_number(floor)
            ))
            .await?;

        Ok(response
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let attributes = entry.get("attributes")?;
                let wallet = text(attributes.get("tx_from_address"))?;
                let hash = text(attributes.get("tx_hash"))?;
                let volume = num(attributes.get("volume_in_usd"))?;
                let buy = attributes.get("kind").and_then(Value::as_str) == Some("buy");
                let (amount, price) = if buy {
                    ("to_token_amount", "price_to_in_usd")
                } else {
                    ("from_token_amount", "price_from_in_usd")
                };
                Some(Trade {
                    tx_hash: hash,
                    timestamp: text(attributes.get("block_timestamp")).unwrap_or_else(iso_now),
                    kind: if buy { TradeKind::Buy } else { TradeKind::Sell },
                    wallet,
                    volume_usd: volume,
                    token_amount: num(attributes.get(amount)).unwrap_or(0.0),
                    price_usd: num(attributes.get(price)).unwrap_or(0.0),
                })
            })
            .collect())
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        self.limiter.run(self.http.get(&url, &[])).await
    }
}

/// `solana_ABC` -> `ABC`. Every GeckoTerminal id is namespaced by network.
fn bare(id: Option<&str>) -> Option<String> {
    let id = id.filter(|id| !id.is_empty())?;
    Some(id.split_once('_').map_or(id, |(_, rest)| rest).to_owned())
}

/// GeckoTerminal spells a percentage "+12.4%"; everything here wants 12.4.
fn percent(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::String(spelled)) => {
            let cleaned: String = spelled.chars().filter(|c| *c != '%' && *c != '+').collect();
            num(Some(&Value::String(cleaned)))
        }
        other => num(other),
    }
}

#[cfg(test)]
#[path = "geckoterminal_tests.rs"]
mod tests;
