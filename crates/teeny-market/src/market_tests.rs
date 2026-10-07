use serde_json::{Value, json};
use wiremock::matchers::{path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::fixture;

const TOKEN: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

fn trade(hash: &str, at: &str, kind: TradeKind, wallet: &str, volume: f64) -> Trade {
    Trade {
        tx_hash: hash.into(),
        timestamp: at.into(),
        kind,
        wallet: wallet.into(),
        volume_usd: volume,
        token_amount: 1.0,
        price_usd: 1.0,
    }
}

fn trades() -> Vec<Trade> {
    vec![
        trade("1", "2026-09-08T10:00:00Z", TradeKind::Buy, "A", 500.0),
        trade("2", "2026-09-08T11:00:00Z", TradeKind::Buy, "A", 300.0),
        trade("3", "2026-09-08T12:00:00Z", TradeKind::Sell, "B", 200.0),
    ]
}

#[test]
fn nets_buys_against_sells_and_counts_each_wallet_once() {
    let summary = summarise_flow(&trades(), &HashMap::new());
    assert_eq!(summary.net_usd, 600.0);
    assert_eq!(summary.unique_wallets, 2);
    assert_eq!(summary.buyers, 1);
    assert_eq!(
        (
            summary.top_buyers[0].wallet.as_str(),
            summary.top_buyers[0].bought_usd
        ),
        ("A", 800.0)
    );
}

#[test]
fn reports_the_window_it_actually_read_so_a_post_cannot_claim_the_whole_day() {
    let summary = summarise_flow(&trades(), &HashMap::new());
    assert_eq!(
        summary.window,
        FlowWindow {
            from: "2026-09-08T10:00:00Z".into(),
            to: "2026-09-08T12:00:00Z".into(),
            trades: 3
        }
    );
}

#[test]
fn labels_a_watchlist_wallet_matching_case_insensitively() {
    let watchlist = HashMap::from([("a".to_owned(), "a friend of the project".to_owned())]);
    let summary = summarise_flow(&trades(), &watchlist);
    let labels: Vec<_> = summary
        .known
        .iter()
        .map(|entry| entry.label.as_deref())
        .collect();
    assert_eq!(labels, vec![Some("a friend of the project")]);
}

#[test]
fn says_nothing_rather_than_dividing_by_zero_on_an_empty_window() {
    let summary = summarise_flow(&[], &HashMap::new());
    assert_eq!(
        (
            summary.net_usd,
            summary.unique_wallets,
            summary.window.trades
        ),
        (0.0, 0, 0)
    );
    assert_eq!(
        (summary.window.from.as_str(), summary.window.to.as_str()),
        ("", "")
    );
}

#[test]
fn ranks_at_most_five_each_side_and_leaves_out_wallets_that_did_not_trade_that_side() {
    let mut many: Vec<Trade> = (0..7)
        .map(|i| {
            trade(
                &format!("b{i}"),
                "2026-09-08T10:00:00Z",
                TradeKind::Buy,
                &format!("W{i}"),
                f64::from(100 + i),
            )
        })
        .collect();
    many.push(trade(
        "s",
        "2026-09-08T11:00:00Z",
        TradeKind::Sell,
        "W0",
        50.0,
    ));
    let summary = summarise_flow(&many, &HashMap::new());
    assert_eq!(
        summary
            .top_buyers
            .iter()
            .map(|w| w.wallet.as_str())
            .collect::<Vec<_>>(),
        vec!["W6", "W5", "W4", "W3", "W2"]
    );
    assert_eq!(
        summary
            .top_sellers
            .iter()
            .map(|w| w.wallet.as_str())
            .collect::<Vec<_>>(),
        vec!["W0"]
    );
    assert_eq!(summary.top_sellers[0].net_usd, 50.0);
}

/// The public endpoints, on one mock server, with GeckoTerminal not waiting.
async fn market(server: &MockServer, gmgn: bool, network: &str) -> TokenMarket {
    let http = JsonClient::new();
    let providers = Providers {
        gecko: GeckoTerminalClient::new(http.clone(), network)
            .with_base_url(format!("{}/gecko", server.uri()))
            .with_min_interval(Duration::ZERO),
        dexscreener: DexscreenerClient::new(http.clone())
            .with_base_url(format!("{}/dex", server.uri())),
        solana: (network == "solana")
            .then(|| SolanaRpc::new(http.clone(), Some(format!("{}/rpc", server.uri())))),
        gmgn: gmgn.then(|| GmgnClient::new(http.clone(), format!("{}/gmgn", server.uri()))),
    };
    let watchlist = [KolWallet {
        wallet: "WALLETONE".into(),
        label: "a known KOL".into(),
    }];
    TokenMarket::with_providers(providers, &watchlist, Duration::from_secs(60))
}

async fn answer(server: &MockServer, at: &str, body: Value) {
    Mock::given(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn overviews(server: &MockServer) {
    answer(
        server,
        &format!("/dex/tokens/{TOKEN}"),
        fixture("dexscreener-tokens"),
    )
    .await;
    answer(
        server,
        &format!("/gecko/networks/solana/tokens/{TOKEN}"),
        fixture("geckoterminal-token"),
    )
    .await;
}

fn daily(count: i64) -> Value {
    let rows: Vec<Value> = (0..count)
        .rev()
        .map(|i| {
            json!([
                1_700_000_000 + i * 86_400,
                1 + i,
                2 + i,
                i,
                1.5 + i as f64,
                10
            ])
        })
        .collect();
    json!({ "data": { "attributes": { "ohlcv_list": rows } } })
}

#[tokio::test]
async fn merges_the_snapshot_filling_each_providers_gaps_from_the_others() {
    let server = MockServer::start().await;
    overviews(&server).await;
    let overview = market(&server, false, "solana")
        .await
        .overview(TOKEN)
        .await
        .unwrap();
    assert_eq!(overview.source, "dexscreener+geckoterminal");
    assert_eq!(
        overview.pool_address.as_deref(),
        Some("LivePool"),
        "the first provider's pool wins"
    );
    assert_eq!(
        overview.price_change.m5,
        Some(0.4),
        "only DexScreener has five-minute change"
    );
    assert_eq!(overview.market_cap_usd, Some(730_709.0));
}

#[tokio::test]
async fn takes_what_answered_when_a_provider_is_down() {
    let server = MockServer::start().await;
    answer(
        &server,
        &format!("/gecko/networks/solana/tokens/{TOKEN}"),
        fixture("geckoterminal-token"),
    )
    .await;
    let overview = market(&server, true, "solana")
        .await
        .overview(TOKEN)
        .await
        .unwrap();
    assert_eq!(overview.source, "geckoterminal");
    assert_eq!(overview.pool_address.as_deref(), Some("GEF9pool"));
}

#[tokio::test]
async fn says_why_nothing_could_price_the_token() {
    let server = MockServer::start().await;
    let error = market(&server, false, "solana")
        .await
        .overview(TOKEN)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with(&format!(
            "No market source could price {TOKEN}: gmgn not configured; "
        )),
        "{error}"
    );
    assert!(error.contains("answered 404"), "{error}");
}

#[tokio::test]
async fn builds_a_weekly_series_from_daily_candles_because_the_provider_has_none() {
    let server = MockServer::start().await;
    overviews(&server).await;
    Mock::given(path("/gecko/networks/solana/pools/LivePool/ohlcv/day"))
        .and(query_param("aggregate", "1"))
        .and(query_param("limit", "42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(daily(70)))
        .expect(1)
        .mount(&server)
        .await;
    let weekly = market(&server, false, "solana")
        .await
        .candles(TOKEN, Timeframe::W1, 5)
        .await
        .unwrap();
    assert_eq!(weekly.len(), 5);
    assert!(weekly.windows(2).all(|pair| pair[1].time > pair[0].time));
    assert!(weekly.iter().all(|candle| candle.time % 604_800 == 0));
}

#[tokio::test]
async fn serves_the_second_read_of_a_timeframe_from_cache_which_keeps_it_under_the_rate_limit() {
    let server = MockServer::start().await;
    overviews(&server).await;
    Mock::given(path("/gecko/networks/solana/pools/LivePool/ohlcv/hour"))
        .respond_with(ResponseTemplate::new(200).set_body_json(daily(3)))
        .expect(1)
        .mount(&server)
        .await;
    let market = market(&server, false, "solana").await;
    let first = market.candles(TOKEN, Timeframe::H1, 120).await.unwrap();
    let second = market.candles(TOKEN, Timeframe::H1, 120).await.unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn charts_from_gmgn_when_it_answers_and_from_geckoterminal_when_it_does_not() {
    let server = MockServer::start().await;
    overviews(&server).await;
    let gmgn = json!({ "code": 0, "data": { "list": [{ "time": 1_700_000_000, "open": 9, "high": 9, "low": 9, "close": 9 }] } });
    Mock::given(path(format!("/gmgn/api/v1/token_candles/sol/{TOKEN}")))
        .and(query_param("resolution", "4h"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gmgn))
        .mount(&server)
        .await;
    answer(
        &server,
        "/gecko/networks/solana/pools/LivePool/ohlcv/minute",
        daily(2),
    )
    .await;
    let market = market(&server, true, "solana").await;
    assert_eq!(
        market.candles(TOKEN, Timeframe::H4, 10).await.unwrap()[0].close,
        9.0
    );
    // GMGN answered nothing for the 5m series (404), so GeckoTerminal draws it.
    assert_eq!(
        market
            .candles(TOKEN, Timeframe::M5, 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn reports_flow_and_one_wallets_footprint_from_the_pools_trades() {
    let server = MockServer::start().await;
    overviews(&server).await;
    Mock::given(path("/gecko/networks/solana/pools/LivePool/trades"))
        .and(query_param("trade_volume_in_usd_greater_than", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("geckoterminal-trades")))
        .mount(&server)
        .await;
    let market = market(&server, false, "solana").await;
    let flow = market.flow(TOKEN, 100.0).await.unwrap();
    assert_eq!((flow.window.trades, flow.buyers, flow.sellers), (2, 1, 1));
    assert_eq!(
        flow.known.len(),
        1,
        "WalletOne is on the watchlist, in another case"
    );
    let one = market
        .wallet(TOKEN, "walletone", 100.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (one.wallet.as_str(), one.label.as_deref(), one.buys),
        ("walletone", Some("a known KOL"), 1)
    );
    assert_eq!(market.wallet(TOKEN, "Stranger", 100.0).await.unwrap(), None);
}

#[tokio::test]
async fn has_no_holders_off_solana_and_no_ranked_traders_without_gmgn() {
    let server = MockServer::start().await;
    let base = market(&server, false, "base").await;
    assert_eq!(
        base.holders(TOKEN).await.unwrap_err().to_string(),
        "Holder data is only wired up for Solana"
    );
    assert!(base.ranked_traders(TOKEN).await.is_empty());
    assert_eq!(base.sources(), vec!["dexscreener", "geckoterminal"]);
    // GMGN wired up but refusing reads as no data, not as an error.
    let solana = market(&server, true, "solana").await;
    assert!(solana.ranked_traders(TOKEN).await.is_empty());
    assert_eq!(
        solana.sources(),
        vec!["gmgn", "dexscreener", "geckoterminal", "solana-rpc"]
    );
}

#[tokio::test]
async fn names_the_missing_pool_rather_than_charting_nothing() {
    let server = MockServer::start().await;
    answer(
        &server,
        &format!("/gecko/networks/solana/tokens/{TOKEN}"),
        json!({ "data": { "attributes": { "price_usd": "1" } } }),
    )
    .await;
    let error = market(&server, false, "solana")
        .await
        .pool(TOKEN)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), format!("No pool known for {TOKEN}"));
}

#[test]
fn reads_its_wiring_from_the_chart_config() {
    let config = teeny_core::Config::from_vars([
        ("X_BOT_USERNAME", "teenydroid"),
        ("X_API_KEY", "k"),
        ("X_API_SECRET", "s"),
        ("X_ACCESS_TOKEN", "t"),
        ("X_ACCESS_TOKEN_SECRET", "ts"),
        ("CORTEX_TOKEN", "c"),
        ("LLM_BASE_URL", "http://ladder:6969/v1"),
        ("LLM_API_KEY", "l"),
        ("LLM_MODEL", "reasoning"),
        ("CHART_CHAIN", "base"),
        ("GMGN_BASE_URL", "https://gmgn.proxy.example"),
    ])
    .expect("a valid environment");
    assert_eq!(
        TokenMarket::from_config(&config.chart).sources(),
        vec!["gmgn", "dexscreener", "geckoterminal"]
    );
}
