//! `render_chart` and `post_chart`: the chart agent's renderer and its one
//! way to publish (`legacy/src/agent/tools/market.ts`). The model never
//! handles image bytes: it renders, gets a short id, and passes the id to the
//! post tool. Both tools of one belt share a [`ChartBook`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;
use teeny_core::config::ChartTimeframe;
use teeny_core::types::PostId;
use teeny_market::market::MarketData;
use teeny_market::reports::validate_bars;
use teeny_market::ta::analysis::analyse;
use teeny_market::types::{Timeframe, TokenOverview};
use teeny_media::chart::{ChartOverview, ChartRead, ChartRequest, RenderedChart};
use teeny_memory::Memory;
use teeny_safety::house_style::{asks_the_reader, without_the_ask};
use teeny_x::model::MediaType;
use teeny_x::{Draft, XApi, XPublisher};

use super::post::{Pacing, refusal_of, upload};
use super::{Access, HostTool, Tool, ToolResult, answer, failed, invalid, is_x_id, parse};

/// The charts rendered in one run, by id (`chart_1`, `chart_2`...).
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

    fn get(&self, id: &str) -> Option<RenderedChart> {
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
    lines.push(format!("Pass chartId \"{id}\" to post_chart to attach it."));
    ToolResult::success(lines.join("\n"))
}

/// Everything `post_chart` needs.
pub struct PostChartTool {
    /// Built with a `PostingGuard` for the chart surface and `post_chart`.
    pub publisher: Arc<XPublisher>,
    /// For the chart upload.
    pub x: Arc<XApi>,
    /// Shared with this belt's `render_chart`.
    pub book: Arc<ChartBook>,
    /// Where the post is remembered, so tomorrow can see the actual text.
    pub memory: Option<Arc<dyn Memory>>,
    /// `CHART_TOKEN_SYMBOL`, stored with the post.
    pub symbol: String,
    pub pacing: Option<Arc<Pacing>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostArgs {
    text: String,
    chart_id: Option<String>,
    reply_to_post_id: Option<String>,
}

#[derive(Serialize)]
struct ChartSize {
    bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WouldHave<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    chart_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_post_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chart: Option<ChartSize>,
}

/// `post_chart`.
pub fn post_chart(tool: PostChartTool) -> Arc<dyn Tool> {
    let tool = Arc::new(tool);
    HostTool::build(
        "post_chart",
        "Publishes one post, with a rendered chart attached when chartId is given. This is the only way this \
         agent publishes. Post once per run.",
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": teeny_x::MAX_POST_LENGTH },
                "chartId": { "type": "string", "description": "An id returned by render_chart." },
                "replyToPostId": { "type": "string", "pattern": "^\\d+$" }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        Access::Write,
        move |args| {
            let tool = Arc::clone(&tool);
            async move { post(&tool, args).await }
        },
    )
}

async fn post(tool: &PostChartTool, args: serde_json::Value) -> ToolResult {
    let args = match parse::<PostArgs>(args) {
        Ok(args) => args,
        Err(refused) => return refused,
    };
    if args.text.is_empty() {
        return invalid("text must not be empty");
    }
    if args
        .reply_to_post_id
        .as_deref()
        .is_some_and(|id| !is_x_id(id))
    {
        return invalid("replyToPostId must be a numeric X id");
    }
    let chart = match &args.chart_id {
        Some(id) => match tool.book.get(id) {
            Some(chart) => Some(chart),
            None => {
                return answer(format!(
                    "Refused: no chart with id {id}. Call render_chart and use the id it returns."
                ));
            }
        },
        None => None,
    };
    // The post ends on its read, not on an ask: a closing question is cut
    // here rather than argued about with the agent.
    let text = if asks_the_reader(&args.text) {
        without_the_ask(&args.text)
    } else {
        args.text.clone()
    };
    let reply_to = args.reply_to_post_id.as_deref().map(PostId::from);
    let draft = Draft {
        reply_to: reply_to.clone(),
        ..Draft::new(text)
    };
    let attach = || async {
        match &chart {
            Some(chart) => upload(&tool.x, chart.png.clone(), MediaType::Png).await,
            None => Ok(Vec::new()),
        }
    };
    let published = match tool.publisher.post_with_media(draft, attach).await {
        Ok(published) => published,
        Err(error) => return refusal_of(error),
    };
    if let Some(pacing) = &tool.pacing {
        pacing.record(&published, reply_to.as_ref()).await;
    }
    remember(tool, &published.text, args.chart_id.as_deref(), &published).await;
    if published.dry_run {
        let would = WouldHave {
            text: &published.text,
            chart_id: args.chart_id.as_deref(),
            reply_to_post_id: args.reply_to_post_id.as_deref(),
            chart: chart.as_ref().map(|chart| ChartSize {
                bytes: chart.png.len(),
                path: chart.path.as_ref().map(|path| path.display().to_string()),
            }),
        };
        return ToolResult::success(format!(
            "DRY RUN, nothing was published. Would have posted: {}",
            serde_json::to_string(&would).unwrap_or_default()
        ));
    }
    ToolResult::success(format!(
        "Published post {}{}.",
        published.id,
        if chart.is_some() {
            " with the chart attached"
        } else {
            ""
        }
    ))
}

/// The post as memory keeps it, rehearsals included and marked. Memory must
/// never lose a post that went out: an outage is logged, not returned, which
/// would read to the agent as a failed publish and invite a second post.
async fn remember(
    tool: &PostChartTool,
    text: &str,
    chart_id: Option<&str>,
    published: &teeny_x::Published,
) {
    let Some(memory) = &tool.memory else { return };
    let id = if published.dry_run {
        format!("dry-run:{}", chrono::Utc::now().timestamp_millis())
    } else {
        published.id.to_string()
    };
    let mut payload =
        json!({ "text": text, "symbol": tool.symbol, "published": !published.dry_run });
    if let Some(chart_id) = chart_id {
        payload["chart"] = json!(chart_id);
    }
    if let Err(error) = memory
        .capture_activity("chart_post", &id, payload, None)
        .await
    {
        tracing::warn!(%error, "post_chart: published but could not remember the post");
    }
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod tests;
