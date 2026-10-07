use serde_json::json;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::fixture;

const TOKEN: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

async fn answering(body: Value) -> (MockServer, DexscreenerClient) {
    let server = MockServer::start().await;
    Mock::given(path(format!("/tokens/{TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = DexscreenerClient::new(JsonClient::new()).with_base_url(server.uri());
    (server, client)
}

/// Caught on live data: $TINY's deepest pool held $139K and did $336 a day,
/// while the pool everybody trades held $93K and did $10K. Ranking on
/// liquidity charted the dead one.
#[tokio::test]
async fn picks_the_pool_with_the_volume_not_the_one_with_the_liquidity() {
    let (_server, client) = answering(fixture("dexscreener-tokens")).await;
    let overview = client.overview(TOKEN).await.unwrap();
    assert_eq!(overview.pool_address.as_deref(), Some("LivePool"));
    assert_eq!(overview.volume24h_usd, Some(10_134.2));
}

#[tokio::test]
async fn reads_the_whole_snapshot_off_that_pool() {
    let (_server, client) = answering(fixture("dexscreener-tokens")).await;
    let overview = client.overview(TOKEN).await.unwrap();
    assert_eq!(
        overview,
        TokenOverview {
            address: TOKEN.into(),
            chain: "solana".into(),
            name: Some("Tiny Humans AI".into()),
            symbol: Some("TINY".into()),
            price_usd: 0.000747,
            market_cap_usd: Some(730_709.0),
            fdv_usd: Some(747_200.0),
            liquidity_usd: Some(93_428.11),
            volume24h_usd: Some(10_134.2),
            price_change: PriceChange {
                m5: Some(0.4),
                h1: Some(-1.25),
                h6: Some(3.05),
                h24: Some(17.38)
            },
            txns24h: Some(Txns {
                buys: 79.0,
                sells: 75.0
            }),
            pool_address: Some("LivePool".into()),
            pool_name: Some("raydium".into()),
            pair_created_at: Some("2025-01-02T03:04:05.000Z".into()),
            source: "dexscreener".into(),
        }
    );
}

#[tokio::test]
async fn ignores_pairs_where_the_token_is_the_quote_side() {
    let body = json!({ "pairs": [{ "baseToken": { "address": "SOMETHING_ELSE" }, "priceUsd": "1", "volume": { "h24": 99 } }] });
    let (_server, client) = answering(body).await;
    let error = client.overview(TOKEN).await.unwrap_err();
    assert!(error.to_string().contains("no pair"), "{error}");
}

#[tokio::test]
async fn refuses_to_invent_a_price() {
    let body = json!({ "pairs": [{ "baseToken": { "address": TOKEN }, "chainId": "solana" }] });
    let (_server, client) = answering(body).await;
    let error = client.overview(TOKEN).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Dexscreener returned no price for {TOKEN}")
    );
}

#[tokio::test]
async fn treats_a_null_pair_list_as_no_pairs() {
    let (_server, client) = answering(json!({ "pairs": null })).await;
    assert!(client.overview(TOKEN).await.is_err());
}
