use std::io::Cursor;

use teeny_core::Secret;
use teeny_x::auth::ThreeLegged;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::cmd::test_support::printed;

async fn x_oauth() -> (MockServer, ThreeLegged) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/request_token"))
        .and(query_param("oauth_callback", "oob"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("oauth_token=req-tok&oauth_token_secret=req-sec"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .and(query_param("oauth_verifier", "7654321"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "oauth_token=USER-TOKEN-1&oauth_token_secret=USER-SECRET-1&screen_name=teenydroid",
        ))
        .mount(&server)
        .await;
    let flow = ThreeLegged::new("ck", Secret::new("cs")).with_base_url(server.uri());
    (server, flow)
}

#[tokio::test]
async fn prints_the_url_takes_the_pin_and_shows_the_pair_once_with_a_warning() {
    let (_server, flow) = x_oauth().await;
    let mut out = Vec::new();

    run(&flow, &mut Cursor::new("7654321\n"), &mut out, None)
        .await
        .unwrap();

    let printed = printed(out);
    assert!(
        printed.contains("https://x.com/oauth/authorize?oauth_token=req-tok"),
        "{printed}"
    );
    assert!(printed.contains("Paste the PIN"), "{printed}");
    assert_eq!(printed.matches("USER-TOKEN-1").count(), 1, "{printed}");
    assert_eq!(printed.matches("USER-SECRET-1").count(), 1, "{printed}");
    assert!(printed.contains("X_ACCESS_TOKEN=USER-TOKEN-1"), "{printed}");
    assert!(printed.contains("shown once"), "{printed}");
    assert!(printed.contains("@teenydroid"), "{printed}");
    assert!(!printed.contains("req-sec"), "{printed}");
}

#[tokio::test]
async fn writes_the_pair_into_the_env_file_without_printing_it() {
    let (_server, flow) = x_oauth().await;
    let file = std::env::temp_dir().join(format!("teeny-auth-x-{}.env", std::process::id()));
    std::fs::write(&file, "# keep me\nX_ACCESS_TOKEN=old\nDRY_RUN=true\n").unwrap();
    let mut out = Vec::new();

    run(&flow, &mut Cursor::new("7654321\n"), &mut out, Some(&file))
        .await
        .unwrap();

    let written = std::fs::read_to_string(&file).unwrap();
    std::fs::remove_file(&file).ok();
    assert_eq!(
        written,
        "# keep me\nX_ACCESS_TOKEN=USER-TOKEN-1\nDRY_RUN=true\nX_ACCESS_TOKEN_SECRET=USER-SECRET-1\n"
    );
    let printed = printed(out);
    assert!(!printed.contains("USER-TOKEN-1"), "{printed}");
    assert!(!printed.contains("USER-SECRET-1"), "{printed}");
    assert!(printed.contains("Wrote X_ACCESS_TOKEN and X_ACCESS_TOKEN_SECRET"), "{printed}");
}

#[tokio::test]
async fn an_empty_pin_stops_before_trading_it() {
    let (server, flow) = x_oauth().await;

    let error = run(&flow, &mut Cursor::new("\n"), &mut Vec::new(), None)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("PIN"), "{error}");
    let calls = server.received_requests().await.unwrap();
    assert!(calls.iter().all(|call| call.url.path() != "/oauth/access_token"));
}

#[test]
fn rewrites_a_key_in_place_or_appends_it() {
    assert_eq!(write_env_value("A=1\nB=2\n", "B", "3"), "A=1\nB=3\n");
    assert_eq!(write_env_value("A=1\n", "C", "3"), "A=1\nC=3\n");
    assert_eq!(write_env_value("A=1", "C", "3"), "A=1\nC=3\n");
    assert_eq!(write_env_value("", "C", "3"), "C=3\n");
    assert_eq!(write_env_value("XB=1\n", "B", "3"), "XB=1\nB=3\n");
}
