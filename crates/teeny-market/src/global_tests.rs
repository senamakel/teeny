use serde::Deserialize;
use serde_json::Value;

use super::*;
use crate::test_support::{assert_same_json, golden};

fn market() -> GlobalSnapshot {
    GlobalSnapshot {
        total_market_cap_usd: Some(2_699_012_632_815.0),
        total_change24h_pct: Some(-1.8),
        btc_dominance_pct: None,
        assets: vec![
            Asset {
                symbol: "BTC".into(),
                price_usd: 78_327.0,
                change24h_pct: Some(-1.11),
            },
            Asset {
                symbol: "SOL".into(),
                price_usd: 103.33,
                change24h_pct: Some(-0.61),
            },
        ],
        source: "coingecko".into(),
    }
}

/// The sentence the account exists to be able to say on a red day.
#[test]
fn finds_the_outperformance_when_the_market_is_down_and_the_token_is_not() {
    let strength = relative_strength(14.2, &market());
    assert_eq!(strength.market_direction, MarketDirection::Down);
    assert!(strength.beating[0].contains("the whole crypto market"));
    assert!(strength.beating[0].contains("16 points"));
}

#[test]
fn ranks_the_biggest_gap_first_so_the_best_true_comparison_is_the_easiest_to_reach() {
    let strength = relative_strength(14.2, &market());
    assert_eq!(strength.beating.len(), 3);
    let best = strength
        .against
        .iter()
        .map(|c| c.points)
        .fold(f64::NEG_INFINITY, f64::max);
    let best_what = &strength
        .against
        .iter()
        .find(|c| c.points == best)
        .unwrap()
        .what;
    assert!(strength.beating[0].contains(best_what.as_str()));
}

#[test]
fn offers_nothing_rather_than_a_losing_comparison() {
    assert!(relative_strength(-9.0, &market()).beating.is_empty());
}

#[test]
fn calls_a_flat_market_flat_rather_than_a_direction() {
    let flat = GlobalSnapshot {
        total_change24h_pct: Some(0.1),
        ..market()
    };
    assert_eq!(
        relative_strength(2.0, &flat).market_direction,
        MarketDirection::Flat
    );
}

#[derive(Deserialize)]
struct Golden {
    change: f64,
    snapshot: GlobalSnapshot,
    strength: Value,
}

#[test]
fn matches_the_typescript_exactly() {
    for case in golden::<Vec<Golden>>("strength") {
        let strength = relative_strength(case.change, &case.snapshot);
        assert_same_json(
            &strength,
            &case.strength,
            &format!("relativeStrength({})", case.change),
        );
    }
}

mod client {
    use wiremock::matchers::{path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::JsonClient;
    use crate::test_support::fixture;

    async fn serve(global: bool, prices: bool) -> (MockServer, GlobalMarketClient) {
        let server = MockServer::start().await;
        if global {
            Mock::given(path("/global"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fixture("coingecko-global")))
                .mount(&server)
                .await;
        }
        if prices {
            Mock::given(path("/simple/price"))
                .and(query_param("ids", "bitcoin,solana,ethereum"))
                .and(query_param("include_24hr_change", "true"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(fixture("coingecko-simple-price")),
                )
                .mount(&server)
                .await;
        }
        let client = GlobalMarketClient::new(JsonClient::new()).with_base_url(server.uri());
        (server, client)
    }

    #[tokio::test]
    async fn reads_the_majors_and_the_whole_market_change() {
        let (_server, client) = serve(true, true).await;
        let snapshot = client.snapshot().await.unwrap();
        assert_eq!(snapshot.total_change24h_pct, Some(-1.8));
        assert_eq!(snapshot.total_market_cap_usd, Some(2_699_012_632_815.4));
        assert_eq!(snapshot.btc_dominance_pct, Some(58.3));
        assert_eq!(
            snapshot.assets,
            vec![
                Asset {
                    symbol: "BTC".into(),
                    price_usd: 78_327.0,
                    change24h_pct: Some(-1.11)
                },
                Asset {
                    symbol: "ETH".into(),
                    price_usd: 2_412.07,
                    change24h_pct: None
                },
                Asset {
                    symbol: "SOL".into(),
                    price_usd: 103.33,
                    change24h_pct: Some(-0.61)
                },
            ]
        );
        assert_eq!(snapshot.source, "coingecko");
    }

    #[tokio::test]
    async fn still_answers_when_one_of_the_two_endpoints_is_down() {
        let (_server, client) = serve(false, true).await;
        let snapshot = client.snapshot().await.unwrap();
        assert_eq!(snapshot.assets.len(), 3);
        assert_eq!(snapshot.total_market_cap_usd, None);
    }

    #[tokio::test]
    async fn fails_only_when_both_are_down() {
        let (_server, client) = serve(false, false).await;
        assert_eq!(
            client.snapshot().await.unwrap_err().to_string(),
            "No global market data is available right now"
        );
    }
}
