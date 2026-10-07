use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use base64::Engine as _;

use super::*;

const TOKEN: &str = "a-token-long-enough-to-pass";

fn basic(user_and_password: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(user_and_password))
}

fn headers(authorization: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, HeaderValue::from_str(authorization).unwrap());
    headers
}

#[test]
fn reads_a_bearer_token_and_a_basic_password() {
    assert_eq!(token_from(Some("Bearer abc")).as_deref(), Some("abc"));
    assert_eq!(token_from(Some(&basic("operator:abc"))).as_deref(), Some("abc"));
    // No colon: the whole decoded string is the token.
    assert_eq!(token_from(Some(&basic("abc"))).as_deref(), Some("abc"));
    // Only the first colon splits.
    assert_eq!(token_from(Some(&basic("u:a:b"))).as_deref(), Some("a:b"));
    assert_eq!(token_from(Some("Token abc")), None);
    assert_eq!(token_from(None), None);
}

#[test]
fn matches_only_the_exact_token() {
    assert!(matches_token(Some(TOKEN), TOKEN));
    assert!(!matches_token(Some("a-token-long-enough-to-pas"), TOKEN));
    assert!(!matches_token(Some("a-token-long-enough-to-paZ"), TOKEN));
    assert!(!matches_token(Some(""), TOKEN));
    assert!(!matches_token(None, TOKEN));
}

#[test]
fn lets_the_operator_through() {
    assert!(authorise(Some(TOKEN), &headers(&format!("Bearer {TOKEN}")), false).is_ok());
    assert!(authorise(Some(TOKEN), &headers(&basic(&format!("operator:{TOKEN}"))), true).is_ok());
}

#[test]
fn challenges_a_script_with_bearer_and_a_browser_with_basic() {
    let script = authorise(Some(TOKEN), &HeaderMap::new(), false).unwrap_err();
    assert_eq!(script.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(script.headers()[header::WWW_AUTHENTICATE], "Bearer");

    let browser = authorise(Some(TOKEN), &headers("Bearer nope"), true).unwrap_err();
    assert_eq!(browser.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(browser.headers()[header::WWW_AUTHENTICATE], "Basic realm=\"teeny\", charset=\"UTF-8\"");
}

#[test]
fn is_closed_when_no_token_is_set() {
    let closed = authorise(None, &headers(&format!("Bearer {TOKEN}")), false).unwrap_err();
    assert_eq!(closed.status(), StatusCode::SERVICE_UNAVAILABLE);
}
