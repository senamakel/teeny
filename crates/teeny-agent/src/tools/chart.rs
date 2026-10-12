//! `render_chart`: the chart agent's renderer (`legacy/src/agent/tools/market.ts`).
//! The model never handles image bytes: it renders, gets a short id, and
//! names the id in its answer. The chart job, which posts, reads the chart
//! back from the same [`ChartBook`] (`Deps::charts`). The agent has no way
//! to publish: `post_chart`, the legacy tool, is gone.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::json;
use teeny_core::config::ChartTimeframe;
use teeny_market::market::MarketData;
use teeny_market::reports::validate_bars;
use teeny_market::ta::analysis::analyse;
use teeny_market::types::{Timeframe, TokenOverview};
use teeny_media::chart::{ChartOverview, ChartRead, ChartRequest, RenderedChart};

use super::{Access, HostTool, Tool, ToolResult, failed, invalid, parse};

/// The charts rendered, by id (`chart_1`, `chart_2`...), shared by the
/// chart belt's `render_chart` and the chart job that attaches one.
#[derive(Default)]
pub struct ChartBook {
    charts: Mutex<Vec<RenderedChart>>,
}

impl ChartBook {
    fn keep(&self, chart: RenderedChart) -> String {
        let mut charts = self.charts.lock().unwrap_or_else(|p| p.into_inner());
        charts.push(chart);
        format!("chart_{}", charts.len())
    }

    /// The chart `render_chart` returned `id` for.
    pub fn get(&self, id: &str) -> Option<RenderedChart> {
        let index: usize = id.strip_prefix("chart_")?.parse().ok()?;
        let charts = self.charts.lock().unwrap_or_else(|p| p.into_inner());
        charts.get(index.checked_sub(1)?).cloned()
    }
}

/// What `render_chart` draws: the token it is bound to, and how.
pub struct ChartSource {
    pub market: Arc<dyn MarketData>,
    pub address: String,
    /// `CHART_TOKEN_SYMBOL`.
    pub symbol: Option<String>,
    /// `CHART_WATERMARK`, or the handle.
    pub watermark: Option<String>,
    /// `CHART_OUTPUT_DIR`: where each PNG is also written.
    pub output_dir: Option<PathBuf>,
}

fn chart_timeframe(timeframe: Timeframe) -> ChartTimeframe {
    match timeframe {
        Timeframe::M5 => ChartTimeframe::M5,
        Timeframe::M15 => ChartTimeframe::M15,
        Timeframe::H1 => ChartTimeframe::H1,
        Timeframe::H4 => ChartTimeframe::H4,
        Timeframe::H12 => ChartTimeframe::H12,
        Timeframe::D1 => ChartTimeframe::D1,
        Timeframe::W1 => ChartTimeframe::W1,
    }
}

fn chart_overview(overview: TokenOverview) -> ChartOverview {
    ChartOverview {
        symbol: overview.symbol,
        name: overview.name,
        price_usd: overview.price_usd,
        market_cap_usd: overview.market_cap_usd,
        fdv_usd: overview.fdv_usd,
        liquidity_usd: overview.liquidity_usd,
        volume_24h_usd: overview.volume24h_usd,
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct RenderArgs {
    timeframe: Timeframe,
    bars: u32,
}

impl Default for RenderArgs {
    fn default() -> Self {
        Self {
            timeframe: Timeframe::H1,
            bars: 120,
        }
    }
}

/// `render_chart`.
pub fn render_chart(source: ChartSource, book: Arc<ChartBook>) -> Arc<dyn Tool> {
    let source = Arc::new(source);
    HostTool::build(
        "render_chart",
        "Draws a candlestick chart of the token with EMA20/EMA50, volume, and the support and resistance levels \
         the analysis found, and returns a chartId. Render once, then attach that id to a single post.",
        json!({
            "type": "object",
            "properties": {
                "timeframe": { "type": "string", "enum": ["5m", "15m", "1h", "4h", "12h", "1d", "1w"], "default": "1h" },
                "bars": { "type": "integer", "minimum": 20, "maximum": 500, "default": 120 }
            },
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let (source, book) = (Arc::clone(&source), Arc::clone(&book));
            async move { render(&source, &book, args).await }
        },
    )
}

async fn render(source: &ChartSource, book: &ChartBook, args: serde_json::Value) -> ToolResult {
    let args = match parse::<RenderArgs>(args) {
        Ok(args) => args,
        Err(refused) => return refused,
    };
    if let Err(why) = validate_bars(args.bars) {
        return invalid(why);
    }
    let series = match source
        .market
        .candles(&source.address, args.timeframe, args.bars)
        .await
    {
        Ok(series) => series,
        Err(error) => return failed(error),
    };
    if series.len() < 5 {
        return ToolResult::success(format!(
            "Only {} bars available; not enough to chart.",
            series.len()
        ));
    }
    let overview = source.market.overview(&source.address).await.ok();
    let read = analyse(&series, args.timeframe).ok();
    let request = ChartRequest {
        overview: overview.map(chart_overview),
        read: read.map(|read| ChartRead {
            fast_period: read.ema.fast_period,
            slow_period: read.ema.slow_period,
            change_pct: read.change_pct,
            support: read.levels.support,
            resistance: read.levels.resistance,
        }),
        symbol: source.symbol.clone(),
        watermark: source.watermark.clone(),
        output_dir: source.output_dir.clone(),
        ..ChartRequest::new(
            series
                .iter()
                .map(|candle| teeny_media::Candle {
                    t: candle.time,
                    o: candle.open,
                    h: candle.high,
                    l: candle.low,
                    c: candle.close,
                    v: candle.volume,
                })
                .collect(),
            chart_timeframe(args.timeframe),
        )
    };
    let drawn =
        tokio::task::spawn_blocking(move || teeny_media::chart::render_chart(&request)).await;
    let chart = match drawn {
        Ok(Ok(chart)) => chart,
        Ok(Err(error)) => return failed(error),
        Err(error) => return failed(error),
    };
    let summary = format!(
        "{}, {} bars, {}x{}px, {}KB.",
        args.timeframe.as_str(),
        series.len(),
        chart.width,
        chart.height,
        (chart.png.len() as f64 / 1024.0).round()
    );
    let saved = chart
        .path
        .as_ref()
        .map(|path| format!("Saved to {}.", path.display()));
    let id = book.keep(chart);
    let mut lines = vec![format!("Rendered {id}: {summary}")];
    lines.extend(saved);
    lines.push(format!("Put chartId \"{id}\" in your answer to attach it."));
    ToolResult::success(lines.join("\n"))
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod tests;
