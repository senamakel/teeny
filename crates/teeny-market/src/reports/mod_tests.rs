use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use teeny_core::config::KolWallet;

use super::*;
use crate::error::MarketError;
use crate::global::GlobalSnapshot;
use crate::gmgn::RankedTrader;
use crate::market::{FlowSummary, WalletActivity, summarise_flow, wallet_activity};
use crate::test_support::golden;
use crate::types::{Candle, HolderSnapshot, TokenOverview, Trade};

const ADDRESS: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

/// The market a scenario describes: fixed answers, and the real flow and
/// wallet arithmetic over a fixed trades list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    name: String,
    symbol: Option<String>,
    watchlist: Vec<Labelled>,
    overview: TokenOverview,
    candles: HashMap<Timeframe, Vec<Candle>>,
    holders: Option<HolderSnapshot>,
    holders_error: Option<String>,
    ranked: Vec<RankedTrader>,
    global: Option<GlobalSnapshot>,
    trades: Vec<Trade>,
}

#[derive(Deserialize)]
struct Labelled {
    wallet: String,
    label: String,
}

impl Scenario {
    fn watchlist(&self) -> HashMap<String, String> {
        let wallets: Vec<KolWallet> = self
            .watchlist
            .iter()
            .map(|w| KolWallet {
                wallet: w.wallet.clone(),
                label: w.label.clone(),
            })
            .collect();
        wallets
            .iter()
            .map(|w| (w.wallet.to_lowercase(), w.label.clone()))
            .collect()
    }

    fn over(&self, floor: f64) -> Vec<Trade> {
        self.trades
            .iter()
            .filter(|t| t.volume_usd > floor.max(0.0))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl MarketData for Scenario {
    async fn overview(&self, _: &str) -> Result<TokenOverview> {
        Ok(self.overview.clone())
    }
    async fn candles(&self, _: &str, timeframe: Timeframe, limit: u32) -> Result<Vec<Candle>> {
        let series = self.candles.get(&timeframe).cloned().unwrap_or_default();
        Ok(series[series.len().saturating_sub(limit as usize)..].to_vec())
    }
    async fn holders(&self, _: &str) -> Result<HolderSnapshot> {
        match (&self.holders, &self.holders_error) {
            (_, Some(error)) => Err(MarketError::data(error.clone())),
            (Some(holders), None) => Ok(holders.clone()),
            (None, None) => Err(MarketError::data("no holders")),
        }
    }
    async fn flow(&self, _: &str, min_volume_usd: f64) -> Result<FlowSummary> {
        Ok(summarise_flow(
            &self.over(min_volume_usd),
            &self.watchlist(),
        ))
    }
    async fn wallet(
        &self,
        _: &str,
        wallet: &str,
        min_volume_usd: f64,
    ) -> Result<Option<WalletActivity>> {
        let mine: Vec<Trade> = self
            .over(min_volume_usd)
            .into_iter()
            .filter(|t| t.wallet.to_lowercase() == wallet.to_lowercase())
            .collect();
        Ok((!mine.is_empty()).then(|| wallet_activity(wallet, &mine, &self.watchlist())))
    }
    async fn ranked_traders(&self, _: &str) -> Vec<RankedTrader> {
        self.ranked.clone()
    }
}

struct Fixed(GlobalSnapshot);

#[async_trait]
impl GlobalSource for Fixed {
    async fn snapshot(&self) -> Result<GlobalSnapshot> {
        Ok(self.0.clone())
    }
}

#[derive(Deserialize)]
struct Call {
    tool: String,
    input: Value,
    output: String,
}

#[derive(Deserialize)]
struct Case {
    scenario: Scenario,
    outputs: Vec<Call>,
}

fn reports(scenario: Scenario) -> MarketReports {
    let global = scenario.global.clone();
    let symbol = scenario.symbol.clone();
    let mut reports = MarketReports::new(Arc::new(scenario), ADDRESS);
    if let Some(symbol) = symbol {
        reports = reports.with_symbol(symbol);
    }
    if let Some(global) = global {
        reports = reports.with_global(Arc::new(Fixed(global)));
    }
    reports
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> T {
    serde_json::from_value(input).expect("tool input")
}

async fn run(reports: &MarketReports, tool: &str, input: Value) -> String {
    match tool {
        "token_overview" => reports.token_overview().await.unwrap().text(),
        "market_context" => reports.market_context().await.unwrap().text(),
        "token_candles" => reports.token_candles(parse(input)).await.unwrap().text(),
        "technical_analysis" => reports
            .technical_analysis(parse(input))
            .await
            .unwrap()
            .text(),
        "chart_timeframes" => reports.chart_timeframes().await.text(),
        "token_progress" => reports.token_progress(parse(input)).await.unwrap().text(),
        "token_holders" => reports.token_holders().await.text(),
        "token_flow" => reports.token_flow(parse(input)).await.unwrap().text(),
        "trader_activity" => reports.trader_activity(&parse(input)).await.unwrap().text(),
        "ranked_traders" => reports.ranked_traders().await.text(),
        other => panic!("no tool {other}"),
    }
}

/// Every tool, on every scenario, says exactly what the TypeScript said.
#[tokio::test]
async fn hands_the_model_exactly_the_text_the_typescript_did() {
    for case in golden::<Vec<Case>>("tools") {
        let name = case.scenario.name.clone();
        let reports = reports(case.scenario);
        for call in case.outputs {
            let text = run(&reports, &call.tool, call.input.clone()).await;
            assert_eq!(text, call.output, "{name}: {} {}", call.tool, call.input);
        }
    }
}

fn scenario(name: &str) -> Scenario {
    golden::<Vec<Case>>("tools")
        .into_iter()
        .map(|case| case.scenario)
        .find(|s| s.name == name)
        .unwrap()
}

#[tokio::test]
async fn reports_the_source_alongside_the_numbers_so_nothing_is_quotable_without_one() {
    let text = reports(scenario("tiny"))
        .token_overview()
        .await
        .unwrap()
        .text();
    assert!(text.contains("$730.7K"));
    assert!(text.contains("source: dexscreener+geckoterminal"));
}

#[tokio::test]
async fn tells_the_agent_the_flow_window_is_short_rather_than_letting_it_say_today() {
    let text = reports(scenario("tiny"))
        .token_flow(FlowInput::default())
        .await
        .unwrap()
        .text();
    assert!(text.contains("in the last few hours"));
    assert!(text.contains("never name a wallet address"));
}

#[tokio::test]
async fn says_plainly_that_there_is_no_kol_data_rather_than_returning_an_empty_list() {
    let text = reports(scenario("falling")).ranked_traders().await.text();
    assert!(text.contains("No ranked-trader source is configured"));
    assert!(text.contains("Do not claim any"));
}

#[tokio::test]
async fn leads_with_outperformance_when_the_market_is_red_and_the_token_is_not() {
    let text = reports(scenario("tiny"))
        .market_context()
        .await
        .unwrap()
        .text();
    assert!(text.contains("TRUE POSITIVES YOU CAN LEAD WITH"));
    assert!(text.contains("outperforming"));
    assert!(text.contains("The whole market is red today"));
}

#[tokio::test]
async fn refuses_to_manufacture_a_comparison_the_token_did_not_win() {
    let mut tiny = scenario("tiny");
    tiny.global = scenario("falling").global;
    let text = reports(tiny).market_context().await.unwrap().text();
    assert!(text.contains("Do not force a comparison that is not there"));
}

#[tokio::test]
async fn says_plainly_when_there_is_no_global_source_rather_than_guessing() {
    let text = reports(scenario("young"))
        .market_context()
        .await
        .unwrap()
        .text();
    assert!(text.contains("No global market source is configured"));
}

#[tokio::test]
async fn reads_the_chart_at_four_zoom_levels_and_says_whether_they_agree() {
    let text = reports(scenario("tiny")).chart_timeframes().await.text();
    for label in ["hourly", "4-hour", "daily", "weekly"] {
        assert!(text.contains(label), "{label}");
    }
    assert!(
        text.contains("timeframes read bullish") || text.contains("agreement is itself the story")
    );
}

#[tokio::test]
async fn tells_the_agent_where_to_look_when_no_timeframe_is_bullish_on_price() {
    let mut falling = scenario("falling");
    let bars: Vec<Candle> = (0..60)
        .map(|i| {
            let step = f64::from(i) * 0.000_02;
            Candle {
                time: 1_700_000_000 + i64::from(i) * 3_600,
                open: 0.002 - step,
                high: 0.002 - step,
                low: 0.0019 - step,
                close: 0.0019 - step,
                volume: 100.0,
            }
        })
        .collect();
    falling.candles = [Timeframe::H1, Timeframe::H4, Timeframe::D1, Timeframe::W1]
        .into_iter()
        .map(|t| (t, bars.clone()))
        .collect();
    let text = reports(falling).chart_timeframes().await.text();
    assert!(text.contains("The positive is on-chain today"), "{text}");
}

#[tokio::test]
async fn reads_the_long_horizon_off_daily_candles() {
    let text = reports(scenario("tiny"))
        .token_progress(ProgressInput::default())
        .await
        .unwrap()
        .text();
    assert!(text.contains("days closed green"));
    assert!(text.contains("long horizon"));
}

#[tokio::test]
async fn carries_the_bullish_score_with_the_warning_attached_to_it() {
    let input = CandlesInput {
        timeframe: Timeframe::H1,
        bars: 60,
    };
    let text = reports(scenario("tiny"))
        .technical_analysis(input)
        .await
        .unwrap()
        .text();
    assert!(text.contains("bullish score:"));
    assert!(text.contains("Below 40 the short-term tape is weak"));
    assert!(text.contains("TRUE POSITIVES YOU CAN LEAD WITH"));
}

#[tokio::test]
async fn degrades_holder_data_to_a_sentence_rather_than_failing_the_run() {
    let text = reports(scenario("falling")).token_holders().await.text();
    assert_eq!(
        text,
        "Holder data is unavailable right now: Solana RPC getTokenLargestAccounts answered 429"
    );
}

#[test]
fn reads_tool_inputs_with_the_schemas_defaults_and_bounds() {
    let candles: CandlesInput = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(
        candles,
        CandlesInput {
            timeframe: Timeframe::H1,
            bars: 120
        }
    );
    let flow: FlowInput = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(flow.min_volume_usd, 100.0);
    let trader: TraderActivityInput =
        serde_json::from_value(serde_json::json!({ "wallet": "W" })).unwrap();
    assert_eq!(trader.min_volume_usd, 0.0);
    assert!(validate_bars(19).is_err() && validate_bars(500).is_ok());
    assert!(validate_days(6).is_err() && validate_days(365).is_ok());
    assert!(validate_min_volume(-1.0).is_err() && validate_min_volume(100.0).is_ok());
    assert!(validate_wallet("short").is_err() && validate_wallet(&"x".repeat(44)).is_ok());
}

#[test]
fn shows_wallets_truncated() {
    assert_eq!(
        short("2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump"),
        "2AF7…pump"
    );
    assert_eq!(short("tiny"), "tiny");
}
