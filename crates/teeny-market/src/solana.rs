//! Holder concentration, read straight off a Solana RPC.
//!
//! Deliberately not a paid indexer: `getTokenLargestAccounts` is a standard
//! RPC method every provider implements, so pointing `SOLANA_RPC_URL` at a
//! private node is the only change needed when the public one rate-limits --
//! which it does, often, and which is why every caller treats a failure as
//! "unknown" rather than as an outage.
//!
//! These are token *accounts*, not owners. A pool's vault is the largest
//! account of almost every token, and one person can hold three. Read it as
//! concentration, never as "the top holder".

use serde_json::{Value, json};

use crate::error::{MarketError, Result};
use crate::http::{JsonClient, num, text};
use crate::types::{HolderAccount, HolderSnapshot};

const MAINNET: &str = "https://api.mainnet-beta.solana.com";

/// A Solana JSON-RPC endpoint.
#[derive(Debug, Clone)]
pub struct SolanaRpc {
    http: JsonClient,
    url: String,
}

impl SolanaRpc {
    /// The RPC at `url`, or the public mainnet one.
    pub fn new(http: JsonClient, url: Option<String>) -> Self {
        Self {
            http,
            url: url.unwrap_or_else(|| MAINNET.to_owned()),
        }
    }

    /// Supply and the largest token accounts of `mint`.
    pub async fn holders(&self, mint: &str) -> Result<HolderSnapshot> {
        let (supply, largest) = futures::try_join!(
            self.call("getTokenSupply", mint),
            self.call("getTokenLargestAccounts", mint)
        )?;

        let total = num(supply.pointer("/value/uiAmount")).unwrap_or(0.0);
        if total == 0.0 {
            return Err(MarketError::data(format!(
                "Solana RPC returned no supply for {mint}"
            )));
        }
        let top: Vec<HolderAccount> = largest
            .get("value")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|account| {
                let address = text(account.get("address")).filter(|address| !address.is_empty())?;
                let amount = account.get("uiAmount").and_then(Value::as_f64)?;
                Some(HolderAccount {
                    address,
                    amount,
                    share: amount / total,
                })
            })
            .collect();
        let top10_share = top
            .iter()
            .take(10)
            .fold(0.0, |sum, account| sum + account.share);

        Ok(HolderSnapshot {
            total_supply: total,
            decimals: supply
                .pointer("/value/decimals")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            top,
            top10_share,
            source: "solana-rpc".into(),
        })
    }

    /// One JSON-RPC call with the mint as its only parameter.
    async fn call(&self, method: &str, mint: &str) -> Result<Value> {
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [mint] });
        let failed = |message: String| MarketError::data(format!("Solana RPC {method}{message}"));
        let response = self
            .http
            .http()
            .post(&self.url)
            .timeout(self.http.timeout())
            .json(&request)
            .send()
            .await
            .map_err(|error| failed(format!(": {error}")))?;
        if !response.status().is_success() {
            return Err(failed(format!(" answered {}", response.status().as_u16())));
        }
        let mut body: Value = response
            .json()
            .await
            .map_err(|error| failed(format!(": {error}")))?;
        if let Some(error) = body.get("error").filter(|error| !error.is_null()) {
            let message = text(error.get("message")).unwrap_or_else(|| "failed".into());
            return Err(failed(format!(": {message}")));
        }
        body.get_mut("result")
            .map(Value::take)
            .ok_or_else(|| failed(" returned no result".into()))
    }
}

#[cfg(test)]
#[path = "solana_tests.rs"]
mod tests;
