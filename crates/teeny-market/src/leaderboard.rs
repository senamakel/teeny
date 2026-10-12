//! The AI meme coin leaderboard, from CoinGecko, for the moon chart.
//!
//! The category listing (`/coins/markets?category=ai-meme-coins`) is the same
//! table as coingecko.com/en/categories/ai-meme-coins, sorted by market cap,
//! and it answers without a key. So does the row for the account's own token,
//! looked up by id rather than found in the list: at half a million of market
//! cap it is not on the first page, and the whole point of the post is where
//! it would be.

use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{MarketError, Result};
use crate::http::{JsonClient, num, text};

const BASE: &str = "https://api.coingecko.com/api/v3";
/// CoinGecko's largest page.
const PAGE: usize = 250;

/// What `encodeURIComponent` leaves alone.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// One row of a CoinGecko markets listing: what the moon chart needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedCoin {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub price_usd: f64,
    pub market_cap_usd: f64,
    pub circulating_supply: f64,
    /// CoinGecko's overall market cap rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<f64>,
    /// Position on the category board, 1-based, when the coin came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_rank: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change24h_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ath_usd: Option<f64>,
    /// The coin's logo, as CoinGecko serves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// A logo's bytes and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logo {
    pub data: Vec<u8>,
    pub mime_type: &'static str,
}

/// CoinGecko's markets endpoint.
#[derive(Debug, Clone)]
pub struct CoinGeckoLeaderboard {
    http: JsonClient,
    base: String,
    logo_timeout: Duration,
}

impl CoinGeckoLeaderboard {
    /// The public API.
    pub fn new(http: JsonClient) -> Self {
        Self {
            http,
            base: BASE.to_owned(),
            logo_timeout: Duration::from_secs(10),
        }
    }

    /// Somewhere else, such as a test server.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// The whole category to `limit`, largest first, walking CoinGecko's
    /// 250-row pages until it has enough or a short page says there are no
    /// more. The board rank is the row's place in the merged list.
    pub async fn category(&self, id: &str, limit: usize) -> Result<Vec<ListedCoin>> {
        let mut coins: Vec<ListedCoin> = Vec::new();
        let mut page = 1;
        while coins.len() < limit {
            let rows = self
                .markets(&format!(
                    "category={}&order=market_cap_desc&per_page={PAGE}&page={page}",
                    utf8_percent_encode(id, COMPONENT)
                ))
                .await?;
            for row in &rows {
                if let Some(coin) = parse_coin(row) {
                    let board_rank = coins.len() + 1;
                    coins.push(ListedCoin {
                        board_rank: Some(board_rank),
                        ..coin
                    });
                }
            }
            if rows.len() < PAGE {
                break;
            }
            page += 1;
        }
        coins.truncate(limit);
        Ok(coins)
    }

    /// One coin by CoinGecko id.
    pub async fn coin(&self, id: &str) -> Result<ListedCoin> {
        let rows = self
            .markets(&format!("ids={}", utf8_percent_encode(id, COMPONENT)))
            .await?;
        rows.first()
            .and_then(parse_coin)
            .ok_or_else(|| MarketError::data(format!("CoinGecko has no market row for \"{id}\"")))
    }

    /// A logo, for embedding in the picture. Only over https, only PNG or
    /// JPEG (what the rasteriser can embed), and never an error: the picture
    /// reads without it.
    pub async fn logo(&self, url: &str) -> Option<Logo> {
        let parsed = reqwest::Url::parse(url).ok()?;
        if parsed.scheme() != "https" {
            return None;
        }
        self.fetch_logo(url).await
    }

    /// The fetch and sniff behind [`Self::logo`], whatever the scheme.
    async fn fetch_logo(&self, url: &str) -> Option<Logo> {
        let response = self
            .http
            .http()
            .get(url)
            .timeout(self.logo_timeout)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let data = response.bytes().await.ok()?.to_vec();
        let mime_type = sniff_image(&data)?;
        Some(Logo { data, mime_type })
    }

    async fn markets(&self, query: &str) -> Result<Vec<Value>> {
        let url = format!("{}/coins/markets?vs_currency=usd&{query}", self.base);
        match self.http.get(&url, &[]).await? {
            Value::Array(rows) => Ok(rows),
            _ => Err(MarketError::data(format!(
                "CoinGecko answered {url} with something other than a list"
            ))),
        }
    }
}

/// A row with no price, cap or supply cannot be placed on the ladder and is dropped.
pub fn parse_coin(row: &Value) -> Option<ListedCoin> {
    let id = text(row.get("id")).filter(|id| !id.is_empty())?;
    let symbol = text(row.get("symbol"))
        .map(|symbol| symbol.to_uppercase())
        .filter(|s| !s.is_empty())?;
    let name = text(row.get("name")).unwrap_or_else(|| symbol.clone());
    if name.is_empty() {
        return None;
    }
    let price_usd = num(row.get("current_price"))?;
    let market_cap_usd = num(row.get("market_cap"))?;
    let circulating_supply = num(row.get("circulating_supply"))?;
    if circulating_supply <= 0.0 || market_cap_usd <= 0.0 {
        return None;
    }
    Some(ListedCoin {
        id,
        symbol,
        name,
        price_usd,
        market_cap_usd,
        circulating_supply,
        rank: num(row.get("market_cap_rank")),
        board_rank: None,
        change24h_pct: num(row.get("price_change_percentage_24h")),
        ath_usd: num(row.get("ath")),
        image_url: text(row.get("image")).filter(|url| !url.is_empty()),
    })
}

/// PNG or JPEG by their magic bytes; anything else is not embeddable.
pub fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.len() > 8 && data[..4] == [0x89, 0x50, 0x4e, 0x47] {
        Some("image/png")
    } else if data.len() > 3 && data[..3] == [0xff, 0xd8, 0xff] {
        Some("image/jpeg")
    } else {
        None
    }
}

#[cfg(test)]
#[path = "leaderboard_tests.rs"]
mod tests;
