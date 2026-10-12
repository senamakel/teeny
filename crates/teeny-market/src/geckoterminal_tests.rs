use serde_json::json;
use wiremock::matchers::{path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::fixture;

const TOKEN: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

async fn server() -> (MockServer, GeckoTerminalClient) {
    let server = MockServer::start().await;
    let client = GeckoTerminalClient::new(JsonClient::new(), "solana")
        .with_base_url(server.uri())
        .with_min_interval(Duration::ZERO);
    (server, client)
}

async fn answer(server: &MockServer, at: &str, body: Value) {
    Mock::given(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn returns_candles_oldest_first_whatever_order_they_arrived_in_and_drops_malformed_rows() {
    let (server, client) = server().await;
    Mock::given(path("/networks/solana/pools/pool/ohlcv/hour"))
        .and(query_param("aggregate", "4"))
        .and(query_param("limit", "100"))
        .and(query_param("currency", "usd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("geckoterminal-ohlcv")))
        .mount(&server)
        .await;
    let candles = client.candles("pool", Timeframe::H4, 100).await.unwrap();
    let times: Vec<i64> = candles.iter().map(|c| c.time).collect();
    // The row with a null open is dropped; the one with no volume reads zero.
    assert_eq!(
        times,
        vec![1_699_992_800, 1_700_000_000, 1_700_003_600, 1_700_007_200]
    );
    assert_eq!(candles[0].volume, 0.0);
    assert_eq!(
        candles[1],
        Candle {
            time: 1_700_000_000,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 30.0
        }
    );
}

#[tokio::test]
async fn clamps_the_bar_count_to_what_the_api_accepts() {
    let (server, client) = server().await;
    Mock::given(path("/networks/solana/pools/pool/ohlcv/day"))
        .and(query_param("aggregate", "1"))
        .and(query_param("limit", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client
            .candles("pool", Timeframe::D1, 5_000)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn refuses_to_invent_a_price_when_the_token_is_unknown() {
    let (server, client) = server().await;
    answer(
        &server,
        "/networks/solana/tokens/missing",
        json!({ "data": { "attributes": {} } }),
    )
    .await;
    let error = client.overview("missing").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "GeckoTerminal returned no price for missing"
    );
}

#[tokio::test]
async fn reads_the_snapshot_and_charts_the_first_top_pool() {
    let (server, client) = server().await;
    Mock::given(path(format!("/networks/solana/tokens/{TOKEN}")))
        .and(query_param("include", "top_pools"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("geckoterminal-token")))
        .mount(&server)
        .await;
    let overview = client.overview(TOKEN).await.unwrap();
    assert_eq!(
        overview,
        TokenOverview {
            address: TOKEN.into(),
            chain: "solana".into(),
            name: Some("Tiny Humans AI".into()),
            symbol: Some("TINY".into()),
            price_usd: 0.00074711,
            market_cap_usd: Some(730_709.3),
            fdv_usd: Some(747_000.1),
            liquidity_usd: Some(93_428.5),
            volume24h_usd: Some(10_134.2),
            // "+17.38%" read as 17.38; no five-minute window from this provider.
            price_change: PriceChange {
                m5: None,
                h1: Some(-1.25),
                h6: Some(3.05),
                h24: Some(17.38)
            },
            txns24h: Some(Txns {
                buys: 79.0,
                sells: 75.0
            }),
            pool_address: Some("GEF9pool".into()),
            pool_name: Some("TINY / SOL".into()),
            pair_created_at: Some("2025-01-02T03:04:05Z".into()),
            source: "geckoterminal".into(),
        }
    );
}

#[tokio::test]
async fn reads_the_wallet_and_the_side_off_each_trade() {
    let (server, client) = server().await;
    Mock::given(path("/networks/solana/pools/pool/trades"))
        .and(query_param("trade_volume_in_usd_greater_than", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("geckoterminal-trades")))
        .mount(&server)
        .await;
    let trades = client.trades("pool", 100.0).await.unwrap();
    assert_eq!(trades.len(), 2, "the row with no wallet is dropped");
    assert_eq!(
        trades[0],
        Trade {
            tx_hash: "0xabc".into(),
            timestamp: "2026-09-08T17:51:03Z".into(),
            kind: TradeKind::Buy,
            wallet: "WalletOne".into(),
            volume_usd: 104.47,
            token_amount: 138_670.49,
            price_usd: 0.00075,
        }
    );
    // A sell is priced and sized on the side the token left from.
    assert_eq!(
        (trades[1].kind, trades[1].token_amount, trades[1].price_usd),
        (TradeKind::Sell, 200_000.0, 0.00074)
    );
}

#[tokio::test]
async fn never_asks_for_a_negative_floor() {
    let (server, client) = server().await;
    Mock::given(path("/networks/solana/pools/pool/trades"))
        .and(query_param("trade_volume_in_usd_greater_than", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client.trades("pool", -5.0).await.unwrap().is_empty());
}
