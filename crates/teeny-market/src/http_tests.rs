use std::sync::Mutex;

use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn recording() -> (JsonClient, Arc<Mutex<Vec<Duration>>>) {
    let slept = Arc::new(Mutex::new(Vec::new()));
    let log = slept.clone();
    let client = JsonClient::new().with_sleep(Arc::new(move |duration| {
        log.lock().unwrap().push(duration);
        Box::pin(async {})
    }));
    (client, slept)
}

#[tokio::test]
async fn waits_as_long_as_the_server_asked_rather_than_on_its_own_schedule() {
    let server = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "7")
                .set_body_string("slow down"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"fine": true})))
        .mount(&server)
        .await;

    let (client, slept) = recording();
    let body = client
        .get(&format!("{}/x", server.uri()), &[])
        .await
        .unwrap();
    assert_eq!(body, json!({"fine": true}));
    assert_eq!(*slept.lock().unwrap(), vec![Duration::from_secs(7)]);
}

#[tokio::test]
async fn caps_a_long_retry_after_at_thirty_seconds_and_backs_off_on_its_own_otherwise() {
    let server = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "600"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let (client, slept) = recording();
    let error = client
        .get(&format!("{}/x", server.uri()), &[])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("answered 502"), "{error}");
    assert_eq!(
        *slept.lock().unwrap(),
        vec![Duration::from_secs(30), Duration::from_millis(1_000)]
    );
}

#[tokio::test]
async fn does_not_retry_a_404_which_would_only_spend_the_budget_a_429_needs() {
    let server = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
        .expect(1)
        .mount(&server)
        .await;

    let (client, slept) = recording();
    let error = client
        .get(&format!("{}/x", server.uri()), &[])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("404"));
    assert!(error.to_string().contains("nope"));
    assert!(slept.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sends_a_user_agent_because_the_cdns_refuse_a_request_without_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("user-agent", USER_AGENT))
        .and(header("x-extra", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let headers = [("x-extra".to_owned(), "1".to_owned())];
    JsonClient::new()
        .get(&server.uri(), &headers)
        .await
        .unwrap();
}

#[tokio::test]
async fn gives_up_on_a_provider_that_never_answers() {
    let server = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let (client, _) = recording();
    let client = client
        .with_timeout(Duration::from_millis(50))
        .with_retries(0);
    let error = client
        .get(&format!("{}/x", server.uri()), &[])
        .await
        .unwrap_err();
    assert!(matches!(error, MarketError::Timeout { .. }), "{error}");
}

#[tokio::test]
async fn refuses_a_body_that_is_not_json_such_as_a_cloudflare_interstitial() {
    let server = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>Just a moment...</html>"))
        .mount(&server)
        .await;
    assert!(
        JsonClient::new()
            .get(&format!("{}/x", server.uri()), &[])
            .await
            .is_err()
    );
}

#[test]
fn reads_numbers_however_the_api_spelled_them() {
    assert_eq!(num(Some(&json!(1.5))), Some(1.5));
    assert_eq!(num(Some(&json!("104.47"))), Some(104.47));
    assert_eq!(num(Some(&json!(" 7 "))), Some(7.0));
    assert_eq!(num(Some(&json!(""))), None);
    assert_eq!(num(Some(&json!("abc"))), None);
    assert_eq!(num(Some(&json!("Infinity"))), None);
    assert_eq!(num(Some(&json!(null))), None);
    assert_eq!(num(Some(&json!(true))), None);
    assert_eq!(num(None), None);
}
