use serde_json::json;
use wiremock::matchers::{header, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::fixture;

async fn answering(at: &str, response: ResponseTemplate) -> (MockServer, GmgnClient) {
    let server = MockServer::start().await;
    Mock::given(path(at))
        .respond_with(response)
        .mount(&server)
        .await;
    let client = GmgnClient::new(crate::test_support::no_wait(), format!("{}/", server.uri()));
    (server, client)
}

#[tokio::test]
async fn surfaces_a_refusal_instead_of_returning_an_empty_chart() {
    let body = json!({ "code": 1, "msg": "blocked" });
    let (_server, client) = answering(
        "/defi/quotation/v1/tokens/sol/TOKEN",
        ResponseTemplate::new(200).set_body_json(body),
    )
    .await;
    let error = client.overview("TOKEN").await.unwrap_err();
    assert_eq!(error.to_string(), "GMGN refused token overview: blocked");
}

/// What gmgn.ai actually answers a server with.
#[tokio::test]
async fn fails_plainly_on_the_cloudflare_interstitial() {
    let page = ResponseTemplate::new(403)
        .set_body_string("<!DOCTYPE html><title>Just a moment...</title>");
    let (_server, client) = answering("/defi/quotation/v1/tokens/sol/TOKEN", page).await;
    let error = client.overview("TOKEN").await.unwrap_err();
    assert!(error.to_string().contains("answered 403"), "{error}");
}

#[tokio::test]
async fn reads_the_snapshot_from_either_envelope_shape() {
    let token = json!({ "price": "0.5", "symbol": "TINY", "market_cap": 1000, "price_change_percent1h": 2, "buys_24h": 3, "sells_24h": "4", "pool_address": "Pool" });
    for body in [
        json!({ "code": 0, "data": { "token": token.clone() } }),
        json!({ "code": 0, "data": token.clone() }),
    ] {
        let (_server, client) = answering(
            "/defi/quotation/v1/tokens/sol/TOKEN",
            ResponseTemplate::new(200).set_body_json(body),
        )
        .await;
        let overview = client.overview("TOKEN").await.unwrap();
        assert_eq!(overview.price_usd, 0.5);
        assert_eq!(overview.chain, "sol");
        assert_eq!(
            overview.price_change,
            PriceChange {
                h1: Some(2.0),
                ..PriceChange::default()
            }
        );
        assert_eq!(
            overview.txns24h,
            Some(Txns {
                buys: 3.0,
                sells: 4.0
            })
        );
        assert_eq!(overview.pool_address.as_deref(), Some("Pool"));
        assert_eq!(overview.source, "gmgn");
    }
}

#[tokio::test]
async fn converts_millisecond_timestamps_which_would_otherwise_chart_50000_years_out() {
    let body = json!({ "code": 0, "data": { "list": [
        { "time": 1_700_003_600_000_i64, "open": 1, "high": 2, "low": 0.5, "close": 1.5, "volume": 3 },
        { "timestamp": 1_700_000_000, "open": 1, "high": 2, "low": 0.5, "close": 1.5 },
        { "time": 1_700_007_200, "open": 1, "high": 2, "low": 0.5 }
    ] } });
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/token_candles/sol/TOKEN"))
        .and(query_param("resolution", "1h"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = GmgnClient::new(JsonClient::new(), server.uri());
    let candles = client.candles("TOKEN", Timeframe::H1, 100).await.unwrap();
    assert_eq!(
        candles.iter().map(|c| c.time).collect::<Vec<_>>(),
        vec![1_700_000_000, 1_700_003_600]
    );
    assert_eq!(candles[0].volume, 0.0);
}

#[tokio::test]
async fn reads_ranked_traders_and_keeps_only_string_tags() {
    let server = MockServer::start().await;
    Mock::given(path("/defi/quotation/v1/tokens/top_traders/sol/TOKEN"))
        .and(header("x-proxy-key", "k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("gmgn-top-traders")))
        .mount(&server)
        .await;
    let client = GmgnClient::new(JsonClient::new(), server.uri())
        .with_headers(vec![("x-proxy-key".into(), "k".into())]);
    let traders = client.top_traders("TOKEN").await.unwrap();
    assert_eq!(
        traders,
        vec![
            RankedTrader {
                wallet: "SmartWallet".into(),
                realized_profit_usd: Some(12_345.6),
                bought_usd: Some(40_000.0),
                sold_usd: Some(52_345.6),
                holding_amount: Some(1_000.0),
                tags: vec!["smart_degen".into(), "kol".into()],
            },
            RankedTrader {
                wallet: "OtherWallet".into(),
                realized_profit_usd: None,
                bought_usd: Some(999.0),
                sold_usd: Some(0.0),
                holding_amount: Some(5.0),
                tags: vec![],
            },
        ]
    );
}

#[tokio::test]
async fn reads_a_bare_array_of_traders_too() {
    let body = json!({ "code": 0, "data": [{ "address": "A", "tags": [] }] });
    let (_server, client) = answering(
        "/defi/quotation/v1/tokens/top_traders/sol/TOKEN",
        ResponseTemplate::new(200).set_body_json(body),
    )
    .await;
    assert_eq!(client.top_traders("TOKEN").await.unwrap()[0].wallet, "A");
}

#[tokio::test]
async fn says_so_when_there_is_no_data() {
    let (_server, client) = answering(
        "/defi/quotation/v1/tokens/top_traders/sol/TOKEN",
        ResponseTemplate::new(200).set_body_json(json!({ "code": 0 })),
    )
    .await;
    assert_eq!(
        client.top_traders("TOKEN").await.unwrap_err().to_string(),
        "GMGN returned no data for top traders"
    );
}
