use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::fixture;

async fn rpc(supply: Value, largest: Value) -> (MockServer, SolanaRpc) {
    let server = MockServer::start().await;
    for (call, body) in [
        ("getTokenSupply", supply),
        ("getTokenLargestAccounts", largest),
    ] {
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({ "jsonrpc": "2.0", "method": call, "params": ["MINT"] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    let client = SolanaRpc::new(JsonClient::new(), Some(server.uri()));
    (server, client)
}

#[tokio::test]
async fn reads_supply_and_concentration_skipping_accounts_with_no_balance() {
    let (_server, client) = rpc(fixture("solana-supply"), fixture("solana-largest")).await;
    let snapshot = client.holders("MINT").await.unwrap();
    let total = 999_870_123.456;
    assert_eq!(snapshot.total_supply, total);
    assert_eq!(snapshot.decimals, 6);
    assert_eq!(snapshot.source, "solana-rpc");
    assert_eq!(
        snapshot
            .top
            .iter()
            .map(|a| a.address.as_str())
            .collect::<Vec<_>>(),
        vec!["PoolVault", "Holder2", "Holder3"]
    );
    assert_eq!(snapshot.top[0].share, 200_000_000.0 / total);
    let summed = 200_000_000.0 / total + 50_000_000.0 / total + 25_000_000.0 / total;
    assert_eq!(snapshot.top10_share, summed);
}

#[tokio::test]
async fn surfaces_an_rpc_error_with_its_message() {
    let error = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "Invalid param: not a Token mint" } });
    let (_server, client) = rpc(error, fixture("solana-largest")).await;
    let failure = client.holders("MINT").await.unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Solana RPC getTokenSupply: Invalid param: not a Token mint"
    );
}

#[tokio::test]
async fn will_not_compute_shares_of_no_supply() {
    let empty = json!({ "jsonrpc": "2.0", "id": 1, "result": { "value": { "amount": "0", "decimals": 6, "uiAmount": 0 } } });
    let (_server, client) = rpc(empty, fixture("solana-largest")).await;
    assert_eq!(
        client.holders("MINT").await.unwrap_err().to_string(),
        "Solana RPC returned no supply for MINT"
    );
}

#[tokio::test]
async fn names_the_method_when_the_rpc_rate_limits() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    let client = SolanaRpc::new(JsonClient::new(), Some(server.uri()));
    let error = client.holders("MINT").await.unwrap_err().to_string();
    assert!(
        error.starts_with("Solana RPC getToken") && error.ends_with("answered 429"),
        "{error}"
    );
}
