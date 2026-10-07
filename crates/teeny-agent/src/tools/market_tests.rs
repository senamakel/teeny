//! Port of legacy/test/market-tools.test.ts (the read-only half; the chart
//! half is chart_tests.rs) and community-tools.test.ts. The texts themselves
//! are pinned against the legacy goldens in teeny-market; these pin the
//! tools around them.

use std::sync::Arc;

use serde_json::json;
use teeny_market::MarketReports;
use teeny_market::news::NewsHit;
use teeny_x::model::XPost;

use super::*;
use crate::fakes::{FakeMarket, FakeNews, TOKEN};
use crate::tools::test_support::{FakeReads, by_name, call};
use crate::tools::{PermissionLevel, text_of};

fn reports() -> MarketReports {
    MarketReports::new(Arc::new(FakeMarket::default()), TOKEN).with_symbol("TINY")
}

#[test]
fn is_the_exact_tool_list_so_a_new_capability_is_a_conscious_change() {
    let names: Vec<String> = market_tools(reports())
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "token_overview",
            "market_context",
            "token_candles",
            "technical_analysis",
            "chart_timeframes",
            "token_progress",
            "token_holders",
            "token_flow",
            "trader_activity",
            "ranked_traders",
        ]
    );
    for tool in market_tools(reports()) {
        assert_eq!(
            tool.permission_level(),
            PermissionLevel::ReadOnly,
            "{}",
            tool.name()
        );
        assert!(!tool.external_effect());
    }
}

#[tokio::test]
async fn reports_the_source_alongside_the_numbers() {
    let tools = by_name(market_tools(reports()));
    let output = text_of(&call(&tools["token_overview"], json!({})).await);
    assert!(output.contains("$730.7K"), "{output}");
    assert!(output.contains("source: dexscreener"));
}

#[tokio::test]
async fn tells_the_agent_the_flow_window_is_short() {
    let tools = by_name(market_tools(reports()));
    let output = text_of(&call(&tools["token_flow"], json!({})).await);
    assert!(output.contains("in the last few hours"), "{output}");
}

#[tokio::test]
async fn says_plainly_what_it_does_not_have() {
    let tools = by_name(market_tools(reports()));
    let ranked = text_of(&call(&tools["ranked_traders"], json!({})).await);
    assert!(ranked.contains("No ranked-trader source is configured"));
    let context = text_of(&call(&tools["market_context"], json!({})).await);
    assert!(context.contains("No global market source is configured"));
    let holders = call(&tools["token_holders"], json!({})).await;
    assert!(!holders.is_error);
    assert!(text_of(&holders).starts_with("Holder data is unavailable right now"));
}

#[tokio::test]
async fn reads_the_chart_at_four_zoom_levels_and_the_long_horizon() {
    let tools = by_name(market_tools(reports()));
    let zooms = text_of(&call(&tools["chart_timeframes"], json!({})).await);
    assert!(zooms.contains("hourly (last 5 days)"));
    let candles = text_of(
        &call(
            &tools["token_candles"],
            json!({ "timeframe": "1h", "bars": 60 }),
        )
        .await,
    );
    assert!(candles.starts_with("60 1h bars"), "{candles}");
    let read = text_of(&call(&tools["technical_analysis"], json!({ "bars": 60 })).await);
    assert!(read.contains("bullish score:"), "{read}");
    let progress = text_of(&call(&tools["token_progress"], json!({})).await);
    assert!(progress.contains("This is the long horizon"), "{progress}");
}

#[tokio::test]
async fn refuses_arguments_outside_the_schema() {
    let tools = by_name(market_tools(reports()));
    assert!(
        call(&tools["token_candles"], json!({ "bars": 5 }))
            .await
            .is_error
    );
    assert!(
        call(&tools["technical_analysis"], json!({ "timeframe": "2h" }))
            .await
            .is_error
    );
    assert!(
        call(&tools["token_progress"], json!({ "days": 3 }))
            .await
            .is_error
    );
    assert!(
        call(&tools["token_flow"], json!({ "minVolumeUsd": -1 }))
            .await
            .is_error
    );
    assert!(
        call(&tools["trader_activity"], json!({ "wallet": "short" }))
            .await
            .is_error
    );
    let quiet = call(
        &tools["trader_activity"],
        json!({ "wallet": "WalletAAAAAAAAAAAAAAAAAAAAAAAA" }),
    )
    .await;
    assert!(
        text_of(&quiet).contains("has not traded this token"),
        "{}",
        text_of(&quiet)
    );
}

fn by(author: &str, id: &str) -> XPost {
    XPost {
        author_username: Some(author.into()),
        ..XPost::new(id, format!("{author}  says\nhi"))
    }
}

#[tokio::test]
async fn reads_the_community_without_the_account_s_own_posts() {
    let reads = Arc::new(FakeReads {
        posts: vec![by("fren", "1"), by("TeenyDroid", "2")],
        ..FakeReads::default()
    });
    let tool = community_pulse(
        Arc::new(XPostSearch(reads.clone())),
        "$TINY -is:retweet",
        "@teenydroid",
        10,
    );
    let output = text_of(&call(&tool, json!({ "maxResults": 30 })).await);
    assert!(
        output.starts_with("1 recent posts from the community:"),
        "{output}"
    );
    assert!(output.contains("- @fren: fren says hi"));
    assert!(!output.contains("TeenyDroid"));
    let searches = reads.searches.lock().unwrap();
    assert_eq!(searches[0].0, "$TINY -is:retweet");
    assert_eq!(searches[0].1.max_results, Some(10));
    drop(searches);
    assert!(call(&tool, json!({ "maxResults": 51 })).await.is_error);
}

#[tokio::test]
async fn reads_recent_news_with_the_default_topic() {
    let news = Arc::new(FakeNews {
        stories: vec![NewsHit {
            title: Some("BTC up".into()),
            url: "https://news/1".into(),
            published_date: None,
            text: Some("bitcoin rose".into()),
            highlights: None,
        }],
        ..FakeNews::default()
    });
    let tool = crypto_news(
        news.clone(),
        Some("crypto market news today bitcoin solana sentiment".into()),
    );
    let output = text_of(&call(&tool, json!({})).await);
    assert!(
        output.starts_with("1 stories from the last 3 days:"),
        "{output}"
    );
    assert_eq!(
        news.asked.lock().unwrap()[0].query,
        "crypto market news today bitcoin solana sentiment"
    );
    assert!(call(&tool, json!({ "days": 15 })).await.is_error);
    assert!(call(&tool, json!({ "numResults": 0 })).await.is_error);
}
