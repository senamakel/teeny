//! `market_context`.

use serde::{Deserialize, Serialize};

use super::Report;
use crate::format::{compact_usd, to_fixed, to_locale};
use crate::global::{GlobalSnapshot, MarketDirection, RelativeStrength};

/// `market_context`: the single most useful thing the account can say on a
/// bad day, and a fact rather than a spin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketContextReport {
    /// `None` when no global source is configured.
    pub market: Option<GlobalSnapshot>,
    /// What the token is called in the comparison.
    pub symbol: Option<String>,
    /// `None` when the token's own 24h change is unknown.
    pub token_change24h_pct: Option<f64>,
    pub strength: Option<RelativeStrength>,
}

impl Report for MarketContextReport {
    fn text(&self) -> String {
        let Some(snapshot) = &self.market else {
            return "No global market source is configured, so there is nothing to compare against.".into();
        };
        let mut lines = vec!["The rest of the market, last 24h:".to_owned()];
        lines.extend(snapshot.assets.iter().map(|asset| {
            format!(
                "  {} ${}{}",
                asset.symbol,
                to_locale(asset.price_usd, 3),
                bracketed(asset.change24h_pct)
            )
        }));
        if let Some(cap) = snapshot.total_market_cap_usd {
            lines.push(format!(
                "  total crypto market cap {}{}",
                compact_usd(cap),
                bracketed(snapshot.total_change24h_pct)
            ));
        }
        if let Some(dominance) = snapshot.btc_dominance_pct {
            lines.push(format!("  BTC dominance {}%", to_fixed(dominance, 1)));
        }

        let (Some(change), Some(strength)) = (self.token_change24h_pct, &self.strength) else {
            lines.push(String::new());
            lines.push("No 24h change for the token, so no comparison can be drawn.".into());
            return lines.join("\n");
        };
        lines.push(String::new());
        lines.push(format!(
            "{} is {}{}% over the same 24h, while the market is {}.",
            self.symbol.as_deref().unwrap_or("the token"),
            if change >= 0.0 { "+" } else { "" },
            to_fixed(change, 2),
            strength.market_direction.as_str()
        ));
        lines.push(String::new());
        if strength.beating.is_empty() {
            lines.push(
                "It did not outperform any of these over 24h. Do not force a comparison that is not there -- \
                 take the positive from the chart, the holders, or the longer horizon instead."
                    .into(),
            );
        } else {
            lines.push("TRUE POSITIVES YOU CAN LEAD WITH, best first:".into());
            lines.extend(strength.beating.iter().map(|entry| format!("  - {entry}")));
        }
        lines.push(String::new());
        lines.push(if strength.market_direction == MarketDirection::Down {
            "The whole market is red today. Holding up while everything else sells off is the strongest true \
             thing you can say, and it is the post."
        } else {
            "The market is not red today, so outperformance is a smaller claim. Say it plainly and let the \
             chart carry the rest."
        }
        .into());
        lines.join("\n")
    }
}

/// ` (+1.25%)`, or nothing when the change is unknown.
fn bracketed(change: Option<f64>) -> String {
    change
        .map(|change| {
            format!(
                " ({}{}%)",
                if change >= 0.0 { "+" } else { "" },
                to_fixed(change, 2)
            )
        })
        .unwrap_or_default()
}
