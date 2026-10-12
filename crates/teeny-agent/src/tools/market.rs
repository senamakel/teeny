//! The ten read-only market tools (`legacy/src/agent/tools/market.ts`),
//! `community_pulse` (`tools/community.ts`) and `crypto_news`
//! (`tools/news.ts`). The data and the text are teeny-market's; this is the
//! tool around them. The token is bound when the [`MarketReports`] is built,
//! never named by the model.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_market::community::{CommunityPost, PostSearch, PulseInput, SearchError};
use teeny_market::news::{NewsInput, NewsSearch};
use teeny_market::reports::{
    CandlesInput, FlowInput, ProgressInput, TraderActivityInput, validate_bars, validate_days,
    validate_min_volume, validate_wallet,
};
use teeny_market::{MarketReports, Report};
use teeny_x::cache::XReads;
use teeny_x::model::{SearchOptions, SortOrder};

use super::{Access, HostTool, Tool, ToolResult, failed, invalid, js_len, parse};

const TIMEFRAMES: [&str; 7] = ["5m", "15m", "1h", "4h", "12h", "1d", "1w"];

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn candles_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "timeframe": { "type": "string", "enum": TIMEFRAMES, "default": "1h" },
            "bars": { "type": "integer", "minimum": 20, "maximum": 500, "default": 120 }
        },
        "additionalProperties": false
    })
}

/// A market tool: `run` gets the reports and the call's arguments.
fn report_tool<F, Fut>(
    name: &str,
    description: &str,
    schema: Value,
    reports: &MarketReports,
    run: F,
) -> Arc<dyn Tool>
where
    F: Fn(MarketReports, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ToolResult> + Send + 'static,
{
    let reports = reports.clone();
    let run = Arc::new(run);
    HostTool::build(name, description, schema, Access::Read, move |args| {
        let (reports, run) = (reports.clone(), Arc::clone(&run));
        async move { run(reports, args).await }
    })
}

/// A report's text, or the failure.
fn text<R: Report>(report: teeny_market::Result<R>) -> ToolResult {
    match report {
        Ok(report) => ToolResult::success(report.text()),
        Err(error) => failed(error),
    }
}

fn candles_args(args: Value) -> Result<CandlesInput, ToolResult> {
    let input = parse::<CandlesInput>(args)?;
    validate_bars(input.bars).map_err(invalid)?;
    Ok(input)
}

/// The ten, in the legacy order.
pub fn market_tools(reports: MarketReports) -> Vec<Arc<dyn Tool>> {
    vec![
        report_tool(
            "token_overview",
            "The live snapshot for the token this account posts about: price, market cap, liquidity, 24h volume, \
             price change and buy/sell counts. Call this first. Every number it returns is quotable in a post; \
             nothing else is.",
            no_args(),
            &reports,
            |reports, _| async move { text(reports.token_overview().await) },
        ),
        report_tool(
            "market_context",
            "How the token is doing against BTC, ETH, SOL and the whole crypto market over the same 24 hours. Call \
             this on every run: a percentage on its own means little, and the same number is a quiet day in a \
             green market and a remarkable one in a red one.",
            no_args(),
            &reports,
            |reports, _| async move { text(reports.market_context().await) },
        ),
        report_tool(
            "token_candles",
            "Raw OHLCV for the token. Use it when a post needs a specific bar; for the shape of the move use \
             technical_analysis instead, which reads the same data properly.",
            candles_schema(),
            &reports,
            |reports, args| async move {
                match candles_args(args) {
                    Ok(input) => text(reports.token_candles(input).await),
                    Err(refused) => refused,
                }
            },
        ),
        report_tool(
            "technical_analysis",
            "Reads the chart the way one of these actually trades: support and resistance levels that have been \
             defended more than once, whether one has just broken, been reclaimed or been lost, whether volume \
             confirmed it, how volume compares to earlier in the window, trend and structure, and a bullish score. \
             No oscillators: RSI and MACD say nothing useful about a token like this. Everything it returns is \
             computed from real candles and is safe to quote. Facts about the past only.",
            candles_schema(),
            &reports,
            |reports, args| async move {
                match candles_args(args) {
                    Ok(input) => text(reports.technical_analysis(input).await),
                    Err(refused) => refused,
                }
            },
        ),
        report_tool(
            "chart_timeframes",
            "The same chart at four zoom levels at once: hourly, 4-hour, daily and weekly. Each one gets its \
             trend, structure, any level just broken or reclaimed, its nearest support and its own best positive, \
             plus whether the timeframes agree. Call this early: it is how you choose which horizon the post \
             should be about, and it makes the later calls free because the candles are cached.",
            no_args(),
            &reports,
            |reports, _| async move { ToolResult::success(reports.chart_timeframes().await.text()) },
        ),
        report_tool(
            "token_progress",
            "Long-term progress on the daily: change over 7, 30 and 90 days, how many days closed green, distance \
             from the daily high and low, and whether the daily lows are stepping up. Use this whenever the post \
             is about where we are rather than about today, and always on a day when the short-term read is weak.",
            json!({
                "type": "object",
                "properties": { "days": { "type": "integer", "minimum": 7, "maximum": 365, "default": 120 } },
                "additionalProperties": false
            }),
            &reports,
            |reports, args| async move {
                let input = match parse::<ProgressInput>(args) {
                    Ok(input) => input,
                    Err(refused) => return refused,
                };
                if let Err(why) = validate_days(input.days) {
                    return invalid(why);
                }
                text(reports.token_progress(input).await)
            },
        ),
        report_tool(
            "token_holders",
            "Supply and holder concentration, read from the chain. Useful for a post about distribution; useless \
             as a claim about any individual.",
            no_args(),
            &reports,
            |reports, _| async move { ToolResult::success(reports.token_holders().await.text()) },
        ),
        report_tool(
            "token_flow",
            "On-chain flow for the token over the most recent trades: buy versus sell volume, how many wallets on \
             each side, the biggest buyers and sellers, and any watchlist wallet that traded. The window is short \
             -- it is the provider's recent trades page.",
            json!({
                "type": "object",
                "properties": {
                    "minVolumeUsd": {
                        "type": "number", "minimum": 0, "maximum": 1_000_000, "default": 100,
                        "description": "Ignore trades smaller than this, in USD. 100 filters out dust."
                    }
                },
                "additionalProperties": false
            }),
            &reports,
            |reports, args| async move {
                let input = match parse::<FlowInput>(args) {
                    Ok(input) => input,
                    Err(refused) => return refused,
                };
                if let Err(why) = validate_min_volume(input.min_volume_usd) {
                    return invalid(why);
                }
                text(reports.token_flow(input).await)
            },
        ),
        // Named for the trader rather than the wallet on purpose: the
        // capabilities test refuses any tool whose name contains "wallet".
        report_tool(
            "trader_activity",
            "What one wallet did in this token over the recent trades window. Use it to check a claim about a \
             wallet, not to build a post around one.",
            json!({
                "type": "object",
                "properties": {
                    "wallet": { "type": "string", "minLength": 26, "maxLength": 64 },
                    "minVolumeUsd": { "type": "number", "minimum": 0, "maximum": 1_000_000, "default": 0 }
                },
                "required": ["wallet"],
                "additionalProperties": false
            }),
            &reports,
            |reports, args| async move {
                let input = match parse::<TraderActivityInput>(args) {
                    Ok(input) => input,
                    Err(refused) => return refused,
                };
                if let Err(why) = validate_wallet(&input.wallet)
                    .and_then(|()| validate_min_volume(input.min_volume_usd))
                {
                    return invalid(why);
                }
                text(reports.trader_activity(&input).await)
            },
        ),
        report_tool(
            "ranked_traders",
            "The wallets a ranked-trader provider labels on this token: KOLs, smart money, whales. Returns nothing \
             unless such a provider is configured, and \"nothing\" means the account must not claim anything about \
             KOLs.",
            no_args(),
            &reports,
            |reports, _| async move { ToolResult::success(reports.ranked_traders().await.text()) },
        ),
    ]
}

/// Recent posts from X, as the community pulse reads them.
pub struct XPostSearch(pub Arc<dyn XReads>);

#[async_trait]
impl PostSearch for XPostSearch {
    async fn search_recent(
        &self,
        query: &str,
        max_results: u32,
    ) -> Result<Vec<CommunityPost>, SearchError> {
        let options = SearchOptions {
            max_results: Some(max_results),
            sort_order: Some(SortOrder::Recency),
            ..SearchOptions::default()
        };
        Ok(self
            .0
            .search_posts(query, &options)
            .await?
            .into_iter()
            .map(|post| CommunityPost {
                id: post.id.to_string(),
                text: post.text,
                author_username: post.author_username,
                created_at: post.created_at,
            })
            .collect())
    }
}

/// `community_pulse`: one search for `query` (`CHART_COMMUNITY_QUERY`),
/// the account's own posts left out, held to `search_limit`.
pub fn community_pulse(
    search: Arc<dyn PostSearch>,
    query: &str,
    bot_username: &str,
    search_limit: u32,
) -> Arc<dyn Tool> {
    let (query, bot) = (query.to_owned(), bot_username.to_owned());
    HostTool::build(
        "community_pulse",
        "Reads what the community has been posting about the token in the last few days: who is talking, and \
         about what. Use it once, to find out whether the post should answer something people are already \
         discussing.",
        json!({
            "type": "object",
            "properties": { "maxResults": { "type": "integer", "minimum": 10, "maximum": 50, "default": 10 } },
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let (search, query, bot) = (Arc::clone(&search), query.clone(), bot.clone());
            async move {
                let input = match parse::<PulseInput>(args) {
                    Ok(input) => input,
                    Err(refused) => return refused,
                };
                if !(10..=50).contains(&input.max_results) {
                    return invalid("maxResults must be between 10 and 50");
                }
                let report = teeny_market::community::community_pulse(
                    search.as_ref(),
                    &query,
                    &bot,
                    Some(search_limit),
                    input,
                )
                .await;
                ToolResult::success(report.text())
            }
        },
    )
}

/// `crypto_news`: the last few days of market news. `default_topic` is what
/// an empty topic searches for.
pub fn crypto_news(news: Arc<dyn NewsSearch>, default_topic: Option<String>) -> Arc<dyn Tool> {
    HostTool::build(
        "crypto_news",
        "Recent crypto and market news, bounded to the last few days. Use it to know what kind of day the whole \
         market is having, so the post can place itself against that rather than talking about one chart in a \
         vacuum.",
        json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string", "maxLength": 200, "description": "What to search for. Leave empty for general crypto market news." },
                "days": { "type": "integer", "minimum": 1, "maximum": 14, "default": 3 },
                "numResults": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 }
            },
            "additionalProperties": false
        }),
        Access::Read,
        move |args| {
            let (news, default_topic) = (Arc::clone(&news), default_topic.clone());
            async move {
                let input = match parse::<NewsInput>(args) {
                    Ok(input) => input,
                    Err(refused) => return refused,
                };
                if input
                    .topic
                    .as_deref()
                    .is_some_and(|topic| js_len(topic) > 200)
                    || !(1..=14).contains(&input.days)
                    || !(1..=10).contains(&input.num_results)
                {
                    return invalid(
                        "topic is at most 200 characters, days 1 to 14, numResults 1 to 10",
                    );
                }
                let report = teeny_market::news::crypto_news(
                    news.as_ref(),
                    &input,
                    default_topic.as_deref(),
                )
                .await;
                ToolResult::success(report.text())
            }
        },
    )
}

#[cfg(test)]
#[path = "market_tests.rs"]
mod tests;
