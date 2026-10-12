//! `token_holders`, `token_flow`, `trader_activity` and `ranked_traders`.

use serde::{Deserialize, Serialize};

use super::{Report, short};
use crate::format::{compact_usd, js_number, js_round, to_fixed, to_locale};
use crate::gmgn::RankedTrader;
use crate::market::{FlowSummary, WalletActivity};
use crate::types::HolderSnapshot;

/// `token_holders`: useful for a post about distribution, useless as a claim
/// about any individual.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HoldersReport {
    pub snapshot: Option<HolderSnapshot>,
    /// Why there is no snapshot.
    pub unavailable: Option<String>,
}

impl Report for HoldersReport {
    fn text(&self) -> String {
        let Some(snapshot) = &self.snapshot else {
            return format!(
                "Holder data is unavailable right now: {}",
                self.unavailable.as_deref().unwrap_or_default()
            );
        };
        let mut lines = vec![
            format!(
                "total supply: {}",
                to_locale(js_round(snapshot.total_supply), 3)
            ),
            format!(
                "top 10 token accounts hold {}% of supply",
                to_fixed(snapshot.top10_share * 100.0, 1)
            ),
            String::new(),
        ];
        lines.extend(
            snapshot
                .top
                .iter()
                .take(10)
                .enumerate()
                .map(|(index, account)| {
                    format!(
                        "{}. {} — {}%",
                        index + 1,
                        account.address,
                        to_fixed(account.share * 100.0, 2)
                    )
                }),
        );
        lines.push(String::new());
        lines.push(
            "These are token accounts, not people: a liquidity pool vault is normally the largest one, and a \
             single person can hold several. Read it as concentration, and never call one of these \"the top \
             holder\" in a post."
                .into(),
        );
        lines.join("\n")
    }
}

/// `token_flow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowReport {
    pub min_volume_usd: f64,
    pub summary: FlowSummary,
}

impl Report for FlowReport {
    fn text(&self) -> String {
        let summary = &self.summary;
        let floor = js_number(self.min_volume_usd);
        if summary.window.trades == 0 {
            return format!("No trades over ${floor} in the window the provider returns.");
        }
        let mut lines = vec![
            format!(
                "window: {} trades over ${floor}, {} to {}",
                summary.window.trades, summary.window.from, summary.window.to
            ),
            format!(
                "buy volume {}, sell volume {}, net {}",
                compact_usd(summary.buy_volume_usd),
                compact_usd(summary.sell_volume_usd),
                compact_usd(summary.net_usd)
            ),
            format!(
                "{} wallets: {} buying, {} selling",
                summary.unique_wallets, summary.buyers, summary.sellers
            ),
            String::new(),
            "top buyers:".into(),
        ];
        lines.extend(
            summary
                .top_buyers
                .iter()
                .map(|entry| format!("- {}", wallet_line(entry))),
        );
        lines.push(String::new());
        lines.push("top sellers:".into());
        lines.extend(
            summary
                .top_sellers
                .iter()
                .map(|entry| format!("- {}", wallet_line(entry))),
        );
        if !summary.known.is_empty() {
            lines.push(String::new());
            lines.push("watchlist wallets active:".into());
            lines.extend(
                summary
                    .known
                    .iter()
                    .map(|entry| format!("- {}", wallet_line(entry))),
            );
        }
        lines.push(String::new());
        lines.push(
            "This is only the recent trades the provider returns, not the full day. Say \"in the last few \
             hours\", never \"today\", and never name a wallet address in a post."
                .into(),
        );
        lines.join("\n")
    }
}

fn wallet_line(entry: &WalletActivity) -> String {
    let who = match &entry.label {
        Some(label) => format!("{label} ({})", short(&entry.wallet)),
        None => short(&entry.wallet),
    };
    format!(
        "{who}: bought {}, sold {}, net {}",
        compact_usd(entry.bought_usd),
        compact_usd(entry.sold_usd),
        compact_usd(entry.net_usd)
    )
}

/// `trader_activity`: to check a claim about a wallet, not to build a post
/// around one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraderActivityReport {
    pub wallet: String,
    /// `None` when the wallet did not trade in the window.
    pub activity: Option<WalletActivity>,
}

impl Report for TraderActivityReport {
    fn text(&self) -> String {
        let Some(activity) = &self.activity else {
            return format!(
                "{} has not traded this token in the window read.",
                short(&self.wallet)
            );
        };
        [
            format!(
                "{}{}",
                activity
                    .label
                    .as_ref()
                    .map(|label| format!("{label} — "))
                    .unwrap_or_default(),
                activity.wallet
            ),
            format!(
                "{} buys totalling {}",
                activity.buys,
                compact_usd(activity.bought_usd)
            ),
            format!(
                "{} sells totalling {}",
                activity.sells,
                compact_usd(activity.sold_usd)
            ),
            format!(
                "net {} between {} and {}",
                compact_usd(activity.net_usd),
                activity.first_seen,
                activity.last_seen
            ),
            String::new(),
            "Do not put this wallet address in a post.".into(),
        ]
        .join("\n")
    }
}

/// `ranked_traders`: empty means the account must not claim anything about KOLs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedTradersReport {
    pub traders: Vec<RankedTrader>,
}

impl Report for RankedTradersReport {
    fn text(&self) -> String {
        if self.traders.is_empty() {
            return "No ranked-trader source is configured, so there is no KOL or smart-money data. Do not claim \
                    any. token_flow shows who is actually buying right now."
                .into();
        }
        self.traders
            .iter()
            .take(15)
            .map(|trader| {
                let tags = if trader.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", trader.tags.join(", "))
                };
                format!(
                    "{}{tags}: bought {}, sold {}, realised {}",
                    short(&trader.wallet),
                    compact_usd(trader.bought_usd.unwrap_or(0.0)),
                    compact_usd(trader.sold_usd.unwrap_or(0.0)),
                    compact_usd(trader.realized_profit_usd.unwrap_or(0.0))
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
