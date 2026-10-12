//! `token_overview` and `token_candles`.

use serde::{Deserialize, Serialize};

use super::Report;
use crate::format::{compact_usd, iso_from_millis, js_number, js_round, price, signed_percent};
use crate::types::{Candle, Timeframe, TokenOverview};

/// `token_overview`: every number it returns is quotable; nothing else is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OverviewReport(pub TokenOverview);

impl Report for OverviewReport {
    fn text(&self) -> String {
        let o = &self.0;
        let mut lines = vec![
            format!(
                "{} ({}) on {}",
                o.symbol.as_deref().unwrap_or("token"),
                o.name.as_deref().unwrap_or("unknown"),
                o.chain
            ),
            format!("price: ${}", price(o.price_usd)),
        ];
        let money = [
            ("market cap", o.market_cap_usd),
            ("fdv", o.fdv_usd),
            ("liquidity", o.liquidity_usd),
            ("24h volume", o.volume24h_usd),
        ];
        lines.extend(
            money
                .into_iter()
                .filter_map(|(what, value)| value.map(|v| format!("{what}: {}", compact_usd(v)))),
        );
        let changes = o.price_change.entries();
        if !changes.is_empty() {
            let spelled: Vec<String> = changes
                .iter()
                .map(|(window, value)| format!("{window} {}", signed_percent(*value)))
                .collect();
            lines.push(format!("change: {}", spelled.join(", ")));
        }
        if let Some(txns) = o.txns24h {
            lines.push(format!(
                "24h transactions: {} buys / {} sells",
                js_number(txns.buys),
                js_number(txns.sells)
            ));
        }
        if let Some(pool) = o.pool_address.as_deref().filter(|pool| !pool.is_empty()) {
            lines.push(format!(
                "pool: {pool} ({})",
                o.pool_name.as_deref().unwrap_or("unknown")
            ));
        }
        if let Some(opened) = o.pair_created_at.as_deref().filter(|at| !at.is_empty()) {
            lines.push(format!("pool opened: {opened}"));
        }
        lines.push(format!("source: {}", o.source));
        lines.retain(|line| !line.is_empty());
        lines.join("\n")
    }
}

/// The span and range of the series read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandleWindow {
    /// ISO time of the first bar.
    pub from: String,
    /// ISO time of the last bar.
    pub to: String,
    pub open: f64,
    pub last_close: f64,
    pub high: f64,
    pub low: f64,
}

/// `token_candles`: the window and the last twelve bars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandlesReport {
    pub timeframe: Timeframe,
    pub bars: usize,
    /// `None` when no candles came back.
    pub window: Option<CandleWindow>,
    /// The last twelve bars, oldest first.
    pub recent: Vec<Candle>,
}

impl CandlesReport {
    /// The report on `series`.
    pub fn from_series(timeframe: Timeframe, series: &[Candle]) -> Self {
        let window = match (series.first(), series.last()) {
            (Some(first), Some(last)) => Some(CandleWindow {
                from: iso(first.time),
                to: iso(last.time),
                open: first.open,
                last_close: last.close,
                high: series
                    .iter()
                    .map(|c| c.high)
                    .fold(f64::NEG_INFINITY, f64::max),
                low: series.iter().map(|c| c.low).fold(f64::INFINITY, f64::min),
            }),
            _ => None,
        };
        Self {
            timeframe,
            bars: series.len(),
            window,
            recent: series[series.len().saturating_sub(12)..].to_vec(),
        }
    }
}

impl Report for CandlesReport {
    fn text(&self) -> String {
        let Some(window) = &self.window else {
            return "No candles came back for that timeframe.".into();
        };
        let mut lines = vec![
            format!(
                "{} {} bars, {} to {}",
                self.bars, self.timeframe, window.from, window.to
            ),
            format!(
                "window open ${}, last close ${}",
                price(window.open),
                price(window.last_close)
            ),
            format!(
                "window high ${}, low ${}",
                price(window.high),
                price(window.low)
            ),
            String::new(),
            "last 12 bars (time, open, high, low, close, volume usd):".into(),
        ];
        lines.extend(self.recent.iter().map(|c| {
            format!(
                "{} {} {} {} {} {}",
                iso(c.time),
                price(c.open),
                price(c.high),
                price(c.low),
                price(c.close),
                js_number(js_round(c.volume))
            )
        }));
        lines.join("\n")
    }
}

fn iso(seconds: i64) -> String {
    iso_from_millis(seconds as f64 * 1_000.0).unwrap_or_default()
}
