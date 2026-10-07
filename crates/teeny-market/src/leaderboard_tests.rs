use serde_json::json;
use wiremock::matchers::{path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::fixture;

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

#[test]
fn parses_a_coingecko_row_and_drops_one_it_cannot_place() {
    let rows = fixture("coingecko-markets");
    let parsed = parse_coin(&rows[0]).unwrap();
    assert_eq!(
        parsed,
        ListedCoin {
            id: "fartcoin".into(),
            symbol: "FARTCOIN".into(),
            name: "Fartcoin".into(),
            price_usd: 0.1428,
            market_cap_usd: 142_859_822.0,
            circulating_supply: 999_972_851.0,
            rank: Some(217.0),
            board_rank: None,
            change24h_pct: Some(-1.2),
            ath_usd: Some(2.48),
            image_url: Some(
                "https://coin-images.coingecko.com/coins/images/50891/large/fart.jpg".into()
            ),
        }
    );
    assert_eq!(parse_coin(&rows[1]), None, "no market cap");
    assert_eq!(parse_coin(&rows[2]), None, "no supply");
    assert_eq!(parse_coin(&json!({ "id": "x", "symbol": "x", "current_price": 1, "market_cap": 1, "circulating_supply": 1 })).unwrap().name, "X");
}

fn row(index: usize) -> Value {
    json!({ "id": format!("c{index}"), "symbol": format!("c{index}"), "name": format!("C{index}"), "current_price": 1, "market_cap": 1e9 - index as f64, "circulating_supply": 1e6 })
}

#[tokio::test]
async fn pages_through_the_category_by_market_cap_numbers_the_board_and_fetches_the_coin_by_id() {
    let server = MockServer::start().await;
    let page = |n: &str, body: Value| {
        Mock::given(path("/coins/markets"))
            .and(query_param("vs_currency", "usd"))
            .and(query_param("category", "ai-meme-coins"))
            .and(query_param("order", "market_cap_desc"))
            .and(query_param("per_page", "250"))
            .and(query_param("page", n))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
    };
    page("1", Value::Array((1..=250).map(row).collect()))
        .expect(2)
        .mount(&server)
        .await;
    page("2", json!([row(251), row(252)]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/coins/markets"))
        .and(query_param("ids", "tiny-humans-ai"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": "tiny-humans-ai", "symbol": "tiny", "name": "Tiny", "current_price": 0.0005, "market_cap": 500_000, "circulating_supply": 1e9 }])))
        .expect(1)
        .mount(&server)
        .await;

    let client = CoinGeckoLeaderboard::new(JsonClient::new()).with_base_url(server.uri());
    let (own, list) = tokio::join!(
        client.coin("tiny-humans-ai"),
        client.category("ai-meme-coins", 500)
    );
    assert_eq!(own.unwrap().symbol, "TINY");
    let list = list.unwrap();
    assert_eq!(list.len(), 252);
    assert_eq!(list[0].board_rank, Some(1));
    assert_eq!(list[251].board_rank, Some(252));
    // A limit inside the first page stops there (the second page-1 call).
    assert_eq!(
        client.category("ai-meme-coins", 10).await.unwrap().len(),
        10
    );
}

#[tokio::test]
async fn says_which_coin_it_could_not_find() {
    let server = MockServer::start().await;
    Mock::given(path("/coins/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let client = CoinGeckoLeaderboard::new(JsonClient::new()).with_base_url(server.uri());
    assert_eq!(
        client.coin("gone").await.unwrap_err().to_string(),
        "CoinGecko has no market row for \"gone\""
    );
}

#[tokio::test]
async fn fetches_a_logo_only_as_png_or_jpeg_and_never_fails() {
    let server = MockServer::start().await;
    Mock::given(path("/a.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG.to_vec()))
        .mount(&server)
        .await;
    Mock::given(path("/a.webp"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57]),
        )
        .mount(&server)
        .await;
    let client = CoinGeckoLeaderboard::new(JsonClient::new());
    let png = client
        .fetch_logo(&format!("{}/a.png", server.uri()))
        .await
        .unwrap();
    assert_eq!((png.mime_type, png.data.len()), ("image/png", 12));
    assert_eq!(
        client.fetch_logo(&format!("{}/a.webp", server.uri())).await,
        None
    );
    assert_eq!(
        client
            .fetch_logo(&format!("{}/missing.png", server.uri()))
            .await,
        None
    );
    assert_eq!(client.fetch_logo("http://127.0.0.1:1/down.png").await, None);
}

#[tokio::test]
async fn refuses_a_logo_that_is_not_served_over_https() {
    let client = CoinGeckoLeaderboard::new(JsonClient::new());
    assert_eq!(
        client.logo("http://coin-images.coingecko.com/a.png").await,
        None
    );
    assert_eq!(client.logo("not a url").await, None);
}

#[test]
fn sniffs_png_and_jpeg_by_their_magic_bytes() {
    assert_eq!(sniff_image(&PNG), Some("image/png"));
    assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
    assert_eq!(
        sniff_image(&[0x89, 0x50, 0x4e, 0x47]),
        None,
        "too short to be a PNG"
    );
}
