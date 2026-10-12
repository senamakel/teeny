//! GMGN.
//!
//! GMGN has the data this account would most like -- smart-money labels, KOL
//! wallets, top-trader PnL -- and no public API. `gmgn.ai` fronts everything
//! with Cloudflare's bot check, which answers a server with an HTML
//! interstitial and HTTP 403 whatever headers it sends.
//!
//! So this client is off unless `GMGN_BASE_URL` names something that will
//! actually answer -- a proxy, a scraping service, a cached mirror. Nothing
//! silently invents data: every method fails if GMGN answers with anything but
//! a JSON envelope carrying data, and the market layer falls back to
//! GeckoTerminal and DexScreener.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{MarketError, Result};
use crate::http::{JsonClient, either, num, text};
use crate::types::{Candle, PriceChange, Timeframe, TokenOverview, Txns};

/// A wallet GMGN has ranked on this token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedTrader {
    pub wallet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_profit_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bought_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sold_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holding_amount: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// GMGN behind whatever base URL will answer.
#[derive(Debug, Clone)]
pub struct GmgnClient {
    http: JsonClient,
    base: String,
    chain: String,
    headers: Vec<(String, String)>,
}

impl GmgnClient {
    /// GMGN at `base` (a trailing slash is fine), on Solana (`sol`).
    pub fn new(http: JsonClient, base: impl Into<String>) -> Self {
        Self {
            http,
            base: base.into().trim_end_matches('/').to_owned(),
            chain: "sol".into(),
            headers: Vec::new(),
        }
    }

    /// Another chain, in GMGN's spelling.
    pub fn with_chain(mut self, chain: impl Into<String>) -> Self {
        self.chain = chain.into();
        self
    }

    /// Extra headers for the proxy in front of GMGN.
    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    /// The snapshot.
    pub async fn overview(&self, address: &str) -> Result<TokenOverview> {
        let data = self
            .get(
                &format!("/defi/quotation/v1/tokens/{}/{address}", self.chain),
                "token overview",
            )
            .await?;
        let token = match data.get("token") {
            Some(token) if !token.is_null() => token,
            _ => &data,
        };
        let price = num(token.get("price"))
            .ok_or_else(|| MarketError::data(format!("GMGN returned no price for {address}")))?;
        let change = |window: &str| num(token.get(format!("price_change_percent{window}")));
        let buys = num(token.get("buys_24h"));
        let sells = num(token.get("sells_24h"));

        Ok(TokenOverview {
            address: address.to_owned(),
            chain: self.chain.clone(),
            name: text(token.get("name")),
            symbol: text(token.get("symbol")),
            price_usd: price,
            market_cap_usd: num(token.get("market_cap")),
            fdv_usd: num(token.get("fdv")),
            liquidity_usd: num(token.get("liquidity")),
            volume24h_usd: num(token.get("volume_24h")),
            price_change: PriceChange {
                m5: change("5m"),
                h1: change("1h"),
                h6: change("6h"),
                h24: change("24h"),
            },
            txns24h: buys.zip(sells).map(|(buys, sells)| Txns { buys, sells }),
            pool_address: text(token.get("pool_address")),
            pool_name: None,
            pair_created_at: None,
            source: "gmgn".into(),
        })
    }

    /// Candles for the token, oldest first, ending now.
    pub async fn candles(
        &self,
        address: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let limit = limit.clamp(1, 1_000);
        let to = chrono::Utc::now().timestamp();
        let from = to - timeframe.spec().seconds * i64::from(limit);
        let data = self
            .get(
                &format!(
                    "/api/v1/token_candles/{}/{address}?resolution={timeframe}&from={from}&to={to}",
                    self.chain
                ),
                "candles",
            )
            .await?;

        let mut candles: Vec<Candle> = data
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let time = num(row.get("time")).or_else(|| num(row.get("timestamp")))?;
                Some(Candle {
                    // GMGN answers in milliseconds where GeckoTerminal answers
                    // in seconds, and a chart drawn on the wrong one is 50,000
                    // years out.
                    time: if time > 1e11 {
                        (time / 1_000.0).floor() as i64
                    } else {
                        time as i64
                    },
                    open: num(row.get("open"))?,
                    high: num(row.get("high"))?,
                    low: num(row.get("low"))?,
                    close: num(row.get("close"))?,
                    volume: num(row.get("volume")).unwrap_or(0.0),
                })
            })
            .collect();
        candles.sort_by_key(|candle| candle.time);
        Ok(candles)
    }

    /// The wallets GMGN ranks on this token, with whatever labels it carries.
    pub async fn top_traders(&self, address: &str) -> Result<Vec<RankedTrader>> {
        let data = self
            .get(
                &format!(
                    "/defi/quotation/v1/tokens/top_traders/{}/{address}",
                    self.chain
                ),
                "top traders",
            )
            .await?;
        let rows = match &data {
            Value::Array(rows) => rows.as_slice(),
            other => other
                .get("list")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice),
        };
        Ok(rows
            .iter()
            .filter_map(|row| {
                let wallet = text(either(row, "address", "wallet_address"))?;
                Some(RankedTrader {
                    wallet,
                    realized_profit_usd: num(row.get("realized_profit")),
                    bought_usd: num(either(row, "buy_volume_cur", "bought_usd")),
                    sold_usd: num(either(row, "sell_volume_cur", "sold_usd")),
                    holding_amount: num(either(row, "balance", "holding_amount")),
                    tags: row
                        .get("tags")
                        .and_then(Value::as_array)
                        .map(|tags| tags.iter().filter_map(|tag| text(Some(tag))).collect())
                        .unwrap_or_default(),
                })
            })
            .collect())
    }

    /// One GMGN call, unwrapped from its `{code, msg, data}` envelope.
    async fn get(&self, path: &str, what: &str) -> Result<Value> {
        let envelope = self
            .http
            .get(&format!("{}{path}", self.base), &self.headers)
            .await?;
        if let Some(code) = envelope
            .get("code")
            .filter(|code| code.as_f64() != Some(0.0))
        {
            let why = text(envelope.get("msg")).unwrap_or_else(|| format!("code {code}"));
            return Err(MarketError::data(format!("GMGN refused {what}: {why}")));
        }
        match envelope.get("data") {
            Some(data) if truthy(data) => Ok(data.clone()),
            _ => Err(MarketError::data(format!(
                "GMGN returned no data for {what}"
            ))),
        }
    }
}

/// JavaScript truthiness, for `if (!envelope.data)`.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
#[path = "gmgn_tests.rs"]
mod tests;
