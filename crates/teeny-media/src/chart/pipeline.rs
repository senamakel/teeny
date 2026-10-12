//! The whole chart pipeline: candles in, a PNG (and maybe a file) out.

use std::path::PathBuf;

use teeny_core::config::ChartTimeframe;

use super::svg::{ChartInput, HorizontalLine, Overlay, Stat, render_chart_svg};
use super::theme::THEME;
use crate::candle::Candle;
use crate::error::{Error, Result};
use crate::format::compact_usd;
use crate::png::svg_to_png;

/// What the chart header shows about the token.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartOverview {
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub price_usd: f64,
    pub market_cap_usd: Option<f64>,
    pub fdv_usd: Option<f64>,
    pub liquidity_usd: Option<f64>,
    pub volume_24h_usd: Option<f64>,
}

/// The parts of a technical read the chart draws.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRead {
    pub fast_period: usize,
    pub slow_period: usize,
    /// Change over the whole window read, in percent.
    pub change_pct: f64,
    pub support: Option<f64>,
    pub resistance: Option<f64>,
}

/// A chart to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRequest {
    pub candles: Vec<Candle>,
    pub timeframe: ChartTimeframe,
    pub overview: Option<ChartOverview>,
    pub read: Option<ChartRead>,
    pub symbol: Option<String>,
    pub watermark: Option<String>,
    /// Written here as `<symbol>-<timeframe>-<unix ms>.png` when set.
    pub output_dir: Option<PathBuf>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl ChartRequest {
    /// Candles and a timeframe; everything else unset.
    pub fn new(candles: Vec<Candle>, timeframe: ChartTimeframe) -> Self {
        Self {
            candles,
            timeframe,
            overview: None,
            read: None,
            symbol: None,
            watermark: None,
            output_dir: None,
            width: None,
            height: None,
        }
    }
}

/// A drawn chart.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedChart {
    pub svg: String,
    pub png: Vec<u8>,
    /// Where the PNG was written, when an output directory was given.
    pub path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
}

/// The whole chart pipeline: candles in, a PNG on disk out.
///
/// The overlays and the header strip are derived here rather than passed in,
/// so every chart this account posts is drawn the same way -- same two EMAs,
/// same stats, same levels. A chart whose indicators change from post to post
/// is a chart nobody can read at a glance, which defeats the point of posting
/// one.
pub fn render_chart(request: &ChartRequest) -> Result<RenderedChart> {
    let candles = &request.candles;
    let (Some(first), Some(last)) = (candles.first(), candles.last()) else {
        return Err(Error::failed("Cannot render a chart with no candles"));
    };

    let values: Vec<f64> = candles.iter().map(|candle| candle.c).collect();
    let read = request.read.as_ref();
    let overview = request.overview.as_ref();
    let fast_period = read.map_or(20, |read| read.fast_period);
    let slow_period = read.map_or(50, |read| read.slow_period);
    let symbol = request
        .symbol
        .clone()
        .or_else(|| overview.and_then(|overview| overview.symbol.clone()))
        .unwrap_or_else(|| "TOKEN".to_owned());

    let mut stats = Vec::new();
    if let Some(overview) = overview {
        if let Some(cap) = overview.market_cap_usd {
            stats.push(stat("MC", cap));
        } else if let Some(fdv) = overview.fdv_usd {
            stats.push(stat("FDV", fdv));
        }
        if let Some(liquidity) = overview.liquidity_usd {
            stats.push(stat("LIQ", liquidity));
        }
        if let Some(volume) = overview.volume_24h_usd {
            stats.push(stat("24H VOL", volume));
        }
    }

    let mut lines = Vec::new();
    if let Some(support) = read.and_then(|read| read.support) {
        lines.push(HorizontalLine {
            label: "support".into(),
            value: support,
            color: Some(THEME.up.into()),
        });
    }
    if let Some(resistance) = read.and_then(|read| read.resistance) {
        lines.push(HorizontalLine {
            label: "resistance".into(),
            value: resistance,
            color: Some(THEME.down.into()),
        });
    }

    let change_pct = read.map_or_else(
        || {
            if first.o == 0.0 {
                0.0
            } else {
                ((last.c - first.o) / first.o) * 100.0
            }
        },
        |read| read.change_pct,
    );
    let input = ChartInput {
        candles: candles.clone(),
        symbol: if symbol.starts_with('$') {
            symbol.clone()
        } else {
            format!("${symbol}")
        },
        name: overview
            .and_then(|overview| overview.name.clone())
            .filter(|name| !name.is_empty()),
        timeframe: request.timeframe,
        price_usd: overview.map(|overview| overview.price_usd),
        change_pct: Some(change_pct),
        overlays: vec![
            Overlay {
                label: format!("EMA{fast_period}"),
                color: THEME.fast.into(),
                values: ema(&values, fast_period),
            },
            Overlay {
                label: format!("EMA{slow_period}"),
                color: THEME.slow.into(),
                values: ema(&values, slow_period),
            },
        ],
        lines,
        stats,
        watermark: request.watermark.clone().filter(|mark| !mark.is_empty()),
        width: request.width,
        height: request.height,
    };

    let svg = render_chart_svg(&input)?;
    let png = svg_to_png(&svg)?;
    let mut rendered = RenderedChart {
        svg,
        png,
        path: None,
        width: input.width.unwrap_or(1_200),
        height: input.height.unwrap_or(675),
    };

    if let Some(dir) = &request.output_dir {
        let clean: String = symbol.chars().filter(char::is_ascii_alphanumeric).collect();
        let name = format!(
            "{clean}-{}-{}.png",
            request.timeframe.as_str(),
            chrono::Utc::now().timestamp_millis()
        );
        std::fs::create_dir_all(dir)?;
        let path = dir.join(name);
        std::fs::write(&path, &rendered.png)?;
        rendered.path = Some(path);
    }
    Ok(rendered)
}

fn stat(label: &str, value: f64) -> Stat {
    Stat {
        label: label.to_owned(),
        value: compact_usd(value),
    }
}

/// Exponential moving average, seeded on the simple average of the first
/// `period` values, which is what every charting package does. Seeding on
/// one close makes the early curve a decay from a single arbitrary point.
/// `None` until there are `period` values; all `None` for a zero period.
pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    if period == 0 || values.len() < period {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut previous = values[..period]
        .iter()
        .fold(0.0, |total, value| total + value)
        / period as f64;
    out[period - 1] = Some(previous);
    for index in period..values.len() {
        previous = values[index] * k + previous * (1.0 - k);
        out[index] = Some(previous);
    }
    out
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
