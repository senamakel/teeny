use teeny_core::Secret;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn flow(server: &MockServer) -> ThreeLegged {
    ThreeLegged::new("ck", Secret::new("cs")).with_base_url(server.uri())
}

fn header(request: &wiremock::Request) -> String {
    request
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn asks_for_a_request_token_with_the_callback_and_no_user_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/request_token"))
        .and(query_param("oauth_callback", "oob"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "oauth_token=req-token&oauth_token_secret=req-secret&oauth_callback_confirmed=true",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let request = flow(&server).request_token("oob").await.unwrap();

    assert_eq!(request.token, "req-token");
    assert_eq!(request.secret.expose(), "req-secret");
    let sent = &server.received_requests().await.unwrap()[0];
    let auth = header(sent);
    assert!(auth.starts_with("OAuth "), "{auth}");
    assert!(auth.contains("oauth_consumer_key=\"ck\""), "{auth}");
    assert!(!auth.contains("oauth_token="), "{auth}");
}

#[test]
fn the_authorize_url_names_the_request_token() {
    let request = RequestToken {
        token: "req token".into(),
        secret: Secret::new("s"),
    };
    assert_eq!(
        authorize_url(&request),
        "https://x.com/oauth/authorize?oauth_token=req%20token"
    );
}

#[tokio::test]
async fn trades_the_pin_for_the_access_pair_signed_with_the_request_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .and(query_param("oauth_verifier", "1234567"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "oauth_token=user-token&oauth_token_secret=user-secret&user_id=42&screen_name=teenydroid",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let request = RequestToken {
        token: "req-token".into(),
        secret: Secret::new("req-secret"),
    };

    let access = flow(&server)
        .access_token(&request, " 1234567\n")
        .await
        .unwrap();

    assert_eq!(access.token, "user-token");
    assert_eq!(access.secret.expose(), "user-secret");
    assert_eq!(access.screen_name.as_deref(), Some("teenydroid"));
    let auth = header(&server.received_requests().await.unwrap()[0]);
    assert!(auth.contains("oauth_token=\"req-token\""), "{auth}");
}

#[tokio::test]
async fn a_refusal_is_an_error_with_the_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/request_token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Invalid consumer"))
        .mount(&server)
        .await;

    let error = flow(&server).request_token("oob").await.unwrap_err();

    assert!(
        matches!(error, XError::Http { status: 401, .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_answer_without_the_pair_is_missing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/request_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("oauth_callback_confirmed=true"))
        .mount(&server)
        .await;

    let error = flow(&server).request_token("oob").await.unwrap_err();

    assert!(matches!(error, XError::Missing(_)), "{error:?}");
}
